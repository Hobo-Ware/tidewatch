//! The done check card as data: the outcome, what passed and one line for each check that failed,
//! could not run, does not apply or waits for a later date. [`card_of`] is the data and
//! [`stamp_text`] the note of when it was made; the Progress panel's Last turn section
//! (`last_turn_view.rs`) draws them.

use jiff::tz::TimeZone;
use tidewatch_core::civil_date::Date;
use tidewatch_core::done_check::{Kind, Line, Outcome, Report, Status, Unverified};
use tidewatch_core::pr_status::Tone;

use crate::job_model::clock_label;
use crate::park::date_text;

const MAX_ITEMS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) tone: Tone,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) title: &'static str,
    pub(crate) tone: Tone,
    pub(crate) summary: String,
    pub(crate) rows: Vec<Row>,
    pub(crate) ask_fix: bool,
    pub(crate) check_again: bool,
}

pub(crate) fn tone_of(outcome: Outcome) -> Tone {
    match outcome {
        Outcome::Done => Tone::Good,
        Outcome::Blocked => Tone::Bad,
        Outcome::NotVerified => Tone::Warn,
        Outcome::Waiting => Tone::Waiting,
    }
}

pub(crate) fn title_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Done",
        Outcome::Blocked => "Blocked",
        Outcome::NotVerified => "Not verified",
        Outcome::Waiting => "Waiting",
    }
}

pub(crate) fn tag_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Check passed",
        Outcome::Blocked => "Check blocked",
        Outcome::NotVerified => "Not verified",
        Outcome::Waiting => "Waiting",
    }
}

fn line_row(line: &Line) -> Option<Row> {
    let label = line.kind.label();
    match line.status {
        Status::Pass | Status::Waiting => None,
        Status::Fail => Some(Row { tone: Tone::Bad, text: format!("{label}: {}", line.detail) }),
        Status::Unknown => {
            Some(Row { tone: Tone::Warn, text: format!("{label}: {}", line.detail) })
        }
        Status::Skipped => {
            Some(Row { tone: Tone::Quiet, text: format!("{label}: not checked, {}", line.detail) })
        }
    }
}

fn capped(mut rows: Vec<Row>, tail: &str) -> Vec<Row> {
    let more = rows.len().saturating_sub(MAX_ITEMS);
    rows.truncate(MAX_ITEMS);
    if more > 0 {
        rows.push(Row { tone: Tone::Quiet, text: format!("    and {more} more{tail}") });
    }
    rows
}

fn unverified_rows(line: &Line, items: &[Unverified], today: Date) -> Vec<Row> {
    let mut rows: Vec<Row> = line_row(line).into_iter().collect();
    let open = items
        .iter()
        .filter(|item| item.waiting_for(today).is_none())
        .map(|item| Row { tone: Tone::Quiet, text: format!("    - {}", item.text) })
        .collect();
    rows.extend(capped(open, ""));
    let waiting = items
        .iter()
        .filter_map(|item| Some((item.waiting_for(today)?, &item.text)))
        .map(|(date, text)| Row {
            tone: Tone::Waiting,
            text: format!("Waiting until {}: {text}", date_text(date)),
        })
        .collect();
    rows.extend(capped(waiting, " waiting"));
    rows
}

pub(crate) fn card_of(report: &Report, today: Date) -> Card {
    let report = report.on(today);
    let outcome = report.outcome();
    let failed = report.lines.iter().filter(|line| line.status == Status::Fail).count();
    let passed: Vec<&str> = report
        .lines
        .iter()
        .filter(|line| line.status == Status::Pass)
        .map(|line| line.detail.as_str())
        .collect();
    let summary = match (outcome, failed) {
        (Outcome::Blocked, 1) => "1 check failed".to_owned(),
        (Outcome::Blocked, n) => format!("{n} checks failed"),
        _ => passed.join(" \u{b7} "),
    };
    let mut rows = Vec::new();
    for line in &report.lines {
        if line.kind == Kind::Unverified {
            rows.extend(unverified_rows(line, &report.unverified, today));
        } else {
            rows.extend(line_row(line));
        }
    }
    Card {
        title: title_of(outcome),
        tone: tone_of(outcome),
        summary,
        rows,
        ask_fix: outcome == Outcome::Blocked,
        check_again: !matches!(outcome, Outcome::Done | Outcome::Waiting),
    }
}

