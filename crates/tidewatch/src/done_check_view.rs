//! The done check card as data: the outcome, what passed and one line for each check that failed,
//! could not run or does not apply. [`card_of`] is the data; the Progress panel's Last turn section
//! (`last_turn_view.rs`) draws it.

use tidewatch_core::done_check::{Kind, Outcome, Report, Status};
use tidewatch_core::pr_status::Tone;

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
    }
}

pub(crate) fn title_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Done",
        Outcome::Blocked => "Blocked",
        Outcome::NotVerified => "Not verified",
    }
}

pub(crate) fn tag_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Check passed",
        Outcome::Blocked => "Check blocked",
        Outcome::NotVerified => "Not verified",
    }
}

pub(crate) fn card_of(report: &Report) -> Card {
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
        let (tone, text) = match line.status {
            Status::Pass => continue,
            Status::Fail => (Tone::Bad, format!("{}: {}", line.kind.label(), line.detail)),
            Status::Unknown => (Tone::Warn, format!("{}: {}", line.kind.label(), line.detail)),
            Status::Skipped => {
                (Tone::Quiet, format!("{}: not checked, {}", line.kind.label(), line.detail))
            }
        };
        rows.push(Row { tone, text });
        if line.kind == Kind::Unverified {
            let more = report.unverified.len().saturating_sub(MAX_ITEMS);
            rows.extend(
                report
                    .unverified
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|item| Row { tone: Tone::Quiet, text: format!("    - {item}") }),
            );
            if more > 0 {
                rows.push(Row { tone: Tone::Quiet, text: format!("    and {more} more") });
            }
        }
    }
    Card {
        title: title_of(outcome),
        tone: tone_of(outcome),
        summary,
        rows,
        ask_fix: outcome == Outcome::Blocked,
        check_again: outcome != Outcome::Done,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    AskFix,
    CheckAgain,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::done_check::Line;

    fn line(kind: Kind, status: Status, detail: &str) -> Line {
        Line { kind, status, detail: detail.to_owned() }
    }

    fn report(lines: Vec<Line>, unverified: &[&str]) -> Report {
        Report {
            head: String::new(),
            lines,
            unverified: unverified.iter().map(|item| (*item).to_owned()).collect(),
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
}
