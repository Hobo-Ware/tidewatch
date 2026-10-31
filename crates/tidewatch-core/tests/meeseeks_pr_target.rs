//! A Meeseeks whose task names a pull request is checked against that pull request, not against a
//! new one: real throwaway repos with a bare remote, a fake `gh`, no network.

mod common;

use std::path::PathBuf;

use common::{FakeGh, commit_file, git_available, ok, reply};
use tidewatch_core::meeseeks_check::{Work, change, hand_off};
use tidewatch_core::meeseeks_target::Target;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;
use tidewatch_core::worktree::{self, Worktree};

const URL: &str = "https://github.com/acme/web/pull/2437";
const HEAD: &str = "claude/elegant-fermi";

struct Fixture {
    repos: common::Repos,
    worktree: Worktree,
}

fn fixture() -> Fixture {
    let repos = common::repos();
    let paths = TidewatchPaths::new(&repos.root.path().join("home"));
    let worktree = worktree::add_named(&paths, &repos.work, "sensei-pr-2437", Some("origin/main"))
        .expect("a worktree");
    Fixture { repos, worktree }
}

fn dir(f: &Fixture) -> PathBuf {
    f.worktree.path.clone()
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn viewed(state: &str) -> &'static str {
    leak(format!(
        "{{\"state\":\"{state}\",\"url\":\"{URL}\",\"headRefName\":\"{HEAD}\",\"headRefOid\":\"{}\"}}\n",
        "1".repeat(40)
    ))
}

fn gh_with(replies: &[common::Reply]) -> FakeGh {
    let mut all = vec![reply("--version", 0, "gh version 2.98.0\n", "")];
    all.extend(replies.iter().map(|r| reply(r.when, r.code, r.stdout, r.stderr)));
    FakeGh::new(&all)
}

fn named() -> Work {
    Work { target: Target::of_task(&format!("Sensei {URL}")), ..Work::default() }
}

fn bare_number() -> Work {
    Work { target: Target::of_task("Finish #99"), ..Work::default() }
}

fn summary(text: &str) -> Work {
    Work { summary: text.to_owned(), ..Work::default() }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The pull request's branch on the remote, one commit past `main`.
fn pr_branch_on_remote(f: &Fixture) {
    let work = &f.repos.work;
    ok(work, &["checkout", "-q", "-b", HEAD]);
    commit_file(work, "pr.txt", "the pr\n", "feat: the pull request");
    ok(work, &["push", "-q", "origin", HEAD]);
    ok(work, &["checkout", "-q", "main"]);
}

/// The worktree on the pull request's head, plus one commit of its own.
fn fold_into_pr_branch(f: &Fixture) {
    ok(&dir(f), &["fetch", "-q", "origin", HEAD]);
    ok(&dir(f), &["merge", "-q", "--ff-only", "FETCH_HEAD"]);
    commit_file(&dir(f), "fix.txt", "fixed\n", "fix: the review");
}

fn hand_off_with(
    f: &Fixture,
    gh: &FakeGh,
    work: &Work,
) -> Result<tidewatch_core::meeseeks_check::Checked, String> {
    hand_off(&Tools::with_path(gh.path()), &dir(f), &f.worktree.branch, work)
}

#[test]
fn a_named_pull_request_that_merged_passes_the_hand_off_and_nothing_is_opened() {
    if !git_available() {
        return;
    }
    let f = fixture();
    commit_file(&dir(&f), "fix.txt", "fixed\n", "fix: the review");
    let gh = gh_with(&[reply("pr view", 0, viewed("MERGED"), "")]);
    let checked = hand_off_with(&f, &gh, &named()).unwrap();
    assert_eq!(checked.detail, "PR #2437 merged");
    assert_eq!(checked.pr_url.as_deref(), Some(URL));
    assert_eq!(gh.calls_to("pr create").len(), 0);
    assert_eq!(gh.calls_to("pr list").len(), 0, "it asked about the named pull request only");
    assert_eq!(gh.calls_to("pr view")[0][2], URL, "a task that gave a link is looked up by it");
}

#[test]
fn a_named_open_pull_request_passes_when_its_branch_holds_the_commits() {
    if !git_available() {
        return;
    }
    let f = fixture();
    pr_branch_on_remote(&f);
    fold_into_pr_branch(&f);
    ok(&dir(&f), &["push", "-q", "origin", &format!("HEAD:{HEAD}")]);
    let gh = gh_with(&[reply("pr view", 0, viewed("OPEN"), "")]);
    let checked = hand_off_with(&f, &gh, &named()).unwrap();
    assert_eq!(checked.detail, "Everything is on PR #2437");
    assert_eq!(checked.pr_url.as_deref(), Some(URL));
}

#[test]
fn commits_missing_from_the_pull_request_branch_say_to_push_there_and_never_to_open_a_new_pr() {
    if !git_available() {
        return;
    }
    let f = fixture();
    pr_branch_on_remote(&f);
    fold_into_pr_branch(&f);
    let gh = gh_with(&[reply("pr view", 0, viewed("OPEN"), "")]);
    let error = hand_off_with(&f, &gh, &named()).unwrap_err();
    assert!(
        error.starts_with(
            "1 commit of this worktree is not on `claude/elegant-fermi`, the branch of PR #2437"
        ),
        "{error}"
    );
    assert!(error.contains("call `meeseeks_done` again"), "{error}");
    assert!(!error.contains("gh pr create"), "{error}");
}

#[test]
fn a_closed_pull_request_fails_the_hand_off_without_asking_for_a_new_one() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let gh = gh_with(&[reply("pr view", 0, viewed("CLOSED"), "")]);
    let error = hand_off_with(&f, &gh, &named()).unwrap_err();
    assert!(error.starts_with("PR #2437 was closed without being merged"), "{error}");
    assert!(!error.contains("gh pr create"), "{error}");
}

