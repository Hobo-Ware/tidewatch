//! What one agent tab does about its pull request (PLAN.md, D4 follow-up), as plain data: when to
//! look at GitHub again, how a look turns into cards, whether a message goes to the agent now, later
//! or only when the user presses the button, and the cap on Auto. Nothing here starts a thread or
//! touches a terminal, so every rule is a unit test; `pr_glue` carries the decisions out.

use std::path::{Path, PathBuf};

use tidewatch_core::github_hook::Hit;
use tidewatch_core::pr_cards::{Cards, send_cap};
use tidewatch_core::pr_gh::Polled;
use tidewatch_core::pr_prompt::{self, Message, short_step};
use tidewatch_core::pr_status::{Body, Seen, State, Summary, Tone as PrTone, Via};
use tidewatch_core::prefs::{GithubPrefs, PrMode};
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::GhError;
use tidewatch_core::worktree_cleanup::Report;

use crate::merge_cleanup::{Busy, Cleanup};
use crate::note_send::{HOLD_SECS, gate, gate_reason};
use crate::status_view::Tone;

/// How soon to look again while checks run.
pub(crate) const FAST_SECS: f64 = 20.0;
/// How soon to look again otherwise.
pub(crate) const SLOW_SECS: f64 = 120.0;
pub(crate) const LIVE_FAST_SECS: f64 = 120.0;
pub(crate) const LIVE_SLOW_SECS: f64 = 600.0;
const NEWS_LOOK_SECS: f64 = 2.0;
/// How soon to look after the user re-ran the checks.
const RERUN_LOOK_SECS: f64 = 5.0;
/// How soon to look at a merged worktree again while its agent still works in it.
const RECHECK_SECS: f64 = 15.0;
pub(crate) const STAYS: &str = "The agent still needs the worktree, so it stays";

/// Why the status is not being read, in the words the timeline uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pause {
    SignedOut,
    NoGh,
    NoRemote,
    Failed(String),
}

impl Pause {
    pub(crate) fn of(error: GhError) -> Self {
        match error {
            GhError::NotAuthenticated => Self::SignedOut,
            GhError::Missing => Self::NoGh,
            GhError::NoGithubRemote => Self::NoRemote,
            GhError::Failed(message) | GhError::Stopped(message) => {
                Self::Failed(message.lines().next().unwrap_or_default().trim().to_owned())
            }
        }
    }

    /// `gh is not signed in`, after `PR status paused:`.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::SignedOut => "gh is not signed in".to_owned(),
            Self::NoGh => "gh is not installed".to_owned(),
            Self::NoRemote => "this repository has no GitHub remote".to_owned(),
            Self::Failed(message) => format!("gh failed: {message}"),
        }
    }

    /// The command that fixes it, shown on the right of the row.
    pub(crate) fn fix(&self) -> Option<&'static str> {
        match self {
            Self::SignedOut => Some("gh auth login"),
            Self::NoGh => Some("brew install gh"),
            Self::NoRemote | Self::Failed(_) => None,
        }
    }
}

/// A message Auto would send as soon as the agent is idle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    pub(crate) id: u64,
    pub(crate) message: Message,
}

/// A desktop notification, before the tab's name goes in front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) tone: Tone,
    pub(crate) what: String,
    pub(crate) detail: String,
}

/// What the watch asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    /// The next `Paste` is an Auto send, the `n`-th of `max` on this pull request.
    Told {
        n: u32,
        max: u32,
    },
    Paste(String),
    Toast(String),
    Notify(Notice),
    Save,
    /// The merged tab's worktree was checked once more and is safe: remove it and its branch.
    Remove,
}

