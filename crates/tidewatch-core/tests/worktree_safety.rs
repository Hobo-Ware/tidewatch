//! The removal check against real throwaway repos: what is proven safe, what is not, and that
//! `remove` itself refuses the rest. Every test builds its own repo and home in a temp dir.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::removal_log;
use tidewatch_core::worktree::{
    self, BranchChoice, BranchOutcome, RemoveMode, Terms, Why, Worktree, WorktreeError,
};
use tidewatch_core::worktree_safety::{Holder, Risk, Safety, Watch};

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    wt: Worktree,
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

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("work").join("web");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let paths = TidewatchPaths::new(&home);
    let wt = worktree::add_named(&paths, &repo, "upbeat-lantern", None).unwrap();
    Fixture { _dir: dir, home, repo, paths, wt }
}

fn commit(wt: &Worktree, file: &str) -> String {
    fs::write(wt.path.join(file), file).unwrap();
    git(&wt.path, &["add", file]);
    git(&wt.path, &["commit", "-q", "-m", file]);
    git(&wt.path, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn with_remote(f: &Fixture) -> PathBuf {
    let bare = f.home.join("origin.git");
    git(&f.home, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    git(&f.repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    bare
}

struct Quiet;

impl Watch for Quiet {
    fn holders(&self, _: &Path) -> Result<Vec<Holder>, String> {
        Ok(Vec::new())
    }
    fn session_age(&self, _: &Path) -> Option<Duration> {
        None
    }
}

struct Seen {
    holders: Result<Vec<Holder>, String>,
    age: Option<Duration>,
}

impl Watch for Seen {
    fn holders(&self, _: &Path) -> Result<Vec<Holder>, String> {
        self.holders.clone()
    }
    fn session_age(&self, _: &Path) -> Option<Duration> {
        self.age
    }
}

fn check(f: &Fixture, watch: &dyn Watch) -> Safety {
    worktree::inspect_watched(&f.wt, None, watch)
}

type Setup = Box<dyn Fn(&Fixture) -> Box<dyn Watch>>;

fn terms(branch: BranchChoice, mode: RemoveMode) -> Terms {
    Terms::new(branch, mode).why(Why::Leftover)
}

fn branches(repo: &Path) -> Vec<String> {
    git(repo, &["branch", "--format=%(refname:short)"]).lines().map(str::to_owned).collect()
}

#[test]
fn a_clean_worktree_with_nothing_unmerged_is_safe() {
    let f = fixture();
    assert_eq!(check(&f, &Quiet), Safety::Safe);
}

#[test]
fn uncommitted_files_make_it_unsafe() {
    let f = fixture();
    fs::write(f.wt.path.join("scratch.txt"), "wip").unwrap();
    fs::write(f.wt.path.join("README.md"), "edited\n").unwrap();
    assert_eq!(check(&f, &Quiet), Safety::Unsafe(vec![Risk::Uncommitted { files: 2 }]));
}

#[test]
fn a_commit_that_is_on_no_remote_and_not_merged_makes_it_unsafe() {
    let f = fixture();
    commit(&f.wt, "a.txt");
    commit(&f.wt, "b.txt");
    assert_eq!(check(&f, &Quiet), Safety::Unsafe(vec![Risk::Unpushed { commits: 2 }]));
}

#[test]
fn pushing_the_branch_or_merging_it_makes_its_commits_safe() {
    let f = fixture();
    with_remote(&f);
    commit(&f.wt, "a.txt");
    assert_eq!(check(&f, &Quiet), Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }]));
    git(&f.wt.path, &["push", "-q", "origin", &f.wt.branch]);
    assert_eq!(check(&f, &Quiet), Safety::Safe, "found on a remote branch");

    let g = fixture();
    commit(&g.wt, "a.txt");
    git(&g.repo, &["merge", "-q", &g.wt.branch]);
    assert_eq!(check(&g, &Quiet), Safety::Safe, "merged into the base");
}

#[test]
fn commits_a_merged_pull_request_head_holds_count_as_delivered() {
    let f = fixture();
    let tip = commit(&f.wt, "a.txt");
    assert_eq!(
        worktree::inspect_watched(&f.wt, None, &Quiet),
        Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }])
    );
    assert_eq!(worktree::inspect_watched(&f.wt, Some(&tip), &Quiet), Safety::Safe);
    assert_eq!(
        worktree::inspect_watched(&f.wt, Some("0000000000000000000000000000000000000000"), &Quiet),
        Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }]),
        "a head this clone does not have proves nothing"
    );
}

