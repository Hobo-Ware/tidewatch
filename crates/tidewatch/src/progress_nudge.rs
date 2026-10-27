//! The nudges Tidewatch sends about the progress report (pure, table-tested): gone quiet after a
//! turn, gone stale mid-turn, and a question the report does not carry as `needs_you`.

use std::collections::HashMap;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::progress_report::WaitingOn;
use tidewatch_core::sent::because;

use crate::reader_turns::duration_label;

pub(crate) const STALE_AFTER_MS: i64 = 60 * 60 * 1000;
pub(crate) const QUIET_FIRST_MS: i64 = 15 * 60 * 1000;
pub(crate) const QUIET_REPEAT_MS: i64 = 30 * 60 * 1000;
pub(crate) const QUIET_MAX: u32 = 4;
pub(crate) const QUIET_RETRY_MS: i64 = 15 * 1000;

const STALE_TEXT: &str = "Your Tidewatch report has not been updated for an hour. Call update_progress now with where the work stands: the state, what is happening now, and your stages and steps.";

const QUIET_ASK: &str = "You are still working, so call update_progress now with fresh numbers: the state, what is happening now, and where your stages and steps stand.";

const QUESTION_TEXT: &str = "Your last message asks the user something, but your Tidewatch report has no needs_you. Call update_progress with state needs_you and the question, 2 to 4 options and the one you recommend, so the user sees it as buttons.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nudge {
    Question,
    Stale,
    Quiet,
}

impl Nudge {
    pub(crate) fn because(self) -> &'static str {
        match self {
            Self::Question => because::REPORT_QUESTION,
            Self::Stale => because::REPORT_STALE,
            Self::Quiet => because::REPORT_QUIET,
        }
    }
}

pub(crate) struct Facts<'a> {
    pub(crate) idle: bool,
    pub(crate) busy: bool,
    pub(crate) ledger: Option<&'a Ledger>,
    pub(crate) saved_ms: Option<i64>,
    pub(crate) turn_started_ms: Option<i64>,
    pub(crate) block: Option<&'a EndBlock>,
    pub(crate) now_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quiet {
    saved: i64,
    sent: u32,
    last_ms: i64,
    retry_ms: i64,
}

/// The saved reports each nudge was already told about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Told {
    question: Option<i64>,
    stale: Option<i64>,
    quiet: Option<Quiet>,
}

impl Told {
    pub(crate) fn quiet_sent(&self, saved_ms: Option<i64>) -> u32 {
        self.quiet_of(key(saved_ms)).sent
    }

