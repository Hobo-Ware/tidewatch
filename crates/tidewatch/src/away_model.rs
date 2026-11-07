//! What flipping the Away toggle does, as plain data the app host and the tests share.

use std::collections::HashMap;

use tidewatch_core::house_style::wake_line;
use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::sent::because;
use tidewatch_core::status::{AgentStatus, Scheduled};

/// What a click on the Away pill changes: the new Away value, and whether keep-awake has to be
/// started with it. Turning Away off never touches keep-awake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flip {
    pub(crate) away: bool,
    pub(crate) start_keep_awake: bool,
}

/// Whether the owner is at the keyboard. An enum keeps the app struct free of one more loose bool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Presence {
    #[default]
    Here,
    Away,
}

impl Presence {
    pub(crate) const fn of(away: bool) -> Self {
        if away { Self::Away } else { Self::Here }
    }

    pub(crate) const fn is_away(self) -> bool {
        matches!(self, Self::Away)
    }
}

pub(crate) const fn flip(away: bool, keep_awake_on: bool) -> Flip {
    let away = !away;
    Flip { away, start_keep_awake: away && !keep_awake_on }
}

const TOLD_ON_START: &str = "I'm away now. Make your own decisions from here instead of asking, and write each one with its reason under decisions in your progress report.";
const TOLD_ON_END: &str = "This Mac stays awake, also with the lid closed.";
pub(crate) const TOLD_OFF: &str = "I'm back. Ask with your question tool when a choice needs me.";

/// What a flip tells a running agent, and the `because` its row in the Reader shows. A CLI that
/// can schedule itself is told how, so it never waits for a ping.
pub(crate) fn told(away: bool, wakes_itself: bool) -> (String, &'static str) {
    if away {
        let text = format!("{TOLD_ON_START} {} {TOLD_ON_END}", wake_line(wakes_itself));
        (text, because::AWAY_ON)
    } else {
        (TOLD_OFF.to_owned(), because::AWAY_OFF)
    }
}

/// The tabs that hear about a flip: an agent that runs. A shell tab, a stopped agent, one that
/// failed or is still setting up has nobody to read it.
pub(crate) fn hearers<'a>(
    tabs: impl IntoIterator<Item = (u64, Option<&'a AgentStatus>)>,
) -> Vec<u64> {
    tabs.into_iter()
        .filter(|(_, status)| {
            matches!(
                status,
                Some(AgentStatus::Working | AgentStatus::NeedsYou(_) | AgentStatus::Done)
            )
        })
        .map(|(id, _)| id)
        .collect()
}

/// How long an agent that stopped with open steps and no wake-up is left alone before Tidewatch
/// nudges it once.
pub(crate) const GRACE_SECS: f64 = 180.0;

/// How long a CLI that cannot schedule itself is left alone after it saved a report as waiting: it
/// has nothing else that wakes it.
pub(crate) const WAITING_GRACE_SECS: f64 = 600.0;

/// What the owner would say to an agent that stopped with work left and set no wake-up. Only a CLI
/// that can schedule itself is pointed at doing so.
pub(crate) fn nudge_text(wakes_itself: bool, report: Open) -> &'static str {
    match (wakes_itself, report.waiting_state()) {
        (true, _) => {
            "I'm still away. Pick up the next open step, or set a wake-up if you're waiting on something."
        }
        (false, true) => "I'm still away. Check on what you were waiting for and carry on.",
        (false, false) => {
            "I'm still away. Pick up the next open step, or save your report as waiting if you're waiting on something."
        }
    }
}

/// How the agent arranged to be woken up, as far as Tidewatch heard of it. A one-off or a background
/// call is spent by the next turn the owner did not start; a recurring job stays until the agent
/// deletes it or the owner writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Wake {
    #[default]
    None,
    Once,
    Recurring,
    Background,
}

impl Wake {
    pub(crate) fn scheduled(self, what: Scheduled) -> Self {
        match (self, what) {
            (Self::Recurring, Scheduled::Once | Scheduled::Background)
            | (_, Scheduled::Recurring) => Self::Recurring,
            (_, Scheduled::Once) => Self::Once,
            (_, Scheduled::Background) | (Self::Background, Scheduled::Cancelled) => {
                Self::Background
            }
            (_, Scheduled::Cancelled) => Self::None,
        }
    }

