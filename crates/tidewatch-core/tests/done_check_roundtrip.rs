//! The done check against a real throwaway repo, a real bare remote and a fake `gh`: what a branch
//! looks like at each stage and which `gh` calls it makes. The real `gh` is never started.

mod common;

use std::path::Path;

use common::{FakeGh, commit_file, git_available, ok, path_with_only_git, reply, repos};
use tidewatch_core::civil_date::Date;
use tidewatch_core::done_check::{Input, Kind, Outcome, PrFact, Report, Status, Unverified, run};
use tidewatch_core::tools::Tools;
use tidewatch_core::workflow_profile::{Checks, Workflow};

const BRANCH: &str = "agent/rename-helper";

fn work_branch(work: &Path) -> String {
    ok(work, &["checkout", "-q", "-b", BRANCH]);
    commit_file(work, "helper.txt", "one\n", "feat: add helper");
    ok(work, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn leaked(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn open_pr(head: &str, checks: &str) -> &'static str {
    leaked(format!(
        r#"{{"number":42,"title":"t","url":"https://github.com/acme/web/pull/42","state":"OPEN","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"","headRefOid":"{head}","baseRefName":"main","headRefName":"{BRANCH}","author":{{"login":"a"}},"createdAt":"2026-01-01T00:00:00Z","statusCheckRollup":[{checks}],"reviews":[],"comments":[]}}"#
    ))
}

const FAILED_RUN: &str = r#"{"__typename":"CheckRun","name":"build","workflowName":"CI","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://github.com/acme/web/actions/runs/1/job/2","startedAt":"2026-01-01T00:00:00Z","completedAt":"2026-01-01T00:01:00Z"}"#;
const PASSED_RUN: &str = r#"{"__typename":"CheckRun","name":"build","workflowName":"CI","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://github.com/acme/web/actions/runs/1/job/2","startedAt":"2026-01-01T00:00:00Z","completedAt":"2026-01-01T00:01:00Z"}"#;

fn check(tools: &Tools, dir: &Path, pr: PrFact, unverified: Option<Vec<Unverified>>) -> Report {
    let today = Date::parse("2026-10-27").unwrap();
    run(&Input {
        tools,
        dir,
        branch: BRANCH,
        pr,
        unverified,
        today,
        checks: Checks::OFF,
        repo: dir,
        plan: None,
        verified: None,
    })
}

fn status_of(report: &Report, kind: Kind) -> (Status, &str) {
    let line = report.lines.iter().find(|line| line.kind == kind).unwrap();
    (line.status, line.detail.as_str())
}

#[test]
fn a_pushed_clean_branch_without_a_pull_request_is_done_and_gh_is_never_called() {
    if !git_available() {
        return;
    }
    let r = repos();
    let head = work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let gh = FakeGh::new(&[]);
    let report = check(&Tools::with_path(gh.path()), &r.work, PrFact::None, Some(Vec::new()));
    assert_eq!(report.head, head);
    assert_eq!(report.outcome(), Outcome::Done);
    assert_eq!(status_of(&report, Kind::Committed).0, Status::Pass);
    assert_eq!(status_of(&report, Kind::Pushed).0, Status::Pass);
    assert_eq!(
        status_of(&report, Kind::PullRequest),
        (Status::Skipped, "no pull request for this branch")
    );
    assert_eq!(gh.calls().len(), 0);
}

#[test]
fn loose_changes_and_an_unpushed_commit_block_with_each_reason_named() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    commit_file(&r.work, "helper.txt", "two\n", "feat: more helper");
    std::fs::write(r.work.join("loose.txt"), "x").unwrap();
    let report = check(&Tools::system(), &r.work, PrFact::None, None);
    assert_eq!(report.outcome(), Outcome::Blocked);
    assert_eq!(
        status_of(&report, Kind::Committed),
        (Status::Fail, "1 file with uncommitted changes")
    );
    assert_eq!(status_of(&report, Kind::Pushed), (Status::Fail, "1 commit not pushed"));
    assert_eq!(
        report.fix_note(Workflow::default()).as_deref(),
        Some(
            "The done check found: 1 file with uncommitted changes; 1 commit not pushed. Please fix that and tell me when it is done."
        )
    );
}

#[test]
fn a_branch_that_never_left_the_machine_fails_pushed_and_a_branch_with_no_commit_fails_committed() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["checkout", "-q", "-b", BRANCH]);
    let report = check(&Tools::system(), &r.work, PrFact::None, None);
    assert_eq!(status_of(&report, Kind::Committed), (Status::Fail, "no commit ahead of main"));
    assert_eq!(status_of(&report, Kind::Pushed), (Status::Fail, "the branch is not on origin yet"));
    assert_eq!(report.outcome(), Outcome::Blocked);
}

