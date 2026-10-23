//! Real throwaway repos: what the cleanup card checks before it offers to remove a merged tab.

mod common;

use std::path::Path;

use common::{commit_file, git_available, ok, repos};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree::{self, Worktree};
use tidewatch_core::worktree_cleanup::{Blocker, check};

struct Fixture {
    repos: common::Repos,
    wt: Worktree,
    _home: tempfile::TempDir,
}

fn fixture() -> Option<Fixture> {
    if !git_available() {
        return None;
    }
    let repos = repos();
    commit_file(
        &repos.work,
        ".gitignore",
        ".env*\nscratch.log\nnode_modules/\ntarget/\n",
        "chore: ignore",
    );
    ok(&repos.work, &["push", "-q", "origin", "main"]);
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let wt = worktree::add_named(&paths, &repos.work, "brave-otter", None).unwrap();
    Some(Fixture { repos, wt, _home: home })
}

impl Fixture {
    fn dir(&self) -> &Path {
        &self.wt.path
    }

    fn commit(&self, name: &str) -> String {
        commit_file(self.dir(), name, "work\n", &format!("feat: {name}"));
        ok(self.dir(), &["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn check(&self, pr_head: Option<&str>) -> tidewatch_core::worktree_cleanup::Report {
        check(&self.wt, pr_head, &[]).unwrap()
    }
}

#[test]
fn a_clean_worktree_whose_commits_are_in_the_pull_request_is_safe() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    let report = fx.check(Some(&head));
    assert_eq!(report.blockers(), []);
    assert!(report.is_safe());
}

#[test]
fn a_squash_merge_is_safe_when_nothing_follows_the_pull_request_head() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    let work = &fx.repos.work;
    ok(work, &["merge", "--squash", "agent/brave-otter"]);
    ok(work, &["commit", "-q", "-m", "feat: feature (#1)"]);
    ok(work, &["push", "-q", "origin", "main"]);
    let report = fx.check(Some(&head));
    assert_eq!(report.local_commits, Some(0), "the branch's own commit is not on main by id");
    assert!(report.is_safe());
}

#[test]
fn a_commit_after_the_pull_request_head_blocks() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    fx.commit("afterthought.rs");
    let report = fx.check(Some(&head));
    assert_eq!(report.blockers(), [Blocker::LocalCommits(1)]);
}

#[test]
fn tracked_changes_block_and_staged_ones_count_too() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    std::fs::write(fx.dir().join("feature.rs"), "edited\n").unwrap();
    std::fs::write(fx.dir().join("readme.md"), "staged\n").unwrap();
    ok(fx.dir(), &["add", "readme.md"]);
    let report = fx.check(Some(&head));
    assert_eq!(report.blockers(), [Blocker::Changes(2)]);
    assert_eq!(report.changed.len(), 2);
}

#[test]
fn untracked_files_block_but_ignored_ones_do_not() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    std::fs::write(fx.dir().join("scratch.log"), "noise\n").unwrap();
    assert_eq!(fx.check(Some(&head)).blockers(), []);
    std::fs::write(fx.dir().join("draft.md"), "unsaved idea\n").unwrap();
    let report = fx.check(Some(&head));
    assert_eq!(report.blockers(), [Blocker::Untracked(1)]);
    assert_eq!(report.untracked, ["draft.md"]);
}

#[test]
fn ignored_files_are_listed_except_build_output_and_copies_of_the_main_checkouts() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    for dir in ["node_modules/pkg", "target/debug"] {
        std::fs::create_dir_all(fx.dir().join(dir)).unwrap();
        std::fs::write(fx.dir().join(dir).join("x"), "bin").unwrap();
    }
    std::fs::write(fx.dir().join("scratch.log"), "noise\n").unwrap();
    std::fs::write(fx.dir().join(".env"), "TOKEN=1\n").unwrap();
    std::fs::write(fx.dir().join(".env.local"), "EDITED=1\n").unwrap();
    std::fs::write(fx.repos.work.join(".env"), "TOKEN=1\n").unwrap();
    std::fs::write(fx.repos.work.join(".env.local"), "ORIGINAL=1\n").unwrap();
    let copied = [".env".to_owned(), ".env.local".to_owned()];
    let report = check(&fx.wt, Some(&head), &copied).unwrap();
    assert!(report.is_safe());
    let mut listed = report.ignored.clone();
    listed.sort();
    assert_eq!(listed, [".env.local", "scratch.log"]);
}

#[test]
fn a_linked_folder_is_not_something_the_removal_would_lose() {
    let Some(fx) = fixture() else { return };
    let head = fx.commit("feature.rs");
    let elsewhere = fx.repos.root.path().join("shared-cache");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fx.dir().join("scratch.log")).unwrap();
    let report = fx.check(Some(&head));
    assert_eq!(report.ignored.len(), 0);
}

#[test]
fn without_a_known_head_only_commits_a_remote_has_count_as_safe() {
    let Some(fx) = fixture() else { return };
    fx.commit("feature.rs");
    assert_eq!(fx.check(None).blockers(), [Blocker::LocalCommits(1)]);
    ok(fx.dir(), &["push", "-q", "origin", "HEAD:refs/heads/agent/brave-otter"]);
    ok(fx.dir(), &["fetch", "-q", "origin"]);
    assert_eq!(fx.check(None).blockers(), []);
    fx.commit("later.rs");
    assert_eq!(fx.check(None).blockers(), [Blocker::LocalCommits(1)]);
}

#[test]
fn a_head_this_clone_has_never_seen_falls_back_to_the_remote_check() {
    let Some(fx) = fixture() else { return };
    fx.commit("feature.rs");
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(fx.check(Some(unknown)).blockers(), [Blocker::LocalCommits(1)]);
}

#[test]
fn a_folder_that_is_gone_is_an_error_not_a_pass() {
    let Some(fx) = fixture() else { return };
    let gone = Worktree { path: fx.dir().join("missing"), ..fx.wt.clone() };
    assert!(check(&gone, None, &[]).is_err());
}