    /// A turn began that the owner did not write.
    pub(crate) fn fired(self) -> Self {
        match self {
            Self::Recurring => Self::Recurring,
            Self::None | Self::Once | Self::Background => Self::None,
        }
    }

    pub(crate) fn is_set(self) -> bool {
        self != Self::None
    }
}

/// What the progress report says, as far as keep-going cares. Two equal values mean the agent has
/// not moved the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Open {
    state: Option<State>,
    done: usize,
    total: usize,
    waiting: bool,
}

impl Open {
    pub(crate) fn of(ledger: &Ledger) -> Self {
        let counts = ledger.counts();
        Self {
            state: ledger.status.state,
            done: counts.done,
            total: counts.total,
            waiting: ledger.waiting_on.is_some(),
        }
    }

    fn steps_todo(self) -> bool {
        self.done < self.total
    }

    fn waiting_state(self) -> bool {
        self.state == Some(State::Waiting)
    }

    /// Whether the report asks for a nudge. A CLI that wakes itself is left alone while its report
    /// says it waits; one that cannot has no other way back, so waiting counts as work to resume.
    fn nudgeable(self, wakes_itself: bool) -> bool {
        if wakes_itself {
            matches!(self.state, None | Some(State::Working)) && !self.waiting
        } else {
            matches!(self.state, None | Some(State::Working | State::Waiting))
        }
    }
}

/// The report a keep-going went out for, and whether the agent began a turn since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Last {
    report: Open,
    turned: bool,
}

impl Last {
    /// A second nudge needs a turn in between that moved the report.
    fn holds(self, now: Open) -> bool {
        !(self.turned && self.report != now)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Verdict {
    Quiet,
    /// Not yet: the seconds left of the grace period.
    Wait(f64),
    Send,
}

/// An agent has been idle for `idle_secs` since its turn ended. Nudge it once, and only when the
/// owner is away, the grace period is over, the report says it is working with steps left (or, for
/// a CLI that cannot wake itself, waiting, after a longer grace), it set no wake-up and the last
/// nudge for this report is old news.
pub(crate) fn keep_going(
    away: bool,
    wakes_itself: bool,
    report: Option<Open>,
    wake: Wake,
    idle_secs: f64,
    last: Option<Last>,
) -> Verdict {
    if !away {
        return Verdict::Quiet;
    }
    if idle_secs < GRACE_SECS {
        return Verdict::Wait(GRACE_SECS - idle_secs);
    }
    let Some(report) = report.filter(|r| r.nudgeable(wakes_itself) && r.steps_todo()) else {
        return Verdict::Quiet;
    };
    if wake.is_set() || last.is_some_and(|last| last.holds(report)) {
        return Verdict::Quiet;
    }
    if report.waiting_state() && idle_secs < WAITING_GRACE_SECS {
        return Verdict::Wait(WAITING_GRACE_SECS - idle_secs);
    }
    Verdict::Send
}

#[derive(Debug, Default)]
struct Tab {
    working: bool,
    /// When the turn that just ended ended, until the nudge is settled.
    idle_since: Option<f64>,
    /// A turn ended since the wake was set, so the next one to begin spends it.
    armed: bool,
    wake: Wake,
    spoke: u32,
    last: Option<Last>,
}

/// Sees each tab's status every frame and keeps what Away needs to know about it.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    tabs: HashMap<u64, Tab>,
}

impl Watch {
    /// One frame of a tab: its status, and how often the owner has written to it so far.
    pub(crate) fn tick(&mut self, tab: u64, status: &AgentStatus, spoke: u32, now: f64) {
        let t = self.tabs.entry(tab).or_default();
        if spoke != t.spoke {
            *t = Tab { spoke, working: t.working, ..Tab::default() };
        }
        let working = matches!(status, AgentStatus::Working);
        if working && !t.working {
            if t.armed {
                t.wake = t.wake.fired();
                t.armed = false;
            }
            t.idle_since = None;
            if let Some(last) = t.last.as_mut() {
                last.turned = true;
            }
        } else if t.working && matches!(status, AgentStatus::Done) {
            t.idle_since = Some(now);
            t.armed = true;
        } else if !matches!(status, AgentStatus::Done) {
            t.idle_since = None;
        }
        t.working = working;
    }