#[test]
fn a_commit_on_a_detached_head_is_not_safe() {
    let f = fixture();
    git(&f.wt.path, &["checkout", "-q", "--detach"]);
    commit(&f.wt, "lonely.txt");
    assert_eq!(check(&f, &Quiet), Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }]));
}

#[test]
fn a_git_error_is_unknown_and_never_safe() {
    let broken_link = fixture();
    fs::write(broken_link.wt.path.join(".git"), "gitdir: /nowhere/at/all\n").unwrap();
    assert!(matches!(check(&broken_link, &Quiet), Safety::Unknown(_)));

    let no_git = fixture();
    fs::remove_file(no_git.wt.path.join(".git")).unwrap();
    assert!(matches!(check(&no_git, &Quiet), Safety::Unknown(_)));

    let no_branch = fixture();
    git(&no_branch.repo, &["update-ref", "-d", &format!("refs/heads/{}", no_branch.wt.branch)]);
    assert!(matches!(check(&no_branch, &Quiet), Safety::Unknown(_)));

    let no_repo = fixture();
    fs::rename(&no_repo.repo, no_repo.home.join("moved")).unwrap();
    assert!(matches!(check(&no_repo, &Quiet), Safety::Unknown(_)));
}

#[test]
fn a_parent_repository_is_never_mistaken_for_the_worktree() {
    let f = fixture();
    fs::remove_file(f.wt.path.join(".git")).unwrap();
    git(&f.wt.path, &["init", "-q", "-b", "main"]);
    assert!(matches!(check(&f, &Quiet), Safety::Unknown(_)), "its .git is not a linked worktree");
}

#[test]
fn a_process_in_the_folder_makes_it_unsafe_and_an_unreadable_list_makes_it_unknown() {
    let f = fixture();
    let running = Holder { pid: 4242, command: "claude --permission-mode auto".into() };
    let seen = Seen { holders: Ok(vec![running.clone()]), age: None };
    assert_eq!(
        check(&f, &seen),
        Safety::Unsafe(vec![Risk::Running { pid: 4242, command: running.command }])
    );
    let blind = Seen { holders: Err("no lsof".into()), age: None };
    assert!(matches!(check(&f, &blind), Safety::Unknown(_)));
}

#[test]
fn a_recent_agent_session_makes_it_unsafe_and_an_old_one_does_not() {
    let f = fixture();
    let recent = Seen { holders: Ok(Vec::new()), age: Some(Duration::from_hours(3)) };
    assert_eq!(check(&f, &recent), Safety::Unsafe(vec![Risk::RecentSession { minutes: 180 }]));
    let old = Seen { holders: Ok(Vec::new()), age: Some(Duration::from_hours(30)) };
    assert_eq!(check(&f, &old), Safety::Safe);
}

#[test]
fn removal_refuses_whatever_is_not_proven_safe_and_touches_nothing() {
    let cases: Vec<(&str, Setup)> = vec![
        (
            "dirty",
            Box::new(|f| {
                fs::write(f.wt.path.join("wip.txt"), "wip").unwrap();
                Box::new(Quiet)
            }),
        ),
        (
            "unpushed",
            Box::new(|f| {
                commit(&f.wt, "a.txt");
                Box::new(Quiet)
            }),
        ),
        (
            "running",
            Box::new(|_| {
                Box::new(Seen {
                    holders: Ok(vec![Holder { pid: 1, command: "zsh".into() }]),
                    age: None,
                })
            }),
        ),
        (
            "recent session",
            Box::new(|_| Box::new(Seen { holders: Ok(Vec::new()), age: Some(Duration::ZERO) })),
        ),
        (
            "unknown",
            Box::new(|f| {
                fs::write(f.wt.path.join(".git"), "gitdir: /nowhere\n").unwrap();
                Box::new(Quiet)
            }),
        ),
    ];
    for (name, setup) in cases {
        let choices: &[BranchChoice] = if name == "unpushed" {
            &[BranchChoice::Delete]
        } else {
            &[BranchChoice::Keep, BranchChoice::Delete]
        };
        for choice in choices {
            let f = fixture();
            let watch = setup(&f);
            let err = worktree::remove_watched(
                &f.paths,
                &f.wt,
                &terms(*choice, RemoveMode::Safe),
                watch.as_ref(),
            )
            .unwrap_err();
            assert!(matches!(err, WorktreeError::Unsafe(_)), "{name}: {err}");
            assert!(f.wt.path.exists(), "{name}: the folder stays");
            assert!(branches(&f.repo).contains(&f.wt.branch), "{name}: the branch stays");
            assert_eq!(removal_log::load(&f.paths), [], "{name}: nothing was logged");
        }
    }
}