/// What a frame knows of the tab, for the watch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) status: &'a AgentStatus,
    pub(crate) title: &'a str,
    /// The worktree.
    pub(crate) dir: &'a Path,
    /// The repository and the tab's name in its state folder.
    pub(crate) repo: &'a Path,
    pub(crate) name: &'a str,
    /// The pull request of the branch and its head, as the tab's lookup found them.
    pub(crate) pr: Option<(u64, &'a str)>,
    /// A YOLO agent or a Meeseeks (`autonomy::autonomous`): the watch acts on its own.
    pub(crate) autonomous: bool,
    /// Whether the agent still needs the worktree.
    pub(crate) busy: Busy,
    /// Whether GitHub tells Tidewatch about the repository now.
    pub(crate) live: bool,
    pub(crate) branch: &'a str,
}

/// Who answers the pull request's news on a tab, for the cards' buttons and notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handling {
    /// The owner: every card keeps its buttons.
    Owner,
    /// Auto answers the cards on a tab the owner drives: the cards are a record, but the commits
    /// it leaves are still the owner's to push.
    Answers,
    /// The agent, through Auto, on a tab that runs on its own: the cards are a record and it
    /// pushes its own commits.
    Agent,
    /// The agent, but the loop guard stopped the sends: the cards stay a record and one line says so.
    Paused,
}

/// Whether the commits left after a merge are to be counted at the next chance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recount {
    Idle,
    Due,
}

/// What a look needs to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) number: u64,
    pub(crate) prev: Option<Seen>,
}

/// How the watch hears about its pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Updates {
    Polling,
    Live,
}

/// Whether news arrived while a look was being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum News {
    None,
    Waiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Unloaded,
    Loading,
    Ready,
}

#[derive(Debug)]
pub(crate) struct Watch {
    pub(crate) cards: Cards,
    pub(crate) file: PathBuf,
    /// The worktree `gh` and `git` run in, and the tab's name, as the last frame saw them.
    pub(crate) dir: PathBuf,
    pub(crate) title: String,
    /// The repository and the branch of the tab, as the last frame saw them: what GitHub's news names.
    pub(crate) repo: PathBuf,
    pub(crate) branch: String,
    stage: Stage,
    due: f64,
    in_flight: bool,
    updates: Updates,
    news: News,
    ended: bool,
    pub(crate) pause: Option<Pause>,
    pub(crate) pending: Vec<Pending>,
    pub(crate) autonomous: bool,
    pub(crate) cleanup: Cleanup,
    pub(crate) busy: Busy,
    check_started: f64,
    hold_until: Option<f64>,
    next_pending: u64,
    status: AgentStatus,
    head: Option<String>,
    recount: Recount,
}

impl Watch {
    pub(crate) fn new(file: PathBuf) -> Self {
        Self {
            cards: Cards::default(),
            file,
            dir: PathBuf::new(),
            title: String::new(),
            repo: PathBuf::new(),
            branch: String::new(),
            stage: Stage::Unloaded,
            due: 0.0,
            in_flight: false,
            updates: Updates::Polling,
            news: News::None,
            ended: false,
            pause: None,
            pending: Vec::new(),
            autonomous: false,
            cleanup: Cleanup::default(),
            busy: Busy::default(),
            check_started: 0.0,
            hold_until: None,
            next_pending: 0,
            status: AgentStatus::SettingUp,
            head: None,
            recount: Recount::Idle,
        }
    }

    /// True once: the saved cards are still to be read.
    pub(crate) fn take_load(&mut self) -> bool {
        let first = self.stage == Stage::Unloaded;
        if first {
            self.stage = Stage::Loading;
        }
        first
    }

    pub(crate) fn loaded(&mut self, cards: Cards, now: f64) {
        self.ended = cards.seen.as_ref().is_some_and(|seen| seen.ended);
        self.cards = cards;
        self.cards.drop_noise();
        self.stage = Stage::Ready;
        self.due = now;
    }

    pub(crate) fn ready(&self) -> bool {
        self.stage == Stage::Ready
    }