    fn quiet_of(&self, saved: i64) -> Quiet {
        self.quiet.filter(|quiet| quiet.saved == saved).unwrap_or(Quiet {
            saved,
            sent: 0,
            last_ms: 0,
            retry_ms: 0,
        })
    }
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

pub(crate) fn due(facts: &Facts<'_>, told: &Told) -> Option<Nudge> {
    let ledger = facts.ledger?;
    if facts.busy {
        return quiet_wait(facts, ledger, told)
            .is_some_and(|wait| wait <= 0)
            .then_some(Nudge::Quiet);
    }
    let ledger = facts.idle.then_some(ledger)?;
    let saved = key(facts.saved_ms);
    let unreported = !ledger.has_question() && ledger.status.state != Some(State::NeedsYou);
    if unreported && facts.block.is_some_and(asks_in_chat) && told.question != Some(saved) {
        return Some(Nudge::Question);
    }
    (stale_in(facts, ledger).is_some_and(|wait| wait <= 0) && told.stale != Some(saved))
        .then_some(Nudge::Stale)
}

pub(crate) fn text(nudge: Nudge, facts: &Facts<'_>) -> String {
    match nudge {
        Nudge::Question => QUESTION_TEXT.to_owned(),
        Nudge::Stale => STALE_TEXT.to_owned(),
        Nudge::Quiet => quiet_text(facts),
    }
}

fn quiet_text(facts: &Facts<'_>) -> String {
    let age = facts
        .saved_ms
        .map_or_else(String::new, |saved| format!(" {} ago", duration_label(facts.now_ms - saved)));
    let now = facts.ledger.map_or_else(String::new, |ledger| {
        ledger.status.note.split_whitespace().collect::<Vec<_>>().join(" ")
    });
    let says = if now.is_empty() { String::new() } else { format!(" and still says \"{now}\"") };
    format!("Your Tidewatch report was last updated{age}{says}. {QUIET_ASK}")
}

fn stale_in(facts: &Facts<'_>, ledger: &Ledger) -> Option<i64> {
    let saved = facts.saved_ms.filter(|_| ledger.status.state == Some(State::Working))?;
    Some(saved + STALE_AFTER_MS - facts.now_ms)
}

fn reports_work(ledger: &Ledger) -> bool {
    match ledger.status.state {
        Some(State::Working) => true,
        Some(State::Waiting) => !matches!(ledger.waiting_on, Some(WaitingOn::Date(_))),
        _ => false,
    }
}

fn quiet_wait(facts: &Facts<'_>, ledger: &Ledger, told: &Told) -> Option<i64> {
    if !reports_work(ledger) {
        return None;
    }
    let saved = facts.saved_ms?;
    let quiet = told.quiet_of(saved);
    if quiet.sent >= QUIET_MAX {
        return None;
    }
    let base = if quiet.sent == 0 {
        saved.max(facts.turn_started_ms.unwrap_or(saved)) + QUIET_FIRST_MS
    } else {
        quiet.last_ms + QUIET_REPEAT_MS
    };
    Some(base.max(quiet.retry_ms) - facts.now_ms)
}

pub(crate) fn wait_ms(facts: &Facts<'_>, told: &Told) -> Option<i64> {
    let ledger = facts.ledger?;
    if facts.busy {
        return quiet_wait(facts, ledger, told).filter(|wait| *wait > 0);
    }
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

    pub(crate) fn mark(&mut self, tab: u64, nudge: Nudge, saved_ms: Option<i64>, now_ms: i64) {
        let told = self.told.entry(tab).or_default();
        match nudge {
            Nudge::Question => told.question = Some(key(saved_ms)),
            Nudge::Stale => told.stale = Some(key(saved_ms)),
            Nudge::Quiet => {
                let quiet = told.quiet_of(key(saved_ms));
                told.quiet =
                    Some(Quiet { sent: quiet.sent + 1, last_ms: now_ms, retry_ms: 0, ..quiet });
            }
        }
    }

    pub(crate) fn retry_later(&mut self, tab: u64, saved_ms: Option<i64>, now_ms: i64) {
        let told = self.told.entry(tab).or_default();
        let quiet = told.quiet_of(key(saved_ms));
        told.quiet = Some(Quiet { retry_ms: now_ms + QUIET_RETRY_MS, ..quiet });
    }

    pub(crate) fn forget_except(&mut self, keep: impl Fn(u64) -> bool) {
        self.told.retain(|tab, _| keep(*tab));
    }
}

#[cfg(test)]
mod tests {
    use tidewatch_core::civil_date::Date;

    use super::*;

    const SAVED: i64 = 1_000_000;
    const MIN: i64 = 60 * 1000;
    const HOUR: i64 = STALE_AFTER_MS;

    fn ledger(state: &str, needs: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\ng\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"))
    }

    fn with_now(state: &str, now: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\ng\n\n## Status\nstate: {state}\nnote: {now}\n"))
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
        Facts {
            idle,
            busy: false,
            ledger: Some(ledger),
            saved_ms: Some(SAVED),
            turn_started_ms: None,
            block,
            now_ms: SAVED + age_ms,
        }
    }

    fn busy(ledger: &Ledger, age_ms: i64) -> Facts<'_> {
        Facts { idle: false, busy: true, ..facts(ledger, None, age_ms, false) }
    }

    #[test]
    fn a_working_tab_polling_a_long_job_with_a_three_hour_old_report_is_nudged_unasked() {
        let working = with_now(
            "working",
            "Waiting on the gemma link check (task b39akfvw7), 3,736 of 6,178 rows at 12:36",
        );
        let polling = busy(&working, 3 * HOUR + 38 * MIN);
        assert_eq!(due(&polling, &Told::default()), Some(Nudge::Quiet));
        let said = text(Nudge::Quiet, &polling);
        assert!(said.contains("3h 38m ago"), "{said}");
        assert!(said.contains("3,736 of 6,178 rows at 12:36"), "{said}");
        assert!(said.contains("update_progress") && said.contains("fresh numbers"), "{said}");
    }

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
    fn the_first_mid_turn_nudge_is_due_after_fifteen_minutes() {
        let working = ledger("working", "- nothing");
        let told = Told::default();
        assert_eq!(due(&busy(&working, QUIET_FIRST_MS - 1), &told), None);
        assert_eq!(due(&busy(&working, QUIET_FIRST_MS), &told), Some(Nudge::Quiet));
        assert_eq!(wait_ms(&busy(&working, 5 * MIN), &told), Some(10 * MIN));
        assert_eq!(wait_ms(&busy(&working, QUIET_FIRST_MS), &told), None);
    }