#[test]
fn unpushed_commits_stop_a_branch_delete_but_not_a_removal_that_keeps_the_branch() {
    let f = fixture();
    let tip = commit(&f.wt, "a.txt");
    let err = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Delete, RemoveMode::Safe),
        &Quiet,
    )
    .unwrap_err();
    assert!(matches!(err, WorktreeError::Unsafe(_)), "{err}");
    assert!(f.wt.path.exists());

    let outcome = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Keep, RemoveMode::Safe),
        &Quiet,
    )
    .unwrap();
    assert_eq!(outcome, BranchOutcome::Kept);
    assert!(!f.wt.path.exists());
    assert_eq!(git(&f.repo, &["rev-parse", &f.wt.branch]).trim(), tip, "the commit is kept");
}

#[test]
fn a_discard_removes_everything_and_the_log_gives_the_one_command_that_brings_the_branch_back() {
    let f = fixture();
    let tip = commit(&f.wt, "a.txt");
    fs::write(f.wt.path.join("wip.txt"), "wip").unwrap();

    let outcome = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Delete, RemoveMode::Discard),
        &Quiet,
    )
    .unwrap();

    let BranchOutcome::Deleted { restore } = outcome else { panic!("deleted: {outcome:?}") };
    assert_eq!(restore, Some(format!("git branch {} {tip}", f.wt.branch)));
    assert!(!f.wt.path.exists());
    assert_eq!(branches(&f.repo), ["main"]);
    let logged = removal_log::load(&f.paths);
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].branch, f.wt.branch);
    assert_eq!(logged[0].sha.as_deref(), Some(tip.as_str()));
    assert_eq!(logged[0].path, f.wt.path);
    assert_eq!(logged[0].mode, "discard");
    assert_eq!(logged[0].why, "leftover worktree");
    assert_eq!(logged[0].lost.len(), 2, "{:?}", logged[0].lost);

    git(&f.repo, &["branch", &f.wt.branch, &tip]);
    assert!(branches(&f.repo).contains(&f.wt.branch), "undone by the logged command");
}

#[test]
fn a_discard_that_keeps_the_branch_does_not_list_its_commits_as_lost() {
    let f = fixture();
    commit(&f.wt, "a.txt");
    worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Keep, RemoveMode::Discard),
        &Quiet,
    )
    .unwrap();
    assert_eq!(removal_log::load(&f.paths)[0].lost, Vec::<String>::new());
}

#[test]
fn a_safe_delete_uses_the_gentle_branch_delete_and_leaves_a_branch_git_calls_unmerged() {
    let f = fixture();
    with_remote(&f);
    commit(&f.wt, "a.txt");
    git(&f.wt.path, &["push", "-q", "origin", &f.wt.branch]);

    let outcome = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Delete, RemoveMode::Safe),
        &Quiet,
    )
    .unwrap();

    assert!(matches!(outcome, BranchOutcome::DeleteFailed(_)), "{outcome:?}");
    assert!(branches(&f.repo).contains(&f.wt.branch), "-d refused: not merged into main");
    assert!(!f.wt.path.exists());
}

#[test]
fn delivered_commits_let_a_safe_delete_take_the_branch() {
    let f = fixture();
    let tip = commit(&f.wt, "a.txt");
    let terms = terms(BranchChoice::Delete, RemoveMode::Safe).delivered(Some(tip.clone()));
    let outcome = worktree::remove_watched(&f.paths, &f.wt, &terms, &Quiet).unwrap();
    assert_eq!(
        outcome,
        BranchOutcome::Deleted { restore: Some(format!("git branch {} {tip}", f.wt.branch)) }
    );
    assert_eq!(branches(&f.repo), ["main"]);
}

#[test]
fn a_removal_the_log_cannot_record_does_not_happen() {
    let f = fixture();
    fs::create_dir_all(f.paths.removals_log()).unwrap();
    let err = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Keep, RemoveMode::Safe),
        &Quiet,
    )
    .unwrap_err();
    assert!(matches!(err, WorktreeError::Io(_)), "{err}");
    assert!(f.wt.path.exists());
}

#[test]
fn removing_after_the_folder_vanished_still_needs_the_branch_to_be_safe() {
    let f = fixture();
    commit(&f.wt, "a.txt");
    fs::remove_dir_all(&f.wt.path).unwrap();
    let err = worktree::remove_watched(
        &f.paths,
        &f.wt,
        &terms(BranchChoice::Delete, RemoveMode::Safe),
        &Quiet,
    )
    .unwrap_err();
    assert!(matches!(err, WorktreeError::Unsafe(_)), "{err}");
    assert!(branches(&f.repo).contains(&f.wt.branch));
}
