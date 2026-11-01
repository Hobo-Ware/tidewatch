//! A ledger that still asks the owner something after the agent went back to work (pure).
//!
//! An agent saves a ledger that waits and asks, the owner answers in chat and the agent carries on
//! without saving again: the hooks say the agent works, the ledger says it waits. The live status
//! wins, so the question, the Answer button and the Overview tag go away until the agent saves.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::civil_date::Date;
use crate::progress::{Ledger, Mark, State, When};
use crate::progress_report::WaitingOn;

/// What the hooks say about the agent, as wall-clock times.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    /// A hook says the agent asks the owner something right now: a permission or a question.
    pub asking: bool,
    /// When the last turn started, which is when the owner's reply reached the agent.
    pub turn_started_ms: Option<i64>,
    pub rested_at_ms: Option<i64>,
}

#[must_use]
pub fn is_outdated(ledger: &Ledger, updated_ms: Option<i64>, live: Live) -> bool {
    let Some((updated, rested)) = updated_ms.zip(live.rested_at_ms) else { return false };
    if rested <= updated {
        return false;
    }
    match ledger.status.state {
        Some(State::Working | State::NeedsYou) => !ledger.has_question(),
        Some(State::Waiting) => !ledger.has_question() && waiting_of(ledger).is_none(),
        Some(State::Blocked | State::Done) | None => false,
    }
}

/// Whether the ledger's question or wait is older than the agent's work. A ledger saved after the
/// turn started is the agent's latest word and stands; so does any ledger while a hook says the
/// agent is asking, and one with no known save time. Only a turn start counts: the status turning
/// Working again inside the same turn (a permission answered) is not the owner's reply.
#[must_use]
pub fn is_stale(ledger: &Ledger, updated_ms: Option<i64>, live: Live) -> bool {
    if !ledger.awaits_owner() || live.asking {
        return false;
    }
    let Some(updated) = updated_ms else { return false };
    live.turn_started_ms.is_some_and(|started| started > updated)
}

/// Whether the owner answered the ledger's question with a ready answer and the agent has not saved
/// a ledger since: `answered_at` is the save time the ledger had then.
#[must_use]
pub fn answered_since_save(answered_at: Option<&Option<i64>>, updated_ms: Option<i64>) -> bool {
    answered_at == Some(&updated_ms)
}

/// The state a report stands for on every screen. A report older than the turn that now runs no
/// longer holds while the agent works, so the tab reads Working until it reports again.
#[must_use]
pub fn standing(
    ledger: &Ledger,
    updated_ms: Option<i64>,
    live: Live,
    (stale, working): (bool, bool),
) -> State {
    if is_outdated(ledger, updated_ms, live) {
        return State::Done;
    }
    let state = fresh(ledger, stale).shown_state();
    let older =
        updated_ms.zip(live.turn_started_ms).is_some_and(|(saved, started)| started > saved);
    if working && older { State::Working } else { state }
}

/// The ledger as the screens should read it: unchanged, or with the question and the wait taken
/// out while `stale`.
#[must_use]
pub fn fresh(ledger: &Ledger, stale: bool) -> Cow<'_, Ledger> {
    if !stale {
        return Cow::Borrowed(ledger);
    }
    let mut working = ledger.clone();
    working.needs_you.clear();
    if matches!(working.status.state, Some(State::NeedsYou | State::Waiting | State::Blocked)) {
        working.status.state = Some(State::Working);
    }
    working.waiting_on = None;
    Cow::Owned(working)
}

/// What a card says of the thing the agent waits on, and when it falls due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub label: String,
    pub date: Option<Date>,
}

/// What a waiting ledger waits on: the typed `waiting_on`, or for a ledger written in markdown
/// the soonest open dated item of its Scheduled list, else its first open `on:` item.
#[must_use]
pub fn waiting_of(ledger: &Ledger) -> Option<Waiting> {
    if let Some(waiting) = &ledger.waiting_on {
        let date = match waiting {
            WaitingOn::Date(date) => Some(*date),
            _ => None,
        };
        return Some(Waiting { label: waiting.label(), date });
    }
    let open =
        || ledger.scheduled.iter().filter(|item| !matches!(item.mark, Mark::Done | Mark::Skipped));
    let dated = open().filter_map(|item| item.when.date()).min();
    if let Some(date) = dated {
        return Some(Waiting { label: date.to_string(), date: Some(date) });
    }
    open().find_map(|item| match &item.when {
        When::On(event) => Some(Waiting { label: event.clone(), date: None }),
        _ => None,
    })
}