#[test]
fn a_gh_that_fails_on_the_named_pull_request_says_which_look_up_failed() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let gh = gh_with(&[reply("pr view", 1, "", "HTTP 502: Bad Gateway")]);
    let error = hand_off_with(&f, &gh, &named()).unwrap_err();
    assert_eq!(error, "could not look up PR #2437: HTTP 502: Bad Gateway");
}

#[test]
fn a_number_gh_does_not_know_is_an_error_when_the_task_said_pr_and_a_new_pr_when_it_only_said_hash()
{
    if !git_available() {
        return;
    }
    let f = fixture();
    let unknown = reply(
        "pr view",
        1,
        "",
        "GraphQL: Could not resolve to a PullRequest with the number of 99. (repository.pullRequest)",
    );
    let gh = gh_with(&[unknown, reply("pr list", 0, "[]\n", "")]);
    let said_pr = Work { target: Target::of_task("Finish PR 99"), ..Work::default() };
    let error = hand_off_with(&f, &gh, &said_pr).unwrap_err();
    assert!(error.starts_with("PR #99 was not found"), "{error}");
    let error = hand_off_with(&f, &gh, &bare_number()).unwrap_err();
    assert!(error.contains("gh pr create --draft --base main"), "{error}");
}

#[test]
fn a_pull_request_in_another_repo_is_not_compared_commit_by_commit() {
    if !git_available() {
        return;
    }
    let f = fixture();
    ok(&dir(&f), &["remote", "set-url", "origin", "https://github.com/other/place.git"]);
    let gh = gh_with(&[reply("pr view", 0, viewed("OPEN"), "")]);
    let checked = hand_off_with(&f, &gh, &named()).unwrap();
    assert_eq!(checked.detail, "PR #2437 is in another repo, commits not compared");
}

