use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tidewatch_core::agents::Level;
use tidewatch_core::prefs::StartFrom;
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree;

use super::*;
use crate::pane::{Body, Pane};
use crate::restore_glue::{Spawn, reopened};
use crate::retarget::Window;
use crate::tabs::Tabs;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const START: &str = "Fri Oct 16 13:23:31 2026";
const SHELL: u32 = 910;
const AGENT: u32 = 921;

struct Fake {
    ps: String,
    sent: RefCell<Vec<u32>>,
    exits: Cell<bool>,
    gone: Cell<bool>,
    looks: Cell<u32>,
    vanishes_after: Cell<Option<u32>>,
}

impl Fake {
    fn new(ps: &str) -> Self {
        Self {
            ps: ps.to_owned(),
            sent: RefCell::new(Vec::new()),
            exits: Cell::new(true),
            gone: Cell::new(false),
            looks: Cell::new(0),
            vanishes_after: Cell::new(None),
        }
    }
}

impl System for Fake {
    fn ps(&self) -> Option<String> {
        self.looks.set(self.looks.get() + 1);
        if self.vanishes_after.get().is_some_and(|after| self.looks.get() > after) {
            return Some(
                self.ps
                    .lines()
                    .filter(|line| !line.contains(" claude"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        Some(self.ps.clone())
    }

    fn cwd_of(&self, _pid: u32) -> Option<PathBuf> {
        None
    }

    fn git_root(&self, dir: &Path) -> Option<PathBuf> {
        Some(dir.to_path_buf())
    }

    fn cwd_holders(&self, _dir: &Path) -> Option<Vec<u32>> {
        Some(Vec::new())
    }

    fn terminate(&self, pid: u32) -> bool {
        self.sent.borrow_mut().push(pid);
        self.gone.set(self.exits.get());
        true
    }

    fn alive(&self, _pid: u32) -> Option<String> {
        (!self.gone.get()).then(|| START.to_owned())
    }
}

fn ps() -> String {
    format!(
        "  PID  PPID STARTED                      ARGS\n\
         500     1 Wed Oct  7 09:30:00 2026     /Applications/Tidewatch.app/Contents/MacOS/tidewatch\n\
         {SHELL} 500 Fri Oct 16 13:20:00 2026     /bin/zsh -l -i\n\
         {AGENT} {SHELL} {START}     claude --permission-mode acceptEdits\n\
         930   500 Fri Oct 16 13:21:00 2026     /bin/zsh -l -i\n\
         931   930 Fri Oct 16 13:22:00 2026     claude\n"
    )
}

struct World {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    worktree: tidewatch_core::worktree::Worktree,
    other: tidewatch_core::worktree::Worktree,
    path_var: OsString,
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("work").join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add(&paths, &repo).unwrap();
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let other = worktree::add(&paths, &repo).unwrap();
    for folder in [&worktree.path, &other.path] {
        transcript(&home, folder);
    }
    World { _dir: dir, home, repo, paths, worktree, other, path_var: bin.into_os_string() }
}

fn transcript(home: &Path, folder: &Path) {
    let transcripts = home.join(".claude").join("projects").join(claude_project_dir_name(folder));
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(
        transcripts.join(format!("{ID}.jsonl")),
        "{\"type\":\"user\",\"permissionMode\":\"auto\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
    )
    .unwrap();
}

fn registry(w: &World, pid: u32, start: &str, status: &str) {
    registry_in(w, &w.worktree.path, pid, start, status);
}

fn registry_in(w: &World, cwd: &Path, pid: u32, start: &str, status: &str) {
    let text = format!(
        r#"{{"pid":{pid},"sessionId":"{ID}","cwd":"{}","startedAt":1792157013549,"procStart":"{start}","kind":"interactive","name":"n","status":"{status}"}}"#,
        cwd.display()
    );
    let path = w.home.join(format!(".claude/sessions/{pid}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn env(w: &World) -> ScanEnv {
    ScanEnv {
        home: w.home.clone(),
        claude_dir: w.home.join(".claude"),
        codex_home: None,
        copilot_home: None,
        own_pid: 500,
    }
}

fn profiles() -> Profiles {
    Profiles::seeded(&["claude".to_owned()])
}

fn request(w: &World, target: Target, interrupt: bool) -> Request {
    Request {
        subject: Subject {
            tab: 7,
            shell_pid: Some(SHELL),
            repo: w.repo.clone(),
            worktree: Some(w.worktree.clone()),
            dir: w.worktree.path.clone(),
            cli: None,
            known_session_id: None,
            remote: false,
            pinned: false,
            settling: false,
            window: Window { repos: vec![w.repo.clone()], shown: Vec::new() },
        },
        target,
        remembered: None,
        interrupt,
        profiles: profiles(),
        installed: vec!["claude".to_owned()],
        paths: w.paths.clone(),
        path_var: w.path_var.clone(),
        listener: None,
        trust: None,
        wait: Duration::from_millis(300),
        start_from: StartFrom::DefaultBranch,
        name: None,
        spare_tab: None,
    }
}

fn builder(level: Option<Level>) -> Target {
    Target::Agent { profile: "builder".to_owned(), level }
}

fn probes() -> Vec<Probe> {
    vec![Probe { tab: 7, shell_pid: SHELL }, Probe { tab: 8, shell_pid: 930 }]
}

#[test]
fn the_look_inside_shell_tabs_names_the_tab_each_session_runs_in() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    registry(&w, 931, "Fri Oct 16 13:22:00 2026", "busy");
    let Done::Detected(found) = run_detect(&Fake::new(&ps()), &env(&w), &profiles(), &probes())
    else {
        panic!("expected a detection");
    };
    let got: Vec<(u64, bool, Option<Level>)> =
        found.iter().map(|(tab, d)| (*tab, d.working, d.level)).collect();
    assert_eq!(got, [(7, false, Some(Level::Auto)), (8, true, Some(Level::Auto))]);
    assert_eq!(found[0].1.session_id.as_deref(), Some(ID));
}

#[test]
fn a_session_with_a_stale_start_time_is_not_detected() {
    let w = world();
    registry(&w, AGENT, "Mon Oct 12 18:01:24 2026", "idle");
    let Done::Detected(found) = run_detect(&Fake::new(&ps()), &env(&w), &profiles(), &probes())
    else {
        panic!("expected a detection");
    };
    assert_eq!(found, []);
}

#[test]
fn a_one_shot_run_in_a_shell_is_not_detected() {
    let w = world();
    registry(&w, AGENT, START, "busy");
    let one_shot = ps().replace("claude --permission-mode acceptEdits", "claude -p fix the build");
    let Done::Detected(found) = run_detect(&Fake::new(&one_shot), &env(&w), &profiles(), &probes())
    else {
        panic!("expected a detection");
    };
    assert_eq!(found, []);
}

struct Landed {
    tabs: Tabs<Pane<u8>>,
    id: u64,
    lines: Vec<String>,
    dirs: Vec<PathBuf>,
    landing: Landing,
}

fn landed(done: Done, w: &World) -> Landed {
    let Done::Retargeted(landed) = done else { panic!("expected a change to land") };
    let Retargeted::Agent { opened, landing, .. } = *landed else { panic!("expected an agent") };
    let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));
    let id = tabs.push_in(
        0,
        "calm-comet".to_owned(),
        Pane { body: Body::Creating, worktree: Some(w.worktree.clone()), agent: None },
    );
    let opened = Reopened { tab: id, ..*opened };
    let lines = RefCell::new(Vec::new());
    let dirs = RefCell::new(Vec::new());
    let agent = |dir: &Path, p: &tidewatch_core::prepare::Prepared| {
        lines.borrow_mut().push(p.launch.shell_line());
        dirs.borrow_mut().push(dir.to_path_buf());
        1u8
    };
    let typed = |_: &Path, _: &tidewatch_core::prepare::Prepared| 2u8;
    let shell = |_: &Path, _: Option<&str>| 3u8;
    let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
    reopened(opened, &mut tabs, &spawn, 1.0, &mut Vec::new());
    Landed { tabs, id, lines: lines.into_inner(), dirs: dirs.into_inner(), landing }
}

fn spawned(done: Done, w: &World) -> (Tabs<Pane<u8>>, u64, Vec<String>) {
    let Landed { tabs, id, lines, .. } = landed(done, w);
    (tabs, id, lines)
}

#[test]
fn a_shell_tab_with_a_session_becomes_an_agent_tab_that_resumes_it_at_the_level_it_ran_at() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert_eq!(*system.sent.borrow(), [AGENT], "only the agent is signalled, never the shell");
    let (tabs, id, lines) = spawned(done, &w);
    let tab = tabs.items().iter().find(|tab| tab.id == id).unwrap();
    assert!(matches!(tab.pane.body, Body::Shell(1)), "the agent runs in the tab's pty");
    assert_eq!(tab.pane.worktree.as_ref(), Some(&w.worktree));
    let agent = tab.pane.agent.as_ref().expect("an agent tab now");
    let launched = agent.launched.as_ref().unwrap();
    assert_eq!((launched.profile_id.as_str(), launched.level), ("builder", Level::Auto));
    assert_eq!(launched.session_id.as_deref(), Some(ID));
    assert!(lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", lines[0]);
    assert_eq!(tabs.items().iter().position(|t| t.id == id), Some(1), "same place in the strip");
}

#[test]
fn another_profile_and_level_resume_the_same_session_in_the_same_tab() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let target = Target::Agent { profile: "reviewer".into(), level: Some(Level::Plan) };
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &request(&w, target, false));
    let (tabs, id, lines) = spawned(done, &w);
    let tab = tabs.items().iter().find(|tab| tab.id == id).unwrap();
    let launched = tab.pane.agent.as_ref().unwrap().launched.as_ref().unwrap();
    assert_eq!((launched.profile_id.as_str(), launched.level), ("reviewer", Level::Plan));
    assert!(lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", lines[0]);
}

#[test]
fn a_session_mid_turn_is_left_running_until_it_is_told_to_cut_in() {
    let w = world();
    registry(&w, AGENT, START, "busy");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert!(matches!(done, Done::Held { tab: 7 }));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), true));
    assert!(matches!(done, Done::Retargeted(_)));
    assert_eq!(*system.sent.borrow(), [AGENT]);
}