/// What a saved report says that the panel weighs against the facts, kept in the log so the log
/// never holds a report body. The decisions are those of the ledger functions above, over a digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDigest {
    pub declared: Option<State>,
    pub shown: State,
    pub shown_if_stale: State,
    pub asks: bool,
    pub awaits_owner: bool,
    pub waits: bool,
}

impl ReportDigest {
    #[must_use]
    pub fn of(ledger: &Ledger) -> Self {
        Self {
            declared: ledger.status.state,
            shown: ledger.shown_state(),
            shown_if_stale: fresh(ledger, true).shown_state(),
            asks: ledger.has_question(),
            awaits_owner: ledger.awaits_owner(),
            waits: waiting_of(ledger).is_some(),
        }
    }

    #[must_use]
    pub fn is_outdated(&self, updated_ms: Option<i64>, live: Live) -> bool {
        let Some((updated, rested)) = updated_ms.zip(live.rested_at_ms) else { return false };
        if rested <= updated {
            return false;
        }
        match self.declared {
            Some(State::Working | State::NeedsYou) => !self.asks,
            Some(State::Waiting) => !self.asks && !self.waits,
            Some(State::Blocked | State::Done) | None => false,
        }
    }

    #[must_use]
    pub fn is_stale(&self, updated_ms: Option<i64>, live: Live) -> bool {
        if !self.awaits_owner || live.asking {
            return false;
        }
        let Some(updated) = updated_ms else { return false };
        live.turn_started_ms.is_some_and(|started| started > updated)
    }