    pub(crate) fn scheduled(&mut self, tab: u64, what: Scheduled) {
        let t = self.tabs.entry(tab).or_default();
        t.wake = t.wake.scheduled(what);
    }

    pub(crate) fn wake(&self, tab: u64) -> Wake {
        self.tabs.get(&tab).map_or(Wake::None, |t| t.wake)
    }

    pub(crate) fn last(&self, tab: u64) -> Option<Last> {
        self.tabs.get(&tab).and_then(|t| t.last)
    }

    /// Seconds since the tab's turn ended, while its nudge is still undecided.
    pub(crate) fn idle(&self, tab: u64, now: f64) -> Option<f64> {
        self.tabs.get(&tab)?.idle_since.map(|since| (now - since).max(0.0))
    }

    /// The nudge for this stop is decided, sent or not.
    pub(crate) fn settled(&mut self, tab: u64) {
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.idle_since = None;
        }
    }

    pub(crate) fn nudged(&mut self, tab: u64, report: Open) {
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.idle_since = None;
            t.last = Some(Last { report, turned: false });
        }
    }

    /// Away was flipped: every tab starts again.
    pub(crate) fn reset(&mut self) {
        for t in self.tabs.values_mut() {
            t.last = None;
            t.idle_since = None;
        }
    }

    pub(crate) fn keep_only(&mut self, live: &[u64]) {
        self.tabs.retain(|tab, _| live.contains(tab));
    }
}

