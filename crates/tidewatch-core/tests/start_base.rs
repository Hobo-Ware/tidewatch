//! Worktrees that start from the remote default branch, against a real bare remote that has a
//! newer commit than the checkout knows about. Nothing reaches the network.

mod common;

use std::path::Path;

use common::{Repos, commit_file, git, git_available, ok, repos};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::start_base::{self, FETCH_TIMEOUT, Start};
use tidewatch_core::worktree;

fn push_newer_commit(r: &Repos) {
    let other = r.root.path().join("other");
    ok(r.root.path(), &["clone", "-q", r.remote.to_str().unwrap(), "other"]);
    commit_file(&other, "news.md", "fresh\n", "feat: newer on the remote");
    ok(&other, &["push", "-q", "origin", "main"]);
}

fn subject_of(dir: &Path) -> String {
    ok(dir, &["log", "-1", "--format=%s"]).trim().to_owned()
}

fn paths_of(r: &Repos) -> TidewatchPaths {
    TidewatchPaths::new(&r.root.path().join("home"))
}

#[test]
fn a_worktree_starts_from_the_remote_commit_even_when_the_checkout_is_behind() {
    if !git_available() {
        return;
    }
    let r = repos();
    push_newer_commit(&r);
    assert_eq!(ok(&r.work, &["log", "-1", "--format=%s", "origin/main"]).trim(), "chore: init");

    let start = start_base::prepare(&r.work, Some("origin/main"), FETCH_TIMEOUT);
    assert_eq!(start, Start { base: Some("origin/main".into()), note: None });
    let made = worktree::add_free(&paths_of(&r), &r.work, "fresh", start.base.as_deref()).unwrap();

    assert_eq!(subject_of(&made.path), "feat: newer on the remote");
    assert_eq!(subject_of(&r.work), "chore: init", "the checkout itself is left alone");
    let tracks = git(&r.work, &["config", "--get", "branch.agent/fresh.remote"]);
    assert!(!tracks.status.success(), "the new branch tracks nothing");
}

#[test]
fn a_fetch_that_fails_falls_back_to_the_copy_already_here_and_says_so() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["remote", "set-url", "origin", r.root.path().join("gone.git").to_str().unwrap()]);

    let start = start_base::prepare(&r.work, Some("origin/main"), FETCH_TIMEOUT);
    assert_eq!(start.base.as_deref(), Some("origin/main"));
    let note = start.note.expect("a note");
    assert!(note.starts_with("Could not fetch origin/main ("), "{note}");
    assert!(note.ends_with("so this started from the copy fetched earlier."), "{note}");
    let made = worktree::add_free(&paths_of(&r), &r.work, "stale", start.base.as_deref()).unwrap();
    assert_eq!(subject_of(&made.path), "chore: init");
}

#[test]
fn with_no_copy_of_the_remote_branch_it_starts_from_the_current_commit() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["remote", "set-url", "origin", r.root.path().join("gone.git").to_str().unwrap()]);
    ok(&r.work, &["update-ref", "-d", "refs/remotes/origin/main"]);

    let start = start_base::prepare(&r.work, Some("origin/main"), FETCH_TIMEOUT);
    assert_eq!(start.base, None);
    assert!(start.note.unwrap().ends_with("so this started from the current branch."));
}

#[test]
fn a_local_branch_is_never_fetched_and_says_nothing() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["remote", "set-url", "origin", r.root.path().join("gone.git").to_str().unwrap()]);
    let start = start_base::prepare(&r.work, Some("main"), FETCH_TIMEOUT);
    assert_eq!(start, Start { base: Some("main".into()), note: None });
}

#[test]
fn the_remote_default_is_what_origin_head_names_else_main_else_master() {
    if !git_available() {
        return;
    }
    let r = repos();
    assert_eq!(start_base::remote_default(&r.work).as_deref(), Some("main"));

    ok(&r.work, &["switch", "-q", "-c", "trunk"]);
    commit_file(&r.work, "t.md", "t\n", "feat: trunk");
    ok(&r.work, &["push", "-q", "-u", "origin", "trunk"]);
    ok(&r.work, &["remote", "set-head", "origin", "trunk"]);
    assert_eq!(start_base::remote_default(&r.work).as_deref(), Some("trunk"));
    assert_eq!(start_base::remote_branches(&r.work).len(), 2);

    ok(&r.work, &["remote", "set-head", "origin", "-d"]);
    assert_eq!(start_base::remote_default(&r.work).as_deref(), Some("main"));

    let lonely = r.root.path().join("lonely");
    std::fs::create_dir_all(&lonely).unwrap();
    ok(&lonely, &["init", "-q", "-b", "main"]);
    assert_eq!(start_base::remote_default(&lonely), None);
}