    #[test]
    fn only_a_working_agent_with_a_report_that_says_work_is_nudged_mid_turn() {
        let waiting_on = |line: &str| {
            let mut report = ledger("waiting", "- nothing");
            report.waiting_on = Some(match line {
                "job" => WaitingOn::Job("link check".into()),
                "pr" => WaitingOn::Pr(7),
                "event" => WaitingOn::Event("the deploy".into()),
                _ => WaitingOn::Date(Date::parse("2030-01-02").unwrap()),
            });
            report
        };
        let cases = [
            ("working", ledger("working", "- nothing"), Some(Nudge::Quiet)),
            ("waiting on nothing named", ledger("waiting", "- nothing"), Some(Nudge::Quiet)),
            ("waiting on a job", waiting_on("job"), Some(Nudge::Quiet)),
            ("waiting on a pull request", waiting_on("pr"), Some(Nudge::Quiet)),
            ("waiting on an event", waiting_on("event"), Some(Nudge::Quiet)),
            ("waiting for a date", waiting_on("date"), None),
            ("needs you", ledger("needs_you", "- nothing"), None),
            ("blocked", ledger("blocked", "- nothing"), None),
            ("done", ledger("done", "- nothing"), None),
            ("no state", Ledger::parse("# Goal\ng\n"), None),
        ];
        for (name, report, want) in cases {
            assert_eq!(due(&busy(&report, 2 * HOUR), &Told::default()), want, "{name}");
        }
    }

    #[test]
    fn an_agent_that_is_neither_busy_nor_idle_and_a_tab_with_no_report_are_left_alone() {
        let working = ledger("working", "- nothing");
        let asking = block(&["Pause", "Go on"]);
        let parked =
            Facts { idle: false, busy: false, ..facts(&working, Some(&asking), 2 * HOUR, true) };
        assert_eq!(due(&parked, &Told::default()), None);
        let none = Facts { ledger: None, ..busy(&working, 2 * HOUR) };
        assert_eq!(due(&none, &Told::default()), None);
        let unsaved = Facts { saved_ms: None, ..busy(&working, 9 * HOUR) };
        assert_eq!(due(&unsaved, &Told::default()), None);
    }

    #[test]
    fn a_busy_agent_is_never_asked_about_a_question() {
        let working = ledger("working", "- nothing");
        let asking = block(&["Pause", "Go on"]);
        let mid_turn = Facts { block: Some(&asking), ..busy(&working, 0) };
        assert_eq!(due(&mid_turn, &Told::default()), None);
    }

    #[test]
    fn the_first_nudge_counts_from_the_start_of_the_turn_when_that_is_later_than_the_save() {
        let working = ledger("working", "- nothing");
        let resumed =
            Facts { turn_started_ms: Some(SAVED + 5 * HOUR), ..busy(&working, 5 * HOUR + MIN) };
        assert_eq!(due(&resumed, &Told::default()), None);
        let later = Facts { now_ms: SAVED + 5 * HOUR + QUIET_FIRST_MS, ..resumed };
        assert_eq!(due(&later, &Told::default()), Some(Nudge::Quiet));
        let before_save =
            Facts { turn_started_ms: Some(SAVED - HOUR), ..busy(&working, QUIET_FIRST_MS) };
        assert_eq!(due(&before_save, &Told::default()), Some(Nudge::Quiet));
    }

