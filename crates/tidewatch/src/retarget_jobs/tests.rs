use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tidewatch_core::agents::Level;
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree;

use super::*;
use crate::pane::{Body, Pane};
use crate::restore_glue::{Spawn, reopened};
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
}

impl Fake {
    fn new(ps: &str) -> Self {
        Self {
            ps: ps.to_owned(),
            sent: RefCell::new(Vec::new()),
            exits: Cell::new(true),
            gone: Cell::new(false),
        }
    }
}

impl System for Fake {
    fn ps(&self) -> Option<String> {
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
    let transcripts =
        home.join(".claude").join("projects").join(claude_project_dir_name(&worktree.path));
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(
        transcripts.join(format!("{ID}.jsonl")),
        "{\"type\":\"user\",\"permissionMode\":\"auto\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
    )
    .unwrap();
    World { _dir: dir, home, repo, paths, worktree, path_var: bin.into_os_string() }
}

fn registry(w: &World, pid: u32, start: &str, status: &str) {
    let text = format!(
        r#"{{"pid":{pid},"sessionId":"{ID}","cwd":"{}","startedAt":1792157013549,"procStart":"{start}","kind":"interactive","name":"n","status":"{status}"}}"#,
        w.worktree.path.display()
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
            shell_pid: SHELL,
            repo: w.repo.clone(),
            worktree: Some(w.worktree.clone()),
            dir: w.worktree.path.clone(),
            cli: None,
            known_session_id: None,
            foreground_busy: false,
            remote: false,
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

fn spawned(done: Done, w: &World) -> (Tabs<Pane<u8>>, u64, Vec<String>) {
    let Done::Retargeted(landed) = done else { panic!("expected a change to land") };
    let Retargeted::Agent(reopened_done) = *landed else { panic!("expected an agent") };
    let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));
    let id = tabs.push_in(
        0,
        "calm-comet".to_owned(),
        Pane { body: Body::Creating, worktree: Some(w.worktree.clone()), agent: None },
    );
    let reopened_done = Reopened { tab: id, ..reopened_done };
    let lines = RefCell::new(Vec::new());
    let agent = |_: &Path, p: &tidewatch_core::prepare::Prepared| {
        lines.borrow_mut().push(p.launch.shell_line());
        1u8
    };
    let typed = |_: &Path, _: &tidewatch_core::prepare::Prepared| 2u8;
    let shell = |_: &Path, _: Option<&str>| 3u8;
    let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
    reopened(reopened_done, &mut tabs, &spawn, 1.0, &mut Vec::new());
    let lines = lines.into_inner();
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
    assert!(matches!(done, Done::Refused { tab: 7, refusal: Refusal::MidTurn }));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), true));
    assert!(matches!(done, Done::Retargeted(_)));
    assert_eq!(*system.sent.borrow(), [AGENT]);
}

#[test]
fn a_process_that_will_not_end_leaves_the_tab_as_it_was() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    system.exits.set(false);
    let done = run_retarget(&system, &env(&w), &request(&w, builder(None), false));
    assert!(matches!(done, Done::Refused { refusal: Refusal::WillNotEnd { .. }, .. }));
    assert_eq!(*system.sent.borrow(), [AGENT], "signalled once, never harder");
}

#[test]
fn a_session_whose_id_cannot_be_read_is_never_replaced_by_a_new_one() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let path = w.home.join(format!(".claude/sessions/{AGENT}.json"));
    let text = std::fs::read_to_string(&path).unwrap().replace(ID, "x");
    std::fs::write(&path, text).unwrap();
    let system = Fake::new(&ps());
    let mut asked = request(&w, builder(None), false);
    asked.subject.foreground_busy = true;
    let done = run_retarget(&system, &env(&w), &asked);
    assert!(matches!(done, Done::Refused { refusal: Refusal::ShellBusy, .. }));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn a_shell_target_ends_the_agent_and_hands_back_the_session_id() {
    let w = world();
    registry(&w, AGENT, START, "idle");
    let system = Fake::new(&ps());
    let done = run_retarget(&system, &env(&w), &request(&w, Target::Shell, false));
    let Done::Retargeted(landed) = done else { panic!("expected the agent to end") };
    let Retargeted::Shell { tab, cli, session_id } = *landed else { panic!("expected a shell") };
    assert_eq!((tab, cli, session_id.as_str()), (7, Cli::Claude, ID));
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