#[test]
fn a_process_that_will_not_end_is_ended_with_its_terminal_and_the_change_goes_ahead() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    system.exits.set(false);
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert!(matches!(done, Done::Retargeted(_)));
    assert_eq!(
        *system.sent.borrow(),
        [AGENT],
        "signalled once; dropping the terminal does the rest"
    );
}

#[test]
fn an_agent_that_stopped_by_itself_meanwhile_is_not_an_error() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    system.vanishes_after.set(Some(1));
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new(), "nothing is signalled that is gone");
    let (_, _, lines) = spawned(done, &w);
    assert!(!lines[0].contains("--resume"), "{}", lines[0]);
}

#[test]
fn a_tab_with_no_terminal_changes_into_a_new_agent_in_its_folder() {
    let w = world();
    let system = Fake::new(&ps());
    let mut asked = request(&w, builder(None), false);
    asked.subject.shell_pid = None;
    let done = run_retarget(&system, &env(&w), &asked);
    let (_, _, lines) = spawned(done, &w);
    assert!(!lines[0].contains("--resume"), "{}", lines[0]);
}

#[test]
fn yolo_runs_exactly_as_picked_in_a_tab_with_no_worktree() {
    let w = world();
    let plain = w.home.join("notes");
    std::fs::create_dir_all(&plain).unwrap();
    transcript(&w.home, &plain);
    registry_in(&w, &plain, AGENT, START, "idle");
    let mut asked = request(&w, builder(Some(Level::Yolo)), false);
    asked.subject.worktree = None;
    asked.subject.dir = w.repo.clone();
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &asked);
    let result = landed(done, &w);
    let tab = result.tabs.items().iter().find(|tab| tab.id == result.id).unwrap();
    let launched = tab.pane.agent.as_ref().unwrap().launched.as_ref().unwrap();
    assert_eq!(launched.level, Level::Yolo);
    assert!(result.lines[0].contains("--dangerously-skip-permissions"), "{}", result.lines[0]);
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", result.lines[0]);
}