    /// The tab's choices: its own once the owner set them, else the default, and for a tab that
    /// runs on its own Auto for the fix jobs whatever the default says. Its push tick is never
    /// forced: it stays what the tab or the default says.
    pub(crate) fn policy(&self, default: GithubPrefs) -> GithubPrefs {
        match self.cards.policy {
            Some(own) => own,
            None if self.autonomous => GithubPrefs { mode: PrMode::Auto, ..default },
            None => default,
        }
    }

    fn cap(&self) -> u32 {
        send_cap(self.autonomous)
    }

    /// The loop guard stopped a tab that runs on its own: it sends nothing more on this pull request.
    fn capped(&self, default: GithubPrefs) -> bool {
        self.autonomous
            && self.policy(default).mode == PrMode::Auto
            && self.cards.auto_sends >= self.cap()
    }

    pub(crate) fn handling(&self, default: GithubPrefs) -> Handling {
        if self.policy(default).mode != PrMode::Auto {
            Handling::Owner
        } else if !self.autonomous {
            if self.cards.auto_sends >= self.cap() { Handling::Owner } else { Handling::Answers }
        } else if self.capped(default) {
            Handling::Paused
        } else {
            Handling::Agent
        }
    }

    pub(crate) fn merged(&self) -> bool {
        self.ended && self.cards.ended_merged()
    }

    fn merged_for_good(&self, polled: &Polled) -> bool {
        self.summary().is_some_and(|summary| summary.state == State::Merged)
            && polled.summary.state != State::Merged
    }

    fn worth_a_look(&self) -> bool {
        !self.ended || self.closed() || self.cards.summary.is_none()
    }

    pub(crate) fn closed(&self) -> bool {
        self.summary().is_some_and(|summary| summary.state == State::Closed)
    }

    /// The merged pull request's head, once per idle moment, for counting the commits left after it.
    pub(crate) fn take_recount(&mut self) -> Option<String> {
        if !self.autonomous || !self.ready() || !self.merged() {
            return None;
        }
        let head = self.pr_head()?;
        (std::mem::replace(&mut self.recount, Recount::Idle) == Recount::Due).then_some(head)
    }

    /// The commits found beyond the merged head. True when the cards changed and should be saved.
    pub(crate) fn counted(&mut self, found: Option<usize>) -> bool {
        let Some(commits) = found else { return false };
        let changed = self.cards.after_merge != commits;
        self.cards.after_merge = commits;
        changed
    }

    /// The commit card whose commits a tab that runs on its own should push now.
    pub(crate) fn push_fix_due(&self, default: GithubPrefs) -> Option<u64> {
        if !matches!(self.handling(default), Handling::Agent | Handling::Paused) || self.closed() {
            return None;
        }
        let cards = self.cards.cards();
        (0..cards.len())
            .find(|&at| matches!(cards[at].body, Body::PushFix { .. }) && self.cards.is_current(at))
            .map(|at| cards[at].id)
    }

    /// The tab's lookup found a pull request: follow it, and look again soon when its head moved
    /// (a push just went out). True when the cards changed and should be saved.
    pub(crate) fn learn(&mut self, number: u64, head: &str, now: f64) -> bool {
        if !self.ready() {
            return false;
        }
        let mut changed = false;
        if self.cards.number != Some(number) {
            self.cards.follow(number);
            self.ended = false;
            self.due = now;
            changed = true;
        }
        if self.head.as_deref() != Some(head) {
            self.head = Some(head.to_owned());
            self.due = self.due.min(now);
        }
        changed
    }

    /// Per frame: the agent's status. True when it just became idle after a message was sent, which
    /// is the moment to see whether it committed something.
    pub(crate) fn observe(&mut self, status: &AgentStatus) -> bool {
        if *status != AgentStatus::Done {
            self.hold_until = None;
        }
        let became_idle = *status == AgentStatus::Done && self.status != AgentStatus::Done;
        if became_idle {
            self.recount = Recount::Due;
        }
        self.status = status.clone();
        became_idle && self.cards.fixing && self.cards.seen.is_some()
    }

