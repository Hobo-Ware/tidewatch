//! The two nudges Tidewatch owns about the progress report (pure, table-tested): a report that said
//! working and then went quiet for an hour, and a question in the agent's last message that the
//! report does not carry as `needs_you`. Each is told once per saved report: a new report is what
//! makes the agent eligible again.

use std::collections::HashMap;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::sent::because;

pub(crate) const STALE_AFTER_MS: i64 = 60 * 60 * 1000;

const STALE_TEXT: &str = "Your Tidewatch report has not been updated for an hour. Call update_progress now with where the work stands: the state, what is happening now, and your stages and steps.";

const QUESTION_TEXT: &str = "Your last message asks the user something, but your Tidewatch report has no needs_you. Call update_progress with state needs_you and the question, 2 to 4 options and the one you recommend, so the user sees it as buttons.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nudge {
    Question,
    Stale,
}

impl Nudge {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Question => QUESTION_TEXT,
            Self::Stale => STALE_TEXT,
        }
    }

    pub(crate) fn because(self) -> &'static str {
        match self {
            Self::Question => because::REPORT_QUESTION,
            Self::Stale => because::REPORT_STALE,
        }
    }
}

pub(crate) struct Facts<'a> {
    pub(crate) idle: bool,
    pub(crate) ledger: Option<&'a Ledger>,
    pub(crate) saved_ms: Option<i64>,
    pub(crate) block: Option<&'a EndBlock>,
    pub(crate) now_ms: i64,
}

/// The saved reports each nudge was already told about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Told {
    question: Option<i64>,
    stale: Option<i64>,
}

fn key(saved_ms: Option<i64>) -> i64 {
    saved_ms.unwrap_or(0)
}

/// The Needs you section lists several answers, or asks with a question mark.
pub(crate) fn asks_in_chat(block: &EndBlock) -> bool {
    let items: Vec<&String> =
        block.needs_you.iter().filter(|item| !item.trim().is_empty()).collect();
    block.needs_input() && (items.len() >= 2 || items.iter().any(|item| item.contains('?')))
}

/// What to tell an idle agent, if anything. A question that is not in the report comes first.
pub(crate) fn due(facts: &Facts<'_>, told: &Told) -> Option<Nudge> {
    let ledger = facts.ledger.filter(|_| facts.idle)?;
    let saved = key(facts.saved_ms);
    let unreported = !ledger.has_question() && ledger.status.state != Some(State::NeedsYou);
    if unreported && facts.block.is_some_and(asks_in_chat) && told.question != Some(saved) {
        return Some(Nudge::Question);
    }
    (stale_in(facts, ledger).is_some_and(|wait| wait <= 0) && told.stale != Some(saved))
        .then_some(Nudge::Stale)
}

fn stale_in(facts: &Facts<'_>, ledger: &Ledger) -> Option<i64> {
    let saved = facts.saved_ms.filter(|_| ledger.status.state == Some(State::Working))?;
    Some(saved + STALE_AFTER_MS - facts.now_ms)
}

