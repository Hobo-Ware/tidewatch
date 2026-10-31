//! A ledger that still asks the owner something after the agent went back to work (pure).
//!
//! An agent saves a ledger that waits and asks, the owner answers in chat and the agent carries on
//! without saving again: the hooks say the agent works, the ledger says it waits. The live status
//! wins, so the question, the Answer button and the Overview tag go away until the agent saves.

use std::borrow::Cow;

use tidewatch_core::progress::{Ledger, State};

use crate::progress_model::waiting_of;

/// What the hooks say about the agent, as wall-clock times.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Live {
    /// A hook says the agent asks the owner something right now: a permission or a question.
    pub(crate) asking: bool,
    /// When the last turn started, which is when the owner's reply reached the agent.
    pub(crate) turn_started_ms: Option<i64>,
    pub(crate) rested_at_ms: Option<i64>,
}

pub(crate) fn is_outdated(ledger: &Ledger, updated_ms: Option<i64>, live: Live) -> bool {
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
pub(crate) fn is_stale(ledger: &Ledger, updated_ms: Option<i64>, live: Live) -> bool {
    if !ledger.awaits_owner() || live.asking {
        return false;
    }
    let Some(updated) = updated_ms else { return false };
    live.turn_started_ms.is_some_and(|started| started > updated)
}

/// Whether the owner answered the ledger's question with a ready answer and the agent has not saved
/// a ledger since: `answered_at` is the save time the ledger had then.
pub(crate) fn answered_since_save(
    answered_at: Option<&Option<i64>>,
    updated_ms: Option<i64>,
) -> bool {
    answered_at == Some(&updated_ms)
}

/// The state a report stands for on every screen. A report older than the turn that now runs no
/// longer holds while the agent works, so the tab reads Working until it reports again.
pub(crate) fn standing(
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
pub(crate) fn fresh(ledger: &Ledger, stale: bool) -> Cow<'_, Ledger> {
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
            ledger.waiting_on = Some(tidewatch_core::progress_report::WaitingOn::Pr(7));
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
            ledger.waiting_on = Some(tidewatch_core::progress_report::WaitingOn::Pr(7));
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
        waiting.waiting_on = Some(tidewatch_core::progress_report::WaitingOn::Pr(7));
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
}
