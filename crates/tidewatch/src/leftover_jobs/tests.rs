use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::worktree::{BranchChoice, RemoveMode};
use tidewatch_core::worktree_safety::Risk;

use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    paths: TidewatchPaths,
    jobs: Jobs<Done>,
    ctx: egui::Context,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    Fixture {
        paths: TidewatchPaths::new(&home),
        repo,
        jobs: Jobs::new(),
        ctx: egui::Context::default(),
        _dir: dir,
    }
}

fn next(jobs: &Jobs<Done>) -> Done {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(done) = jobs.drain().into_iter().next() {
            return done;
        }
        assert!(Instant::now() < deadline, "the job never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn removal(worktree: &Worktree, branch: BranchChoice, mode: RemoveMode) -> Removal {
    Removal { worktree: worktree.clone(), branch, mode }
}

#[test]
fn inspecting_names_uncommitted_files_and_commits_found_nowhere_else() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    std::fs::write(wt.path.join("a.txt"), "a").unwrap();
    git(&wt.path, &["add", "a.txt"]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    std::fs::write(wt.path.join("b.txt"), "b").unwrap();
    inspect(&f.jobs, &f.ctx, f.paths.clone(), wt.clone());
    let Done::Inspected { path, safety } = next(&f.jobs) else { panic!("expected Inspected") };
    assert_eq!(path, wt.path);
    assert_eq!(
        safety,
        Safety::Unsafe(vec![Risk::Uncommitted { files: 1 }, Risk::Unpushed { commits: 1 }])
    );
}

#[test]
fn a_clean_worktree_is_removed_with_its_empty_branch() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Delete, RemoveMode::Safe),
        None,
    );
    let Done::Removed { path, result } = next(&f.jobs) else { panic!("expected Removed") };
    assert_eq!(path, wt.path);
    assert!(matches!(result, Ok(BranchOutcome::Deleted { .. })), "{result:?}");
    assert!(!wt.path.exists());
    assert_eq!(git(&f.repo, &["branch", "--list", &wt.branch]).trim(), "");
}

#[test]
fn a_dirty_worktree_is_refused_unless_the_discard_is_explicit_and_nothing_is_lost_on_refusal() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    std::fs::write(wt.path.join("keep-me.txt"), "precious").unwrap();
    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Delete, RemoveMode::Safe),
        None,
    );
    let Done::Removed { result, .. } = next(&f.jobs) else { panic!("expected Removed") };
    assert_eq!(result, Err("not removed: 1 file with uncommitted changes".to_owned()));
    assert_eq!(std::fs::read_to_string(wt.path.join("keep-me.txt")).unwrap(), "precious");

    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Keep, RemoveMode::Discard),
        None,
    );
    let Done::Removed { result, .. } = next(&f.jobs) else { panic!("expected Removed") };
    assert_eq!(result, Ok(BranchOutcome::Kept));
    assert!(!wt.path.exists());
}

#[test]
fn a_branch_with_unmerged_commits_survives_when_the_removal_keeps_it() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    std::fs::write(wt.path.join("a.txt"), "a").unwrap();
    git(&wt.path, &["add", "a.txt"]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Keep, RemoveMode::Safe),
        None,
    );
    let Done::Removed { result, .. } = next(&f.jobs) else { panic!("expected Removed") };
    assert_eq!(result, Ok(BranchOutcome::Kept));
    assert_eq!(git(&f.repo, &["log", "--oneline", &wt.branch]).lines().count(), 2);
}

#[test]
fn removing_a_leftover_takes_its_folder_trust_entry_away_too() {
    use tidewatch_core::trust::{TrustCli, is_trusted};
    use tidewatch_core::trust_apply::ConfigFiles;
    let f = fixture();
    let home = f.paths.root().parent().unwrap().to_path_buf();
    let claude_json = home.join(".claude.json");
    std::fs::write(&claude_json, "{\n  \"numStartups\": 1\n}\n").unwrap();
    let grant = Grant::new(&f.paths, ConfigFiles::resolve(&home, |_| None));
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let _ = grant.trust_worktree(TrustCli::Claude, &wt.path);
    let folder = wt.path.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(
        is_trusted(TrustCli::Claude, &std::fs::read_to_string(&claude_json).unwrap(), &folder)
            .unwrap()
    );
    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Delete, RemoveMode::Safe),
        Some(grant),
    );
    let Done::Removed { result, .. } = next(&f.jobs) else { panic!("expected Removed") };
    assert!(result.is_ok());
    assert_eq!(std::fs::read_to_string(&claude_json).unwrap(), "{\n  \"numStartups\": 1\n}\n");
}

#[test]
fn a_leftover_with_commits_found_nowhere_else_is_refused_when_its_branch_would_go() {
    let f = fixture();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    std::fs::write(wt.path.join("a.txt"), "a").unwrap();
    git(&wt.path, &["add", "a.txt"]);
    git(&wt.path, &["commit", "-q", "-m", "work"]);
    remove(
        &f.jobs,
        &f.ctx,
        f.paths.clone(),
        removal(&wt, BranchChoice::Delete, RemoveMode::Safe),
        None,
    );
    let Done::Removed { result, .. } = next(&f.jobs) else { panic!("expected Removed") };
    assert!(result.is_err(), "{result:?}");
    assert!(wt.path.exists());
    assert_eq!(git(&f.repo, &["log", "--oneline", &wt.branch]).lines().count(), 2);
}