    /// A look is due: the pull request is known, open, watched, and not already being read.
    pub(crate) fn should_look(&self, default: GithubPrefs, now: f64) -> Option<Look> {
        let number = self.cards.number?;
        let due = self.ready()
            && !self.in_flight
            && self.worth_a_look()
            && self.policy(default).mode != PrMode::Off
            && now >= self.due;
        due.then(|| Look { number, prev: self.cards.seen.clone() })
    }

    pub(crate) fn look_started(&mut self) {
        self.in_flight = true;
    }

    fn fast_secs(&self) -> f64 {
        if self.live() { LIVE_FAST_SECS } else { FAST_SECS }
    }

    fn slow_secs(&self) -> f64 {
        if self.live() { LIVE_SLOW_SECS } else { SLOW_SECS }
    }

    /// Whether GitHub tells Tidewatch about this repository now, so routine looks can be rarer.
    /// Falling back to polling pulls the next look in to the ordinary gap.
    pub(crate) fn set_live(&mut self, live: bool, now: f64) {
        if self.live() && !live {
            self.due = self.due.min(now + SLOW_SECS);
        }
        self.updates = if live { Updates::Live } else { Updates::Polling };
    }

    pub(crate) fn live(&self) -> bool {
        self.updates == Updates::Live
    }

    /// News from GitHub names this tab's pull request or branch.
    pub(crate) fn named_by(&self, repos: &[PathBuf], hit: &Hit) -> bool {
        repos.contains(&self.repo)
            && (self.cards.number.is_some_and(|number| hit.numbers.contains(&number))
                || (!self.branch.is_empty() && hit.branches.contains(&self.branch)))
    }

    /// Look in a moment: a look already under way is followed by one more.
    pub(crate) fn look_for_news(&mut self, now: f64) {
        if self.in_flight {
            self.news = News::Waiting;
        } else {
            self.due = self.due.min(now + NEWS_LOOK_SECS);
        }
    }

    /// Look again in a few seconds, such as after the checks were re-run.
    pub(crate) fn look_soon(&mut self, now: f64) {
        self.due = self.due.min(now + RERUN_LOOK_SECS);
    }

    /// When the next look is due, for a wake-up; `None` when none will be.
    pub(crate) fn next_look(&self, default: GithubPrefs) -> Option<f64> {
        let watching = self.ready()
            && self.worth_a_look()
            && self.cards.number.is_some()
            && self.policy(default).mode != PrMode::Off;
        (watching && !self.in_flight).then_some(self.due)
    }

    pub(crate) fn polled(
        &mut self,
        result: Result<Polled, GhError>,
        default: GithubPrefs,
        tab_title: &str,
        now: f64,
        now_ms: i64,
    ) -> Vec<Out> {
        self.in_flight = false;
        let queued = std::mem::replace(&mut self.news, News::None) == News::Waiting;
        let polled = match result {
            Ok(polled) => polled,
            Err(error) => {
                self.pause = Some(Pause::of(error));
                self.due = now + if queued { NEWS_LOOK_SECS } else { SLOW_SECS };
                return Vec::new();
            }
        };
        self.pause = None;
        self.due = now
            + if queued {
                NEWS_LOOK_SECS
            } else if polled.checks_pending {
                self.fast_secs()
            } else {
                self.slow_secs()
            };
        if self.merged_for_good(&polled) {
            return Vec::new();
        }
        self.ended = polled.ended;
        self.cards.seen = Some(polled.seen);
        self.cards.summary = Some(polled.summary);
        self.cards.drop_pushed_fix(&polled.head);
        if polled.ended {
            self.pending.clear();
        }
        let number = self.cards.number.unwrap_or_default();
        let mut outs = Vec::new();
        for event in polled.events {
            self.cards.add(event.at_ms, event.body.clone());
            if let Some(notice) = notice(&event.body, number, tab_title) {
                outs.push(Out::Notify(notice));
            }
            outs.extend(self.react(&event.body, number, default, now, now_ms));
        }
        outs.push(Out::Save);
        outs
    }