/// Milliseconds until an idle agent whose report says working turns stale and was not told yet.
pub(crate) fn wait_ms(facts: &Facts<'_>, told: &Told) -> Option<i64> {
    let ledger = facts.ledger.filter(|_| facts.idle)?;
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
            Nudge::Question => told.question = Some(key(saved_ms)),
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

    fn ledger(state: &str, needs: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\ng\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"))
    }

    fn block(needs: &[&str]) -> EndBlock {
        EndBlock {
            done: vec!["did it".into()],
            needs_you: needs.iter().map(|item| (*item).to_owned()).collect(),
            next: Vec::new(),
        }
    }

    fn facts<'a>(
        ledger: &'a Ledger,
        block: Option<&'a EndBlock>,
        age_ms: i64,
        idle: bool,
    ) -> Facts<'a> {
        Facts { idle, ledger: Some(ledger), saved_ms: Some(SAVED), block, now_ms: SAVED + age_ms }
    }

    const HOUR: i64 = STALE_AFTER_MS;

    #[test]
    fn a_working_report_gone_quiet_for_an_hour_nudges_an_idle_agent_once() {
        let working = ledger("working", "- nothing");
        let told = Told::default();
        assert_eq!(due(&facts(&working, None, HOUR, true), &told), Some(Nudge::Stale));
        assert_eq!(due(&facts(&working, None, HOUR - 1, true), &told), None);
        assert_eq!(due(&facts(&working, None, 5 * HOUR, true), &told), Some(Nudge::Stale));
    }

    #[test]
    fn the_stale_nudge_waits_for_an_idle_agent_and_a_working_report() {
        let cases = [
            ("a busy agent", ledger("working", "- nothing"), false, None),
            ("a waiting report", ledger("waiting", "- nothing"), true, None),
            ("a blocked report", ledger("blocked", "- nothing"), true, None),
            ("a done report", ledger("done", "- nothing"), true, None),
            ("a working report", ledger("working", "- nothing"), true, Some(Nudge::Stale)),
        ];
        for (name, ledger, idle, want) in cases {
            assert_eq!(
                due(&facts(&ledger, None, 2 * HOUR, idle), &Told::default()),
                want,
                "{name}"
            );
        }
    }

    #[test]
    fn it_is_not_repeated_until_a_new_report_is_saved() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        let old = facts(&working, None, 2 * HOUR, true);
        nudges.mark(7, Nudge::Stale, old.saved_ms);
        assert_eq!(due(&old, &nudges.told(7)), None);
        let saved_again = Facts { saved_ms: Some(SAVED + 10), now_ms: SAVED + 10 + HOUR, ..old };
        assert_eq!(due(&saved_again, &nudges.told(7)), Some(Nudge::Stale));
        assert_eq!(due(&old, &nudges.told(8)), Some(Nudge::Stale), "another tab is not told");
    }

    #[test]
    fn nothing_known_about_the_save_time_is_never_stale() {
        let working = ledger("working", "- nothing");
        let facts = Facts { saved_ms: None, ..facts(&working, None, 9 * HOUR, true) };
        assert_eq!(due(&facts, &Told::default()), None);
    }

    #[test]
    fn a_question_in_the_last_message_that_the_report_lacks_nudges_at_once() {
        let working = ledger("working", "- nothing");
        let asking = block(&["Pause after step 2", "Let it all run"]);
        assert_eq!(
            due(&facts(&working, Some(&asking), 0, true), &Told::default()),
            Some(Nudge::Question)
        );
        let marked = Told { question: Some(SAVED), ..Told::default() };
        assert_eq!(due(&facts(&working, Some(&asking), 0, true), &marked), None);
    }

    #[test]
    fn a_question_the_report_already_carries_is_not_nudged() {
        let asking = block(&["Pause or go on?"]);
        let carried = [
            ledger("needs_you", "- Pause or go on? -- options: Pause* | Go on"),
            ledger("working", "- Pause or go on? -- options: Pause* | Go on"),
            ledger("needs_you", "- nothing"),
        ];
        for ledger in carried {
            assert_eq!(due(&facts(&ledger, Some(&asking), 0, true), &Told::default()), None);
        }
    }

    #[test]
    fn what_counts_as_a_question_in_the_end_block() {
        let cases = [
            ("two options", block(&["Pause", "Go on"]), true),
            ("one line with a question mark", block(&["Which base should I use?"]), true),
            ("one plain request", block(&["Review the pull request"]), false),
            ("nothing", block(&["nothing"]), false),
            ("an empty section", block(&[]), false),
        ];
        for (name, block, want) in cases {
            assert_eq!(asks_in_chat(&block), want, "{name}");
        }
    }

    #[test]
    fn a_busy_agent_and_a_tab_with_no_report_are_left_alone() {
        let working = ledger("working", "- nothing");
        let asking = block(&["Pause", "Go on"]);
        assert_eq!(due(&facts(&working, Some(&asking), 2 * HOUR, false), &Told::default()), None);
        let none = Facts { ledger: None, ..facts(&working, Some(&asking), 2 * HOUR, true) };
        assert_eq!(due(&none, &Told::default()), None);
    }

    #[test]
    fn the_question_comes_before_the_stale_report() {
        let working = ledger("working", "- nothing");
        let asking = block(&["Pause", "Go on"]);
        let both = facts(&working, Some(&asking), 2 * HOUR, true);
        assert_eq!(due(&both, &Told::default()), Some(Nudge::Question));
        let told_question = Told { question: Some(SAVED), ..Told::default() };
        assert_eq!(due(&both, &told_question), Some(Nudge::Stale));
    }

    #[test]
    fn the_wake_is_the_time_left_for_an_idle_working_report_not_told_yet() {
        let working = ledger("working", "- nothing");
        assert_eq!(
            wait_ms(&facts(&working, None, 10 * 60_000, true), &Told::default()),
            Some(50 * 60_000)
        );
        assert_eq!(wait_ms(&facts(&working, None, 2 * HOUR, true), &Told::default()), None);
        assert_eq!(wait_ms(&facts(&working, None, 10, false), &Told::default()), None);
        let told = Told { stale: Some(SAVED), ..Told::default() };
        assert_eq!(wait_ms(&facts(&working, None, 10, true), &told), None);
    }

    #[test]
    fn the_texts_name_the_tool_the_field_and_are_plain_ascii() {
        for nudge in [Nudge::Question, Nudge::Stale] {
            assert!(nudge.text().is_ascii() && nudge.text().contains("update_progress"));
            assert!(!nudge.text().contains('\n'));
        }
        assert!(Nudge::Question.text().contains("needs_you"));
        assert!(Nudge::Stale.because().starts_with("the report said working"));
    }

    #[test]
    fn a_closed_tab_is_forgotten() {
        let mut nudges = Nudges::default();
        nudges.mark(1, Nudge::Stale, Some(5));
        nudges.mark(2, Nudge::Question, Some(5));
        nudges.forget_except(|tab| tab == 2);
        assert_eq!(nudges.told(1), Told::default());
        assert_ne!(nudges.told(2), Told::default());
    }
}
