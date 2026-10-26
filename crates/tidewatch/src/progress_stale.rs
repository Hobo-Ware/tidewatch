//! A ledger that still asks the owner something after the agent went back to work (pure).
//!
//! An agent saves a ledger that waits and asks, the owner answers in chat and the agent carries on
//! without saving again: the hooks say the agent works, the ledger says it waits. The live status
//! wins, so the question, the Answer button and the Overview tag go away until the agent saves.

use std::borrow::Cow;

use tidewatch_core::progress::{Ledger, State};

/// What the hooks say about the agent, as wall-clock times.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Live {
    /// A hook says the agent asks the owner something right now: a permission or a question.
    pub(crate) asking: bool,
    /// When the last turn started, which is when the owner's reply reached the agent.
    pub(crate) turn_started_ms: Option<i64>,
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

/// The ledger as the screens should read it: unchanged, or with the question and the wait taken
/// out while `stale`.
pub(crate) fn fresh(ledger: &Ledger, stale: bool) -> Cow<'_, Ledger> {
    if !stale {
        return Cow::Borrowed(ledger);
    }
    let mut working = ledger.clone();
    working.needs_you.clear();
    if matches!(working.status.state, Some(State::Waiting | State::Blocked)) {
        working.status.state = Some(State::Working);
    }
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
            ("waiting", ledger("state: waiting", "- Nothing"), true),
            ("blocked", ledger("state: blocked", "- Nothing"), true),
            ("working", ledger("state: working", "- Nothing"), false),
            ("done with a hand-off", ledger("state: done", "- Merge it"), false),
        ];
        for (what, ledger, want) in cases {
            assert_eq!(is_stale(&ledger, Some(SAVED), turn(SAVED + 1)), want, "{what}");
        }
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