    /// Auto on a new card: send now when the agent is idle, else wait for it.
    fn react(
        &mut self,
        body: &Body,
        number: u64,
        default: GithubPrefs,
        now: f64,
        now_ms: i64,
    ) -> Vec<Out> {
        let policy = self.policy(default);
        let Some(message) = pr_prompt::message(body, number) else { return Vec::new() };
        if policy.mode != PrMode::Auto
            || self.closed()
            || !pr_prompt::wanted(&policy, body)
            || self.cards.auto_sends >= self.cap()
        {
            return Vec::new();
        }
        self.pending.retain(|pending| pending.message.ask.tag != message.ask.tag);
        self.next_pending += 1;
        self.pending.push(Pending { id: self.next_pending, message });
        self.flush(default, now, now_ms)
    }

    /// Sends the waiting message when the agent is idle and the last send has had its turn (the hook
    /// that says the agent started may arrive a moment after the paste). Call every frame.
    pub(crate) fn flush(&mut self, default: GithubPrefs, now: f64, now_ms: i64) -> Vec<Out> {
        let mut outs = Vec::new();
        if self.pending.is_empty() || self.policy(default).mode != PrMode::Auto {
            return outs;
        }
        if self.cards.auto_sends >= self.cap() {
            if self.autonomous {
                self.pending.clear();
                return outs;
            }
            self.fall_back_to_asking(default);
            return vec![Out::Toast(fall_back_text()), Out::Save];
        }
        let idle =
            self.status == AgentStatus::Done && self.hold_until.is_none_or(|until| now >= until);
        if !idle {
            return outs;
        }
        let pending = self.pending.remove(0);
        self.cards.auto_sends += 1;
        outs.extend(self.send(pending.message, Via::Auto, now, now_ms));
        if self.cards.auto_sends >= self.cap() && !self.autonomous {
            self.fall_back_to_asking(default);
            outs.push(Out::Toast(fall_back_text()));
        }
        outs
    }

    fn fall_back_to_asking(&mut self, default: GithubPrefs) {
        let policy = GithubPrefs { mode: PrMode::Ask, ..self.policy(default) };
        self.cards.policy = Some(policy);
        self.pending.clear();
    }

    /// The user pressed `Ask <profile> to ...` on card `id`: the message, or why it cannot go.
    pub(crate) fn ask(&mut self, id: u64, now: f64, now_ms: i64) -> Result<Vec<Out>, String> {
        let number = self.cards.number.unwrap_or_default();
        let message = self
            .cards
            .get(id)
            .and_then(|card| pr_prompt::message(&card.body, number))
            .ok_or_else(|| "There is nothing to ask the agent about".to_owned())?;
        self.manual(message, now, now_ms)
    }

    /// `Send now` on a waiting row.
    pub(crate) fn send_now(
        &mut self,
        pending_id: u64,
        now: f64,
        now_ms: i64,
    ) -> Result<Vec<Out>, String> {
        let Some(at) = self.pending.iter().position(|pending| pending.id == pending_id) else {
            return Ok(Vec::new());
        };
        let message = self.pending[at].message.clone();
        let outs = self.manual(message, now, now_ms)?;
        self.pending.retain(|pending| pending.id != pending_id);
        Ok(outs)
    }

    pub(crate) fn cancel(&mut self, pending_id: u64) {
        self.pending.retain(|pending| pending.id != pending_id);
    }

    fn manual(&mut self, message: Message, now: f64, now_ms: i64) -> Result<Vec<Out>, String> {
        if let Some(reason) = gate_reason(gate(Some(&self.status))) {
            return Err(reason.to_owned());
        }
        self.pending.retain(|pending| pending.message.ask.tag != message.ask.tag);
        Ok(self.send(message, Via::You, now, now_ms))
    }

