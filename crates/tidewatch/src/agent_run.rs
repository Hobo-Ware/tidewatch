//! The live state of one agent tab: the status tracker fed by hooks and terminal signals, the last
//! thing the agent reported over MCP, and the status change each frame produces for notifications.
//! Nothing here touches a pty or the UI, so every rule is a plain unit test; `TermSnapshot::of`
//! is the one place that reads the terminal.

use std::path::PathBuf;

use stdusk_core::progress::Progress;
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::end_block::{self, EndBlock};
use tidewatch_core::events::TabEvent;
use tidewatch_core::resume_mode::exit_is_abnormal;
use tidewatch_core::status::{
    AgentStatus, HookKind, ProgressSignal, Reason, StatusTracker, TerminalSignal,
};

/// A finding the agent reported with `report_finding`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) blocking: bool,
    pub(crate) text: String,
}

/// What the agent last said through the Tidewatch MCP tools. The workflow UI reads this later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) stage_done: Option<String>,
    pub(crate) finding: Option<Finding>,
}

/// What one frame saw on the terminal side of an agent tab. One-shot flags (`bell`, `activity`) are
/// consumed by reading them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TermSnapshot {
    pub(crate) progress: Progress,
    pub(crate) bell: bool,
    pub(crate) activity: bool,
    pub(crate) title: Option<String>,
    pub(crate) exit_code: Option<i32>,
}

impl TermSnapshot {
    pub(crate) fn of(term: &PtyTerm) -> Self {
        Self {
            progress: term.progress(),
            bell: term.take_bell(),
            activity: term.take_activity(),
            title: term.title_osc(),
            exit_code: term.exited().map(|exit| exit.code),
        }
    }
}

pub(crate) fn progress_signal(progress: Progress) -> ProgressSignal {
    match progress {
        Progress::None => ProgressSignal::Off,
        Progress::Normal(_) | Progress::Indeterminate => ProgressSignal::Working,
        Progress::Error(_) => ProgressSignal::Error,
        Progress::Paused(_) => ProgressSignal::Paused,
    }
}

/// A real percentage the agent reported with OSC 9;4, for the tab's bar.
pub(crate) fn progress_fraction(progress: Progress) -> Option<f32> {
    match progress {
        Progress::Normal(pct) if pct > 0 => Some(f32::from(pct.min(100)) / 100.0),
        _ => None,
    }
}

/// A status that just changed, for notifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) from: AgentStatus,
    pub(crate) to: AgentStatus,
}

pub(crate) struct AgentRun {
    pub(crate) agent_id: String,
    pub(crate) notes_file: Option<PathBuf>,
    pub(crate) report: Report,
    /// The end-block card on the tab is open.
    pub(crate) end_open: bool,
    tracker: StatusTracker,
    status: AgentStatus,
    started_at: f64,
    finished_at: Option<f64>,
    title_seen: Option<String>,
    goal: Option<String>,
    detail: Option<String>,
    end_block: Option<EndBlock>,
    failed: Option<String>,
    held: Option<AgentStatus>,
    /// The first question of an `ask_user` call the user has not answered: the agent is blocked on it.
    asking: Option<String>,
    exited_abnormally: bool,
}

impl AgentRun {
    /// A tab whose worktree is still being made.
    pub(crate) fn setting_up(agent_id: String, now: f64) -> Self {
        Self {
            agent_id,
            notes_file: None,
            report: Report::default(),
            end_open: false,
            tracker: StatusTracker::setting_up(now),
            status: AgentStatus::SettingUp,
            started_at: now,
            finished_at: None,
            title_seen: None,
            goal: None,
            detail: None,
            end_block: None,
            failed: None,
            held: None,
            asking: None,
            exited_abnormally: false,
        }
    }

    /// An `ask_user` call waits for the user (`Some(its first question)`) or does not any more.
    /// While it waits, a working agent shows as needing the user.
    pub(crate) fn set_asking(&mut self, question: Option<String>) {
        if question.is_some() {
            self.detail.clone_from(&question);
        }
        self.asking = question;
    }

