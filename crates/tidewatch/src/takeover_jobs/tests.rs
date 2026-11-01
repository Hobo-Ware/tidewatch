use std::cell::{Cell, RefCell};
use std::process::Command;

use tidewatch_core::elsewhere::{Activity, Host};
use tidewatch_core::elsewhere_flags::Flags;
use tidewatch_core::elsewhere_scan::{Held, TitleCache};

use super::*;
use crate::takeover::status_of;
use crate::test_support::git;

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
    MoveJob {
        spec,
        agents: agents(),
        paths: None,
        root: None,
        group: None,
        wait: Duration::from_secs(1),
    }
}

fn spec(session: tidewatch_core::elsewhere::Session, worktree: bool) -> MoveSpec {
    MoveSpec { session, worktree, allow_quiet: false }
}

fn throwaway_repo(home: &Path) -> PathBuf {
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    repo.canonicalize().unwrap()
}

fn scan_spec(folders: Vec<PathBuf>) -> ScanSpec {
    ScanSpec {
        agents: agents(),
        folders,
        held: Held::default(),
        titles: TitleCache::default(),
        roots: HashMap::new(),
        idents: HashMap::new(),
    }
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
    spec.held.add_id(ID_B);
    let system = Fake::new(PS);
    let Done::Scanned(scanned) = run_scan(&system, &env(home.path()), &spec) else {
        panic!("expected a scan")
    };
    assert_eq!(scanned.sessions.len(), 1);
    assert_eq!(system.git_lookups.get(), 0);
    assert_eq!(scanned.roots, HashMap::new());
}

#[test]
fn a_scan_lists_one_ended_session_per_folder_and_none_that_a_tab_holds_by_id_or_file() {
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("acme");
    registry(home.path(), 11, ID, &work, "idle");
    let projects = home
        .path()
        .join(".claude/projects")
        .join(tidewatch_core::transcript_locate::claude_project_dir_name(&work));
    std::fs::create_dir_all(&projects).unwrap();
    let ids = [
        "aaaaaaaa-0000-4000-8000-000000000001",
        "aaaaaaaa-0000-4000-8000-000000000002",
        "aaaaaaaa-0000-4000-8000-000000000003",
        "aaaaaaaa-0000-4000-8000-000000000004",
    ];
    for (n, id) in ids.iter().enumerate() {
        let file = projects.join(format!("{id}.jsonl"));
        std::fs::write(&file, "{}\n").unwrap();
        let at = std::time::SystemTime::now() - Duration::from_secs(60 * (n as u64 + 1));
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(at).unwrap();
    }
    let mut spec = scan_spec(vec![work]);
    spec.held.add_id(ids[0]);
    spec.held.add_transcript(&projects.join(format!("{}.jsonl", ids[1])));
    let Done::Scanned(scanned) = run_scan(&Fake::new(PS), &env(home.path()), &spec) else {
        panic!("expected a scan")
    };
    let stored: Vec<_> = scanned
        .sessions
        .iter()
        .filter(|s| status_of(s) == crate::takeover::Status::NotRunning)
        .filter_map(|s| s.session_id.as_deref())
        .collect();
    assert_eq!(stored, [ids[2]]);
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
        fn cwd_holders(&self, _: &Path) -> Option<Vec<u32>> {
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

fn linked(repo: &Path, name: &str) -> PathBuf {
    let dir = repo.parent().unwrap().join(name);
    git(repo, &["worktree", "add", "-q", dir.to_str().unwrap(), "-b", name]);
    dir.canonicalize().unwrap()
}

fn ps_of(pids: &[u32]) -> String {
    let line = |pid: &u32| format!("{pid} 1 {START} claude --permission-mode acceptEdits\n");
    pids.iter().map(line).collect::<Vec<_>>().concat()
}

fn clone_of(url: &str, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["remote", "add", "origin", url]);
}

#[test]
fn sessions_in_linked_worktrees_land_in_the_group_of_their_main_repo_and_keep_their_folder() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    let trees = [linked(&repo, "web_TREE"), linked(&repo, "web_TREE_2"), repo.clone()];
    for (pid, tree) in (11..).zip(&trees) {
        registry(home.path(), pid, ID, tree, "idle");
        let system = Fake::new(&ps_of(&[pid]));
        let moved = run_move(
            &system,
            &env(home.path()),
            &job(spec(session(tree, pid, Activity::Idle), false)),
        );
        let Done::Moved(landed) = moved else { panic!("expected the move to land") };
        assert_eq!(landed.workspace.dir(), repo.as_path(), "{tree:?}");
        assert!(landed.worktree.is_none(), "{tree:?} is not a Tidewatch worktree");
        assert_eq!(landed.spec.session.cwd, *tree);
    }
}

#[test]
fn a_group_chosen_for_the_move_wins_over_the_folder_of_the_session() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    let clone = home.path().join("web_TREE");
    clone_of(repo.to_str().unwrap(), &clone);
    registry(home.path(), 11, ID, &clone, "idle");
    let system = Fake::new(PS);
    let moved = run_move(
        &system,
        &env(home.path()),
        &MoveJob {
            group: Some(repo.clone()),
            ..job(spec(session(&clone, 11, Activity::Idle), false))
        },
    );
    let Done::Moved(landed) = moved else { panic!("expected the move to land") };
    assert_eq!(landed.workspace.dir(), repo.as_path());
    assert_eq!(landed.spec.session.cwd, clone);
}

#[test]
fn a_scan_knows_the_main_repo_and_remote_of_every_folder_once() {
    let home = tempfile::tempdir().unwrap();
    let repo = throwaway_repo(home.path());
    git(&repo, &["remote", "add", "origin", "git@github.com:Acme/Web.git"]);
    let tree = linked(&repo, "web_TREE");
    let clone = home.path().join("web_TREE_2");
    clone_of("https://github.com/acme/web", &clone);
    let plain = home.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    for (pid, dir) in [(11, &tree), (12, &clone), (13, &plain)] {
        registry(home.path(), pid, ID, dir, "idle");
    }
    let ps = ps_of(&[11, 12, 13]);
    let mut spec = scan_spec(vec![repo.clone()]);
    let Done::Scanned(scanned) = run_scan(&Fake::new(&ps), &env(home.path()), &spec) else {
        panic!("expected a scan")
    };
    let remote = Some("github.com/acme/web".to_owned());
    let clone_main = clone.canonicalize().unwrap();
    assert_eq!(
        scanned.idents,
        HashMap::from([
            (repo.clone(), Some(Ident { main: repo.clone(), origin: remote.clone() })),
            (tree.clone(), Some(Ident { main: repo.clone(), origin: remote.clone() })),
            (clone.clone(), Some(Ident { main: clone_main, origin: remote })),
            (plain.clone(), None),
        ])
    );
    spec.idents = scanned.idents;
    let Done::Scanned(again) = run_scan(&Fake::new(&ps), &env(home.path()), &spec) else {
        panic!("expected a scan")
    };
    assert_eq!(again.idents, HashMap::new());
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
            group: None,
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
            group: None,
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
            group: None,
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