    fn send(&mut self, message: Message, via: Via, now: f64, now_ms: i64) -> Vec<Out> {
        self.cards.add(
            now_ms,
            Body::Sent {
                via,
                tag: message.ask.tag.to_owned(),
                headline: message.headline,
                extra_lines: message.extra_lines,
                text: message.text.clone(),
            },
        );
        self.cards.fixing = true;
        self.hold_until = Some(now + HOLD_SECS);
        let told =
            (via == Via::Auto).then_some(Out::Told { n: self.cards.auto_sends, max: self.cap() });
        told.into_iter().chain([Out::Paste(message.text), Out::Save]).collect()
    }

    /// The agent finished after a message: `Some((head, commits))` is what it committed.
    pub(crate) fn committed(&mut self, found: Option<(String, usize)>, now_ms: i64) -> bool {
        self.cards.fixing = false;
        match found {
            Some((head, commits)) if commits > 0 => {
                self.cards.add(now_ms, Body::PushFix { commits, head });
                true
            }
            _ => false,
        }
    }

    /// The user changed the tab's choices.
    pub(crate) fn set_policy(&mut self, policy: GithubPrefs, now: f64) {
        let was_off = self.cards.policy.is_some_and(|old| old.mode == PrMode::Off);
        self.cards.policy = Some(policy);
        if policy.mode != PrMode::Auto {
            self.pending.clear();
        }
        if was_off && policy.mode != PrMode::Off {
            self.due = now;
        }
    }

    /// A merge card still offers its buttons.
    pub(crate) fn merge_offered(&self) -> bool {
        let cards = self.cards.cards();
        (0..cards.len())
            .any(|at| matches!(cards[at].body, Body::Merged { .. }) && self.cards.is_current(at))
    }

    /// The head GitHub last reported for the pull request.
    pub(crate) fn pr_head(&self) -> Option<String> {
        self.cards.seen.as_ref().map(|seen| seen.head.clone()).filter(|head| !head.is_empty())
    }

    pub(crate) fn start_check(&mut self, now: f64) {
        self.cleanup = Cleanup::Checking;
        self.check_started = now;
    }

    /// What the agent needs of the worktree this frame. The last look is thrown away when the agent
    /// lets go, and every few seconds while it works, so the card never rests on an old count.
    pub(crate) fn see_busy(&mut self, busy: Busy, now: f64) {
        let let_go = self.busy.any() && !busy.any();
        self.busy = busy;
        if let_go || self.next_recheck().is_some_and(|at| at <= now) {
            self.cleanup = Cleanup::Unchecked;
        }
    }

    pub(crate) fn next_recheck(&self) -> Option<f64> {
        (self.busy.any()
            && self.merge_offered()
            && matches!(self.cleanup, Cleanup::Checked(_) | Cleanup::Failed(_)))
        .then_some(self.check_started + RECHECK_SECS)
    }

    /// A safety check finished. With `remove` set, the worktree safe and its agent done with it, the
    /// removal goes ahead.
    pub(crate) fn checked(&mut self, remove: bool, result: Result<Report, String>) -> Vec<Out> {
        let outs = match &result {
            Ok(_) if remove && self.busy.any() => {
                vec![Out::Toast(STAYS.into())]
            }
            Ok(report) if remove && report.is_safe() => vec![Out::Remove],
            Ok(_) if remove => {
                vec![Out::Toast("The worktree changed since the check, so it stays".into())]
            }
            _ => Vec::new(),
        };
        self.cleanup = result.map_or_else(Cleanup::Failed, Cleanup::Checked);
        outs
    }

    /// The next wake-up for the hold after a send, when messages wait.
    pub(crate) fn next_flush(&self, now: f64) -> Option<f64> {
        self.hold_until.filter(|until| *until > now && !self.pending.is_empty())
    }

    pub(crate) fn summary(&self) -> Option<&Summary> {
        self.cards.summary.as_ref()
    }