    /// A workflow run decides what this tab shows while it holds the next move (tests running, a
    /// question for the user, the end of the run). `None` gives the status back to the agent.
    pub(crate) fn hold(&mut self, status: Option<AgentStatus>, detail: Option<String>) {
        if self.held != status {
            self.held = status;
            if detail.is_some() {
                self.detail = detail;
            }
        }
    }

    /// The agent process starts now.
    pub(crate) fn started(&mut self, notes_file: Option<PathBuf>, now: f64) {
        self.notes_file = notes_file;
        self.tracker.agent_started(now);
        self.started_at = now;
    }

    /// The agent process resumed a saved session: it waits at its prompt, which reads as done.
    pub(crate) fn resumed(&mut self, now: f64) {
        self.tracker.agent_resumed(now);
    }

    /// The tab never got an agent: it shows this error as its status.
    pub(crate) fn fail(&mut self, message: String) {
        self.failed = Some(message);
    }

    pub(crate) fn exited_abnormally(&self) -> bool {
        self.exited_abnormally
    }

    pub(crate) fn status(&self) -> &AgentStatus {
        &self.status
    }

    /// The agent's last message, the prompt it waits on, or its error: what a notification or the
    /// Overview shows next to the status.
    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The Done / Needs you / Next block of the agent's last message (D13), when it had one. Cleared
    /// when the next turn starts.
    pub(crate) fn end_block(&self) -> Option<&EndBlock> {
        self.end_block.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn tracker(&self) -> &StatusTracker {
        &self.tracker
    }

    /// Seconds since the agent started, frozen while it is done or failed.
    pub(crate) fn elapsed(&self, now: f64) -> f64 {
        (self.finished_at.unwrap_or(now) - self.started_at).max(0.0)
    }

    pub(crate) fn on_event(&mut self, event: TabEvent, now: f64) {
        match event {
            TabEvent::Hook(hook) => {
                match (&hook.kind, &hook.message) {
                    (HookKind::TurnStarted, _) => self.end_block = None,
                    (HookKind::TurnDone, Some(message)) => {
                        self.end_block = end_block::parse(message);
                    }
                    _ => {}
                }
                match (&hook.kind, &hook.message) {
                    (HookKind::TurnFailed(reason), _) => self.detail = Some(reason.clone()),
                    (HookKind::IdleWaiting, _) => {}
                    (_, Some(message)) => self.detail = Some(message.clone()),
                    _ => {}
                }
                self.tracker.on_hook(&hook, now);
            }
            TabEvent::StageDone { summary } => self.report.stage_done = Some(summary),
            TabEvent::Finding { file, line, blocking, text } => {
                self.report.finding = Some(Finding { file, line, blocking, text });
            }
            TabEvent::NoteReply { .. }
            | TabEvent::Proposal { .. }
            | TabEvent::Progress { .. }
            | TabEvent::Job { .. }
            | TabEvent::Avatar { .. }
            | TabEvent::MeeseeksDone { .. }
            | TabEvent::Ask { .. } => {}
        }
    }

    pub(crate) fn terminal_title(&self) -> Option<&str> {
        self.title_seen.as_deref()
    }

    pub(crate) fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
    }

    pub(crate) fn set_goal(&mut self, goal: Option<&str>) {
        if self.goal.as_deref() != goal {
            self.goal = goal.map(str::to_owned);
        }
    }

    /// Feeds what the terminal showed this frame into the tracker.
    pub(crate) fn feed(&mut self, snapshot: TermSnapshot, now: f64) {
        self.tracker.on_terminal(TerminalSignal::Progress(progress_signal(snapshot.progress)), now);
        if snapshot.title != self.title_seen {
            self.title_seen = snapshot.title;
            self.tracker.on_terminal(TerminalSignal::TitleChanged, now);
        }
        if snapshot.bell {
            self.tracker.on_terminal(TerminalSignal::Bell, now);
        }
        if snapshot.activity {
            self.tracker.on_output(now);
        }
        if let Some(code) = snapshot.exit_code
            && self.tracker.inputs().process.alive
        {
            self.tracker.on_exit(Some(code));
            self.exited_abnormally = exit_is_abnormal(code);
        }
    }

