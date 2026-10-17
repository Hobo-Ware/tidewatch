//! Round trips against real throwaway repos: build commits on a main checkout and on worktree
//! branches, then load the history, ahead/behind, file lists and diffs the panes show.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kelp_core::commit::ChangeKind;
use kelp_core::diff::{Body, LineKind};
use tempfile::TempDir;
use tidewatch_core::history::{self, Loaded, Scope, Source};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree::{self, Worktree};

struct Fixture {
    _dir: TempDir,
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

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn titles(loaded: &Loaded) -> Vec<String> {
    (0..loaded.history.len())
        .map(|row| loaded.summaries.get(loaded.history.id(row)).title)
        .collect()
}

fn sorted(mut list: Vec<String>) -> Vec<String> {
    list.sort();
    list
}

fn assert_send<T: Send>() {}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("work").join("web");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    commit(&repo, "README.md", "hello\n", "c1 init");
    commit(&repo, "app.txt", "one\ntwo\nthree\n", "c2 app");
    Fixture { paths: TidewatchPaths::new(&home), repo, _dir: dir }
}

fn agent(f: &Fixture, name: &str) -> Worktree {
    worktree::add_named(&f.paths, &f.repo, name, None).unwrap()
}

#[test]
fn history_is_loadable_on_a_worker_thread() {
    assert_send::<Loaded>();
}

#[test]
fn a_worktree_branch_shows_its_commits_and_the_main_commits_under_them() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    commit(&a.path, "a.txt", "a\n", "a1 first");
    commit(&a.path, "a.txt", "a\nb\n", "a2 second");
    commit(&f.repo, "later.txt", "x\n", "c3 later on main");
    let b = agent(&f, "calm-comet");
    commit(&b.path, "b.txt", "b\n", "b1 other agent");

    let loaded = history::load(&a.path, Scope::Current).unwrap();

    assert_eq!(titles(&loaded), ["a2 second", "a1 first", "c2 app", "c1 init"]);
    assert_eq!(loaded.branch.as_deref(), Some("agent/brave-otter"));
    assert_eq!(loaded.head_row, Some(0));
    assert_ne!(loaded.author, "");
}

#[test]
fn the_whole_repo_shows_every_branch() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    commit(&a.path, "a.txt", "a\n", "a1 first");
    commit(&f.repo, "later.txt", "x\n", "c3 later on main");
    let b = agent(&f, "calm-comet");
    commit(&b.path, "b.txt", "b\n", "b1 other agent");

    let loaded = history::load(&a.path, Scope::Whole).unwrap();

    assert_eq!(
        sorted(titles(&loaded)),
        ["a1 first", "b1 other agent", "c1 init", "c2 app", "c3 later on main"]
    );
    assert_eq!(loaded.history.refs.labels.iter().filter(|l| !l.hidden).count(), 3);
}

#[test]
fn the_main_checkout_shows_its_own_branch_only() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    commit(&a.path, "a.txt", "a\n", "a1 first");
    commit(&f.repo, "later.txt", "x\n", "c3 later on main");

    let loaded = history::load(&f.repo, Scope::Current).unwrap();

    assert_eq!(titles(&loaded), ["c3 later on main", "c2 app", "c1 init"]);
    assert_eq!(loaded.branch.as_deref(), Some("main"));
    assert_eq!(loaded.base, None);
}

#[test]
fn ahead_and_behind_count_against_the_default_branch() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    commit(&a.path, "a.txt", "a\n", "a1 first");
    commit(&a.path, "a.txt", "a\nb\n", "a2 second");
    commit(&f.repo, "later.txt", "x\n", "c3 later on main");

    let base = history::load(&a.path, Scope::Current).unwrap().base.unwrap();

    assert_eq!((base.base.as_str(), base.ahead, base.behind), ("main", 2, 1));
    assert_eq!(base.describe(), "2 ahead · 1 behind main");
}

#[test]
fn a_fresh_worktree_is_the_same_as_main() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    let base = history::load(&a.path, Scope::Current).unwrap().base.unwrap();
    assert_eq!(base.describe(), "same as main");
}