    /// `#42 checks failed` for the Overview row, with its tone.
    pub(crate) fn pill(&self) -> Option<(PrTone, String)> {
        let summary = self.summary()?;
        let (tone, word) = summary.headline()?;
        Some((tone, format!("#{} {word}", summary.number)))
    }
}

/// What a tab's delivery may do now (see `push_of`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Push {
    /// Auto may ask the agent to push and open the pull request: the tab's mode is Auto with the
    /// push tick on, its saved cards are read and its pull request is not closed.
    pub(crate) auto: bool,
    /// The delivery stays shut: Auto would push but the saved cards are not read yet, or the pull
    /// request merged and its head is not known, so the branch could still be the merged one.
    pub(crate) shut: bool,
    /// The head the tab's pull request merged with: what comes after it goes to a new branch.
    pub(crate) merged_head: Option<String>,
}

/// The tab's delivery rules from its watch. A Meeseeks hands off through its own check and never
/// pushes through Auto. A tab that waits for the owner is never held back by `shut`.
pub(crate) fn push_of(
    watch: Option<&Watch>,
    default: GithubPrefs,
    autonomous: bool,
    meeseeks: bool,
) -> Push {
    let policy = match watch {
        Some(watch) => watch.policy(default),
        None if autonomous => GithubPrefs { mode: PrMode::Auto, ..default },
        None => default,
    };
    let wants = !meeseeks && policy.pushes_on_its_own();
    let ready = watch.is_some_and(Watch::ready);
    let merged = watch.is_some_and(Watch::merged);
    let merged_head = watch.filter(|watch| watch.merged()).and_then(Watch::pr_head);
    let head_unknown = merged && merged_head.is_none();
    let closed = watch.is_some_and(Watch::closed);
    Push {
        auto: wants && ready && !closed && !head_unknown,
        shut: (wants && !ready) || head_unknown,
        merged_head,
    }
}

fn fall_back_text() -> String {
    format!("Auto sent {} times on this pull request. Back to Ask me", send_cap(false))
}

/// The one line a paused tab shows at the end of its timeline.
pub(crate) fn paused_note() -> String {
    format!("Auto paused after {} sends on this pull request", send_cap(true))
}

/// The notification for a new card, `None` for the quiet ones.
pub(crate) fn notice(body: &Body, number: u64, tab_title: &str) -> Option<Notice> {
    let (tone, what, detail) = match body {
        Body::ChecksFailed { jobs, .. } => {
            let first = jobs.iter().find(|job| !job.passed)?;
            let at = first
                .step
                .as_deref()
                .map(short_step)
                .filter(|step| !step.is_empty())
                .map_or_else(String::new, |step| format!(" at {step}"));
            (Tone::Failed, "checks failed", format!("{} failed{at}", first.name))
        }
        Body::ChangesRequested { author, comments, .. } => (
            Tone::Needs,
            "changes requested",
            comments.first().map_or_else(
                || format!("by @{author}"),
                |comment| format!("@{author}: {}", comment.text.lines().next().unwrap_or_default()),
            ),
        ),
        Body::Commented { author, text, .. } => (
            Tone::Needs,
            "new comment",
            format!("@{author}: {}", text.lines().next().unwrap_or_default()),
        ),
        Body::Conflict { base } => (Tone::Needs, "merge conflicts", format!("with {base}")),
        Body::Approved { author } => (Tone::Done, "approved", format!("by @{author}")),
        Body::Merged { base, .. } => (Tone::Done, "merged", format!("into {base}")),
        Body::Closed { .. } => (Tone::Needs, "closed", "without merging".to_owned()),
        Body::Opened { .. }
        | Body::ChecksRunning { .. }
        | Body::ChecksPassed { .. }
        | Body::PushFix { .. }
        | Body::Sent { .. } => return None,
    };
    Some(Notice { tone, what: format!("{tab_title}: {what} on #{number}"), detail })
}

#[cfg(test)]
mod tests;