#[test]
fn an_unreachable_origin_could_not_be_checked() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["remote", "set-url", "origin", r.root.path().join("gone.git").to_str().unwrap()]);
    let report = check(&Tools::system(), &r.work, PrFact::None, None);
    assert_eq!(status_of(&report, Kind::Pushed).0, Status::Unknown);
    assert_eq!(report.outcome(), Outcome::NotVerified);
}

#[test]
fn a_failing_check_on_the_pushed_head_blocks_and_one_pr_view_is_made() {
    if !git_available() {
        return;
    }
    let r = repos();
    let head = work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let gh = FakeGh::new(&[reply("pr view", 0, open_pr(&head, FAILED_RUN), "")]);
    let report = check(&Tools::with_path(gh.path()), &r.work, PrFact::Number(42), None);
    assert_eq!(report.outcome(), Outcome::Blocked);
    assert_eq!(
        status_of(&report, Kind::PullRequest),
        (Status::Fail, "PR #42 checks failing: build")
    );
    let calls = gh.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][..3], ["pr", "view", "42"]);
}

#[test]
fn a_green_pull_request_by_branch_name_is_done_and_unverified_ledger_items_make_it_not_verified() {
    if !git_available() {
        return;
    }
    let r = repos();
    let head = work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let gh = FakeGh::new(&[reply("pr view", 0, open_pr(&head, PASSED_RUN), "")]);
    let tools = Tools::with_path(gh.path());
    let green = check(&tools, &r.work, PrFact::Unknown, Some(Vec::new()));
    assert_eq!(green.outcome(), Outcome::Done);
    assert_eq!(gh.calls()[0][..3], ["pr", "view", BRANCH]);
    let items = vec![Unverified::new("the staging migration")];
    let unverified = check(&tools, &r.work, PrFact::Number(42), Some(items.clone()));
    assert_eq!(unverified.outcome(), Outcome::NotVerified);
    assert_eq!(unverified.unverified, items);
}

#[test]
fn a_merged_pull_request_covers_a_branch_that_was_deleted_on_the_remote() {
    if !git_available() {
        return;
    }
    let r = repos();
    let head = work_branch(&r.work);
    let merged = leaked(open_pr(&head, PASSED_RUN).replace("\"OPEN\"", "\"MERGED\""));
    let gh = FakeGh::new(&[reply("pr view", 0, merged, "")]);
    let report = check(&Tools::with_path(gh.path()), &r.work, PrFact::Number(42), None);
    assert_eq!(report.outcome(), Outcome::Done);
    assert_eq!(status_of(&report, Kind::PullRequest), (Status::Pass, "PR #42 merged"));
}

#[test]
fn no_pull_request_for_the_branch_is_not_checked_rather_than_an_error() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let gh = FakeGh::new(&[reply(
        "pr view",
        1,
        "",
        "no pull requests found for branch \"agent/rename-helper\"\n",
    )]);
    let report = check(&Tools::with_path(gh.path()), &r.work, PrFact::Unknown, None);
    assert_eq!(status_of(&report, Kind::PullRequest).0, Status::Skipped);
    assert_eq!(report.outcome(), Outcome::Done);
}

#[test]
fn a_missing_gh_leaves_the_pull_request_unchecked_with_the_reason() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let (_dir, path) = path_with_only_git();
    let report = check(&Tools::with_path(path), &r.work, PrFact::Number(42), None);
    assert_eq!(
        status_of(&report, Kind::PullRequest),
        (Status::Unknown, "the GitHub CLI (gh) is not installed")
    );
    assert_eq!(report.outcome(), Outcome::NotVerified);
}

