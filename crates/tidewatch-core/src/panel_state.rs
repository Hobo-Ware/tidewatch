//! One tab's panel as a fold over its events (pure): `apply` is the only mutation and reads event
//! time only; `view` weighs the report against the facts and ages every block.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::end_block::{self, EndBlock};
use crate::job::{self, JobBoard};
use crate::link_board::LinkBoard;
use crate::panel_event::{AskKind, Event, Fact, GitFacts, PrFacts, PrWord, Source, StepBy};
use crate::panel_limits::limit;
use crate::panel_rules::{Live, ReportDigest};
use crate::panel_view::{Age, Ages, Asking, PanelView, Quiet, ReportMark, Revs, Tone, Turn, Why};
use crate::progress::State;
use crate::status::{
    AgentStatus, HookEvent, HookKind, REST_AFTER_SECS, Reason, StatusTracker, at_rest,
};

/// A transcript line is stamped by the CLI's clock and the hook that matches it by ours.
const CLOCK_SLACK_MS: i64 = 2_000;
const REST_AFTER_MS: i64 = (REST_AFTER_SECS as i64) * 1000;
const MAX_CHECKS: usize = 32;
const MAX_MARKS: usize = 64;
const MAX_ANSWERS: usize = 16;

fn secs(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ReportSeen {
    saved_ms: i64,
    digest: ReportDigest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSeen {
    pub name: String,
    pub passed: bool,
    pub at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkSeen {
    pub step: String,
    pub by: StepBy,
    pub at_ms: i64,
}

/// A question the owner answered since the report was saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerSeen {
    pub question: String,
    pub answer: String,
    pub at_ms: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Since {
    hook_tools: u32,
    transcript_tools: u32,
    other: u32,
}

impl Since {
    fn total(self) -> u32 {
        self.hook_tools.max(self.transcript_tools) + self.other
    }
}

/// What one `apply` did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The event altered what the panel shows at the moment it was folded.
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelState {
    tracker: StatusTracker,
    clock_ms: i64,
    shown_seq: u64,
    turn: Turn,
    said: Option<String>,
    last_message: Option<String>,
    end_block: Option<EndBlock>,
    asking: Option<Asking>,
    report: Option<ReportSeen>,
    answered_after_save: bool,
    jobs: JobBoard,
    links: LinkBoard,
    git: Option<GitFacts>,
    pr: Option<PrFacts>,
    checks: Vec<CheckSeen>,
    marks: Vec<MarkSeen>,
    #[serde(default)]
    answered: Vec<AnswerSeen>,
    last_by_source: BTreeMap<Source, i64>,
    since: Since,
    revs: Revs,
    exited_ms: Option<i64>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl PanelState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracker: StatusTracker::agent(0.0),
            clock_ms: 0,
            shown_seq: 0,
            turn: Turn::default(),
            said: None,
            last_message: None,
            end_block: None,
            asking: None,
            report: None,
            answered_after_save: false,
            jobs: JobBoard::default(),
            links: LinkBoard::default(),
            git: None,
            pr: None,
            checks: Vec::new(),
            marks: Vec::new(),
            answered: Vec::new(),
            last_by_source: BTreeMap::new(),
            since: Since::default(),
            revs: Revs::default(),
            exited_ms: None,
        }
    }

    #[must_use]
    pub fn shown_seq(&self) -> u64 {
        self.shown_seq
    }

    #[must_use]
    pub fn clock_ms(&self) -> i64 {
        self.clock_ms
    }

    #[must_use]
    pub fn turn(&self) -> Turn {
        self.turn
    }

    #[must_use]
    pub fn jobs(&self) -> &JobBoard {
        &self.jobs
    }

    #[must_use]
    pub fn links(&self) -> &LinkBoard {
        &self.links
    }

    #[must_use]
    pub fn git(&self) -> Option<&GitFacts> {
        self.git.as_ref()
    }

    #[must_use]
    pub fn pr(&self) -> Option<&PrFacts> {
        self.pr.as_ref()
    }

    #[must_use]
    pub fn checks(&self) -> &[CheckSeen] {
        &self.checks
    }

    #[must_use]
    pub fn marks(&self) -> &[MarkSeen] {
        &self.marks
    }

    /// The questions answered since the report was saved, oldest first.
    #[must_use]
    pub fn answered(&self) -> &[AnswerSeen] {
        &self.answered
    }

    #[must_use]
    pub fn last_message(&self) -> Option<&str> {
        self.last_message.as_deref()
    }

    /// Folds one event. An event at or before `shown_seq` was folded already and changes nothing.
    pub fn apply(&mut self, event: &Event) -> Applied {
        if self.is_folded(event) {
            return Applied::default();
        }
        let t = event.at_ms.max(self.clock_ms);
        let before = self.view(t);
        self.step(event, t);
        let after = self.view(t);
        Applied { changed: shape(before) != shape(after) }
    }

    /// The state after `events` in order, from nothing.
    #[must_use]
    pub fn replay<'a>(events: impl IntoIterator<Item = &'a Event>) -> Self {
        let mut state = Self::new();
        for event in events {
            if !state.is_folded(event) {
                let t = event.at_ms.max(state.clock_ms);
                state.step(event, t);
            }
        }
        state
    }

    fn is_folded(&self, event: &Event) -> bool {
        self.shown_seq != 0 && event.seq <= self.shown_seq
    }

    fn step(&mut self, event: &Event, t: i64) {
        if let Fact::Checkpoint(state) = &event.fact {
            *self = (**state).clone();
        } else {
            self.clock_ms = t;
            self.last_by_source.insert(event.source, t);
            self.fold(&event.fact, t);
        }
        self.shown_seq = event.seq;
    }

    fn fold(&mut self, fact: &Fact, t: i64) {
        let s = secs(t);
        match fact {
            Fact::Started { hooked } => {
                self.tracker.agent_started(s);
                self.tracker.set_hooked(*hooked);
                self.turn = Turn::default();
                self.asking = None;
                self.exited_ms = None;
                self.said = None;
                self.last_message = None;
                self.end_block = None;
            }
            Fact::Restart => self.tracker.hooks_lost(),
            Fact::Hook(hook) => self.fold_hook(hook, t),
            Fact::Turn { started, ts_ms } => self.fold_turn(*started, *ts_ms, t),
            Fact::Said { tail } => {
                self.tracker.on_transcript(s);
                self.said = Some(tail.clone());
                if !self.turn.running && self.last_message.is_none() {
                    self.keep_message(tail);
                }
            }
            Fact::ToolOpened { .. } => {
                self.tracker.on_transcript(s);
                self.since.transcript_tools += 1;
            }
            Fact::ToolClosed { .. } => self.tracker.on_transcript(s),
            Fact::AskOpened { summary, .. } => self.open_ask(AskKind::Question, summary, t),
            Fact::PermissionOpened { tool } => self.open_ask(AskKind::Permission, tool, t),
            Fact::AskClosed => self.close_ask(AskKind::Question, s),
            Fact::PermissionClosed => self.close_ask(AskKind::Permission, s),
            Fact::Process { alive, exit_code } => {
                if !alive {
                    self.tracker.on_exit(*exit_code);
                    self.exited_ms = Some(t);
                    self.close_turn(t);
                }
            }
            Fact::Terminal(signal) => self.tracker.on_terminal(*signal, s),
            Fact::Output => self.tracker.on_output(s),
            Fact::ReportSaved { saved_ms, digest } => {
                self.report = Some(ReportSeen { saved_ms: *saved_ms, digest: *digest });
                self.answered_after_save = false;
                self.since = Since::default();
                self.answered.clear();
                self.revs.report += 1;
            }
            Fact::Job(report) => self.fold_job(report, t),
            Fact::Link(link) => {
                if self.links.absorb(link) {
                    self.revs.links += 1;
                }
            }
            Fact::Git(git) => {
                let moved_head = self.git.as_ref().is_some_and(|known| known.head != git.head);
                if self.git.as_ref() != Some(git) {
                    self.revs.git += 1;
                }
                if moved_head {
                    self.since.other += 1;
                }
                self.git = Some(git.clone());
            }
            Fact::Pr(pr) => {
                if self.pr.as_ref() != Some(pr) {
                    self.revs.pr += 1;
                    self.since.other += 1;
                }
                self.pr = Some(pr.clone());
            }
            Fact::Check { name, passed } => self.fold_check(name, *passed, t),
            Fact::Answered { kind } => self.fold_answered(*kind, s),
            Fact::QuestionAnswered { question, answer } => {
                self.fold_question_answered(question, answer, t);
            }
            Fact::StepMark { step, by } => self.fold_mark(step, *by, t),
            Fact::Checkpoint(_) => {}
        }
    }

    fn fold_hook(&mut self, hook: &HookEvent, t: i64) {
        self.tracker.on_hook(hook, secs(t));
        match &hook.kind {
            HookKind::TurnStarted => self.begin_turn(t),
            HookKind::ToolStarted { .. } => self.since.hook_tools += 1,
            HookKind::TurnDone => {
                if let Some(message) = &hook.message {
                    self.keep_message(message);
                }
            }
            _ => {}
        }
        let open = self.tracker.turn_open();
        match (&hook.kind, open, self.turn.running) {
            (_, true, false) => self.open_turn(t),
            (HookKind::TurnDone | HookKind::TurnFailed(_) | HookKind::IdleWaiting, false, true) => {
                self.close_turn(t);
            }
            _ => {}
        }
    }

    fn fold_turn(&mut self, started: bool, ts_ms: Option<i64>, t: i64) {
        let s = secs(t);
        self.tracker.on_transcript(s);
        if started {
            let ended_already =
                ts_ms.zip(self.turn.ended_ms).is_some_and(|(written, ended)| written <= ended);
            if !self.turn.running && !ended_already {
                self.tracker.on_hook(&bare(HookKind::TurnStarted), s);
                self.begin_turn(t);
            }
        } else {
            let older = ts_ms
                .zip(self.turn.started_ms)
                .is_some_and(|(written, began)| written + CLOCK_SLACK_MS < began);
            if self.turn.running && !older {
                self.tracker.on_hook(&bare(HookKind::TurnDone), s);
                self.close_turn(t);
            }
        }
    }

    fn begin_turn(&mut self, t: i64) {
        self.turn.running = true;
        self.turn.started_ms = Some(t);
        self.end_block = None;
        self.last_message = None;
        self.said = None;
        self.asking = None;
    }

    fn open_turn(&mut self, t: i64) {
        self.turn.running = true;
        if self.turn.started_ms.is_none() {
            self.turn.started_ms = Some(t);
        }
    }

    fn close_turn(&mut self, t: i64) {
        if !self.turn.running {
            return;
        }
        self.turn.running = false;
        self.turn.ended_ms = Some(t);
        self.asking = None;
        if self.last_message.is_none()
            && let Some(said) = self.said.clone()
        {
            self.keep_message(&said);
        }
    }

    fn keep_message(&mut self, message: &str) {
        self.last_message = Some(message.to_owned());
        self.end_block = end_block::parse(message);
    }

    fn open_ask(&mut self, kind: AskKind, summary: &str, t: i64) {
        self.tracker.on_transcript(secs(t));
        self.asking = Some(Asking { kind, since_ms: t, summary: summary.to_owned() });
    }

    fn close_ask(&mut self, kind: AskKind, s: f64) {
        self.tracker.on_transcript(s);
        if self.asking.as_ref().is_some_and(|asking| asking.kind == kind) {
            self.asking = None;
        }
    }

    fn fold_answered(&mut self, kind: AskKind, s: f64) {
        if self.asking.as_ref().is_some_and(|asking| asking.kind == kind) {
            self.asking = None;
        }
        match kind {
            AskKind::Question => self.answered_after_save = true,
            AskKind::Permission => self.tracker.on_permission_answered(s),
        }
    }

    fn fold_question_answered(&mut self, question: &str, answer: &str, t: i64) {
        self.fold_answered(AskKind::Question, secs(t));
        let same = |seen: &AnswerSeen| same_question(&seen.question, question);
        if self.answered.iter().any(same) {
            return;
        }
        self.answered.push(AnswerSeen {
            question: question.trim().to_owned(),
            answer: answer.trim().to_owned(),
            at_ms: t,
        });
        if self.answered.len() > MAX_ANSWERS {
            self.answered.remove(0);
        }
        self.revs.marks += 1;
    }

    fn fold_job(&mut self, report: &job::Report, t: i64) {
        let count = |board: &JobBoard| {
            (board.jobs.len(), board.jobs.iter().filter(|job| job.state.is_finished()).count())
        };
        let before = count(&self.jobs);
        let reported = self.jobs.report(report, t);
        let agent = self.job_agent(t);
        let ticked = self.jobs.tick(t, agent);
        if reported || ticked {
            self.revs.jobs += 1;
        }
        if count(&self.jobs) != before {
            self.since.other += 1;
        }
    }

    fn fold_check(&mut self, name: &str, passed: bool, t: i64) {
        self.checks.retain(|check| check.name != name);
        self.checks.push(CheckSeen { name: name.to_owned(), passed, at_ms: t });
        if self.checks.len() > MAX_CHECKS {
            self.checks.remove(0);
        }
        self.revs.checks += 1;
        self.since.other += 1;
    }

    fn fold_mark(&mut self, step: &str, by: StepBy, t: i64) {
        if self.marks.iter().any(|mark| mark.step == step) {
            return;
        }
        self.marks.push(MarkSeen { step: step.to_owned(), by, at_ms: t });
        if self.marks.len() > MAX_MARKS {
            self.marks.remove(0);
        }
        self.revs.marks += 1;
    }

    fn job_agent(&self, now: i64) -> job::Agent {
        match self.tracker.status(secs(now)) {
            AgentStatus::Working => job::Agent::Working,
            AgentStatus::Done | AgentStatus::Failed(_) => job::Agent::Done,
            _ => job::Agent::Idle,
        }
    }

    fn status_at(&self, now: i64) -> (AgentStatus, bool) {
        let at = secs(now);
        let raw = self.tracker.status(at);
        let hook_asks = matches!(raw, AgentStatus::NeedsYou(Reason::Permission | Reason::Question));
        let status = match (&raw, &self.asking) {
            (AgentStatus::Working | AgentStatus::NeedsYou(Reason::Quiet), Some(asking)) => {
                AgentStatus::NeedsYou(asking.reason())
            }
            _ => raw,
        };
        let status = at_rest(&status, self.tracker.inputs().process.silent_for(at));
        (status, hook_asks || self.asking.is_some())
    }

    fn rested_ms(&self, status: &AgentStatus, now: i64) -> Option<i64> {
        if !matches!(status, AgentStatus::Done | AgentStatus::Failed(_)) {
            return None;
        }
        if let Some(exited) = self.exited_ms {
            return Some(exited);
        }
        if !self.turn.running {
            return self.turn.ended_ms;
        }
        let silent = (self.tracker.inputs().process.silent_for(secs(now)) * 1000.0) as i64;
        Some(now - silent + REST_AFTER_MS)
    }

    fn standing(
        &self,
        status: &AgentStatus,
        asked: bool,
        now: i64,
    ) -> (Option<State>, Option<ReportMark>) {
        let Some(report) = &self.report else { return (None, None) };
        let digest = &report.digest;
        let saved = Some(report.saved_ms);
        let live = Live {
            asking: asked,
            turn_started_ms: self.turn.started_ms,
            rested_at_ms: self.rested_ms(status, now),
        };
        if digest.is_outdated(saved, live) {
            let why = match digest.declared {
                Some(State::NeedsYou) => Why::NeedsYouNoQuestion,
                Some(State::Waiting) => Why::WaitingOnNothing,
                _ => Why::WorkingButTurnEnded,
            };
            return (Some(State::Done), Some(ReportMark::OutOfDate(why)));
        }
        if digest.declared == Some(State::NeedsYou) && !digest.asks && !asked {
            let state = if self.turn.running { State::Working } else { State::Done };
            return (Some(state), Some(ReportMark::OutOfDate(Why::NeedsYouNoQuestion)));
        }
        let answered = self.answered_after_save && (digest.awaits_owner || digest.asks);
        let stale = answered || digest.is_stale(saved, live);
        let working = *status == AgentStatus::Working;
        let older = self.turn.started_ms.is_some_and(|started| started > report.saved_ms);
        let state = digest.standing(saved, live, (stale, working));
        let mark = if answered {
            ReportMark::OutOfDate(Why::QuestionAnswered)
        } else if stale {
            ReportMark::OutOfDate(Why::OlderThanTurn)
        } else if working && older && digest.shown != State::Working {
            let why = if digest.shown == State::Done {
                Why::DoneButTurnRunning
            } else {
                Why::OlderThanTurn
            };
            ReportMark::OutOfDate(why)
        } else {
            ReportMark::Fresh
        };
        (Some(state), Some(mark))
    }

    /// The state the report stands for and its mark at `now_ms`: the `reported` and `report`
    /// fields of `view`, without building the rest of it.
    #[must_use]
    pub fn standing_at(&self, now_ms: i64) -> (Option<State>, Option<ReportMark>) {
        let now = now_ms.max(self.clock_ms);
        let (status, asked) = self.status_at(now);
        self.standing(&status, asked, now)
    }

    #[must_use]
    pub fn end_block(&self) -> Option<&EndBlock> {
        self.end_block.as_ref()
    }

    /// What the panel shows at `now_ms`. A `now_ms` before the last event folded reads as that event.
    #[must_use]
    pub fn view(&self, now_ms: i64) -> PanelView {
        let now = now_ms.max(self.clock_ms);
        let (status, asked) = self.status_at(now);
        let (reported, report) = self.standing(&status, asked, now);
        PanelView {
            tone: Tone::shown(&status, reported),
            ages: self.ages(),
            quiet: self.quiet(now),
            since: self.since.total(),
            answers: u32::try_from(self.answered.len()).unwrap_or(u32::MAX),
            turn: self.turn,
            asking: self.asking.clone(),
            end_block: self.end_block.clone(),
            revs: self.revs,
            shown_seq: self.shown_seq,
            status,
            reported,
            report,
        }
    }

    fn ages(&self) -> Ages {
        let age = |at: Option<i64>| at.map(|at_ms| Age { at_ms });
        let spoke = |source| self.last_by_source.get(&source).copied();
        let turn_at = if self.turn.running { self.turn.started_ms } else { self.turn.ended_ms };
        Ages {
            report: age(self.report.as_ref().map(|report| report.saved_ms)),
            turn: age(turn_at),
            jobs: age(spoke(Source::Job)),
            links: age(spoke(Source::Link)),
            git: age(spoke(Source::Git).filter(|_| self.git.is_some())),
            pr: age(spoke(Source::Pr).filter(|_| self.pr.is_some())),
            checks: age(self.checks.iter().map(|check| check.at_ms).max()),
        }
    }

    fn quiet(&self, now: i64) -> Vec<Quiet> {
        let spoke = |source| self.last_by_source.get(&source).copied();
        let past = |source, since: i64| {
            limit(source).quiet_after_ms.is_some_and(|after| now - since > after)
        };
        let mut quiet = Vec::new();
        if self.turn.running
            && let Some(started) = self.turn.started_ms
        {
            let heard = spoke(Source::Hook);
            if heard.is_none_or(|at| at < started) && past(Source::Hook, started) {
                quiet.push(Quiet { source: Source::Hook, since_ms: heard.unwrap_or(started) });
            }
            if let Some(heard) = spoke(Source::Transcript)
                && past(Source::Transcript, heard)
            {
                quiet.push(Quiet { source: Source::Transcript, since_ms: heard });
            }
        }
        if self.jobs.has_running()
            && let Some(heard) = spoke(Source::Job)
            && past(Source::Job, heard)
        {
            quiet.push(Quiet { source: Source::Job, since_ms: heard });
        }
        if !self.turn.running
            && let Some(ended) = self.turn.ended_ms
            && let Some(heard) = spoke(Source::Git)
            && heard < ended
            && past(Source::Git, ended)
        {
            quiet.push(Quiet { source: Source::Git, since_ms: heard });
        }
        if self.pr.as_ref().is_some_and(|pr| pr.state == PrWord::Open)
            && let Some(heard) = spoke(Source::Pr)
            && past(Source::Pr, heard)
        {
            quiet.push(Quiet { source: Source::Pr, since_ms: heard });
        }
        quiet
    }
}

/// The view without what changes on every event: its number and the exact times behind the ages.
fn shape(mut view: PanelView) -> PanelView {
    view.shown_seq = 0;
    for age in [
        &mut view.ages.report,
        &mut view.ages.turn,
        &mut view.ages.jobs,
        &mut view.ages.links,
        &mut view.ages.git,
        &mut view.ages.pr,
        &mut view.ages.checks,
    ]
    .into_iter()
    .flatten()
    {
        age.at_ms = 0;
    }
    view
}

fn same_question(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn bare(kind: HookKind) -> HookEvent {
    HookEvent { kind, session_id: None, transcript_path: None, message: None }
}

#[cfg(test)]
mod tests;
