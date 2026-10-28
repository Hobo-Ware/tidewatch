use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::agents::Level;
use tidewatch_core::restore::TabAction;
use tidewatch_core::setup_recipe::{Recipe, Store};
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree::{BranchChoice, RemoveMode, Terms, Why};
use tidewatch_core::worktree_safety::Owner;

use super::*;

const SESSION: &str = "0c2cbc96-1111-4222-8333-444455556666";

struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    path_var: OsString,
}

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

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\necho fake claude\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path_var = OsString::from(format!("{}:/usr/bin:/bin", bin.display()));
    Fixture { paths: TidewatchPaths::new(&home), _dir: dir, home, repo, path_var }
}

fn tab() -> Tab {
    Tab {
        title: "heron".into(),
        agent: "claude".into(),
        profile: "builder".into(),
        level: Level::Edits,
        session_id: Some(SESSION.into()),
    }
}

fn close_with_discard(f: &Fixture) -> Closed {
    let made = worktree::add_named(&f.paths, &f.repo, "heron", None).unwrap();
    let dir = f.home.join(".claude/projects").join(claude_project_dir_name(&made.path));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{SESSION}.jsonl")), "{}\n").unwrap();
    let terms = Terms::new(BranchChoice::Delete, RemoveMode::Discard)
        .why(Why::ClosedTab)
        .owner(Owner::ThisWindow)
        .closed(Some(tab()));
    worktree::remove_with(&f.paths, &made, &terms).unwrap();
    closed_tabs::load(&f.paths).remove(0)
}

fn spec(f: &Fixture, closed: Closed, profiles: Profiles) -> ReviveSpec {
    ReviveSpec {
        paths: f.paths.clone(),
        closed,
        profiles,
        path_var: f.path_var.clone(),
        disk: RealDisk::new(&f.home),
    }
}

fn next(jobs: &Jobs<Done>) -> Done {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(done) = jobs.drain().into_iter().next() {
            return done;
        }
        assert!(Instant::now() < deadline, "the job never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_list_drops_a_tab_whose_session_file_is_gone() {
    let f = fixture();
    let closed = close_with_discard(&f);
    let disk = RealDisk::new(&f.home);
    assert_eq!(run_load(&f.paths, &disk).len(), 1);
    let transcripts = f.home.join(".claude/projects").join(claude_project_dir_name(&closed.path));
    std::fs::remove_dir_all(transcripts).unwrap();
    assert_eq!(run_load(&f.paths, &disk), []);
}

#[test]
fn a_reopen_makes_the_worktree_runs_the_recipe_and_plans_the_resume() {
    let f = fixture();
    std::fs::write(f.repo.join(".env"), "TOKEN=1\n").unwrap();
    Store::update(&f.paths, |store| {
        store.set(&f.repo, &Recipe { copy: vec![".env".into()], ..Recipe::default() });
    })
    .unwrap();
    let closed = close_with_discard(&f);
    let jobs = Jobs::new();
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    revive(&jobs, &egui::Context::default(), spec(&f, closed.clone(), profiles));
    let Done::Revived(landed) = next(&jobs) else { panic!("expected Revived") };
    assert_eq!(std::fs::read_to_string(closed.path.join(".env")).unwrap(), "TOKEN=1\n");
    assert_eq!(landed.notes, Vec::<String>::new());
    assert_eq!(landed.workspace.dir(), f.repo);
    let [tab] = landed.plan.tabs.as_slice() else { panic!("{:?}", landed.plan.tabs) };
    assert_eq!(tab.dir, closed.path);
    assert!(
        matches!(&tab.action, TabAction::ResumeAgent { session_id, .. } if session_id == SESSION),
        "{:?}",
        tab.action
    );
}

#[test]
fn a_worktree_that_is_already_there_gets_no_second_recipe_run() {
    let f = fixture();
    let closed = close_with_discard(&f);
    worktree::revive(&f.paths, &closed).unwrap();
    std::fs::write(f.repo.join(".env"), "NEW\n").unwrap();
    Store::update(&f.paths, |store| {
        store.set(&f.repo, &Recipe { copy: vec![".env".into()], ..Recipe::default() });
    })
    .unwrap();
    let jobs = Jobs::new();
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    revive(&jobs, &egui::Context::default(), spec(&f, closed.clone(), profiles));
    let Done::Revived(_) = next(&jobs) else { panic!("expected Revived") };
    assert!(!closed.path.join(".env").exists());
}

#[test]
fn a_missing_commit_is_refused_with_its_id_in_the_toast_text() {
    let f = fixture();
    let mut closed = close_with_discard(&f);
    let gone = "1234567890abcdef1234567890abcdef12345678".to_owned();
    closed.sha = Some(gone.clone());
    closed.head = None;
    let jobs = Jobs::new();
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    revive(&jobs, &egui::Context::default(), spec(&f, closed.clone(), profiles));
    let Done::Refused { session_id, text } = next(&jobs) else { panic!("expected Refused") };
    assert_eq!(session_id, SESSION);
    assert!(text.contains(&gone) && text.starts_with("Could not reopen heron"), "{text}");
    assert!(!closed.path.exists());
}

#[test]
fn a_profile_that_is_gone_is_refused_before_anything_is_made() {
    let f = fixture();
    let closed = close_with_discard(&f);
    let jobs = Jobs::new();
    revive(&jobs, &egui::Context::default(), spec(&f, closed.clone(), Profiles::default()));
    let Done::Refused { text, .. } = next(&jobs) else { panic!("expected Refused") };
    assert!(text.contains("profile is gone"), "{text}");
    assert!(!closed.path.exists());
    assert_eq!(git(&f.repo, &["branch", "--list", "agent/*"]).trim(), "");
}