#[test]
fn the_default_branch_falls_back_to_master_and_is_none_without_either() {
    let f = fixture();
    assert_eq!(history::default_branch(&f.repo).as_deref(), Some("main"));
    git(&f.repo, &["branch", "-m", "main", "master"]);
    assert_eq!(history::default_branch(&f.repo).as_deref(), Some("master"));
    git(&f.repo, &["branch", "-m", "master", "trunk"]);
    assert_eq!(history::default_branch(&f.repo), None);
}

#[test]
fn the_default_branch_follows_origin_head() {
    let f = fixture();
    git(&f.repo, &["branch", "develop"]);
    git(&f.repo, &["update-ref", "refs/remotes/origin/develop", "HEAD"]);
    git(&f.repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/develop"]);
    assert_eq!(history::default_branch(&f.repo).as_deref(), Some("develop"));
}

#[test]
fn a_detached_head_has_no_branch_and_still_loads() {
    let f = fixture();
    git(&f.repo, &["checkout", "-q", "--detach", "HEAD~1"]);
    let loaded = history::load(&f.repo, Scope::Current).unwrap();
    assert_eq!(loaded.branch, None);
    assert_eq!(titles(&loaded), ["c1 init"]);
    assert_eq!(loaded.head_row, Some(0));
}

#[test]
fn a_commit_lists_its_files_and_diffs_each_one() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    commit(&a.path, "app.txt", "one\n2\nthree\n", "a1 edit");
    let head = history::load(&a.path, Scope::Current).unwrap();
    let source = Source::Commit(head.history.id(0));

    let files = history::changed_files(&a.path, source).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].path.as_str(), files[0].kind), ("app.txt", ChangeKind::Modified));

    let diff = history::file_diff(&a.path, source, "app.txt").unwrap();
    assert_eq!((diff.added, diff.removed), (1, 1));
    let Body::Text(hunks) = &diff.body else { panic!("a text file has a text body") };
    let kinds: Vec<_> = hunks[0].lines.iter().map(|l| l.kind).collect();
    assert_eq!(kinds, [LineKind::Context, LineKind::Removed, LineKind::Added, LineKind::Context]);
}

#[test]
fn the_working_tree_lists_edits_and_new_files_and_diffs_against_head() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    assert!(history::is_clean(&a.path).unwrap());
    fs::write(a.path.join("app.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    fs::write(a.path.join("new.txt"), "fresh\n").unwrap();
    assert!(!history::is_clean(&a.path).unwrap());

    let files = history::changed_files(&a.path, Source::WorkTree).unwrap();
    let listed: Vec<_> = files.iter().map(|c| (c.path.as_str(), c.kind)).collect();
    assert_eq!(listed, [("app.txt", ChangeKind::Modified), ("new.txt", ChangeKind::Added)]);

    let edit = history::file_diff(&a.path, Source::WorkTree, "app.txt").unwrap();
    assert_eq!((edit.added, edit.removed), (1, 0));
    let fresh = history::file_diff(&a.path, Source::WorkTree, "new.txt").unwrap();
    assert_eq!((fresh.added, fresh.removed), (1, 0));
}

#[test]
fn the_working_tree_of_a_repo_without_commits_diffs_against_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "x\ny\n").unwrap();

    let loaded = history::load(&repo, Scope::Current).unwrap();
    assert_eq!(loaded.history.len(), 0);
    let diff = history::file_diff(&repo, Source::WorkTree, "a.txt").unwrap();
    assert_eq!((diff.added, diff.removed), (2, 0));
}

#[test]
fn watching_reports_a_commit_made_in_the_checkout() {
    let f = fixture();
    let a = agent(&f, "brave-otter");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _watcher = history::watch(&a.path, move |change| {
        let _ = sender.send(change);
    })
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    commit(&a.path, "w.txt", "w\n", "w1 watched");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut saw_history = false;
    while !saw_history && std::time::Instant::now() < deadline {
        if let Ok(change) = receiver.recv_timeout(std::time::Duration::from_millis(200)) {
            saw_history = change.history;
        }
    }
    assert!(saw_history, "a commit never reached the watcher");
}
