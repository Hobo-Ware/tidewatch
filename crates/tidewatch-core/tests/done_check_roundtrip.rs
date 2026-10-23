//! The done check against a real throwaway repo, a real bare remote and a fake `gh`: what a branch
//! looks like at each stage and which `gh` calls it makes. The real `gh` is never started.

mod common;

use std::path::Path;

use common::{FakeGh, commit_file, git_available, ok, path_with_only_git, reply, repos};
use tidewatch_core::done_check::{Input, Kind, Outcome, PrFact, Report, Status, run};
use tidewatch_core::tools::Tools;

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

fn check(tools: &Tools, dir: &Path, pr: PrFact, unverified: Option<Vec<String>>) -> Report {
    run(&Input { tools, dir, branch: BRANCH, pr, unverified })
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
        report.fix_note().as_deref(),
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
    let items = vec!["the staging migration".to_owned()];
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
