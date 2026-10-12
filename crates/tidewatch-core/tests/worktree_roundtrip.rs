//! Round trips against real throwaway repos: every test builds its own repo and home folder in a
//! temp dir and runs the actual `git` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree::{
    self, BranchChoice, BranchOutcome, Inspection, RemoveMode, WorktreeError,
};

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=test@example.com", "-c", "user.name=Test"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
    fs::write(path.join("README.md"), "hello\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "init"]);
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("work").join("web");
    init_repo(&repo);
    let paths = TidewatchPaths::new(&home);
    Fixture { _dir: dir, home, repo, paths }
}

fn branches(repo: &Path) -> Vec<String> {
    git(repo, &["branch", "--format=%(refname:short)"]).lines().map(str::to_owned).collect()
}

fn assert_only_main(repo: &Path) {
    let listed: Vec<PathBuf> = worktree::list(repo).unwrap().into_iter().map(|w| w.path).collect();
    assert_eq!(listed, vec![repo.to_path_buf()]);
}

#[test]
fn add_makes_a_locked_worktree_on_a_new_agent_branch() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();

    assert!(wt.path.join("README.md").is_file());
    assert!(wt.path.starts_with(f.paths.repo_dir(&f.repo)));
    assert_eq!(wt.branch, format!("agent/{}", wt.name()));
    assert_eq!(wt.path.file_name().unwrap().to_string_lossy(), wt.name());
    let listed = worktree::list(&f.repo).unwrap();
    let ours = listed.iter().find(|w| w.path == wt.path).expect("git lists the worktree");
    assert_eq!(ours.branch.as_deref(), Some(wt.branch.as_str()));
    assert_eq!(ours.locked.as_deref(), Some("tidewatch"));
    assert!(branches(&f.repo).contains(&wt.branch));
}

#[test]
fn two_adds_get_two_names() {
    let f = fixture();
    let a = worktree::add(&f.paths, &f.repo).unwrap();
    let b = worktree::add(&f.paths, &f.repo).unwrap();
    assert_ne!(a.name(), b.name());
    assert_ne!(a.path, b.path);
}

#[test]
fn a_taken_branch_name_is_refused_and_leaves_nothing() {
    let f = fixture();
    git(&f.repo, &["branch", "agent/dup"]);
    let err = worktree::add_named(&f.paths, &f.repo, "dup").unwrap_err();
    assert!(matches!(&err, WorktreeError::Git(m) if m.contains("already exists")), "{err}");
    assert!(!f.paths.worktree_dir(&f.repo, "dup").exists());
    assert_only_main(&f.repo);
    assert_eq!(branches(&f.repo), ["agent/dup", "main"]);
}

#[test]
fn a_taken_folder_is_refused_without_leaving_a_branch() {
    let f = fixture();
    let taken = f.paths.worktree_dir(&f.repo, "busy");
    fs::create_dir_all(&taken).unwrap();
    fs::write(taken.join("keep.txt"), "mine").unwrap();
    let err = worktree::add_named(&f.paths, &f.repo, "busy").unwrap_err();
    assert_eq!(err, WorktreeError::PathTaken(taken.clone()));
    assert_eq!(branches(&f.repo), ["main"]);
    assert!(taken.join("keep.txt").is_file());
}

#[cfg(unix)]
#[test]
fn a_failing_checkout_hook_rolls_the_whole_attempt_back() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let hook = f.repo.join(".git/hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    let err = worktree::add_named(&f.paths, &f.repo, "hooked").unwrap_err();
    assert!(matches!(err, WorktreeError::Git(_)), "{err}");
    assert!(!f.paths.worktree_dir(&f.repo, "hooked").exists());
    assert_eq!(branches(&f.repo), ["main"]);
    assert_only_main(&f.repo);
}

#[test]
fn a_repo_without_commits_cannot_get_a_worktree_and_stays_clean() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("empty");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let paths = TidewatchPaths::new(&home);

    let err = worktree::add(&paths, &repo).unwrap_err();
    assert!(matches!(err, WorktreeError::Git(_)), "{err}");
    assert_eq!(worktree::leftovers(&paths, &[]), []);
    assert!(branches(&repo).iter().all(|b| !b.starts_with("agent/")));
}

#[test]
fn removing_a_clean_worktree_keeps_the_branch_and_the_folder_is_gone() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let outcome = worktree::remove(&f.paths, &wt, BranchChoice::Keep, RemoveMode::Safe).unwrap();

    assert_eq!(outcome, BranchOutcome::Kept);
    assert!(!wt.path.exists());
    assert!(!f.paths.repo_dir(&f.repo).exists(), "the empty per-repo folder is cleaned up too");
    assert_only_main(&f.repo);
    assert!(branches(&f.repo).contains(&wt.branch));
    assert_eq!(worktree::leftovers(&f.paths, &[]), []);
}

#[test]
fn removing_with_delete_branch_deletes_it() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let outcome = worktree::remove(&f.paths, &wt, BranchChoice::Delete, RemoveMode::Safe).unwrap();

    assert_eq!(outcome, BranchOutcome::Deleted);
    assert!(!wt.path.exists());
    assert_eq!(branches(&f.repo), ["main"]);
    assert_only_main(&f.repo);
}