#[test]
fn a_tab_still_being_set_up_waits() {
    let w = world();
    let mut asked = request(&w, builder(None), false);
    asked.subject.settling = true;
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &asked);
    assert!(matches!(done, Done::Held { tab: 7 }));
}

#[test]
fn a_shell_tab_in_one_worktree_follows_a_session_that_runs_in_another() {
    let w = world();
    registry_in(&w, &w.other.path, AGENT, START, "idle");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert_eq!(*system.sent.borrow(), [AGENT], "only the agent is signalled, never the shell");
    let result = landed(done, &w);
    let tab = result.tabs.items().iter().find(|tab| tab.id == result.id).unwrap();
    assert_eq!(
        tab.pane.worktree.as_ref(),
        Some(&w.other),
        "the tab owns the folder of the session"
    );
    assert_eq!(tab.title, w.other.name());
    assert_eq!(result.landing.repo, w.repo);
    assert_eq!(
        result.dirs,
        std::slice::from_ref(&w.other.path),
        "the agent starts in the session's folder"
    );
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", result.lines[0]);
    let launched = tab.pane.agent.as_ref().unwrap().launched.as_ref().unwrap();
    assert_eq!(launched.session_id.as_deref(), Some(ID));
    assert_eq!(launched.dir.as_deref(), Some(w.other.path.as_path()));
    assert!(w.worktree.path.join(".git").exists(), "the old folder is left on disk");
    let listed = worktree::list(&w.repo).unwrap();
    assert!(listed.iter().any(|entry| entry.path == w.worktree.path), "git still lists it");
    assert!(listed.iter().any(|entry| entry.path == w.other.path));
}