    #[must_use]
    pub fn standing(
        &self,
        updated_ms: Option<i64>,
        live: Live,
        (stale, working): (bool, bool),
    ) -> State {
        if self.is_outdated(updated_ms, live) {
            return State::Done;
        }
        let state = if stale { self.shown_if_stale } else { self.shown };
        let older =
            updated_ms.zip(live.turn_started_ms).is_some_and(|(saved, started)| started > saved);
        if working && older { State::Working } else { state }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(status: &str, needs: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\nShip\n\n## Status\n{status}\n\n## Needs you\n{needs}\n"))
    }

    fn asks() -> Ledger {
        ledger("state: waiting", "- Yes or changes on the board?")
    }

    const SAVED: i64 = 1_000;

    fn turn(at: i64) -> Live {
        Live { turn_started_ms: Some(at), ..Live::default() }
    }

    #[test]
    fn a_ready_answer_hides_the_question_until_the_agent_saves_a_new_ledger() {
        let cases = [
            ("answered, same save", Some(&Some(SAVED)), Some(SAVED), true),
            ("answered, saved again since", Some(&Some(SAVED)), Some(SAVED + 5), false),
            ("never answered", None, Some(SAVED), false),
            ("answered with no known save time, still none", Some(&None), None, true),
            ("no known save time then, one now", Some(&None), Some(SAVED), false),
        ];
        for (name, answered, updated, want) in cases {
            assert_eq!(answered_since_save(answered, updated), want, "{name}");
        }
    }

    #[test]
    fn a_question_saved_before_the_turn_that_followed_it_is_stale() {
        assert!(is_stale(&asks(), Some(SAVED), turn(SAVED + 1)));
    }

    #[test]
    fn a_question_saved_after_the_turn_started_still_shows() {
        assert!(!is_stale(&asks(), Some(SAVED), turn(SAVED - 1)));
        assert!(!is_stale(&asks(), Some(SAVED), turn(SAVED)));
    }

    #[test]
    fn a_hook_that_says_the_agent_asks_keeps_the_question() {
        let live = Live { asking: true, ..turn(SAVED + 1) };
        assert!(!is_stale(&asks(), Some(SAVED), live));
    }

    #[test]
    fn nothing_known_about_the_agent_or_the_save_leaves_the_ledger_alone() {
        assert!(!is_stale(&asks(), Some(SAVED), Live::default()));
        assert!(!is_stale(&asks(), None, turn(SAVED + 1)));
    }

    #[test]
    fn the_status_turning_working_inside_the_turn_leaves_the_question_standing() {
        let live = Live { turn_started_ms: Some(SAVED - 1), ..Live::default() };
        assert!(!is_stale(&asks(), Some(SAVED), live));
    }

    #[test]
    fn only_a_ledger_that_awaits_the_owner_can_be_stale() {
        let cases = [
            ("a question", ledger("state: working", "- Pick one"), true),
            ("waiting with a question", ledger("state: waiting", "- Pick one"), true),
            ("waiting", ledger("state: waiting", "- Nothing"), false),
            ("blocked", ledger("state: blocked", "- Nothing"), true),
            ("working", ledger("state: working", "- Nothing"), false),
            ("done with a hand-off", ledger("state: done", "- Merge it"), false),
        ];
        for (what, ledger, want) in cases {
            assert_eq!(is_stale(&ledger, Some(SAVED), turn(SAVED + 1)), want, "{what}");
        }
    }

    fn typed(state: &str, extra: &str) -> Ledger {
        let mut ledger = Ledger::parse(&format!("# Goal\nx\n\n## Status\nstate: {state}\n"));
        if !extra.is_empty() {
            ledger.waiting_on = Some(WaitingOn::Pr(7));
        }
        ledger
    }

    #[test]
    fn a_report_older_than_the_turn_that_runs_now_no_longer_holds_while_the_agent_works() {
        let cases = [
            ("done", "", true, SAVED + 1, State::Working),
            ("waiting", "pr", true, SAVED + 1, State::Working),
            ("blocked", "", true, SAVED + 1, State::Working),
            ("done", "", false, SAVED + 1, State::Done),
            ("done", "", true, SAVED - 1, State::Done),
            ("waiting", "pr", false, SAVED + 1, State::Waiting),
            ("working", "", true, SAVED + 1, State::Working),
        ];
        for (state, extra, working, turn_at, want) in cases {
            let got = standing(&typed(state, extra), Some(SAVED), turn(turn_at), (false, working));
            assert_eq!(got, want, "{state} working={working} turn at {turn_at}");
        }
        let owner_answered = standing(&asks(), Some(SAVED), turn(SAVED - 1), (true, false));
        assert_eq!(owner_answered, State::Working, "an answered question stops asking");
        assert_eq!(
            standing(&asks(), Some(SAVED), turn(SAVED - 1), (false, false)),
            State::NeedsYou
        );
    }

    fn rested(at: i64) -> Live {
        Live { rested_at_ms: Some(at), ..Live::default() }
    }

    fn with_wait(state: &str, wait: bool) -> Ledger {
        let mut ledger = typed(state, "");
        if wait {
            ledger.waiting_on = Some(WaitingOn::Pr(7));
        }
        ledger
    }

    #[test]
    fn a_finished_turn_beats_an_older_report_that_still_says_unfinished_work() {
        let cases = [
            ("working", with_wait("working", false), SAVED, SAVED + 1, true),
            (
                "working, saved after the turn ended",
                with_wait("working", false),
                SAVED,
                SAVED - 1,
                false,
            ),
            (
                "working, saved the instant it ended",
                with_wait("working", false),
                SAVED,
                SAVED,
                false,
            ),
            ("needs you with no question", typed("needs_you", ""), SAVED, SAVED + 1, true),
            (
                "needs you with a question",
                ledger("state: needs_you", "- Pick one"),
                SAVED,
                SAVED + 1,
                false,
            ),
            ("waiting on nothing", with_wait("waiting", false), SAVED, SAVED + 1, true),
            ("waiting on a pull request", with_wait("waiting", true), SAVED, SAVED + 1, false),
            ("blocked", typed("blocked", ""), SAVED, SAVED + 1, false),
            ("done", typed("done", ""), SAVED, SAVED + 1, false),
        ];
        for (what, report, saved, ended, want) in cases {
            assert_eq!(is_outdated(&report, Some(saved), rested(ended)), want, "{what}");
        }
    }

    #[test]
    fn nothing_known_about_the_save_or_the_turn_end_leaves_the_report_alone() {
        let working = with_wait("working", false);
        assert!(!is_outdated(&working, None, rested(SAVED + 1)));
        assert!(!is_outdated(&working, Some(SAVED), Live::default()));
    }

    #[test]
    fn an_idle_agent_reads_done_past_a_working_report_until_it_saves_again() {
        let working = with_wait("working", false);
        let after_stop = standing(&working, Some(SAVED), rested(SAVED + 1), (false, false));
        assert_eq!(after_stop, State::Done);
        let saved_again = standing(&working, Some(SAVED + 2), rested(SAVED + 1), (false, false));
        assert_eq!(saved_again, State::Working);
    }

    #[test]
    fn a_stale_wait_drops_what_it_waited_on() {
        let mut waiting = asks();
        waiting.waiting_on = Some(WaitingOn::Pr(7));
        assert_eq!(fresh(&waiting, true).waiting_on, None);
        assert!(fresh(&waiting, false).waiting_on.is_some());
    }

    #[test]
    fn a_stale_ledger_reads_as_working_with_no_question() {
        let ledger = asks();
        let read = fresh(&ledger, true);
        assert_eq!(read.status.state, Some(State::Working));
        assert!(!read.has_question());
        assert_eq!(read.goal, ledger.goal);
    }

    #[test]
    fn a_ledger_that_is_not_stale_is_not_copied() {
        let ledger = asks();
        assert!(matches!(fresh(&ledger, false), Cow::Borrowed(_)));
    }

    #[test]
    fn the_digest_decides_exactly_as_the_ledger_does() {
        let states = ["working", "needs_you", "waiting", "blocked", "done"];
        let needs = ["- Nothing", "- Pick one"];
        let times = [None, Some(SAVED - 1), Some(SAVED), Some(SAVED + 1)];
        let mut checked = 0;
        for state in states {
            for ask in needs {
                for wait in [false, true] {
                    let mut report = ledger(&format!("state: {state}"), ask);
                    if wait {
                        report.waiting_on = Some(WaitingOn::Pr(7));
                    }
                    let digest = ReportDigest::of(&report);
                    for updated in [None, Some(SAVED)] {
                        for turn_at in times {
                            for rested_at in times {
                                for asking in [false, true] {
                                    let live = Live {
                                        asking,
                                        turn_started_ms: turn_at,
                                        rested_at_ms: rested_at,
                                    };
                                    let what =
                                        format!("{state} {ask} wait={wait} {updated:?} {live:?}");
                                    assert_eq!(
                                        digest.is_outdated(updated, live),
                                        is_outdated(&report, updated, live),
                                        "outdated: {what}"
                                    );
                                    assert_eq!(
                                        digest.is_stale(updated, live),
                                        is_stale(&report, updated, live),
                                        "stale: {what}"
                                    );
                                    for flags in
                                        [(false, false), (true, false), (false, true), (true, true)]
                                    {
                                        assert_eq!(
                                            digest.standing(updated, live, flags),
                                            standing(&report, updated, live, flags),
                                            "standing {flags:?}: {what}"
                                        );
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 5 * 2 * 2 * 2 * 4 * 4 * 2 * 4);
    }

    #[test]
    fn a_digest_keeps_what_the_panel_weighs() {
        let mut report = ledger("state: waiting", "- Nothing");
        let plain = ReportDigest::of(&report);
        assert_eq!(plain.declared, Some(State::Waiting));
        assert!(!plain.asks && !plain.waits && !plain.awaits_owner);
        report.waiting_on = Some(WaitingOn::Pr(7));
        assert!(ReportDigest::of(&report).waits);
        let asks = ReportDigest::of(&ledger("state: working", "- Pick one"));
        assert!(asks.asks && asks.awaits_owner);
        assert_eq!((asks.shown, asks.shown_if_stale), (State::NeedsYou, State::Working));
        let text = serde_json::to_string(&asks).unwrap();
        assert_eq!(serde_json::from_str::<ReportDigest>(&text).unwrap(), asks);
    }
}
