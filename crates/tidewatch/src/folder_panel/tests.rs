use std::fs;
use std::time::{Duration, Instant};

use super::*;
use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, Launched};
use tidewatch_core::agents::Level;

fn tab(pinned: bool, pane: Pane<u8>) -> Tab<Pane<u8>> {
    Tab { id: 3, group: 0, title: "t".into(), pinned, pane, park: None, keep: false }
}

fn agent(dir: Option<&str>) -> AgentTab {
    let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
    agent.launched = Some(Launched {
        profile_id: "builder".into(),
        level: Level::Edits,
        session_id: None,
        remote: false,
        dir: dir.map(PathBuf::from),
    });
    agent
}

fn agent_pane(dir: Option<&str>) -> Pane<u8> {
    Pane { body: Body::Shell(0), worktree: None, agent: Some(agent(dir)) }
}

fn repo_workspace() -> Workspace {
    Workspace::Repo(crate::repo::Repo { root: "/work/web".into(), label: "web".into() })
}

fn other_workspace() -> Workspace {
    Workspace::Other { dir: "/notes".into(), label: "notes".into() }
}

#[test]
fn only_an_agent_tab_without_a_worktree_has_a_folder() {
    let worktree = tidewatch_core::worktree::Worktree {
        repo: "/work/web".into(),
        path: "/home/.tidewatch/web-abc123/brave-otter".into(),
        branch: "agent/brave-otter".into(),
    };
    let with_worktree =
        Pane { body: Body::Shell(0), worktree: Some(worktree), agent: Some(agent(None)) };
    let creating = Pane { body: Body::Creating, worktree: None, agent: Some(agent(None)) };
    let cases = [
        (
            "an agent in its own folder",
            &repo_workspace(),
            tab(false, agent_pane(Some("/ws"))),
            Some("/ws"),
        ),
        (
            "an agent with no folder of its own takes the group's",
            &other_workspace(),
            tab(false, agent_pane(None)),
            Some("/notes"),
        ),
        ("a plain shell has none", &repo_workspace(), tab(false, Pane::shell(0)), None),
        ("a worktree tab has none", &repo_workspace(), tab(false, with_worktree), None),
        ("the main tab has none", &repo_workspace(), tab(true, agent_pane(Some("/ws"))), None),
        ("a tab still being made has none", &repo_workspace(), tab(false, creating), None),
    ];
    for (name, workspace, tab, want) in cases {
        assert_eq!(folder_of(workspace, &tab), want.map(PathBuf::from), "{name}");
    }
}

fn git_init(dir: &Path) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q", "-b", "main"])
        .output()
        .unwrap();
    assert!(out.status.success());
}

fn wait_for_scan(folders: &mut Folders, tab: u64) -> &Scan {
    let deadline = Instant::now() + Duration::from_secs(10);
    while folders.scan(tab).is_none() {
        assert!(Instant::now() < deadline, "the folder was never looked at");
        folders.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    folders.scan(tab).unwrap()
}

#[test]
fn a_folder_is_looked_at_off_the_ui_thread_and_the_pick_follows_the_agent_then_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for name in ["alpha", "beta"] {
        fs::create_dir(root.join(name)).unwrap();
        git_init(&root.join(name));
    }
    let ctx = egui::Context::default();
    let mut folders = Folders::new();
    assert!(folders.shown(1, None, &[]).is_none(), "nothing is known before the first look");
    folders.look(&ctx, 1, &root);
    assert_eq!(wait_for_scan(&mut folders, 1).repos.len(), 2);
    let alpha_file = [root.join("alpha/a.rs")];
    let beta_file = [root.join("alpha/a.rs"), root.join("beta/b.rs")];
    let cases = [
        ("the newest edit", None, &alpha_file[..], "alpha"),
        ("a newer edit in another repo", None, &beta_file[..], "beta"),
        ("the owner's pick over the edit", Some(root.join("alpha")), &beta_file[..], "alpha"),
    ];
    for (name, hand, edits, want) in cases {
        let Some(Shown::Repo { root: picked, repos }) = folders.shown(1, hand.as_deref(), edits)
        else {
            panic!("{name}: no repo shown");
        };
        assert_eq!((picked, repos.len()), (root.join(want).as_path(), 2), "{name}");
    }
}

#[test]
fn a_folder_with_no_repo_shows_the_plain_folder() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("notes.txt"), "x").unwrap();
    let ctx = egui::Context::default();
    let mut folders = Folders::new();
    folders.look(&ctx, 1, dir.path());
    wait_for_scan(&mut folders, 1);
    let Some(Shown::Folder(scan)) = folders.shown(1, None, &[]) else { panic!("no folder view") };
    assert_eq!(scan.recent.len(), 1);
}

#[test]
fn a_folder_inside_a_repo_shows_that_repo_with_no_picker() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git_init(&root);
    fs::create_dir(root.join("src")).unwrap();
    let ctx = egui::Context::default();
    let mut folders = Folders::new();
    folders.look(&ctx, 1, &root.join("src"));
    wait_for_scan(&mut folders, 1);
    let Some(Shown::Repo { root: shown, repos }) = folders.shown(1, None, &[]) else {
        panic!("no repo shown")
    };
    assert_eq!((shown, repos.len()), (root.as_path(), 0));
}

#[test]
fn a_tab_that_changes_folder_forgets_the_old_look_and_closed_tabs_are_dropped() {
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ctx = egui::Context::default();
    let mut folders = Folders::new();
    folders.look(&ctx, 1, one.path());
    assert_eq!(wait_for_scan(&mut folders, 1).folder, one.path());
    folders.look(&ctx, 1, two.path());
    assert!(folders.scan(1).is_none());
    assert_eq!(wait_for_scan(&mut folders, 1).folder, two.path());
    folders.keep(&HashSet::new());
    assert!(folders.scan(1).is_none());
}