#[test]
fn a_summary_that_links_a_merged_pull_request_stands_in_for_a_task_that_named_none() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let gh = gh_with(&[reply("pr list", 0, "[]\n", ""), reply("pr view", 0, viewed("MERGED"), "")]);
    let work = summary(&format!("Folded the fix into {URL}"));
    assert_eq!(hand_off_with(&f, &gh, &work).unwrap().detail, "PR #2437 merged");
    let nothing = summary("Folded the fix in, see PR 2437");
    let error = hand_off_with(&f, &gh, &nothing).unwrap_err();
    assert!(error.contains("gh pr create --draft --base main"), "only a link counts: {error}");
}

#[test]
fn a_summary_link_that_does_not_check_out_leaves_the_usual_open_a_pr_note() {
    if !git_available() {
        return;
    }
    let f = fixture();
    pr_branch_on_remote(&f);
    fold_into_pr_branch(&f);
    let gh = gh_with(&[reply("pr list", 0, "[]\n", ""), reply("pr view", 0, viewed("OPEN"), "")]);
    let error = hand_off_with(&f, &gh, &summary(&format!("Pushed to {URL}"))).unwrap_err();
    assert!(error.contains("gh pr create --draft --base main"), "{error}");
}

#[test]
fn a_task_that_names_no_pull_request_keeps_the_open_pr_at_head_rule() {
    if !git_available() {
        return;
    }
    let f = fixture();
    commit_file(&dir(&f), "hello.txt", "hi\n", "feat: add hello");
    let sha = ok(&dir(&f), &["rev-parse", "HEAD"]);
    let listed = leak(format!("[{{\"url\":\"{URL}\",\"headRefOid\":\"{}\"}}]\n", sha.trim()));
    let gh = gh_with(&[reply("pr list", 0, listed, "")]);
    let work = Work { target: Target::of_task("Add hello"), ..Work::default() };
    assert_eq!(hand_off_with(&f, &gh, &work).unwrap().detail, "Found PR #2437");
    assert_eq!(gh.calls_to("pr view").len(), 0);
}

fn change_with(f: &Fixture, gh: &FakeGh, work: &Work) -> Result<String, String> {
    change(&Tools::with_path(gh.path()), &dir(f), &f.worktree.branch, work)
        .map(|checked| checked.detail)
}

#[test]
fn work_that_went_onto_a_merged_pull_request_passes_the_change_check_only_for_a_task_that_named_it()
{
    if !git_available() {
        return;
    }
    let f = fixture();
    let gh = gh_with(&[reply("pr view", 0, viewed("MERGED"), "")]);
    assert_eq!(change_with(&f, &gh, &named()), Ok("Work is on PR #2437, merged".to_owned()));
    assert_eq!(
        change_with(&f, &gh, &Work::default()),
        Err("no commit ahead of main".to_owned()),
        "a new pull request still needs a commit"
    );
}

#[test]
fn new_commits_on_the_open_pull_request_since_the_task_started_pass_the_change_check() {
    if !git_available() {
        return;
    }
    let f = fixture();
    pr_branch_on_remote(&f);
    let gh = gh_with(&[reply("pr view", 0, viewed("OPEN"), "")]);
    let started = |ms| Work { started_ms: ms, ..named() };
    assert_eq!(
        change_with(&f, &gh, &started(now_ms() - 3_600_000)),
        Ok("1 new commit on PR #2437".to_owned())
    );
    assert_eq!(
        change_with(&f, &gh, &started(now_ms() + 3_600_000)),
        Err("no commit on this branch, and none new on PR #2437".to_owned()),
        "a commit from before the task is not the task's work"
    );
}

#[test]
fn a_closed_pull_request_fails_the_change_check_and_commits_on_the_branch_still_pass_it() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let gh = gh_with(&[reply("pr view", 0, viewed("CLOSED"), "")]);
    let error = change_with(&f, &gh, &named()).unwrap_err();
    assert!(error.starts_with("PR #2437 was closed without being merged"), "{error}");
    commit_file(&dir(&f), "fix.txt", "fixed\n", "fix: the review");
    assert_eq!(change_with(&f, &gh, &named()), Ok("1 commit ahead of main".to_owned()));
    assert_eq!(gh.calls_to("pr view").len(), 1, "gh was not asked again once a commit was there");
}