pub(crate) fn stamp_text(at_secs: i64, same: bool, zone: &TimeZone) -> String {
    let clock = clock_label(at_secs.saturating_mul(1000), zone).unwrap_or_default();
    let stamp = format!("Checked {clock}").trim_end().to_owned();
    if same { format!("{stamp} \u{b7} same result") } else { stamp }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    AskFix,
    CheckAgain,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::done_check::unverified_of;
    use tidewatch_core::progress::Ledger;

    fn day(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    fn today() -> Date {
        day("2026-10-27")
    }

    fn card_of(report: &Report) -> Card {
        super::card_of(report, today())
    }

    fn line(kind: Kind, status: Status, detail: &str) -> Line {
        Line { kind, status, detail: detail.to_owned() }
    }

    fn report(lines: Vec<Line>, unverified: &[&str]) -> Report {
        Report {
            head: String::new(),
            lines,
            unverified: unverified.iter().map(|item| Unverified::new(*item)).collect(),
        }
    }

    fn texts(card: &Card) -> Vec<&str> {
        card.rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn a_green_report_is_one_line_with_what_passed() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::Pushed, Status::Pass, "origin has 3f9a1c2"),
            ],
            &[],
        ));
        assert_eq!(card.title, "Done");
        assert_eq!(card.tone, Tone::Good);
        assert_eq!(card.summary, "clean tree, 1 commit ahead of main \u{b7} origin has 3f9a1c2");
        assert_eq!(card.rows.len(), 0);
        assert_eq!((card.ask_fix, card.check_again), (false, false));
    }

    #[test]
    fn a_blocked_report_names_every_failed_check_in_one_line_and_offers_the_fix() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 2 commits ahead of main"),
                line(Kind::Pushed, Status::Fail, "2 commits not pushed"),
                line(Kind::PullRequest, Status::Fail, "PR #7 checks failing: build"),
            ],
            &[],
        ));
        assert_eq!((card.title, card.tone), ("Blocked", Tone::Bad));
        assert_eq!(card.summary, "2 checks failed");
        assert_eq!(
            texts(&card),
            ["Pushed: 2 commits not pushed", "Pull request: PR #7 checks failing: build"]
        );
        assert!(card.rows.iter().all(|row| row.tone == Tone::Bad));
        assert_eq!((card.ask_fix, card.check_again), (true, true));
    }

    #[test]
    fn checks_that_could_not_run_say_why_and_the_ledger_items_are_listed() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::PullRequest, Status::Unknown, "the GitHub CLI (gh) is not installed"),
                line(Kind::Unverified, Status::Unknown, "5 items the agent did not verify"),
            ],
            &["a", "b", "c", "d", "e"],
        ));
        assert_eq!((card.title, card.tone), ("Not verified", Tone::Warn));
        assert_eq!(
            texts(&card),
            [
                "Pull request: the GitHub CLI (gh) is not installed",
                "Not verified: 5 items the agent did not verify",
                "    - a",
                "    - b",
                "    - c",
                "    and 2 more",
            ]
        );
        assert_eq!((card.ask_fix, card.check_again), (false, true));
    }

    #[test]
    fn a_check_that_does_not_apply_shows_as_not_checked_with_the_reason() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::PullRequest, Status::Skipped, "no pull request for this branch"),
            ],
            &[],
        ));
        assert_eq!(card.title, "Done");
        assert_eq!(texts(&card), ["Pull request: not checked, no pull request for this branch"]);
        assert_eq!(card.rows[0].tone, Tone::Quiet);
    }

    fn ledger_report(not_verified: &str, scheduled: &str) -> Report {
        let ledger = Ledger::parse(&format!(
            "# Goal\nx\n\n## Status\nstate: done\n\n## Not verified\n{not_verified}\n\n## Scheduled\n{scheduled}\n"
        ));
        let items = unverified_of(&ledger);
        let (status, detail) = tidewatch_core::done_check::unverified_status(Some(&items), today());
        Report {
            head: String::new(),
            lines: vec![
                line(Kind::Committed, Status::Pass, "nothing left uncommitted"),
                line(Kind::Pushed, Status::Pass, "the pull request is merged"),
                line(Kind::PullRequest, Status::Pass, "PR #2359 merged"),
                Line { kind: Kind::Unverified, status, detail },
            ],
            unverified: items,
        }
    }

    const OWNERS_SCHEDULE: &str = "- [ ] 2026-10-28 -- Tomorrow (Oct 28), check that a test account's list picked up yesterday's watch";
    const OWNERS_ITEM: &str = "- Live list across a day boundary";

    #[test]
    fn the_owners_merged_pull_request_waits_in_blue_and_offers_no_check_again() {
        let report = ledger_report(OWNERS_ITEM, OWNERS_SCHEDULE);
        let card = card_of(&report);
        assert_eq!((card.title, card.tone), ("Waiting", Tone::Waiting));
        assert_eq!(
            card.summary,
            "nothing left uncommitted \u{b7} the pull request is merged \u{b7} PR #2359 merged"
        );
        assert_eq!(texts(&card), ["Waiting until Wed 28 Oct: Live list across a day boundary"]);
        assert_eq!(card.rows[0].tone, Tone::Waiting);
        assert_eq!((card.ask_fix, card.check_again), (false, false));
    }

    #[test]
    fn the_same_report_asks_to_be_checked_again_on_the_day_it_is_due() {
        let report = ledger_report(OWNERS_ITEM, OWNERS_SCHEDULE);
        let due = super::card_of(&report, day("2026-10-28"));
        assert_eq!((due.title, due.tone), ("Not verified", Tone::Warn));
        assert_eq!(
            texts(&due),
            [
                "Not verified: 1 item the agent did not verify",
                "    - Live list across a day boundary"
            ]
        );
        assert!(due.check_again);
    }

    #[test]
    fn an_unscheduled_item_stays_unverified_beside_a_waiting_one() {
        let report = ledger_report(&format!("{OWNERS_ITEM}\n- the Safari layout"), OWNERS_SCHEDULE);
        let card = card_of(&report);
        assert_eq!((card.title, card.tone), ("Not verified", Tone::Warn));
        assert_eq!(
            texts(&card),
            [
                "Not verified: 1 item the agent did not verify",
                "    - the Safari layout",
                "Waiting until Wed 28 Oct: Live list across a day boundary",
            ]
        );
        assert_eq!(
            card.rows.iter().map(|row| row.tone).collect::<Vec<_>>(),
            [Tone::Warn, Tone::Quiet, Tone::Waiting]
        );
        assert!(card.check_again);
    }

    #[test]
    fn an_item_with_no_schedule_reads_as_before() {
        let card = card_of(&ledger_report("- the Safari layout", ""));
        assert_eq!((card.title, card.tone), ("Not verified", Tone::Warn));
        assert_eq!(
            texts(&card),
            ["Not verified: 1 item the agent did not verify", "    - the Safari layout"]
        );
    }

    #[test]
    fn a_failure_outranks_waiting_and_a_check_that_could_not_run_keeps_the_button() {
        let mut report = ledger_report(OWNERS_ITEM, OWNERS_SCHEDULE);
        report.lines[2] =
            line(Kind::PullRequest, Status::Unknown, "PR #7 checks still running (1 of 2)");
        let card = card_of(&report);
        assert_eq!(card.title, "Not verified");
        assert!(card.check_again);
        report.lines[1] = line(Kind::Pushed, Status::Fail, "2 commits not pushed");
        let card = card_of(&report);
        assert_eq!((card.title, card.ask_fix, card.check_again), ("Blocked", true, true));
    }

    #[test]
    fn many_waiting_items_are_capped_with_a_count() {
        let lines = |make: fn(u32) -> String| (1..=5).map(make).collect::<Vec<_>>().join("\n");
        let schedule = lines(|n| format!("- [ ] 2026-10-28 -- thing{n} topic{n}"));
        let items = lines(|n| format!("- the topic{n}"));
        let card = card_of(&ledger_report(&items, &schedule));
        assert_eq!(card.title, "Waiting");
        assert_eq!(card.rows.len(), 4);
        assert_eq!(card.rows[3].text, "    and 2 more waiting");
    }

    #[test]
    fn the_stamp_gives_the_clock_and_says_when_nothing_changed() {
        let zone = TimeZone::UTC;
        let at = 1_793_142_060;
        assert_eq!(stamp_text(at, false, &zone), "Checked 23:01");
        assert_eq!(stamp_text(at, true, &zone), "Checked 23:01 \u{b7} same result");
        let plus_two = TimeZone::fixed(jiff::tz::offset(2));
        assert_eq!(stamp_text(at, false, &plus_two), "Checked 01:01");
    }
}