#[test]
fn a_folder_another_tab_shows_is_followed_without_owning_it() {
    let w = world();
    registry_in(&w, &w.other.path, AGENT, START, "idle");
    let mut asked = request(&w, builder(None), false);
    asked.subject.window.shown = vec![w.other.path.clone()];
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &asked);
    let result = landed(done, &w);
    let tab = result.tabs.items().iter().find(|tab| tab.id == result.id).unwrap();
    assert_eq!(
        tab.pane.worktree, None,
        "closing this tab never removes a folder another tab shows"
    );
    assert_eq!(result.dirs, std::slice::from_ref(&w.other.path));
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")));
}

fn repo_in(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("src/lib.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir.canonicalize().unwrap()
}

fn edit_of(file: &Path) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Edit","input":{{"file_path":"{}"}}}}]}}}}"#,
        file.display()
    )
}

const MODE_LINE: &str =
    r#"{"type":"user","permissionMode":"auto","message":{"role":"user","content":"hi"}}"#;

fn session_in(w: &World, folder: &Path, lines: &[String]) -> PathBuf {
    let project = w.home.join(".claude/projects").join(claude_project_dir_name(folder));
    std::fs::create_dir_all(project.join(ID).join("subagents")).unwrap();
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(project.join(format!("{ID}.jsonl")), text).unwrap();
    std::fs::write(project.join(ID).join("subagents").join("agent-1.jsonl"), "{}\n").unwrap();
    project
}

fn project_of(w: &World, folder: &Path) -> PathBuf {
    w.home.join(".claude/projects").join(claude_project_dir_name(folder))
}

fn in_workspace(w: &World, ws: &Path) -> Request {
    let mut asked = request(w, builder(None), false);
    asked.subject.worktree = None;
    asked.subject.repo = ws.to_path_buf();
    asked.subject.dir = ws.to_path_buf();
    asked.subject.window.repos = vec![ws.to_path_buf()];
    asked
}