    /// Recomputes the status at `now` and reports a change.
    pub(crate) fn refresh(&mut self, now: f64) -> Option<Change> {
        let next = match (&self.failed, &self.held) {
            (Some(message), _) => AgentStatus::Failed(message.clone()),
            (None, Some(held)) => held.clone(),
            (None, None) => match self.tracker.status(now) {
                AgentStatus::Working if self.asking.is_some() => {
                    AgentStatus::NeedsYou(Reason::Question)
                }
                tracked => tracked,
            },
        };
        if next == self.status {
            return None;
        }
        let from = std::mem::replace(&mut self.status, next.clone());
        let finished = matches!(next, AgentStatus::Done | AgentStatus::Failed(_));
        self.finished_at = finished.then_some(now);
        Some(Change { from, to: next })
    }

    /// When the status next changes with no new input, in the egui clock.
    pub(crate) fn next_change_at(&self, now: f64) -> Option<f64> {
        self.tracker.next_change_at(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::status::{HookEvent, Reason};

    fn hook(kind: HookKind, message: Option<&str>) -> TabEvent {
        TabEvent::Hook(HookEvent {
            kind,
            session_id: None,
            transcript_path: None,
            message: message.map(str::to_owned),
        })
    }

    fn running(now: f64) -> AgentRun {
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        run.started(None, now);
        run
    }

    #[test]
    fn a_tab_starts_setting_up_then_works() {
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        assert_eq!(run.refresh(0.0), None);
        assert_eq!(run.status(), &AgentStatus::SettingUp);
        run.started(None, 3.0);
        let change = run.refresh(3.5).unwrap();
        assert_eq!(change.from, AgentStatus::SettingUp);
        assert_eq!(change.to, AgentStatus::Working);
    }

    #[test]
    fn a_permission_hook_turns_the_tab_amber_in_the_same_frame() {
        let mut run = running(0.0);
        run.refresh(1.0);
        run.on_event(hook(HookKind::TurnStarted, None), 2.0);
        run.on_event(hook(HookKind::PermissionNeeded, Some("Bash: touch hello.txt")), 3.0);
        let change = run.refresh(3.0).unwrap();
        assert_eq!(change.to, AgentStatus::NeedsYou(Reason::Permission));
        assert_eq!(run.detail(), Some("Bash: touch hello.txt"));
    }

    #[test]
    fn a_stop_hook_makes_it_done_and_freezes_the_elapsed_time() {
        let mut run = running(10.0);
        run.on_event(hook(HookKind::TurnStarted, None), 11.0);
        run.refresh(11.0);
        assert_eq!(run.elapsed(20.0), 10.0);
        run.on_event(hook(HookKind::TurnDone, Some("All done")), 25.0);
        assert_eq!(run.refresh(25.0).unwrap().to, AgentStatus::Done);
        assert_eq!(run.elapsed(500.0), 15.0);
        assert_eq!(run.detail(), Some("All done"));
        run.on_event(hook(HookKind::TurnStarted, None), 600.0);
        run.refresh(600.0);
        assert_eq!(run.elapsed(610.0), 600.0);
    }

    #[test]
    fn the_end_block_of_the_last_message_is_kept_until_the_next_turn_starts() {
        let mut run = running(0.0);
        run.on_event(hook(HookKind::TurnStarted, None), 1.0);
        let message =
            "Added it.\n\n**Done**\n- the helper\n\n**Needs you**\n- nothing\n\n**Next**\n- review";
        run.on_event(hook(HookKind::TurnDone, Some(message)), 2.0);
        let block = run.end_block().unwrap();
        assert_eq!(block.done, ["the helper"]);
        assert_eq!(block.next, ["review"]);
        run.on_event(hook(HookKind::TurnStarted, None), 3.0);
        assert_eq!(run.end_block(), None);
    }

    #[test]
    fn a_stop_without_text_or_without_the_block_leaves_no_card() {
        let mut run = running(0.0);
        run.on_event(hook(HookKind::TurnDone, None), 1.0);
        assert_eq!(run.end_block(), None);
        run.on_event(hook(HookKind::TurnDone, Some("Just prose, no block.")), 2.0);
        assert_eq!(run.end_block(), None);
    }

    #[test]
    fn a_failed_turn_keeps_its_reason_as_the_detail() {
        let mut run = running(0.0);
        run.on_event(hook(HookKind::TurnFailed("rate_limit".into()), None), 5.0);
        let change = run.refresh(5.0).unwrap();
        assert_eq!(change.to, AgentStatus::Failed("rate_limit".into()));
        assert_eq!(run.detail(), Some("rate_limit"));
    }

    #[test]
    fn terminal_signals_decide_when_no_hook_arrives() {
        let mut run = running(0.0);
        run.refresh(0.5);
        run.feed(
            TermSnapshot {
                progress: Progress::Indeterminate,
                activity: true,
                ..Default::default()
            },
            1.0,
        );
        assert_eq!(run.refresh(2.0), None);
        assert_eq!(run.status(), &AgentStatus::Working);
        run.feed(TermSnapshot { bell: true, activity: true, ..Default::default() }, 3.0);
        run.feed(TermSnapshot::default(), 3.1);
        assert_eq!(run.refresh(3.2).unwrap().to, AgentStatus::NeedsYou(Reason::Attention));
    }

    #[test]
    fn only_an_exit_that_is_not_a_plain_quit_marks_the_run_as_ended_abnormally() {
        for (code, want) in [(0, false), (130, false), (143, false), (1, true), (137, true)] {
            let mut run = running(0.0);
            assert!(!run.exited_abnormally());
            run.feed(TermSnapshot { exit_code: Some(code), ..Default::default() }, 4.0);
            assert_eq!(run.exited_abnormally(), want, "exit {code}");
        }
    }

    #[test]
    fn a_run_that_is_still_going_has_not_ended_abnormally() {
        let mut run = running(0.0);
        run.feed(TermSnapshot::default(), 4.0);
        assert!(!run.exited_abnormally());
    }

    #[test]
    fn a_new_title_counts_once_and_the_same_title_does_not() {
        let mut run = running(0.0);
        let titled = || TermSnapshot { title: Some("Fixing tests".into()), ..Default::default() };
        run.feed(titled(), 5.0);
        assert_eq!(run.tracker().inputs().terminal.title_changed_at, Some(5.0));
        run.feed(titled(), 6.0);
        assert_eq!(run.tracker().inputs().terminal.title_changed_at, Some(5.0));
        run.feed(TermSnapshot::default(), 7.0);
        assert_eq!(run.tracker().inputs().terminal.title_changed_at, Some(7.0));
    }

    #[test]
    fn the_exit_code_ends_the_run_once() {
        let mut run = running(0.0);
        run.feed(TermSnapshot { exit_code: Some(2), ..Default::default() }, 4.0);
        assert_eq!(run.refresh(4.0).unwrap().to, AgentStatus::Failed("exited with code 2".into()));
        run.feed(TermSnapshot { exit_code: Some(0), ..Default::default() }, 5.0);
        assert_eq!(run.refresh(5.0), None);

        let mut clean = running(0.0);
        clean.feed(TermSnapshot { exit_code: Some(0), ..Default::default() }, 4.0);
        assert_eq!(clean.refresh(4.0).unwrap().to, AgentStatus::Done);
    }

    #[test]
    fn an_agent_that_never_started_shows_its_error() {
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        run.fail("disk full".into());
        assert_eq!(run.refresh(1.0).unwrap().to, AgentStatus::Failed("disk full".into()));
        assert_eq!(run.refresh(2.0), None);
    }

    #[test]
    fn a_working_agent_blocked_on_a_question_shows_as_needing_the_user() {
        let mut run = running(0.0);
        run.on_event(hook(HookKind::TurnStarted, None), 1.0);
        run.refresh(1.0);
        assert_eq!(run.status(), &AgentStatus::Working);
        run.set_asking(Some("Where should it live?".into()));
        assert_eq!(run.refresh(1.5).unwrap().to, AgentStatus::NeedsYou(Reason::Question));
        assert_eq!(run.detail(), Some("Where should it live?"));
        run.set_asking(None);
        assert_eq!(run.refresh(2.0).unwrap().to, AgentStatus::Working);
    }

    #[test]
    fn mcp_reports_are_kept_for_the_workflow_ui() {
        let mut run = running(0.0);
        run.on_event(TabEvent::StageDone { summary: "tests added".into() }, 1.0);
        run.on_event(
            TabEvent::Finding {
                file: "src/a.rs".into(),
                line: 4,
                blocking: true,
                text: "x".into(),
            },
            2.0,
        );
        run.on_event(
            TabEvent::Finding {
                file: "src/b.rs".into(),
                line: 9,
                blocking: false,
                text: "y".into(),
            },
            3.0,
        );
        assert_eq!(run.report.stage_done.as_deref(), Some("tests added"));
        assert_eq!(
            run.report.finding,
            Some(Finding { file: "src/b.rs".into(), line: 9, blocking: false, text: "y".into() })
        );
    }

    #[test]
    fn progress_maps_onto_the_status_layer_and_the_bar() {
        let cases = [
            (Progress::None, ProgressSignal::Off, None),
            (Progress::Normal(40), ProgressSignal::Working, Some(0.4)),
            (Progress::Normal(0), ProgressSignal::Working, None),
            (Progress::Indeterminate, ProgressSignal::Working, None),
            (Progress::Error(10), ProgressSignal::Error, None),
            (Progress::Paused(10), ProgressSignal::Paused, None),
        ];
        for (progress, signal, fraction) in cases {
            assert_eq!(progress_signal(progress), signal, "{progress:?}");
            assert_eq!(progress_fraction(progress), fraction, "{progress:?}");
        }
    }

    #[test]
    fn an_idle_notice_does_not_replace_what_the_agent_said() {
        let mut run = running(0.0);
        run.on_event(hook(HookKind::TurnDone, Some("Created hello.txt")), 5.0);
        run.on_event(hook(HookKind::IdleWaiting, Some("Claude is waiting for your input")), 65.0);
        run.refresh(65.0);
        assert_eq!(run.status(), &AgentStatus::Done);
        assert_eq!(run.detail(), Some("Created hello.txt"));
    }

    #[test]
    fn a_held_status_wins_over_the_agents_until_it_is_given_back() {
        let mut run = running(0.0);
        run.refresh(1.0);
        run.on_event(hook(HookKind::TurnStarted, None), 2.0);
        run.refresh(2.0);
        assert_eq!(run.status(), &AgentStatus::Working);
        run.hold(Some(AgentStatus::NeedsYou(Reason::Input)), Some("Review 2/4: asks".into()));
        let change = run.refresh(3.0).unwrap();
        assert_eq!(change.to, AgentStatus::NeedsYou(Reason::Input));
        assert_eq!(run.detail(), Some("Review 2/4: asks"));
        run.on_event(hook(HookKind::TurnDone, Some("bye")), 4.0);
        assert_eq!(run.refresh(4.0), None, "the agent finishing changes nothing while held");
        run.hold(None, None);
        assert_eq!(run.refresh(5.0).unwrap().to, AgentStatus::Done);
    }

    #[test]
    fn holding_the_same_status_again_keeps_the_detail_the_agent_gave_since() {
        let mut run = running(0.0);
        run.hold(Some(AgentStatus::Working), Some("Running the tests".into()));
        run.on_event(hook(HookKind::TurnDone, Some("agent words")), 1.0);
        run.hold(Some(AgentStatus::Working), Some("Running the tests".into()));
        assert_eq!(run.detail(), Some("agent words"));
    }

    #[test]
    fn the_terminal_title_and_the_ledger_goal_are_kept_for_the_tab_label() {
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        assert_eq!((run.terminal_title(), run.goal()), (None, None));
        run.feed(TermSnapshot { title: Some("Fixing tests".into()), ..Default::default() }, 1.0);
        run.set_goal(Some("Ship it"));
        assert_eq!((run.terminal_title(), run.goal()), (Some("Fixing tests"), Some("Ship it")));
        run.set_goal(None);
        assert_eq!(run.goal(), None);
    }
}
