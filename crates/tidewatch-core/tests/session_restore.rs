//! Restore against real throwaway repos and a fake home: a moved repo is repaired and its tabs
//! come back, a worktree that is gone or cut off from its repo is not resumed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use tidewatch_core::agents::{AgentTable, ArgList, Level};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::restore::{
    self, RealDisk, Reason, RestoreEnv, TabAction, plan_window, repair_moved,
};
use tidewatch_core::session::{SavedAgentTab, SavedTab, SavedWindow, Session};
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree::{self, BranchChoice, RemoveMode, Worktree};

const SESSION_ID: &str = "0c2cbc96-1111-4222-8333-444455556666";

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    worktree: Worktree,
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
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add(&paths, &repo).unwrap();
    let transcripts =
        home.join(".claude").join("projects").join(claude_project_dir_name(&worktree.path));
    fs::create_dir_all(&transcripts).unwrap();
    fs::write(transcripts.join(format!("{SESSION_ID}.jsonl")), "{}\n").unwrap();
    Fixture { _dir: dir, home, repo, paths, worktree }
}

fn session_for(dir: &Path, worktree: &Worktree) -> Session {
    Session {
        windows: vec![SavedWindow {
            dir: dir.to_path_buf(),
            other: false,
            active: 1,
            focus_mode: false,
            tabs: vec![
                SavedTab::Main,
                SavedTab::Agent(SavedAgentTab {
                    dir: worktree.path.clone(),
                    branch: Some(worktree.branch.clone()),
                    agent: "claude".into(),
                    profile: "builder".into(),
                    level: Level::Edits,
                    session_id: Some(SESSION_ID.into()),
                    stages: None,
                    crashed: false,
                    remote: false,
                }),
            ],
            parked: Vec::new(),
        }],
    }
}

fn plan(f: &Fixture, window: &SavedWindow) -> restore::WindowPlan {
    let table = AgentTable::builtin();
    let profile = Profile {
        name: "Builder".into(),
        agent: "claude".into(),
        model: None,
        effort: None,
        level: Level::Edits,
        setup: String::new(),
        never_push: true,
        extra_args: ArgList::default(),
        house_style: true,
    };
    let profiles = Profiles {
        profile: BTreeMap::from([("builder".to_owned(), profile)]),
        agent: BTreeMap::new(),
    };
    let installed = ["claude".to_owned()];
    let env = RestoreEnv {
        agents: &table,
        profiles: &profiles,
        installed: &installed,
        resume: ResumeAgents::Auto,
        app_crashed: false,
    };
    plan_window(window, &env, &RealDisk::new(&f.home))
}

fn move_repo(f: &Fixture) -> PathBuf {
    let moved = f.home.join("elsewhere").join("web");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&f.repo, &moved).unwrap();
    moved
}

#[test]
fn a_session_saved_and_loaded_restores_every_tab_when_nothing_changed() {
    let f = fixture();
    session_for(&f.repo, &f.worktree).save(&f.paths).unwrap();
    let loaded = Session::load(&f.paths);
    let plan = plan(&f, &loaded.windows[0]);
    assert_eq!(plan.skipped, []);
    assert_eq!(plan.tabs.len(), 2);
    assert!(
        matches!(plan.tabs[1].action, TabAction::ResumeAgent { .. }),
        "{:?}",
        plan.tabs[1].action
    );
    assert_eq!(plan.active, 1);
}

#[test]
fn a_moved_repo_is_repaired_and_its_agent_tab_resumes_in_the_same_folder() {
    let f = fixture();
    let mut session = session_for(&f.repo, &f.worktree);
    let moved = move_repo(&f);

    let repaired = repair_moved(&f.paths, &mut session, &moved);

    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].repo, moved);
    assert_eq!(session.windows[0].dir, moved);
    let plan = plan(&f, &session.windows[0]);
    assert_eq!(plan.skipped, []);
    assert_eq!(plan.tabs[0].dir, moved);
    assert_eq!(plan.tabs[1].dir, f.worktree.path, "the worktree folder does not move");
    assert!(
        matches!(plan.tabs[1].action, TabAction::ResumeAgent { .. }),
        "{:?}",
        plan.tabs[1].action
    );
    assert_eq!(plan.tabs[1].worktree.as_ref().map(|w| &w.repo), Some(&moved));
}

#[test]
fn without_the_repair_a_moved_repo_cuts_its_worktree_tabs_off() {
    let f = fixture();
    let mut session = session_for(&f.repo, &f.worktree);
    let moved = move_repo(&f);

    let untouched = plan(&f, &session.windows[0]);
    assert_eq!(untouched.tabs, []);
    assert!(untouched.skipped.iter().all(|s| s.reason == Reason::RepoMissing));

    session.windows[0].dir = moved;
    let unrepaired = plan(&f, &session.windows[0]);
    assert_eq!(
        unrepaired.skipped,
        [restore::Skipped { label: f.worktree.name().to_owned(), reason: Reason::WorktreeBroken }]
    );
    assert_eq!(unrepaired.tabs.len(), 1, "the main tab still opens");
}

#[test]
fn repair_moved_leaves_other_repos_and_known_windows_alone() {
    let f = fixture();
    let mut session = session_for(&f.repo, &f.worktree);
    let before = session.clone();

    assert_eq!(repair_moved(&f.paths, &mut session, &f.repo), []);
    let other = f.home.join("personal").join("api");
    fs::create_dir_all(&other).unwrap();
    assert_eq!(repair_moved(&f.paths, &mut session, &other), []);
    assert_eq!(session, before);
}

#[test]
fn a_removed_worktree_is_skipped_as_gone() {
    let f = fixture();
    let session = session_for(&f.repo, &f.worktree);
    worktree::remove(&f.paths, &f.worktree, BranchChoice::Keep, RemoveMode::Safe).unwrap();

    let plan = plan(&f, &session.windows[0]);

    assert_eq!(
        plan.skipped,
        [restore::Skipped { label: f.worktree.name().to_owned(), reason: Reason::WorktreeGone }]
    );
    assert_eq!(plan.tabs.len(), 1);
    assert_eq!(plan.active, 0);
}

#[test]
fn a_missing_session_file_reopens_the_worktree_as_a_shell() {
    let f = fixture();
    let mut session = session_for(&f.repo, &f.worktree);
    if let SavedTab::Agent(tab) = &mut session.windows[0].tabs[1] {
        tab.session_id = Some("ffffffff-1111-4222-8333-444455556666".into());
    }
    let plan = plan(&f, &session.windows[0]);
    assert_eq!(plan.tabs[1].action, TabAction::ShellInstead(Reason::TranscriptMissing));
    assert_eq!(plan.tabs[1].dir, f.worktree.path);
}