#[test]
fn a_shell_tab_in_a_workspace_folder_becomes_an_agent_on_a_new_branch_of_the_repo_its_session_edited()
 {
    let w = world();
    let ws = w.home.join("ws");
    let repo_a = repo_in(&ws, "repo-a");
    let repo_b = repo_in(&ws, "repo-b");
    let ws = ws.canonicalize().unwrap();
    let old_project = session_in(
        &w,
        &ws,
        &[
            MODE_LINE.to_owned(),
            edit_of(&repo_b.join("src/lib.rs")),
            edit_of(&repo_b.join("src/new.rs")),
        ],
    );
    std::fs::write(repo_b.join("src/lib.rs"), "fn main() { dirty }\n").unwrap();
    registry_in(&w, &ws, AGENT, START, "idle");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &in_workspace(&w, &ws));
    assert_eq!(*system.sent.borrow(), [AGENT], "only the agent is signalled, never the shell");

    let result = landed(done, &w);
    let made = result.landing.worktree.clone().expect("the tab owns a new worktree");
    assert_eq!(result.landing.repo, repo_b, "the repo the session edited, not the workspace");
    assert!(made.branch.starts_with("agent/"), "{}", made.branch);
    assert_eq!(made.repo, repo_b);
    assert!(made.path.starts_with(w.paths.root()), "{}", made.path.display());
    assert!(made.path.join("src/lib.rs").is_file());
    assert_eq!(std::fs::read_to_string(made.path.join("src/lib.rs")).unwrap(), "fn main() {}\n");
    assert_eq!(result.dirs, std::slice::from_ref(&made.path), "the agent starts in the worktree");
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", result.lines[0]);

    let new_project = project_of(&w, &made.path);
    for relative in [format!("{ID}.jsonl"), format!("{ID}/subagents/agent-1.jsonl")] {
        let (from, to) = (old_project.join(&relative), new_project.join(&relative));
        assert_eq!(std::fs::read(&from).unwrap(), std::fs::read(&to).unwrap(), "{relative}");
    }
    assert!(old_project.join(format!("{ID}.jsonl")).is_file(), "the original is still there");

    let tab = result.tabs.items().iter().find(|tab| tab.id == result.id).unwrap();
    assert_eq!(tab.pane.worktree.as_ref(), Some(&made));
    assert_eq!(
        tab.pane.agent.as_ref().unwrap().launched.as_ref().unwrap().dir.as_deref(),
        Some(made.path.as_path())
    );
    let listed = worktree::list(&repo_b).unwrap();
    assert!(listed.iter().any(|entry| entry.path == made.path), "git lists the new worktree");
    assert_eq!(worktree::list(&repo_a).unwrap().len(), 1, "the other repo is untouched");
    assert_eq!(
        std::fs::read_to_string(repo_b.join("src/lib.rs")).unwrap(),
        "fn main() { dirty }\n",
        "uncommitted changes stay where they were"
    );
}

#[test]
fn the_pinned_main_tab_keeps_its_shell_and_the_agent_opens_in_the_spare_tab() {
    let w = world();
    let ws = w.home.join("ws");
    let repo_b = repo_in(&ws, "repo-b");
    let ws = ws.canonicalize().unwrap();
    session_in(&w, &ws, &[MODE_LINE.to_owned(), edit_of(&repo_b.join("src/lib.rs"))]);
    registry_in(&w, &ws, AGENT, START, "idle");
    let mut asked = in_workspace(&w, &ws);
    asked.subject.pinned = true;
    asked.spare_tab = Some(99);
    let Done::Retargeted(landed) = run_retarget(&Fake::new(&ps()), &env(&w), &asked) else {
        panic!("expected a change to land")
    };
    let Retargeted::Agent { opened, landing, from, .. } = *landed else {
        panic!("expected an agent")
    };
    assert_eq!((opened.tab, from), (99, 7));
    assert_eq!(landing.repo, repo_b);
    assert!(landing.worktree.is_some());
}

