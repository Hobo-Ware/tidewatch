use std::path::PathBuf;

use tidewatch_core::civil_date::Date;
use tidewatch_core::meeseeks_branch::Found;
use tidewatch_core::meeseeks_log::StageLine;

use super::*;

fn bare() -> Poofed {
    Poofed::bare("promo-test", "Added the promo test.", None, 360, Date::new(2026, 10, 25).unwrap())
}

fn at_1142() -> i64 {
    "2026-10-25T11:42:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn full() -> Poofed {
    let mut record = bare();
    record.pr_url = Some("https://github.com/acme/web/pull/213".into());
    record.task = "Add the promo test".into();
    record.repo = PathBuf::from("/code/web");
    record.branch = "agent/promo-test".into();
    record.ended_ms = at_1142();
    record.checks = vec![
        CheckLine { step: Step::Change, verdict: Verdict::Passed, detail: "2 commits".into() },
        CheckLine { step: Step::Tests, verdict: Verdict::Skipped, detail: "Not asked for".into() },
        CheckLine { step: Step::Proof, verdict: Verdict::Passed, detail: "Worktree clean".into() },
        CheckLine {
            step: Step::HandOff,
            verdict: Verdict::Passed,
            detail: "Opened PR #213".into(),
        },
    ];
    record.done = vec!["Added `promo.test.ts`".into()];
    record.next = vec!["Review it".into()];
    record.stages = vec![StageLine { name: "Build".into(), done: 3, total: 3 }];
    record
}

type BranchCase<'a> = (Option<&'a Branch>, &'a str, Option<(&'a str, &'a str, &'a str)>);

fn found(ahead: usize, files: usize, added: u64, removed: u64) -> Branch {
    Branch::Found(Found { base: "main".into(), ahead, changed: Changed { files, added, removed } })
}

#[test]
fn the_header_says_how_long_it_took_and_when_it_ended_and_the_prompt_is_kept_whole() {
    let summary = summary(&full(), None, &TimeZone::UTC);
    assert_eq!(summary.prompt.as_deref(), Some("Add the promo test"));
    assert_eq!(summary.meta, "Poofed \u{b7} 6m \u{b7} 11:42");
    let plus_three = TimeZone::fixed(jiff::tz::offset(3));
    assert_eq!(super::summary(&full(), None, &plus_three).meta, "Poofed \u{b7} 6m \u{b7} 14:42");
}

#[test]
fn a_poof_logged_before_the_summary_existed_still_reads() {
    let summary = summary(&bare(), None, &TimeZone::UTC);
    assert_eq!(summary.prompt, None);
    assert_eq!(summary.meta, "Poofed \u{b7} 6m");
    let rows: Vec<_> =
        summary.checks.iter().map(|c| (c.label, c.look, c.result, c.detail.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("Change", Look::Done, "Passed", ""),
            ("Tests", Look::Done, "Passed", ""),
            ("Proof", Look::Done, "Passed", ""),
            ("Hand off", Look::Done, "Passed", ""),
        ]
    );
    assert_eq!((summary.pr, summary.branch), (None, None));
    assert_eq!(summary.said.as_deref(), Some("Added the promo test."));
}

#[test]
fn every_check_shows_its_result_and_line_and_the_bars_follow_them() {
    let summary = summary(&full(), None, &TimeZone::UTC);
    let rows: Vec<_> =
        summary.checks.iter().map(|c| (c.label, c.look, c.result, c.detail.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("Change", Look::Done, "Passed", "2 commits"),
            ("Tests", Look::Skipped, "Skipped", "Not asked for"),
            ("Proof", Look::Done, "Passed", "Worktree clean"),
            ("Hand off", Look::Done, "Passed", "Opened PR #213"),
        ]
    );
    assert_eq!(
        summary.card.bars.iter().map(|(_, look)| *look).collect::<Vec<_>>(),
        [Look::Done, Look::Skipped, Look::Done, Look::Done]
    );
    assert_eq!(summary.card.meta, "3 of 4");
}

#[test]
fn the_pull_request_is_a_link_labelled_by_its_number_else_by_its_address() {
    let cases = [
        (Some("https://github.com/acme/web/pull/213"), Some("PR #213")),
        (Some("https://example.test/review"), Some("https://example.test/review")),
        (None, None),
    ];
    for (url, label) in cases {
        let mut record = full();
        record.pr_url = url.map(str::to_owned);
        let pr = summary(&record, None, &TimeZone::UTC).pr;
        assert_eq!(pr.as_ref().map(|pr| pr.label.as_str()), label, "{url:?}");
        assert_eq!(pr.as_ref().map(|pr| pr.url.as_str()), url);
    }
}

#[test]
fn the_branch_line_says_what_git_found_and_never_guesses() {
    let missing = Branch::Missing;
    let broken = Branch::Unreadable("not a git repository".into());
    let cases: [BranchCase; 6] = [
        (None, "Reading the branch...", None),
        (Some(&missing), "The branch is gone, deleted since it poofed", None),
        (Some(&broken), "Cannot read the branch: not a git repository", None),
        (
            Some(&found(2, 3, 42, 7)),
            "2 commits ahead of main",
            Some(("3 files changed", "+42", "-7")),
        ),
        (Some(&found(1, 1, 1, 0)), "1 commit ahead of main", Some(("1 file changed", "+1", "-0"))),
        (
            Some(&found(0, 0, 0, 0)),
            "0 commits ahead of main",
            Some(("No files changed", "+0", "-0")),
        ),
    ];
    for (facts, text, files) in cases {
        let line = summary(&full(), facts, &TimeZone::UTC).branch.unwrap();
        assert_eq!((line.name.as_str(), line.text.as_str()), ("agent/promo-test", text));
        let files = files.map(|(text, added, removed)| FilesLine {
            text: text.into(),
            added: added.into(),
            removed: removed.into(),
        });
        assert_eq!(line.files, files, "{text}");
    }
}