pub(crate) const TIP: &str =
    "Away: agents decide on their own and keep working. Also keeps the Mac awake.";

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: f64 = GRACE_SECS + 1.0;

    fn report(state: Option<State>, done: usize, total: usize, waiting: bool) -> Open {
        Open { state, done, total, waiting }
    }

    fn open() -> Open {
        report(Some(State::Working), 1, 3, false)
    }

    #[test]
    fn keep_going_follows_away_the_report_the_wake_and_the_grace_period() {
        let none = Wake::None;
        let cases = [
            ("away off", false, Some(open()), none, AFTER, None, Verdict::Quiet),
            ("no report", true, None, none, AFTER, None, Verdict::Quiet),
            ("working", true, Some(open()), none, AFTER, None, Verdict::Send),
            ("no state", true, Some(report(None, 0, 2, false)), none, AFTER, None, Verdict::Send),
            (
                "waiting",
                true,
                Some(report(Some(State::Waiting), 1, 3, false)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            (
                "needs you",
                true,
                Some(report(Some(State::NeedsYou), 1, 3, false)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            (
                "done",
                true,
                Some(report(Some(State::Done), 1, 3, false)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            (
                "blocked",
                true,
                Some(report(Some(State::Blocked), 1, 3, false)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            (
                "working with waiting_on",
                true,
                Some(report(Some(State::Working), 1, 3, true)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            (
                "no steps left",
                true,
                Some(report(Some(State::Working), 3, 3, false)),
                none,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            ("a one-off wake", true, Some(open()), Wake::Once, AFTER, None, Verdict::Quiet),
            ("a recurring wake", true, Some(open()), Wake::Recurring, AFTER, None, Verdict::Quiet),
            (
                "a background call",
                true,
                Some(open()),
                Wake::Background,
                AFTER,
                None,
                Verdict::Quiet,
            ),
            ("just stopped", true, Some(open()), none, 0.0, None, Verdict::Wait(GRACE_SECS)),
            ("a minute in", true, Some(open()), none, 60.0, None, Verdict::Wait(GRACE_SECS - 60.0)),
            (
                "exactly at the grace period",
                true,
                Some(open()),
                none,
                GRACE_SECS,
                None,
                Verdict::Send,
            ),
        ];
        for (what, away, report, wake, idle, last, want) in cases {
            assert_eq!(keep_going(away, true, report, wake, idle, last), want, "{what}");
        }
    }

    #[test]
    fn a_cli_that_cannot_wake_itself_is_nudged_for_a_waiting_report_after_a_longer_grace() {
        let waiting = report(Some(State::Waiting), 1, 3, true);
        let none = Wake::None;
        let sent = Last { report: waiting, turned: false };
        let moved = report(Some(State::Waiting), 2, 3, true);
        let turned = Last { report: waiting, turned: true };
        let cases = [
            (
                "waiting, past the short grace",
                true,
                false,
                Some(waiting),
                none,
                AFTER,
                None,
                Verdict::Wait(WAITING_GRACE_SECS - AFTER),
            ),
            (
                "waiting, just stopped",
                true,
                false,
                Some(waiting),
                none,
                0.0,
                None,
                Verdict::Wait(GRACE_SECS),
            ),
            (
                "waiting, at the long grace",
                true,
                false,
                Some(waiting),
                none,
                WAITING_GRACE_SECS,
                None,
                Verdict::Send,
            ),
            ("waiting, well after", true, false, Some(waiting), none, 5000.0, None, Verdict::Send),
            (
                "waiting with a wake pending",
                true,
                false,
                Some(waiting),
                Wake::Once,
                5000.0,
                None,
                Verdict::Quiet,
            ),
            ("waiting, away off", false, false, Some(waiting), none, 5000.0, None, Verdict::Quiet),
            (
                "waiting, nudged already",
                true,
                false,
                Some(waiting),
                none,
                5000.0,
                Some(sent),
                Verdict::Quiet,
            ),
            (
                "waiting, a turn did not move it",
                true,
                false,
                Some(waiting),
                none,
                5000.0,
                Some(turned),
                Verdict::Quiet,
            ),
            (
                "waiting, a turn moved it",
                true,
                false,
                Some(moved),
                none,
                5000.0,
                Some(turned),
                Verdict::Send,
            ),
            (
                "waiting, no steps left",
                true,
                false,
                Some(report(Some(State::Waiting), 3, 3, true)),
                none,
                5000.0,
                None,
                Verdict::Quiet,
            ),
            (
                "working keeps the short grace",
                true,
                false,
                Some(open()),
                none,
                AFTER,
                None,
                Verdict::Send,
            ),
            (
                "needs you stays quiet",
                true,
                false,
                Some(report(Some(State::NeedsYou), 1, 3, false)),
                none,
                5000.0,
                None,
                Verdict::Quiet,
            ),
            (
                "blocked stays quiet",
                true,
                false,
                Some(report(Some(State::Blocked), 1, 3, false)),
                none,
                5000.0,
                None,
                Verdict::Quiet,
            ),
            (
                "done stays quiet",
                true,
                false,
                Some(report(Some(State::Done), 1, 3, false)),
                none,
                5000.0,
                None,
                Verdict::Quiet,
            ),
            (
                "claude waiting stays quiet",
                true,
                true,
                Some(waiting),
                none,
                5000.0,
                None,
                Verdict::Quiet,
            ),
        ];
        for (what, away, wakes, report, wake, idle, last, want) in cases {
            assert_eq!(keep_going(away, wakes, report, wake, idle, last), want, "{what}");
        }
    }

    #[test]
    fn a_stop_gets_one_keep_going_and_a_second_needs_a_turn_that_moved_the_report() {
        let moved = report(Some(State::Working), 2, 3, false);
        let cases = [
            ("sent, no turn since", Last { report: open(), turned: false }, open(), Verdict::Quiet),
            (
                "a turn since but the report is the same",
                Last { report: open(), turned: true },
                open(),
                Verdict::Quiet,
            ),
            (
                "the report moved but no turn began",
                Last { report: open(), turned: false },
                moved,
                Verdict::Quiet,
            ),
            (
                "a turn moved the report",
                Last { report: open(), turned: true },
                moved,
                Verdict::Send,
            ),
        ];
        for (what, last, now, want) in cases {
            assert_eq!(
                keep_going(true, true, Some(now), Wake::None, AFTER, Some(last)),
                want,
                "{what}"
            );
        }
    }

    #[test]
    fn a_scheduled_wake_is_spent_by_the_next_turn_the_owner_did_not_write_except_a_recurring_one() {
        let cases = [
            (Wake::None, Scheduled::Once, Wake::Once),
            (Wake::None, Scheduled::Recurring, Wake::Recurring),
            (Wake::None, Scheduled::Background, Wake::Background),
            (Wake::None, Scheduled::Cancelled, Wake::None),
            (Wake::Once, Scheduled::Cancelled, Wake::None),
            (Wake::Recurring, Scheduled::Cancelled, Wake::None),
            (Wake::Background, Scheduled::Cancelled, Wake::Background),
            (Wake::Recurring, Scheduled::Once, Wake::Recurring),
            (Wake::Recurring, Scheduled::Background, Wake::Recurring),
            (Wake::Once, Scheduled::Recurring, Wake::Recurring),
        ];
        for (before, what, after) in cases {
            assert_eq!(before.scheduled(what), after, "{before:?} then {what:?}");
        }
        let fired = [
            (Wake::None, Wake::None),
            (Wake::Once, Wake::None),
            (Wake::Background, Wake::None),
            (Wake::Recurring, Wake::Recurring),
        ];
        for (before, after) in fired {
            assert_eq!(before.fired(), after, "{before:?}");
        }
    }

    fn turn(watch: &mut Watch, tab: u64, spoke: u32, at: f64) {
        watch.tick(tab, &AgentStatus::Working, spoke, at);
        watch.tick(tab, &AgentStatus::Done, spoke, at + 1.0);
    }

    #[test]
    fn a_stop_starts_the_idle_clock_once_and_a_new_turn_stops_it() {
        let mut watch = Watch::default();
        let (working, done) = (AgentStatus::Working, AgentStatus::Done);
        watch.tick(1, &done, 0, 0.0);
        assert_eq!(watch.idle(1, 5.0), None, "a tab that was never working has not stopped");
        watch.tick(1, &working, 0, 10.0);
        assert_eq!(watch.idle(1, 11.0), None);
        watch.tick(1, &done, 0, 20.0);
        watch.tick(2, &done, 0, 20.0);
        assert_eq!(watch.idle(1, 50.0), Some(30.0));
        assert_eq!(watch.idle(2, 50.0), None, "another tab has its own turn");
        watch.tick(1, &done, 0, 40.0);
        assert_eq!(watch.idle(1, 50.0), Some(30.0), "staying done does not restart it");
        watch.tick(1, &working, 0, 60.0);
        assert_eq!(watch.idle(1, 70.0), None);
        watch.tick(1, &done, 0, 80.0);
        watch.settled(1);
        assert_eq!(watch.idle(1, 90.0), None);
        watch.tick(1, &done, 0, 95.0);
        assert_eq!(watch.idle(1, 99.0), None, "a settled stop is not reopened");
        watch.keep_only(&[2]);
        assert_eq!(watch.idle(1, 99.0), None);
    }

    #[test]
    fn a_once_wake_set_in_a_turn_is_spent_when_the_next_turn_begins() {
        let mut watch = Watch::default();
        watch.tick(1, &AgentStatus::Working, 0, 0.0);
        watch.scheduled(1, Scheduled::Once);
        watch.tick(1, &AgentStatus::Working, 0, 1.0);
        assert_eq!(watch.wake(1), Wake::Once, "the turn that set it does not spend it");
        watch.tick(1, &AgentStatus::Done, 0, 2.0);
        assert_eq!(watch.wake(1), Wake::Once);
        watch.tick(1, &AgentStatus::Working, 0, 3.0);
        assert_eq!(watch.wake(1), Wake::None);
    }

    #[test]
    fn a_cron_create_hook_sets_the_wake_and_the_owner_writing_clears_a_recurring_one() {
        use serde_json::json;
        let payload = |recurring| {
            json!({"hook_event_name": "PreToolUse", "tool_name": "CronCreate",
                "tool_input": {"cron": "*/10 * * * *", "prompt": "check CI", "recurring": recurring}})
        };
        let mut watch = Watch::default();
        turn(&mut watch, 1, 0, 0.0);
        let once = tidewatch_core::status::scheduled_from_hook("claude", &payload(false)).unwrap();
        watch.scheduled(1, once);
        assert_eq!(watch.wake(1), Wake::Once);
        let every = tidewatch_core::status::scheduled_from_hook("claude", &payload(true)).unwrap();
        watch.scheduled(1, every);
        assert_eq!(watch.wake(1), Wake::Recurring);
        turn(&mut watch, 1, 0, 10.0);
        turn(&mut watch, 1, 0, 20.0);
        assert_eq!(watch.wake(1), Wake::Recurring, "its own firings do not spend it");
        turn(&mut watch, 1, 1, 30.0);
        assert_eq!(watch.wake(1), Wake::None, "the owner wrote to the agent");
    }

    #[test]
    fn the_owner_writing_or_a_flip_lets_the_next_stop_be_nudged_again() {
        let mut watch = Watch::default();
        turn(&mut watch, 1, 0, 0.0);
        watch.nudged(1, open());
        assert_eq!(watch.last(1), Some(Last { report: open(), turned: false }));
        watch.tick(1, &AgentStatus::Working, 0, 5.0);
        assert_eq!(watch.last(1), Some(Last { report: open(), turned: true }));
        watch.tick(1, &AgentStatus::Done, 1, 6.0);
        assert_eq!(watch.last(1), None, "the owner spoke");
        watch.nudged(1, open());
        watch.reset();
        assert_eq!(watch.last(1), None, "Away was flipped");
    }

    #[test]
    fn the_nudge_text_names_the_way_to_wait_per_cli_and_resumes_a_waiting_report() {
        let waiting = report(Some(State::Waiting), 1, 3, true);
        let claude = nudge_text(true, open());
        let other = nudge_text(false, open());
        let resume = nudge_text(false, waiting);
        assert!(claude.contains("next open step") && claude.contains("set a wake-up"));
        assert!(other.contains("next open step") && other.contains("save your report as waiting"));
        assert_eq!(resume, "I'm still away. Check on what you were waiting for and carry on.");
        for text in [claude, other, resume] {
            assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
            assert!(!text.to_lowercase().contains("recommend"));
        }
    }

    #[test]
    fn only_a_running_agent_hears_about_a_flip() {
        use tidewatch_core::status::Reason;
        let working = AgentStatus::Working;
        let asking = AgentStatus::NeedsYou(Reason::Question);
        let done = AgentStatus::Done;
        let failed = AgentStatus::Failed("x".into());
        let tabs = [
            (1, Some(&working)),
            (2, None),
            (3, Some(&AgentStatus::Shell)),
            (4, Some(&AgentStatus::Stopped)),
            (5, Some(&done)),
            (6, Some(&failed)),
            (7, Some(&asking)),
            (8, Some(&AgentStatus::SettingUp)),
        ];
        assert_eq!(hearers(tabs), [1, 5, 7]);
    }

    #[test]
    fn the_messages_say_who_decides_and_never_name_an_option() {
        let (on, why_on) = told(true, true);
        let (off, why_off) = told(false, true);
        assert!(on.contains("Make your own decisions") && on.contains("lid closed"));
        assert!(off.contains("Ask with your question tool"));
        assert_eq!((why_on, why_off), ("you turned Away on", "you turned Away off"));
        for text in [on, off] {
            assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
            assert!(!text.to_lowercase().contains("recommend"));
        }
    }

    #[test]
    fn the_flip_message_names_the_tools_to_wake_up_with_for_claude_only() {
        let claude = told(true, true).0;
        let other = told(true, false).0;
        for tool in ["CronCreate", "Monitor", "waiting_on"] {
            assert!(claude.contains(tool), "{tool}");
        }
        for tool in ["CronCreate", "Monitor"] {
            assert!(!other.contains(tool), "{tool}");
        }
        assert!(other.contains("waiting_on") && other.contains("lid closed"));
    }

    #[test]
    fn turning_away_on_starts_keep_awake_only_when_it_is_off() {
        let cases = [
            (false, false, Flip { away: true, start_keep_awake: true }),
            (false, true, Flip { away: true, start_keep_awake: false }),
            (true, false, Flip { away: false, start_keep_awake: false }),
            (true, true, Flip { away: false, start_keep_awake: false }),
        ];
        for (away, keep_awake_on, want) in cases {
            assert_eq!(flip(away, keep_awake_on), want, "away {away}, awake {keep_awake_on}");
        }
    }
}