#[test]
fn an_unverified_item_the_ledger_schedules_for_tomorrow_makes_the_check_wait() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let gh = FakeGh::new(&[]);
    let tomorrow = Date::parse("2026-10-28").unwrap();
    let items = vec![Unverified::until("Live list across a day boundary", tomorrow)];
    let report = check(&Tools::with_path(gh.path()), &r.work, PrFact::None, Some(items));
    assert_eq!(report.outcome(), Outcome::Waiting);
    assert_eq!(
        status_of(&report, Kind::Unverified),
        (Status::Waiting, "1 item waiting until 2026-10-28")
    );
    assert_eq!(report.outcome_on(tomorrow), Outcome::NotVerified);
}

fn check_with(
    r: &common::Repos,
    checks: Checks,
    plan: Option<&[&str]>,
    verified: &[(&str, Option<&str>)],
) -> Report {
    let today = Date::parse("2026-10-27").unwrap();
    let own = |text: &&str| (*text).to_owned();
    run(&Input {
        tools: &Tools::system(),
        dir: &r.work,
        branch: BRANCH,
        pr: PrFact::None,
        unverified: Some(Vec::new()),
        today,
        checks,
        repo: r.root.path(),
        plan: plan.map(|steps| steps.iter().map(own).collect()),
        verified: Some(
            verified
                .iter()
                .map(|(text, why)| tidewatch_core::done_proof::Verified {
                    text: (*text).to_owned(),
                    why: why.map(str::to_owned),
                })
                .collect(),
        ),
    })
}

fn configure_tests(r: &common::Repos, command: &str) {
    std::fs::write(r.root.path().join(".tidewatch.toml"), format!("test = {command:?}\n")).unwrap();
}

#[test]
fn tidewatch_runs_the_tests_itself_and_shows_the_command_and_the_result() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    configure_tests(&r, "echo 'test result: ok. 3 passed; 0 failed'");
    let report = check_with(&r, Checks { tests: true, ..Checks::OFF }, None, &[]);
    let (status, detail) = status_of(&report, Kind::Tests);
    assert_eq!(status, Status::Pass);
    assert!(
        detail.starts_with("Ran echo 'test result: ok. 3 passed; 0 failed': passed in "),
        "{detail}"
    );
    assert_eq!(report.outcome(), Outcome::Done);
    assert_eq!(report.found.tests.as_ref().map(|run| run.exit_code), Some(0));
}

#[test]
fn a_failing_test_run_blocks_and_the_fix_note_carries_the_count_the_profile_words_and_the_output() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    configure_tests(&r, "echo 'test result: FAILED. 1 passed; 2 failed; 0 ignored'; exit 101");
    let report = check_with(&r, Checks { tests: true, ..Checks::OFF }, None, &[]);
    let (status, detail) = status_of(&report, Kind::Tests);
    assert_eq!(status, Status::Fail);
    assert!(detail.ends_with("2 failed"), "{detail}");
    assert_eq!(report.outcome(), Outcome::Blocked);
    let note = report.fix_note(Workflow::default()).unwrap();
    assert!(note.starts_with("Tidewatch ran echo 'test result: FAILED."), "{note}");
    assert!(
        note.contains(": 2 failed (output below). Fix them, fold each fix into its commit, and say done again."),
        "{note}"
    );
    assert!(
        note.ends_with("```\ntest result: FAILED. 1 passed; 2 failed; 0 ignored\n```"),
        "{note}"
    );
}

#[test]
fn a_repo_with_no_test_command_skips_the_tests_line_and_a_switch_that_is_off_adds_none() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    std::fs::write(r.root.path().join(".tidewatch.toml"), "test = \"\"\n").unwrap();
    let report = check_with(&r, Checks { tests: true, ..Checks::OFF }, None, &[]);
    assert_eq!(
        status_of(&report, Kind::Tests),
        (Status::Skipped, "no test command found for this repo")
    );
    assert_eq!(report.found.tests, None);
    let off = check_with(&r, Checks::OFF, None, &[]);
    assert_eq!(off.lines.iter().filter(|line| line.kind == Kind::Tests).count(), 0);
}

