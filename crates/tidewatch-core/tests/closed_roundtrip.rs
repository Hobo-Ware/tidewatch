//! Closing a tab with Discard and bringing it back, against a real throwaway repo with a bare
//! remote, a fake home and a fake `claude` on PATH (the real one never runs): the branch and the
//! worktree come back at the same path on the same commit, and the launch resumes the session.

mod common;

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{commit_file, ok, repos};
use tempfile::TempDir;
use tidewatch_core::agents::Level;
use tidewatch_core::closed_tabs::{self, Closed, Tab};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::prepare::{PrepareInput, prepare_resume};
use tidewatch_core::profiles::Profiles;
use tidewatch_core::restore::{RealDisk, RestoreEnv, TabAction, plan_window};
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree::{self, BranchChoice, RemoveMode, ReviveError, Terms, Why, Worktree};
use tidewatch_core::worktree_safety::Owner;

const SESSION: &str = "0c2cbc96-1111-4222-8333-444455556666";

struct Fixture {
    _remotes: common::Repos,
    _home: TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    bin: PathBuf,
}

fn fixture() -> Fixture {
    let remotes = repos();
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().canonicalize().unwrap();
    let bin = home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(&claude, "#!/bin/sh\necho fake claude \"$@\"\n").unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let repo = remotes.work.canonicalize().unwrap();
    Fixture {
        paths: TidewatchPaths::new(&home),
        _remotes: remotes,
        _home: home_dir,
        home,
        repo,
        bin,
    }
}

fn tab() -> Tab {
    Tab {
        title: "Fix login".into(),
        agent: "claude".into(),
        profile: "builder".into(),
        level: Level::Edits,
        session_id: Some(SESSION.into()),
    }
}

fn write_transcript(f: &Fixture, folder: &Path) {
    let dir = f.home.join(".claude").join("projects").join(claude_project_dir_name(folder));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{SESSION}.jsonl")), "{}\n").unwrap();
}

fn branches(repo: &Path) -> Vec<String> {
    ok(repo, &["branch", "--format=%(refname:short)"]).lines().map(str::to_owned).collect()
}

fn discard(f: &Fixture, made: &Worktree) {
    let terms = Terms::new(BranchChoice::Delete, RemoveMode::Discard)
        .why(Why::ClosedTab)
        .owner(Owner::ThisWindow)
        .closed(Some(tab()));
    worktree::remove_with(&f.paths, made, &terms).unwrap();
}

fn closed_worktree(f: &Fixture) -> (Worktree, String) {
    let made = worktree::add_named(&f.paths, &f.repo, "heron", None).unwrap();
    commit_file(&made.path, "login.rs", "fn login() {}\n", "feat: login");
    let head = ok(&made.path, &["rev-parse", "HEAD"]).trim().to_owned();
    write_transcript(f, &made.path);
    discard(f, &made);
    (made, head)
}