#[test]
fn the_last_turn_and_the_ledger_stages_come_through_as_saved() {
    let summary = summary(&full(), None, &TimeZone::UTC);
    assert_eq!(summary.done, ["Added `promo.test.ts`"]);
    assert_eq!(summary.next, ["Review it"]);
    assert_eq!(summary.stages, [StageRow { name: "Build".into(), done: 3, total: 3 }]);
}

#[test]
fn an_empty_summary_from_the_agent_is_left_out() {
    let mut record = full();
    record.summary = "  ".into();
    assert_eq!(summary(&record, None, &TimeZone::UTC).said, None);
}

#[test]
fn the_card_says_answered_blocked_or_nothing_to_check_when_no_check_ran() {
    let skipped = |detail: &str| {
        Step::ALL
            .iter()
            .map(|&step| CheckLine { step, verdict: Verdict::Skipped, detail: detail.into() })
            .collect::<Vec<_>>()
    };
    let cases = [
        (Kind::ReadOnly, "It is an LRU over the disk", "Answered"),
        (Kind::Edit, "Blocked: the staging key is missing", "Blocked"),
        (Kind::Edit, "Nothing needed doing", "Nothing to check"),
    ];
    for (kind, said, title) in cases {
        let record = Poofed { kind, summary: said.into(), checks: skipped("Not run"), ..bare() };
        let shown = summary(&record, None, &TimeZone::UTC);
        assert_eq!((shown.card.title.as_str(), shown.card.meta.as_str()), (title, "no checks"));
        assert_eq!(shown.said.as_deref(), Some(said));
    }
    assert_eq!(summary(&full(), None, &TimeZone::UTC).card.title, "Checks passed");
}

fn read_only() -> Poofed {
    let mut record = full();
    record.kind = Kind::ReadOnly;
    record.pr_url = None;
    record.summary = "It is an LRU over the disk".into();
    record.checks = Step::ALL
        .iter()
        .map(|&step| CheckLine {
            step,
            verdict: Verdict::Skipped,
            detail: "Nothing changed".into(),
        })
        .collect();
    record
}

#[test]
fn a_read_only_poof_says_it_changed_nothing_in_one_line_and_hides_checks_and_branch() {
    let shown = summary(&read_only(), Some(&found(0, 0, 0, 0)), &TimeZone::UTC);
    let quiet = shown.quiet.expect("one quiet line");
    assert_eq!(quiet.text, "Read-only \u{b7} nothing changed \u{b7} branch kept");
    assert_eq!(quiet.branch, "agent/promo-test");
    assert_eq!((shown.checks.len(), shown.branch), (0, None));
    assert_eq!(shown.said.as_deref(), Some("It is an LRU over the disk"));
    assert!(shown.read_only);
}

#[test]
fn an_edit_poof_keeps_its_checks_branch_and_pull_request_and_has_no_quiet_line() {
    let shown = summary(&full(), Some(&found(2, 3, 42, 7)), &TimeZone::UTC);
    assert_eq!(shown.quiet, None);
    assert_eq!(shown.checks.len(), 4);
    assert!(shown.branch.is_some() && shown.pr.is_some());
}

#[test]
fn an_edit_that_did_nothing_is_quiet_unless_the_branch_holds_commits() {
    let mut record = read_only();
    record.kind = Kind::Edit;
    record.summary = "Nothing needed doing".into();
    let quiet = summary(&record, Some(&found(0, 0, 0, 0)), &TimeZone::UTC).quiet;
    assert_eq!(quiet.map(|q| q.text), Some("Nothing changed \u{b7} branch kept".to_owned()));
    let kept = summary(&record, Some(&found(1, 1, 3, 0)), &TimeZone::UTC);
    assert_eq!(kept.quiet, None);
    assert!(kept.branch.is_some());
}

#[test]
fn the_prompt_is_kept_whole_and_stages_without_steps_are_left_out() {
    let mut record = full();
    record.task = "Why is the compile step so slow? ".repeat(30);
    record.stages = vec![
        StageLine { name: "Understand".into(), done: 5, total: 5 },
        StageLine { name: "Build".into(), done: 0, total: 0 },
    ];
    let shown = summary(&record, None, &TimeZone::UTC);
    assert_eq!(shown.prompt.as_deref(), Some(record.task.trim()));
    assert_eq!(shown.stages, [StageRow { name: "Understand".into(), done: 5, total: 5 }]);
}

#[test]
fn the_hand_off_row_of_a_task_that_named_a_pull_request_reads_update_pr() {
    let mut record = full();
    "Sensei https://github.com/acme/web/pull/213".clone_into(&mut record.task);
    let shown = summary(&record, None, &TimeZone::UTC);
    let labels: Vec<&str> = shown.checks.iter().map(|check| check.label).collect();
    assert_eq!(labels, ["Change", "Tests", "Proof", "Update PR"]);
    let names: Vec<&str> = shown.card.bars.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, labels);
    let unnamed = summary(&full(), None, &TimeZone::UTC);
    assert_eq!(unnamed.checks[3].label, "Hand off");
}