#[test]
fn a_session_in_a_repo_that_is_not_open_gets_a_new_worktree_there_and_one_with_no_repo_stays_put() {
    let w = world();
    let elsewhere = repo_in(&w.home.join("code"), "lib");
    transcript(&w.home, &elsewhere);
    registry_in(&w, &elsewhere, AGENT, START, "idle");
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &request(&w, builder(None), false));
    let result = landed(done, &w);
    let made = result.landing.worktree.clone().expect("a new worktree of the repo");
    assert_eq!(
        (result.landing.repo.as_path(), made.repo.as_path()),
        (elsewhere.as_path(), elsewhere.as_path())
    );
    assert_eq!(result.dirs, std::slice::from_ref(&made.path));
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")));

    let plain = w.home.join("notes");
    std::fs::create_dir_all(&plain).unwrap();
    transcript(&w.home, &plain);
    registry_in(&w, &plain, AGENT, START, "idle");
    let done = run_retarget(&Fake::new(&ps()), &env(&w), &request(&w, builder(None), false));
    let result = landed(done, &w);
    assert_eq!(result.landing.repo, w.repo, "a folder outside git stays in the tab's own group");
    assert_eq!(result.dirs, [plain]);
    assert!(result.lines[0].starts_with(&format!("claude --resume {ID} ")));
}

#[test]
fn a_repo_that_cannot_get_a_worktree_leaves_the_tab_in_its_folder_with_a_note() {
    let w = world();
    let ws = w.home.join("ws");
    let bare = ws.join("no-commits");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "-b", "main"]);
    let bare = bare.canonicalize().unwrap();
    let ws = ws.canonicalize().unwrap();
    session_in(&w, &ws, &[MODE_LINE.to_owned(), edit_of(&bare.join("a.rs"))]);
    registry_in(&w, &ws, AGENT, START, "idle");
    let Done::Retargeted(landed) =
        run_retarget(&Fake::new(&ps()), &env(&w), &in_workspace(&w, &ws))
    else {
        panic!("expected a change to land")
    };
    let Retargeted::Agent { landing, note, .. } = *landed else { panic!("expected an agent") };
    assert_eq!((landing.dir.as_path(), landing.worktree), (ws.as_path(), None));
    assert!(note.unwrap().contains("could not be made"));
}

#[test]
fn a_conversation_that_cannot_be_copied_starts_a_new_session_in_the_new_worktree() {
    let w = world();
    let ws = w.home.join("ws");
    let repo_b = repo_in(&ws, "repo-b");
    let ws = ws.canonicalize().unwrap();
    session_in(&w, &ws, &[MODE_LINE.to_owned(), edit_of(&repo_b.join("src/lib.rs"))]);
    registry_in(&w, &ws, AGENT, START, "idle");
    let asked = in_workspace(&w, &ws);
    std::fs::remove_dir_all(w.home.join(".claude/projects")).unwrap();
    let Done::Retargeted(landed) = run_retarget(&Fake::new(&ps()), &env(&w), &asked) else {
        panic!("expected a change to land")
    };
    let Retargeted::Agent { landing, note, .. } = *landed else { panic!("expected an agent") };
    assert!(landing.worktree.is_some());
    assert!(note.unwrap().contains("so a new session started"));
}

#[test]
fn a_shell_target_ends_the_agent_and_hands_back_the_session_id() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, Target::Shell, false));
    let Done::Retargeted(landed) = done else { panic!("expected the agent to end") };
    let Retargeted::Shell { tab, remembered } = *landed else { panic!("expected a shell") };
    assert_eq!((tab, remembered), (7, Some((Cli::Claude, ID.to_owned()))));
    assert_eq!(*system.sent.borrow(), [AGENT]);
}

#[test]
fn a_remembered_session_resumes_with_nothing_to_end() {
    let w = world();
    let system = Fake::new(&ps());
    let request = Request {
        remembered: Some((Cli::Claude, ID.to_owned())),
        ..request(&w, builder(None), false)
    };
    let done = run_retarget(&system, &env(&w), &request);
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
    let (_, _, lines) = spawned(done, &w);
    assert!(lines[0].starts_with(&format!("claude --resume {ID} ")), "{}", lines[0]);
}

#[test]
fn a_plain_shell_starts_a_new_session_in_place() {
    let w = world();
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
    let (tabs, id, lines) = spawned(done, &w);
    assert!(!lines[0].contains("--resume"), "{}", lines[0]);
    let agent = tabs.items().iter().find(|tab| tab.id == id).unwrap().pane.agent.as_ref().unwrap();
    assert!(agent.launched.as_ref().unwrap().session_id.is_some());
}