#[test]
fn a_discarded_tab_comes_back_on_its_commit_at_its_path_and_resumes_its_session() {
    let f = fixture();
    let (made, head) = closed_worktree(&f);
    assert!(!made.path.exists());
    assert!(!branches(&f.repo).contains(&made.branch));

    let all = closed_tabs::load(&f.paths);
    let [closed] = all.as_slice() else { panic!("{all:?}") };
    assert_eq!(
        (closed.sha.as_deref(), closed.head.as_deref()),
        (Some(head.as_str()), Some(head.as_str()))
    );
    assert!(!closed.branch_kept);
    let now = closed.at + 3600;
    assert_eq!(closed_tabs::listing(&all, now, &|_| false).len(), 1);

    let revived = worktree::revive(&f.paths, closed).unwrap();
    assert!(revived.made_branch && revived.made_folder);
    assert_eq!(revived.worktree, made);
    assert_eq!(ok(&made.path, &["rev-parse", "HEAD"]).trim(), head);
    assert_eq!(ok(&made.path, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "agent/heron");
    assert!(made.path.join("login.rs").is_file());
    assert!(branches(&f.repo).contains(&made.branch));

    let profiles = Profiles::seeded(&["claude".to_owned()]);
    let agents = profiles.agent_table();
    let path_var = OsString::from(format!("{}:/usr/bin:/bin", f.bin.display()));
    let installed = agents.found_on_path(Some(&path_var));
    assert_eq!(installed, ["claude"]);
    let env = RestoreEnv {
        agents: &agents,
        profiles: &profiles,
        installed: &installed,
        resume: ResumeAgents::Auto,
        crash_loop: false,
    };
    let plan = plan_window(&closed.saved_window(), &env, &RealDisk::new(&f.home));
    assert_eq!(plan.skipped, []);
    let [tab_plan] = plan.tabs.as_slice() else { panic!("{:?}", plan.tabs) };
    assert_eq!(tab_plan.dir, made.path);
    let TabAction::ResumeAgent { profile, session_id, .. } = &tab_plan.action else {
        panic!("{:?}", tab_plan.action)
    };
    assert_eq!(session_id, SESSION);

    let agent = agents.get("claude").unwrap();
    let prepared = prepare_resume(
        &PrepareInput {
            paths: &f.paths,
            agent,
            profile,
            repo: &f.repo,
            tab: made.name(),
            dir: &made.path,
            worktree_tab: true,
            task: "",
            session_id: None,
            remote: false,
            meeseeks: None,
            path_var: Some(&path_var),
            listener: None,
            trust: None,
        },
        session_id,
    )
    .unwrap();
    let line = prepared.launch.shell_line();
    assert!(line.contains(&format!("--resume {SESSION}")), "{line}");
}

#[test]
fn a_commit_that_is_gone_is_refused_by_name_and_nothing_is_made() {
    let f = fixture();
    let (made, _) = closed_worktree(&f);
    let mut closed = closed_tabs::load(&f.paths).remove(0);
    let missing = "1234567890abcdef1234567890abcdef12345678".to_owned();
    closed.sha = Some(missing.clone());
    closed.head = Some(missing.clone());
    let err = worktree::revive(&f.paths, &closed).unwrap_err();
    assert_eq!(err, ReviveError::MissingCommit(missing.clone()));
    assert!(err.to_string().contains(&missing));
    assert!(!made.path.exists());
    assert!(!branches(&f.repo).contains(&made.branch));
}

#[test]
fn a_line_with_no_commit_is_refused() {
    let f = fixture();
    let (_, _) = closed_worktree(&f);
    let mut closed = closed_tabs::load(&f.paths).remove(0);
    closed.sha = None;
    closed.head = None;
    assert_eq!(worktree::revive(&f.paths, &closed).unwrap_err(), ReviveError::NoCommit);
}

#[test]
fn a_path_taken_by_something_else_is_refused_and_left_as_it_is() {
    let f = fixture();
    let (made, _) = closed_worktree(&f);
    fs::create_dir_all(&made.path).unwrap();
    fs::write(made.path.join("mine.txt"), "keep me").unwrap();
    let closed = closed_tabs::load(&f.paths).remove(0);
    let err = worktree::revive(&f.paths, &closed).unwrap_err();
    assert_eq!(err, ReviveError::PathTaken(made.path.clone()));
    assert_eq!(fs::read_to_string(made.path.join("mine.txt")).unwrap(), "keep me");
    assert!(!branches(&f.repo).contains(&made.branch), "no branch is left behind");
}

#[test]
fn a_worktree_that_is_already_there_is_used_as_it_is_with_its_changes() {
    let f = fixture();
    let (made, _) = closed_worktree(&f);
    let closed = closed_tabs::load(&f.paths).remove(0);
    worktree::revive(&f.paths, &closed).unwrap();
    fs::write(made.path.join("wip.txt"), "unsaved").unwrap();
    let again = worktree::revive(&f.paths, &closed).unwrap();
    assert!(!again.made_folder && !again.made_branch);
    assert_eq!(fs::read_to_string(made.path.join("wip.txt")).unwrap(), "unsaved");
}

#[test]
fn a_branch_that_moved_on_stays_where_it_is() {
    let f = fixture();
    let made = worktree::add_named(&f.paths, &f.repo, "heron", None).unwrap();
    commit_file(&made.path, "a.txt", "a", "feat: a");
    let terms = Terms::new(BranchChoice::Keep, RemoveMode::Discard)
        .why(Why::ClosedTab)
        .owner(Owner::ThisWindow)
        .closed(Some(tab()));
    worktree::remove_with(&f.paths, &made, &terms).unwrap();
    ok(&f.repo, &["update-ref", "refs/heads/agent/heron", "main"]);
    let closed = closed_tabs::load(&f.paths).remove(0);
    assert!(closed.branch_kept);
    let revived = worktree::revive(&f.paths, &closed).unwrap();
    assert!(!revived.made_branch && revived.made_folder);
    assert_eq!(
        ok(&made.path, &["rev-parse", "HEAD"]).trim(),
        ok(&f.repo, &["rev-parse", "main"]).trim()
    );
}

#[test]
fn a_tab_closed_with_its_worktree_kept_is_noted_without_a_removal() {
    let f = fixture();
    let made = worktree::add_named(&f.paths, &f.repo, "kept", None).unwrap();
    worktree::note_kept(&f.paths, &made, &tab()).unwrap();
    let all = closed_tabs::load(&f.paths);
    let [Closed { branch_kept: true, sha: Some(_), head: Some(_), .. }] = all.as_slice() else {
        panic!("{all:?}")
    };
    assert!(made.path.is_dir());
}

#[test]
fn a_removal_that_is_refused_notes_no_closed_tab() {
    let f = fixture();
    let made = worktree::add_named(&f.paths, &f.repo, "dirty", None).unwrap();
    fs::write(made.path.join("wip.txt"), "x").unwrap();
    let terms = Terms::new(BranchChoice::Delete, RemoveMode::Safe)
        .why(Why::ClosedTab)
        .owner(Owner::ThisWindow)
        .closed(Some(tab()));
    assert!(worktree::remove_with(&f.paths, &made, &terms).is_err());
    assert_eq!(closed_tabs::load(&f.paths), []);
}
