use std::cell::{Cell, RefCell};
use std::process::Command;

use tidewatch_core::elsewhere::{Activity, Host};
use tidewatch_core::elsewhere_flags::Flags;

use super::*;
use crate::takeover::status_of;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const ID_B: &str = "79fdd8f6-5176-4a47-a46f-8db10ec660b5";
const START: &str = "Fri Oct 16 13:23:31 2026";

struct Fake {
    ps: String,
    sent: RefCell<Vec<u32>>,
    exits: Cell<bool>,
    gone: Cell<bool>,
    git_lookups: Cell<usize>,
}

impl Fake {
    fn new(ps: &str) -> Self {
        Self {
            ps: ps.to_owned(),
            sent: RefCell::new(Vec::new()),
            exits: Cell::new(true),
            gone: Cell::new(false),
            git_lookups: Cell::new(0),
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
        self.git_lookups.set(self.git_lookups.get() + 1);
        Some(dir.to_path_buf())
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

fn env(home: &Path) -> ScanEnv {
    ScanEnv {
        home: home.to_path_buf(),
        claude_dir: home.join(".claude"),
        codex_home: None,
        copilot_home: None,
        own_pid: 424_242,
    }
}

fn registry(home: &Path, pid: u32, id: &str, cwd: &Path, status: &str) {
    let text = format!(
        r#"{{"pid":{pid},"sessionId":"{id}","cwd":"{}","startedAt":1792157013549,"procStart":"{START}","kind":"interactive","name":"n{pid}","status":"{status}"}}"#,
        cwd.display()
    );
    let path = home.join(format!(".claude/sessions/{pid}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const PS: &str = "11 1 Fri Oct 16 13:23:31 2026 claude --permission-mode acceptEdits\n";

fn agents() -> AgentTable {
    AgentTable::builtin()
}

fn session(cwd: &Path, pid: u32, activity: Activity) -> tidewatch_core::elsewhere::Session {
    tidewatch_core::elsewhere::Session {
        cli: tidewatch_core::elsewhere::Cli::Claude,
        session_id: Some(ID.to_owned()),
        cwd: cwd.to_path_buf(),
        name: Some("retry-stage-gates".to_owned()),
        flags: Flags::default(),
        presence: tidewatch_core::elsewhere::Presence::Running(
            tidewatch_core::elsewhere::Running {
                pid,
                proc_start: START.to_owned(),
                host: Host::Terminal,
                activity,
            },
        ),
        since_ms: None,
    }
}

fn job(spec: MoveSpec) -> MoveJob {
    MoveJob { spec, agents: agents(), paths: None, root: None, wait: Duration::from_secs(1) }
}

fn spec(session: tidewatch_core::elsewhere::Session, worktree: bool) -> MoveSpec {
    MoveSpec { session, worktree, allow_quiet: false }
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

fn throwaway_repo(home: &Path) -> PathBuf {
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    repo.canonicalize().unwrap()
}

fn scan_spec(folders: Vec<PathBuf>) -> ScanSpec {
    ScanSpec { agents: agents(), folders, held: HashSet::new(), roots: HashMap::new() }
}

#[test]
fn a_scan_returns_running_sessions_then_recent_ones_and_the_repo_of_each_folder() {
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("acme");
    std::fs::create_dir_all(&work).unwrap();
    registry(home.path(), 11, ID, &work, "idle");
    let projects = home
        .path()
        .join(".claude/projects")
        .join(tidewatch_core::transcript_locate::claude_project_dir_name(&work));
    std::fs::create_dir_all(&projects).unwrap();
    for id in [ID, ID_B] {
        std::fs::write(projects.join(format!("{id}.jsonl")), "{}\n").unwrap();
    }
    let system = Fake::new(PS);
    let Done::Scanned(scanned) =
        run_scan(&system, &env(home.path()), &scan_spec(vec![work.clone()]))
    else {
        panic!("expected a scan")
    };
    let seen: Vec<_> =
        scanned.sessions.iter().map(|s| (s.session_id.clone().unwrap(), status_of(s))).collect();
    assert_eq!(
        seen,
        [
            (ID.to_owned(), crate::takeover::Status::Idle),
            (ID_B.to_owned(), crate::takeover::Status::NotRunning)
        ],
        "the running session is not also listed as recent"
    );
    assert_eq!(scanned.roots, HashMap::from([(work.clone(), Some(work))]));
    assert_eq!(system.git_lookups.get(), 1, "one look-up for the one folder");
}

#[test]
fn a_folder_looked_up_before_is_not_looked_up_again_and_open_tabs_are_not_recent() {
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("acme");
    registry(home.path(), 11, ID, &work, "idle");
    let projects = home
        .path()
        .join(".claude/projects")
        .join(tidewatch_core::transcript_locate::claude_project_dir_name(&work));
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(projects.join(format!("{ID_B}.jsonl")), "{}\n").unwrap();
    let mut spec = scan_spec(vec![work.clone()]);
    spec.roots.insert(work.clone(), None);
    spec.held.insert(ID_B.to_owned());
    let system = Fake::new(PS);
    let Done::Scanned(scanned) = run_scan(&system, &env(home.path()), &spec) else {
        panic!("expected a scan")
    };
    assert_eq!(scanned.sessions.len(), 1);
    assert_eq!(system.git_lookups.get(), 0);
    assert_eq!(scanned.roots, HashMap::new());
}

#[test]
fn a_scan_that_cannot_read_the_process_list_says_so() {
    struct NoPs;
    impl System for NoPs {
        fn ps(&self) -> Option<String> {
            None
        }
        fn cwd_of(&self, _: u32) -> Option<PathBuf> {
            None
        }
        fn git_root(&self, _: &Path) -> Option<PathBuf> {
            None
        }
        fn terminate(&self, _: u32) -> bool {
            false
        }
        fn alive(&self, _: u32) -> Option<String> {
            None
        }
    }
    let home = tempfile::tempdir().unwrap();
    assert!(matches!(run_scan(&NoPs, &env(home.path()), &scan_spec(vec![])), Done::ScanFailed));
}

#[test]
fn an_idle_session_is_ended_and_its_folder_becomes_the_workspace_of_its_tab() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    registry(home.path(), 11, ID, &repo, "idle");
    let system = Fake::new(PS);
    let moved =
        run_move(&system, &env(home.path()), &job(spec(session(&repo, 11, Activity::Idle), false)));
    let Done::Moved(landed) = moved else { panic!("expected the move to land") };
    assert_eq!(*system.sent.borrow(), [11]);
    assert_eq!(landed.workspace.dir(), repo.as_path());
    assert!(matches!(landed.workspace, Workspace::Repo(_)));
    assert!(landed.worktree.is_none());
}

#[test]
fn a_folder_outside_any_repo_lands_in_the_other_group() {
    let home = tempfile::tempdir().unwrap();
    let plain = home.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    registry(home.path(), 11, ID, &plain, "idle");
    let system = Fake::new(PS);
    let moved = run_move(
        &system,
        &env(home.path()),
        &job(spec(session(&plain, 11, Activity::Idle), false)),
    );
    let Done::Moved(landed) = moved else { panic!("expected the move to land") };
    assert!(matches!(landed.workspace, Workspace::Other { .. }));
}

#[test]
fn a_session_that_started_working_again_is_left_running_and_nothing_opens() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    registry(home.path(), 11, ID, &repo, "busy");
    let system = Fake::new(PS);
    let moved =
        run_move(&system, &env(home.path()), &job(spec(session(&repo, 11, Activity::Idle), false)));
    let Done::Failed(failure) = moved else { panic!("expected a refusal") };
    assert_eq!(failure.why, "it started working again; it was left running");
    assert_eq!(failure.name, "retry-stage-gates");
    assert!(failure.key.is_some());
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn a_process_that_ignores_sigterm_is_reported_and_never_killed_harder() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    registry(home.path(), 11, ID, &repo, "idle");
    let system = Fake::new(PS);
    system.exits.set(false);
    let started = std::time::Instant::now();
    let moved = run_move(
        &system,
        &env(home.path()),
        &MoveJob {
            spec: spec(session(&repo, 11, Activity::Idle), false),
            agents: agents(),
            paths: None,
            root: None,
            wait: Duration::from_secs(1),
        },
    );
    let Done::Failed(failure) = moved else { panic!("expected a failure") };
    assert!(failure.why.starts_with("it did not exit within 1 s"), "{}", failure.why);
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(*system.sent.borrow(), [11], "one SIGTERM and nothing else");
}

#[test]
fn a_session_not_running_is_opened_without_ending_anything() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    let mut stored = session(&repo, 0, Activity::Quiet);
    stored.presence = tidewatch_core::elsewhere::Presence::NotRunning;
    let system = Fake::new("");
    let moved = run_move(&system, &env(home.path()), &job(spec(stored, false)));
    assert!(matches!(moved, Done::Moved(_)));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn moving_into_a_worktree_makes_one_in_the_repo_before_the_session_is_ended() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    let paths = TidewatchPaths::new(&home.path().canonicalize().unwrap());
    registry(home.path(), 11, ID, &repo, "idle");
    let system = Fake::new(PS);
    let moved = run_move(
        &system,
        &env(home.path()),
        &MoveJob {
            spec: spec(session(&repo, 11, Activity::Idle), true),
            agents: agents(),
            paths: Some(paths),
            root: Some(repo.clone()),
            wait: Duration::from_secs(1),
        },
    );
    let Done::Moved(landed) = moved else { panic!("expected the move to land") };
    let made = landed.worktree.expect("a worktree");
    assert!(made.path.join(".git").exists());
    assert_eq!(made.repo, repo);
    assert_eq!(landed.workspace.dir(), repo.as_path());
}

#[test]
fn a_refused_move_into_a_worktree_takes_the_new_worktree_back() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    let paths = TidewatchPaths::new(&home.path().canonicalize().unwrap());
    registry(home.path(), 11, ID, &repo, "busy");
    let system = Fake::new(PS);
    let moved = run_move(
        &system,
        &env(home.path()),
        &MoveJob {
            spec: spec(session(&repo, 11, Activity::Idle), true),
            agents: agents(),
            paths: Some(paths.clone()),
            root: Some(repo.clone()),
            wait: Duration::from_secs(1),
        },
    );
    assert!(matches!(moved, Done::Failed(_)));
    let listed =
        Command::new("git").arg("-C").arg(&repo).args(["worktree", "list"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&listed.stdout).lines().count(),
        1,
        "no worktree is left behind"
    );
}

#[test]
fn a_worktree_needs_a_repo_and_a_home() {
    let home = tempfile::tempdir().unwrap();
    let system = Fake::new(PS);
    let moved = run_move(
        &system,
        &env(home.path()),
        &job(spec(session(home.path(), 11, Activity::Idle), true)),
    );
    let Done::Failed(failure) = moved else { panic!("expected a failure") };
    assert_eq!(failure.why, "a worktree needs a git repo");
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn every_refusal_has_a_sentence() {
    for refusal in [
        Refusal::Gone,
        Refusal::Working,
        Refusal::NotConfirmed,
        Refusal::Stdusk,
        Refusal::SignalFailed,
        Refusal::ScanFailed,
    ] {
        assert_ne!(refusal_text(refusal), "");
    }
}