    #[test]
    fn the_mid_turn_nudge_repeats_on_a_backoff_up_to_a_cap() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        let mut sent_at = Vec::new();
        let mut age = 0;
        while age < 24 * HOUR {
            age += MIN;
            let now = busy(&working, age);
            if due(&now, &nudges.told(1)) == Some(Nudge::Quiet) {
                nudges.mark(1, Nudge::Quiet, now.saved_ms, now.now_ms);
                sent_at.push(age / MIN);
            }
        }
        assert_eq!(sent_at, [15, 45, 75, 105]);
        assert_eq!(nudges.told(1).quiet_sent(Some(SAVED)), QUIET_MAX);
        assert_eq!(wait_ms(&busy(&working, 24 * HOUR), &nudges.told(1)), None);
    }

    #[test]
    fn a_fresh_report_starts_the_mid_turn_clock_again() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        for sent in 0..QUIET_MAX {
            nudges.mark(1, Nudge::Quiet, Some(SAVED), SAVED + i64::from(sent));
        }
        assert_eq!(due(&busy(&working, 9 * HOUR), &nudges.told(1)), None);
        let saved_again = Facts {
            saved_ms: Some(SAVED + 9 * HOUR),
            now_ms: SAVED + 9 * HOUR + QUIET_FIRST_MS - 1,
            ..busy(&working, 0)
        };
        assert_eq!(due(&saved_again, &nudges.told(1)), None);
        let due_again = Facts { now_ms: saved_again.now_ms + 1, ..saved_again };
        assert_eq!(due(&due_again, &nudges.told(1)), Some(Nudge::Quiet));
        assert_eq!(nudges.told(1).quiet_sent(due_again.saved_ms), 0);
        assert_eq!(
            due(&busy(&working, 9 * HOUR), &nudges.told(2)),
            Some(Nudge::Quiet),
            "another tab"
        );
    }

    #[test]
    fn a_nudge_that_had_to_wait_is_tried_again_in_fifteen_seconds_without_counting() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        let now = busy(&working, 20 * MIN);
        assert_eq!(due(&now, &nudges.told(1)), Some(Nudge::Quiet));
        nudges.retry_later(1, now.saved_ms, now.now_ms);
        assert_eq!(due(&now, &nudges.told(1)), None);
        assert_eq!(wait_ms(&now, &nudges.told(1)), Some(QUIET_RETRY_MS));
        let again = Facts { now_ms: now.now_ms + QUIET_RETRY_MS, ..now };
        assert_eq!(due(&again, &nudges.told(1)), Some(Nudge::Quiet));
        assert_eq!(nudges.told(1).quiet_sent(again.saved_ms), 0);
    }

    #[test]
    fn the_mid_turn_text_names_the_age_and_the_now_line_in_plain_words() {
        let cases = [
            ("a long wait", 3 * HOUR, "last updated 3h ago and still says \"Polling the build\"."),
            ("a short one", 16 * MIN, "last updated 16m ago and still says \"Polling the build\"."),
        ];
        for (name, age, want) in cases {
            let working = with_now("working", "Polling   the build");
            let said = text(Nudge::Quiet, &busy(&working, age));
            assert!(said.contains(want), "{name}: {said}");
            assert!(!said.contains('\n'), "{name}");
        }
        let blank = ledger("working", "- nothing");
        let said = text(Nudge::Quiet, &busy(&blank, 20 * MIN));
        assert!(
            said.starts_with("Your Tidewatch report was last updated 20m ago. You are"),
            "{said}"
        );
    }

    #[test]
    fn it_is_not_repeated_until_a_new_report_is_saved() {
        let working = ledger("working", "- nothing");
        let mut nudges = Nudges::default();
        let old = facts(&working, None, 2 * HOUR, true);
        nudges.mark(7, Nudge::Stale, old.saved_ms, old.now_ms);
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
        let working = ledger("working", "- nothing");
        for nudge in [Nudge::Question, Nudge::Stale, Nudge::Quiet] {
            let said = text(nudge, &busy(&working, HOUR));
            assert!(said.is_ascii() && said.contains("update_progress"), "{said}");
            assert!(!said.contains('\n'));
        }
        assert!(text(Nudge::Question, &busy(&working, 0)).contains("needs_you"));
        assert!(Nudge::Stale.because().starts_with("the report said working"));
        assert!(Nudge::Quiet.because().starts_with("the agent kept working"));
    }

    #[test]
    fn a_closed_tab_is_forgotten() {
        let mut nudges = Nudges::default();
        nudges.mark(1, Nudge::Stale, Some(5), 9);
        nudges.mark(2, Nudge::Question, Some(5), 9);
        nudges.forget_except(|tab| tab == 2);
        assert_eq!(nudges.told(1), Told::default());
        assert_ne!(nudges.told(2), Told::default());
    }
}
