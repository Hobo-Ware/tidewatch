//! The Meeseeks checks against real throwaway repos, a real bare remote and a fake `gh`: the commit
//! check, the clean tree, the tests, the pull request and the poof that keeps the branch.

mod common;

use std::path::PathBuf;

use common::{FakeGh, commit_file, git_available, ok, remote_branches, reply, repos};
use tidewatch_core::meeseeks_check::{Checked, change, hand_off, poof, proof, tests};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;
use tidewatch_core::worktree::{self, Worktree};

const URL: &str = "https://github.com/acme/web/pull/42";

struct Fixture {
    repos: common::Repos,
    paths: TidewatchPaths,
    worktree: Worktree,
}

fn fixture() -> Fixture {
    let repos = repos();
    let home = repos.root.path().join("home");
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add_named(&paths, &repos.work, "rename-helper", Some("origin/main"))
        .expect("a worktree");
    Fixture { repos, paths, worktree }
}

fn dir(fixture: &Fixture) -> PathBuf {
    fixture.worktree.path.clone()
}

fn said(result: Result<Checked, String>) -> String {
    result.expect("the check passes").detail
}

#[test]
fn a_branch_with_no_commit_ahead_of_the_base_fails_the_change_check_in_words() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let error = change(&Tools::system(), &dir(&f), &f.worktree.branch).unwrap_err();
    assert_eq!(error, "no commit ahead of main");
}

#[test]
fn loose_changes_fail_the_change_check_and_a_commit_clears_it() {
    if !git_available() {
        return;
    }
    let f = fixture();
    std::fs::write(dir(&f).join("hello.txt"), "hi\n").unwrap();
    let error = change(&Tools::system(), &dir(&f), &f.worktree.branch).unwrap_err();
    assert_eq!(error, "1 file with uncommitted changes");
    ok(&dir(&f), &["add", "hello.txt"]);
    ok(&dir(&f), &["commit", "-q", "-m", "feat: add hello"]);
    assert_eq!(
        said(change(&Tools::system(), &dir(&f), &f.worktree.branch)),
        "1 commit ahead of main"
    );
    assert_eq!(said(proof(&Tools::system(), &dir(&f))), "Worktree clean");
    std::fs::write(dir(&f).join("a.txt"), "x").unwrap();
    std::fs::write(dir(&f).join("b.txt"), "x").unwrap();
    assert_eq!(
        proof(&Tools::system(), &dir(&f)),
        Err("2 files with uncommitted changes".to_owned())
    );
}

#[test]
fn the_tests_pass_fail_with_their_output_or_are_skipped_without_a_command() {
    if !git_available() {
        return;
    }
    let f = fixture();
    let repo = f.repos.work.clone();
    assert_eq!(said(tests(&repo, &dir(&f))), "No test command for this repo");
    std::fs::write(repo.join(".tidewatch.toml"), "test = \"true\"\n").unwrap();
    assert_eq!(said(tests(&repo, &dir(&f))), "true passed");
    std::fs::write(repo.join(".tidewatch.toml"), "test = \"echo broken thing; exit 3\"\n").unwrap();
    let error = tests(&repo, &dir(&f)).unwrap_err();
    assert!(error.starts_with("exit 3 from `echo broken thing; exit 3`"), "{error}");
    assert!(error.contains("broken thing"), "{error}");
}

#[test]
fn the_hand_off_pushes_the_branch_and_opens_the_pr_titled_with_the_summary() {
    if !git_available() {
        return;
    }
    let f = fixture();
    commit_file(&dir(&f), "hello.txt", "hi\n", "feat: add hello");
    let gh = FakeGh::new(&[
        reply("--version", 0, "gh version 2.98.0\n", ""),
        reply("pr create", 0, Box::leak(format!("{URL}\n").into_boxed_str()), ""),
    ]);
    let tools = Tools::with_path(gh.path());
    let url = hand_off(&tools, &dir(&f), &f.worktree.branch, "Add hello.txt with a greeting");
    let opened = url.unwrap();
    assert_eq!(opened.pr_url.as_deref(), Some(URL));
    assert_eq!(opened.detail, "Opened PR #42 against main");
    assert!(remote_branches(&f.repos.remote).contains(&f.worktree.branch));
    let create = gh.calls_to("pr create");
    assert_eq!(create.len(), 1);
    assert!(create[0].windows(2).any(|pair| pair == ["--title", "Add hello.txt with a greeting"]));
}

#[test]
fn a_failed_hand_off_says_why_and_a_missing_gh_is_not_a_panic() {
    if !git_available() {
        return;
    }
    let f = fixture();
    commit_file(&dir(&f), "hello.txt", "hi\n", "feat: add hello");
    let (_keep, only_git) = common::path_with_only_git();
    let error =
        hand_off(&Tools::with_path(only_git), &dir(&f), &f.worktree.branch, "x").unwrap_err();
    assert!(error.to_lowercase().contains("gh"), "{error}");
    assert!(!remote_branches(&f.repos.remote).contains(&f.worktree.branch));
}

#[test]
fn the_poof_removes_the_folder_and_keeps_the_branch_with_its_commit() {
    if !git_available() {
        return;
    }
    let f = fixture();
    commit_file(&dir(&f), "hello.txt", "hi\n", "feat: add hello");
    assert_eq!(poof(&f.paths, &f.worktree), Ok(()));
    assert!(!dir(&f).exists());
    let branches = ok(&f.repos.work, &["branch", "--format=%(refname:short)"]);
    assert!(branches.lines().any(|b| b == f.worktree.branch), "{branches}");
    let log = ok(&f.repos.work, &["log", "--format=%s", &f.worktree.branch]);
    assert!(log.lines().any(|line| line == "feat: add hello"), "{log}");
}

#[test]
fn the_poof_refuses_a_dirty_tree_and_leaves_everything_where_it_was() {
    if !git_available() {
        return;
    }
    let f = fixture();
    std::fs::write(dir(&f).join("loose.txt"), "x").unwrap();
    let error = poof(&f.paths, &f.worktree).unwrap_err();
    assert_eq!(error, "1 file with uncommitted changes");
    assert!(dir(&f).join("loose.txt").exists());
    assert!(ok(&f.repos.work, &["worktree", "list"]).contains("rename-helper"));
}