#[test]
fn a_fix_branch_with_no_test_fails_and_one_that_changes_a_test_passes() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["checkout", "-q", "-b", BRANCH]);
    commit_file(&r.work, "helper.txt", "one\n", "fix: stop the helper hanging");
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let bare = check_with(&r, Checks { fix_test: true, ..Checks::OFF }, None, &[]);
    assert_eq!(
        status_of(&bare, Kind::FixTest),
        (Status::Fail, "the branch fixes something but adds no test")
    );
    assert_eq!(
        bare.fix_note(Workflow::default()).as_deref(),
        Some(
            "This branch fixes something but adds no test that fails without the fix. Add one, or say why none fits."
        )
    );
    std::fs::create_dir_all(r.work.join("tests")).unwrap();
    commit_file(
        &r.work,
        "tests/helper_hang.rs",
        "#[test]\nfn it_stops() {}\n",
        "test: cover the hang",
    );
    let covered = check_with(&r, Checks { fix_test: true, ..Checks::OFF }, None, &[]);
    assert_eq!(status_of(&covered, Kind::FixTest), (Status::Pass, "the fix has a test"));
}

#[test]
fn a_branch_that_fixes_nothing_gets_no_fix_test_line() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    let report = check_with(&r, Checks { fix_test: true, ..Checks::OFF }, None, &[]);
    assert_eq!(report.lines.iter().filter(|line| line.kind == Kind::FixTest).count(), 0);
}

#[test]
fn verified_items_with_no_command_are_claimed_and_never_block() {
    if !git_available() {
        return;
    }
    let r = repos();
    work_branch(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let verified = [("`bun run check` is clean", None), ("it looks right in Safari", None)];
    let report = check_with(&r, Checks { verified: true, ..Checks::OFF }, None, &verified);
    assert_eq!(
        status_of(&report, Kind::Verified),
        (Status::Noted, "1 of 2 verified items name no command, so they are claimed, not checked")
    );
    assert_eq!(report.found.claimed, ["it looks right in Safari"]);
    assert_eq!(report.outcome(), Outcome::Done);
    assert_eq!(report.fix_note(Workflow::default()), None);
}

#[test]
fn files_changed_outside_what_the_plan_named_are_listed_and_never_block() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["checkout", "-q", "-b", BRANCH]);
    std::fs::create_dir_all(r.work.join("src")).unwrap();
    commit_file(&r.work, "src/helper.rs", "one\n", "feat: helper");
    commit_file(&r.work, "src/surprise.rs", "two\n", "feat: surprise");
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    let plan = ["Rename the helper in src/helper.rs"];
    let report = check_with(&r, Checks { scope: true, ..Checks::OFF }, Some(&plan), &[]);
    assert_eq!(
        status_of(&report, Kind::Scope),
        (Status::Noted, "1 changed file outside what the plan named")
    );
    assert_eq!(report.found.outside, ["src/surprise.rs"]);
    assert_eq!(report.outcome(), Outcome::Done);
    assert_eq!(report.fix_note(Workflow::default()), None);
}

#[test]
fn a_merged_pull_request_is_not_tested_again() {
    if !git_available() {
        return;
    }
    let r = repos();
    let head = work_branch(&r.work);
    configure_tests(&r, "echo ran; exit 1");
    let merged = leaked(open_pr(&head, PASSED_RUN).replace("\"OPEN\"", "\"MERGED\""));
    let gh = FakeGh::new(&[reply("pr view", 0, merged, "")]);
    let report = run(&Input {
        tools: &Tools::with_path(gh.path()),
        dir: &r.work,
        branch: BRANCH,
        pr: PrFact::Number(42),
        unverified: None,
        today: Date::parse("2026-10-27").unwrap(),
        checks: Checks { tests: true, ..Checks::OFF },
        repo: r.root.path(),
        plan: None,
        verified: None,
    });
    assert_eq!(report.lines.iter().filter(|line| line.kind == Kind::Tests).count(), 0);
    assert_eq!(report.found.tests, None);
    assert_eq!(report.outcome(), Outcome::Done);
}
