//! The nudge Tidewatch sends about the progress report (pure, table-tested): an idle agent whose
//! report still says it works, an hour after it was saved. The report is asked for only at the end
//! of a turn, so there is no mid-turn nudge and none for a question that sits only in the chat.

use std::collections::HashMap;

use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::sent::because;

pub(crate) const STALE_AFTER_MS: i64 = 60 * 60 * 1000;

const STALE_TEXT: &str = "Your Tidewatch report has not been updated for an hour. Call update_progress now with where the work stands: the state, what is happening now, and your stages and steps.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nudge {
    Stale,
}

impl Nudge {
    pub(crate) fn because(self) -> &'static str {
        match self {
            Self::Stale => because::REPORT_STALE,
        }
    }
}

pub(crate) struct Facts<'a> {
    pub(crate) idle: bool,
    pub(crate) ledger: Option<&'a Ledger>,
    pub(crate) saved_ms: Option<i64>,
    pub(crate) now_ms: i64,
}

/// The saved reports each nudge was already told about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Told {
    stale: Option<i64>,
}

fn key(saved_ms: Option<i64>) -> i64 {
    saved_ms.unwrap_or(0)
}

pub(crate) fn due(facts: &Facts<'_>, told: &Told) -> Option<Nudge> {
    let ledger = facts.ledger.filter(|_| facts.idle)?;
    let saved = key(facts.saved_ms);
    (stale_in(facts, ledger).is_some_and(|wait| wait <= 0) && told.stale != Some(saved))
        .then_some(Nudge::Stale)
}

pub(crate) fn text(nudge: Nudge) -> &'static str {
    match nudge {
        Nudge::Stale => STALE_TEXT,
    }
}

fn stale_in(facts: &Facts<'_>, ledger: &Ledger) -> Option<i64> {
    let saved = facts.saved_ms.filter(|_| ledger.status.state == Some(State::Working))?;
    Some(saved + STALE_AFTER_MS - facts.now_ms)
}

pub(crate) fn wait_ms(facts: &Facts<'_>, told: &Told) -> Option<i64> {
    let ledger = facts.ledger?;
    let ledger = facts.idle.then_some(ledger)?;
    let wait = stale_in(facts, ledger)?;
    (wait > 0 && told.stale != Some(key(facts.saved_ms))).then_some(wait)
}

#[derive(Debug, Default)]
pub(crate) struct Nudges {
    told: HashMap<u64, Told>,
}

impl Nudges {
    pub(crate) fn told(&self, tab: u64) -> Told {
        self.told.get(&tab).copied().unwrap_or_default()
    }

    pub(crate) fn mark(&mut self, tab: u64, nudge: Nudge, saved_ms: Option<i64>) {
        let told = self.told.entry(tab).or_default();
        match nudge {
            Nudge::Stale => told.stale = Some(key(saved_ms)),
        }
    }

    pub(crate) fn forget_except(&mut self, keep: impl Fn(u64) -> bool) {
        self.told.retain(|tab, _| keep(*tab));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVED: i64 = 1_000_000;
    const HOUR: i64 = STALE_AFTER_MS;

    fn ledger(state: &str, needs: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\ng\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"))
    }

    fn facts(ledger: &Ledger, age_ms: i64, idle: bool) -> Facts<'_> {
        Facts { idle, ledger: Some(ledger), saved_ms: Some(SAVED), now_ms: SAVED + age_ms }
    }

    #[test]
    fn a_working_report_gone_quiet_for_an_hour_nudges_an_idle_agent_once() {
        let working = ledger("working", "- nothing");
        let told = Told::default();
        assert_eq!(due(&facts(&working, HOUR, true), &told), Some(Nudge::Stale));
        assert_eq!(due(&facts(&working, HOUR - 1, true), &told), None);
        assert_eq!(due(&facts(&working, 5 * HOUR, true), &told), Some(Nudge::Stale));
    }

    #[test]
    fn the_stale_nudge_waits_for_an_idle_agent_and_a_working_report() {
        let cases = [
            ("a waiting report", ledger("waiting", "- nothing"), true, None),
            ("a blocked report", ledger("blocked", "- nothing"), true, None),
            ("a done report", ledger("done", "- nothing"), true, None),
            ("a working report", ledger("working", "- nothing"), true, Some(Nudge::Stale)),
            ("a working report, agent not idle", ledger("working", "- nothing"), false, None),
        ];
        for (name, ledger, idle, want) in cases {
            assert_eq!(due(&facts(&ledger, 2 * HOUR, idle), &Told::default()), want, "{name}");
        }
    }

    #[test]
    fn an_agent_that_works_is_never_asked_for_its_report_however_old_it_is() {
        let working = ledger("working", "- nothing");
        for hours in [1, 3, 24] {
            let mid_turn = facts(&working, hours * HOUR, false);
            assert_eq!(due(&mid_turn, &Told::default()), None, "{hours}h");
            assert_eq!(wait_ms(&mid_turn, &Told::default()), None, "{hours}h");
        }
    }

    #[test]
    fn it_is_not_repeated_until_a_new_report_is_saved() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        let old = facts(&working, 2 * HOUR, true);
        nudges.mark(7, Nudge::Stale, old.saved_ms);
        assert_eq!(due(&old, &nudges.told(7)), None);
        let saved_again = Facts { saved_ms: Some(SAVED + 10), now_ms: SAVED + 10 + HOUR, ..old };
        assert_eq!(due(&saved_again, &nudges.told(7)), Some(Nudge::Stale));
        assert_eq!(due(&old, &nudges.told(8)), Some(Nudge::Stale), "another tab is not told");
    }

    #[test]
    fn nothing_known_about_the_save_time_is_never_stale() {
        let working = ledger("working", "- nothing");
        let facts = Facts { saved_ms: None, ..facts(&working, 9 * HOUR, true) };
        assert_eq!(due(&facts, &Told::default()), None);
    }

    #[test]
    fn the_wake_is_the_time_left_for_an_idle_working_report_not_told_yet() {
        let working = ledger("working", "- nothing");
        assert_eq!(
            wait_ms(&facts(&working, 10 * 60_000, true), &Told::default()),
            Some(50 * 60_000)
        );
        assert_eq!(wait_ms(&facts(&working, 2 * HOUR, true), &Told::default()), None);
        assert_eq!(wait_ms(&facts(&working, 10, false), &Told::default()), None);
        let mut nudges = Nudges::default();
        nudges.mark(1, Nudge::Stale, Some(SAVED));
        assert_eq!(wait_ms(&facts(&working, 10, true), &nudges.told(1)), None);
    }

    #[test]
    fn the_text_names_the_tool_and_is_plain_ascii_and_the_reason_says_why() {
        let said = text(Nudge::Stale);
        assert!(said.is_ascii() && said.contains("update_progress"), "{said}");
        assert!(!said.contains('\n'));
        assert!(Nudge::Stale.because().starts_with("the report said working"));
    }

    #[test]
    fn a_closed_tab_is_forgotten() {
        let mut nudges = Nudges::default();
        nudges.mark(1, Nudge::Stale, Some(5));
        nudges.mark(2, Nudge::Stale, Some(5));
        nudges.forget_except(|tab| tab == 2);
        assert_eq!(nudges.told(1), Told::default());
        assert_ne!(nudges.told(2), Told::default());
    }
}