#[test]
fn a_dirty_worktree_is_refused_until_the_caller_discards() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    fs::write(wt.path.join("scratch.txt"), "wip").unwrap();
    fs::write(wt.path.join("README.md"), "edited\n").unwrap();

    let err = worktree::remove(&f.paths, &wt, BranchChoice::Delete, RemoveMode::Safe).unwrap_err();
    assert_eq!(err, WorktreeError::Dirty { changed: 2 });
    assert!(wt.path.join("scratch.txt").is_file(), "nothing was touched");
    let still = worktree::list(&f.repo).unwrap();
    assert_eq!(
        still.iter().find(|w| w.path == wt.path).and_then(|w| w.locked.as_deref()),
        Some("tidewatch"),
        "a refused removal keeps the lock"
    );

    let outcome =
        worktree::remove(&f.paths, &wt, BranchChoice::Delete, RemoveMode::Discard).unwrap();
    assert_eq!(outcome, BranchOutcome::Deleted);
    assert!(!wt.path.exists());
}

#[test]
fn inspect_counts_changed_files_and_commits_found_nowhere_else() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    assert_eq!(worktree::inspect(&wt), Inspection { changed: 0, unmerged: 0 });

    fs::write(wt.path.join("a.txt"), "a").unwrap();
    git(&wt.path, &["add", "a.txt"]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    fs::write(wt.path.join("b.txt"), "b").unwrap();
    assert_eq!(worktree::inspect(&wt), Inspection { changed: 1, unmerged: 1 });

    git(&f.repo, &["merge", "-q", &wt.branch]);
    assert_eq!(worktree::inspect(&wt).unmerged, 0, "merged into main, so it exists elsewhere");
}

#[test]
fn removing_after_the_folder_was_deleted_by_hand_still_cleans_up() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    fs::remove_dir_all(&wt.path).unwrap();

    let outcome = worktree::remove(&f.paths, &wt, BranchChoice::Delete, RemoveMode::Safe).unwrap();
    assert_eq!(outcome, BranchOutcome::Deleted);
    assert_only_main(&f.repo);
    assert_eq!(branches(&f.repo), ["main"]);
}

#[test]
fn removing_with_an_unreachable_repo_reports_git_s_error() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let lost = tidewatch_core::worktree::Worktree { repo: f.home.join("nowhere"), ..wt.clone() };
    let err =
        worktree::remove(&f.paths, &lost, BranchChoice::Keep, RemoveMode::Discard).unwrap_err();
    assert!(matches!(err, WorktreeError::Git(_)), "{err}");
    assert!(wt.path.exists());
}

#[test]
fn a_moved_repo_is_repaired_and_its_worktree_works_again() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let moved = f.home.join("elsewhere").join("web");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&f.repo, &moved).unwrap();
    let broken = Command::new("git").arg("-C").arg(&wt.path).arg("status").output().unwrap();
    assert!(!broken.status.success(), "the worktree is broken until repaired");

    let repaired = worktree::repair(&f.paths, &moved);

    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].repo, moved);
    assert_eq!(repaired[0].path, wt.path);
    git(&wt.path, &["status"]);
    let listed = worktree::list(&moved).unwrap();
    let ours = listed.iter().find(|w| w.path == wt.path).expect("listed from the new location");
    assert!(!ours.prunable);

    let outcome =
        worktree::remove(&f.paths, &repaired[0], BranchChoice::Delete, RemoveMode::Safe).unwrap();
    assert_eq!(outcome, BranchOutcome::Deleted);
    assert!(!wt.path.exists());
    assert_eq!(worktree::leftovers(&f.paths, &[]), []);
}

#[test]
fn repair_leaves_worktrees_of_other_repos_alone() {
    let f = fixture();
    let other = f.home.join("personal").join("web");
    init_repo(&other);
    let wt = worktree::add(&f.paths, &f.repo).unwrap();

    assert_eq!(worktree::repair(&f.paths, &other), []);
    git(&wt.path, &["status"]);
}

#[test]
fn leftovers_are_worktree_folders_without_an_open_tab() {
    let f = fixture();
    let a = worktree::add(&f.paths, &f.repo).unwrap();
    let b = worktree::add(&f.paths, &f.repo).unwrap();
    let plain = f.paths.repo_dir(&f.repo).join("not-a-worktree");
    fs::create_dir_all(&plain).unwrap();

    let all = worktree::leftovers(&f.paths, &[]);
    let mut paths: Vec<&PathBuf> = all.iter().map(|l| &l.path).collect();
    paths.sort();
    let mut want = vec![&a.path, &b.path];
    want.sort();
    assert_eq!(paths, want);
    assert!(all.iter().all(|l| l.branch.is_some()), "branches come from the registry");

    let left = worktree::leftovers(&f.paths, std::slice::from_ref(&a.path));
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].path, b.path);
    assert_eq!(left[0].branch.as_deref(), Some(b.branch.as_str()));
}

#[test]
fn a_worktree_git_made_outside_tidewatch_is_a_leftover_without_a_branch() {
    let f = fixture();
    let by_hand = f.paths.worktree_dir(&f.repo, "by-hand");
    fs::create_dir_all(by_hand.parent().unwrap()).unwrap();
    git(&f.repo, &["worktree", "add", "-q", "-b", "hand", by_hand.to_str().unwrap()]);

    let left = worktree::leftovers(&f.paths, &[]);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].path, by_hand);
    assert_eq!(left[0].branch, None);
}
