use tidewatch_core::elsewhere::{Presence, Running};
use tidewatch_core::elsewhere_flags::Flags;

use super::*;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const ID_B: &str = "79fdd8f6-5176-4a47-a46f-8db10ec660b5";

struct Fixture {
    agents: AgentTable,
    profiles: Profiles,
}

impl Fixture {
    fn new() -> Self {
        let profiles = Profiles::seeded(&["claude".to_owned(), "gemini".to_owned()]);
        Self { agents: profiles.agent_table(), profiles }
    }

    fn env<'a>(&'a self, installed: &'a dyn Fn(&str) -> bool) -> Env<'a> {
        Env {
            agents: &self.agents,
            profiles: Some(&self.profiles),
            installed,
            now_ms: 1_000_000_000_000,
            home: Some(Path::new("/Users/me")),
        }
    }
}

fn everything(_: &str) -> bool {
    true
}

fn running(cli: Cli, pid: u32, activity: Activity, host: Host) -> Session {
    Session {
        cli,
        session_id: Some(ID.to_owned()),
        cwd: PathBuf::from("/Users/me/code/acme"),
        name: Some(format!("session-{pid}")),
        flags: Flags { level: Some(Level::Edits), ..Flags::default() },
        presence: Presence::Running(Running {
            pid,
            proc_start: "Fri Oct 16 13:23:31 2026".to_owned(),
            host,
            activity,
        }),
        since_ms: Some(1_000_000_000_000 - 840_000),
    }
}

fn claude(pid: u32, activity: Activity) -> Session {
    running(Cli::Claude, pid, activity, Host::Terminal)
}

fn stored(cli: Cli, id: &str) -> Session {
    Session {
        presence: Presence::NotRunning,
        session_id: Some(id.to_owned()),
        name: None,
        ..running(cli, 0, Activity::Quiet, Host::Other)
    }
}

fn scanned(sessions: Vec<Session>) -> Scanned {
    Scanned { sessions, roots: HashMap::new(), idents: HashMap::new() }
}

fn takeover_with(sessions: Vec<Session>) -> Takeover {
    let mut takeover = Takeover::default();
    takeover.finish_scan(scanned(sessions));
    takeover
}

fn key_of(session: &Session) -> Key {
    session.key().unwrap()
}

#[test]
fn what_a_row_offers_follows_its_status_host_and_whether_it_can_resume() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let no_id = Session { session_id: None, ..running(Cli::Codex, 4, Activity::Idle, Host::Other) };
    let gemini_no_id =
        Session { session_id: None, ..running(Cli::Gemini, 5, Activity::Quiet, Host::Other) };
    let cases = [
        ("idle", claude(1, Activity::Idle), Offer::MoveHere),
        ("quiet", claude(1, Activity::Quiet), Offer::MoveHere),
        ("working", claude(1, Activity::Working), Offer::MoveWhenIdle),
        ("idle in stdusk", running(Cli::Claude, 1, Activity::Idle, Host::Stdusk), Offer::MoveLive),
        (
            "working in stdusk",
            running(Cli::Claude, 1, Activity::Working, Host::Stdusk),
            Offer::MoveLive,
        ),
        ("not running", stored(Cli::Claude, ID), Offer::ResumeHere),
        ("gemini resumes latest", gemini_no_id, Offer::MoveHere),
        (
            "codex without an id",
            no_id,
            Offer::Blocked("its session id could not be found".to_owned()),
        ),
    ];
    for (why, session, want) in cases {
        assert_eq!(Takeover::default().offer(&env, &session), want, "{why}");
    }
}

#[test]
fn a_cli_that_is_not_installed_is_blocked_with_the_reason() {
    let fixture = Fixture::new();
    let none = |_: &str| false;
    let env = fixture.env(&none);
    let offer = Takeover::default().offer(&env, &claude(1, Activity::Idle));
    assert_eq!(offer, Offer::Blocked("Claude Code is not installed".to_owned()));
}

#[test]
fn a_row_being_moved_or_waited_for_says_so() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let session = claude(1, Activity::Working);
    let mut takeover = takeover_with(vec![session.clone()]);
    takeover.confirm =
        Some(Confirm { sessions: vec![session.clone()], worktree: false, batch: false });
    assert_eq!(takeover.accept(), []);
    assert_eq!(takeover.offer(&env, &session), Offer::Waiting);
    let idle = claude(2, Activity::Idle);
    takeover.confirm =
        Some(Confirm { sessions: vec![idle.clone()], worktree: false, batch: false });
    takeover.accept();
    assert_eq!(takeover.offer(&env, &idle), Offer::Moving);
    takeover.moved(&key_of(&idle));
    assert_eq!(takeover.offer(&env, &idle), Offer::MoveHere);
}

#[test]
fn scans_start_when_the_card_opens_then_every_five_seconds_one_at_a_time() {
    let mut takeover = Takeover::default();
    assert!(!takeover.begin_scan(false, 0.0), "nothing shows, nothing waits");
    assert!(takeover.begin_scan(true, 1.0), "the first time it shows");
    assert!(!takeover.begin_scan(true, 20.0), "one at a time");
    takeover.finish_scan(scanned(vec![]));
    assert!(!takeover.begin_scan(true, 5.9));
    assert!(takeover.begin_scan(true, 6.0));
    takeover.finish_scan(scanned(vec![]));
    assert!(!takeover.begin_scan(false, 30.0), "hidden: no scan");
    assert!(takeover.begin_scan(true, 31.0), "shown again: a scan at once");
}

#[test]
fn the_next_scan_is_due_five_seconds_after_the_last_and_never_while_one_runs_or_nothing_shows() {
    let mut takeover = Takeover::default();
    assert_eq!(takeover.next_scan_in(false, 0.0), None);
    assert_eq!(takeover.next_scan_in(true, 0.0), Some(0.0));
    takeover.begin_scan(true, 10.0);
    assert_eq!(takeover.next_scan_in(true, 11.0), None, "one is under way");
    takeover.finish_scan(scanned(vec![]));
    assert_eq!(takeover.next_scan_in(true, 11.0), Some(4.0));
    assert_eq!(takeover.next_scan_in(true, 99.0), Some(0.0));
    assert_eq!(takeover.next_scan_in(false, 11.0), None);
}

#[test]
fn a_watch_keeps_scanning_while_the_card_is_hidden_and_stops_when_it_is_gone() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let busy = claude(1, Activity::Working);
    let mut takeover = takeover_with(vec![busy.clone()]);
    takeover.press(&env, &key_of(&busy));
    takeover.accept();
    assert!(takeover.watching());
    assert!(takeover.begin_scan(false, 100.0));
    takeover.finish_scan(scanned(vec![claude(1, Activity::Working)]));
    assert!(takeover.watching());
    takeover.finish_scan(scanned(vec![]));
    assert!(!takeover.watching(), "the session was closed by its owner");
    assert!(!takeover.begin_scan(false, 200.0));
}

#[test]
fn a_watched_session_moves_when_a_scan_finds_it_idle_and_not_before() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let busy = claude(1, Activity::Working);
    let mut takeover = takeover_with(vec![busy.clone()]);
    takeover.confirm = Some(Confirm { sessions: vec![busy.clone()], worktree: true, batch: false });
    assert_eq!(takeover.accept(), []);
    assert_eq!(takeover.finish_scan(scanned(vec![claude(1, Activity::Working)])), []);
    assert_eq!(takeover.offer(&env, &busy), Offer::Waiting);
    let ready = takeover.finish_scan(scanned(vec![claude(1, Activity::Idle)]));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].session, claude(1, Activity::Idle));
    assert!(ready[0].worktree, "the choice made in the dialog is kept");
    assert!(!ready[0].allow_quiet);
    assert!(!takeover.watching());
    assert!(takeover.is_moving(&key_of(&busy)));
}

#[test]
fn a_watch_is_dropped_when_the_session_turns_quiet_or_is_replaced() {
    let busy = claude(1, Activity::Working);
    for next in [vec![claude(1, Activity::Quiet)], vec![claude(2, Activity::Idle)], vec![]] {
        let mut takeover = takeover_with(vec![busy.clone()]);
        takeover.armed.push(Armed { session: busy.clone(), worktree: false });
        assert_eq!(takeover.finish_scan(scanned(next)), []);
        assert!(!takeover.watching());
    }
}

#[test]
fn cancelling_from_the_row_drops_the_watch() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let busy = claude(1, Activity::Working);
    let mut takeover = takeover_with(vec![busy.clone()]);
    takeover.armed.push(Armed { session: busy.clone(), worktree: false });
    assert_eq!(takeover.press(&env, &key_of(&busy)), Effect::None);
    assert!(!takeover.watching());
    assert!(!takeover.dialog_open());
}

#[test]
fn pressing_a_row_opens_the_dialog_except_in_stdusk_where_it_only_hints() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let live = running(Cli::Claude, 1, Activity::Idle, Host::Stdusk);
    let here = claude(2, Activity::Idle);
    let mut takeover = takeover_with(vec![live.clone(), here.clone()]);
    assert_eq!(takeover.press(&env, &key_of(&live)), Effect::StdduskHint);
    assert!(!takeover.dialog_open());
    assert_eq!(takeover.press(&env, &key_of(&here)), Effect::None);
    assert!(takeover.dialog_open());
    assert_eq!(takeover.confirm.as_ref().map(|c| c.sessions.len()), Some(1));
    takeover.cancel();
    assert!(!takeover.dialog_open());
    assert_eq!(
        takeover.press(&env, &Key::Stored { cli: Cli::Claude, id: "gone".into() }),
        Effect::None
    );
}

#[test]
fn accepting_starts_an_idle_session_asks_leave_to_a_quiet_one_and_arms_a_busy_one() {
    let sessions = vec![
        claude(1, Activity::Idle),
        running(Cli::Codex, 2, Activity::Quiet, Host::Other),
        claude(3, Activity::Working),
        stored(Cli::Claude, ID_B),
    ];
    let mut takeover = takeover_with(sessions.clone());
    takeover.confirm = Some(Confirm { sessions: sessions.clone(), worktree: true, batch: false });
    let started = takeover.accept();
    let shape: Vec<(Option<u32>, bool, bool)> = started
        .iter()
        .map(|s| (s.session.running().map(|r| r.pid), s.worktree, s.allow_quiet))
        .collect();
    assert_eq!(shape, [(Some(1), true, false), (Some(2), true, true), (None, true, false)]);
    assert!(takeover.watching());
    assert!(!takeover.dialog_open());
}

#[test]
fn a_batch_never_moves_into_a_worktree() {
    let sessions = vec![claude(1, Activity::Idle), claude(2, Activity::Idle)];
    let mut takeover = takeover_with(sessions.clone());
    takeover.confirm = Some(Confirm { sessions, worktree: true, batch: true });
    assert!(takeover.accept().iter().all(|spec| !spec.worktree));
}

#[test]
fn move_all_idle_takes_idle_sessions_outside_stdusk() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let yolo = yolo_of(Cli::Claude, 4);
    let mut takeover = takeover_with(vec![
        claude(1, Activity::Idle),
        running(Cli::Claude, 2, Activity::Idle, Host::Stdusk),
        claude(3, Activity::Working),
        yolo,
        running(Cli::Gemini, 5, Activity::Quiet, Host::Other),
        stored(Cli::Claude, ID_B),
        claude(6, Activity::Idle),
    ]);
    let pids: Vec<_> =
        takeover.idle_movable(&env).iter().filter_map(|s| s.running().map(|r| r.pid)).collect();
    assert_eq!(pids, [1, 4, 6]);
    takeover.armed.push(Armed { session: claude(6, Activity::Working), worktree: false });
    let pids: Vec<_> =
        takeover.idle_movable(&env).iter().filter_map(|s| s.running().map(|r| r.pid)).collect();
    assert_eq!(pids, [1, 4], "one that is already waiting is left alone");
    assert!(takeover.open_batch(&env));
    assert!(takeover.confirm.as_ref().is_some_and(|c| c.batch && c.sessions.len() == 2));
    let mut none = takeover_with(vec![claude(3, Activity::Working)]);
    assert!(!none.open_batch(&env));
}

#[test]
fn the_card_counts_running_and_idle_sessions_only() {
    let takeover = takeover_with(vec![
        claude(1, Activity::Idle),
        claude(2, Activity::Idle),
        claude(3, Activity::Working),
        stored(Cli::Claude, ID_B),
    ]);
    assert_eq!(takeover.counts(), (3, 2));
}

#[test]
fn rows_are_ordered_idle_then_working_then_unknown_then_ended_newest_first() {
    let older = Session { since_ms: Some(1), ..claude(1, Activity::Idle) };
    let newer = Session { since_ms: Some(9), ..claude(2, Activity::Idle) };
    let mut list = vec![
        stored(Cli::Claude, ID_B),
        running(Cli::Codex, 3, Activity::Quiet, Host::Other),
        claude(4, Activity::Working),
        older,
        newer,
    ];
    order(&mut list);
    let names: Vec<_> = list.iter().map(status_of).collect();
    assert_eq!(
        names,
        [Status::Idle, Status::Idle, Status::Working, Status::Quiet, Status::NotRunning]
    );
    assert_eq!(list[0].running().map(|r| r.pid), Some(2));
}

#[test]
fn rows_read_as_the_mockup_does() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let yolo = Session {
        flags: Flags { level: Some(Level::Yolo), ..Flags::default() },
        name: None,
        ..running(Cli::Codex, 9, Activity::Quiet, Host::Other)
    };
    let mut sessions = vec![yolo, claude(1, Activity::Idle), stored(Cli::Gemini, ID_B)];
    order(&mut sessions);
    let takeover = takeover_with(sessions);
    let rows = takeover.rows(&env);
    let got: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.folder.as_str(),
                r.host.as_str(),
                r.level.as_str(),
                r.yolo,
                r.status,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("session-1", "~/code/acme", "Terminal", "Auto edit", false, Status::Idle),
            ("codex · 14 min ago", "~/code/acme", "another terminal", "YOLO", true, Status::Quiet),
            (
                "gemini · ended 14 min ago",
                "~/code/acme",
                "ended 14 min ago",
                "Auto edit",
                false,
                Status::NotRunning
            ),
        ]
    );
}

#[test]
fn the_resume_profile_is_the_sessions_own_flags_on_the_profile_of_its_cli() {
    let fixture = Fixture::new();
    let session = Session {
        flags: Flags {
            level: Some(Level::Plan),
            model: Some("opus".into()),
            effort: Some("high".into()),
            extra: vec!["--add-dir".into(), "/tmp".into()],
        },
        ..claude(1, Activity::Idle)
    };
    let agent = fixture.agents.get("claude").unwrap();
    let (id, profile) = resolve_profile(Some(&fixture.profiles), agent, &session);
    assert_eq!(id, "reviewer", "the profile at the same level");
    assert_eq!(profile.name, "Reviewer");
    assert_eq!(profile.level, Level::Plan);
    assert_eq!(profile.model.as_deref(), Some("opus"));
    assert_eq!(profile.effort.as_deref(), Some("high"));
    assert_eq!(profile.extra_args.0, ["--add-dir", "/tmp"]);
    assert_eq!(profile.setup, "");
    let plain = Session { flags: Flags::default(), ..claude(1, Activity::Idle) };
    let (id, profile) = resolve_profile(Some(&fixture.profiles), agent, &plain);
    assert_eq!((id.as_str(), profile.level, profile.model), ("builder", Level::Ask, None));
    let codex = Session { cli: Cli::Codex, ..claude(1, Activity::Idle) };
    let agent = AgentTable::builtin();
    let (id, profile) =
        resolve_profile(Some(&fixture.profiles), agent.get("codex").unwrap(), &codex);
    assert_eq!((id.as_str(), profile.name.as_str()), ("", "Codex"), "no profile of that CLI");
}

#[test]
fn the_dialog_shows_the_command_a_person_would_type() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let session = Session {
        flags: Flags {
            level: Some(Level::Edits),
            model: Some("opus".into()),
            effort: None,
            extra: vec!["--add-dir".into(), "/tmp".into()],
        },
        ..claude(1, Activity::Idle)
    };
    assert_eq!(
        preview_command(&env, &session, false).as_deref(),
        Some(
            format!(
                "claude --resume {ID} --permission-mode acceptEdits --model opus --effort medium --add-dir /tmp"
            )
            .as_str()
        )
    );
    let gemini = Session {
        session_id: None,
        cli: Cli::Gemini,
        flags: Flags::default(),
        ..claude(1, Activity::Idle)
    };
    assert_eq!(
        preview_command(&env, &gemini, false).as_deref(),
        Some("gemini --resume latest --approval-mode default")
    );
    let codex = Session { cli: Cli::Codex, flags: Flags::default(), ..claude(1, Activity::Idle) };
    assert_eq!(
        preview_command(&env, &codex, false).as_deref(),
        Some(
            format!("codex resume {ID} --sandbox read-only --ask-for-approval on-request").as_str()
        )
    );
}

#[test]
fn a_moved_session_opens_through_the_restore_path() {
    let fixture = Fixture::new();
    let edits =
        MoveSpec { session: claude(1, Activity::Idle), worktree: false, allow_quiet: false };
    let plan = window_plan(
        &fixture.agents,
        Some(&fixture.profiles),
        &edits,
        (Path::new("/Users/me/code/acme"), false),
        None,
    )
    .unwrap();
    assert_eq!(plan.dir, PathBuf::from("/Users/me/code/acme"));
    let tab = &plan.tabs[0];
    assert_eq!(tab.dir, PathBuf::from("/Users/me/code/acme"));
    let TabAction::ResumeAgent { agent, profile, session_id, how } = &tab.action else {
        panic!("expected a resume")
    };
    assert_eq!((agent.as_str(), session_id.as_str(), profile.level), ("claude", ID, Level::Edits));
    assert_eq!(*how, tidewatch_core::resume_mode::Resume::Run);
    let SavedTab::Agent(saved) = &tab.saved else { panic!("expected an agent tab") };
    assert_eq!((saved.profile.as_str(), saved.session_id.as_deref()), ("builder", Some(ID)));
    assert_eq!(saved.branch, None);
}

#[test]
fn a_session_in_a_tree_folder_runs_there_in_the_group_of_the_main_repo_and_is_no_tidewatch_worktree()
 {
    let fixture = Fixture::new();
    let in_tree =
        Session { cwd: PathBuf::from("/Users/me/code/acme_TREE"), ..claude(1, Activity::Idle) };
    let spec = MoveSpec { session: in_tree, worktree: false, allow_quiet: false };
    let plan = window_plan(
        &fixture.agents,
        Some(&fixture.profiles),
        &spec,
        (Path::new("/Users/me/code/acme"), false),
        None,
    )
    .unwrap();
    assert_eq!(plan.dir, PathBuf::from("/Users/me/code/acme"));
    let tab = &plan.tabs[0];
    assert_eq!(tab.dir, PathBuf::from("/Users/me/code/acme_TREE"));
    assert_eq!(tab.worktree, None);
    let SavedTab::Agent(saved) = &tab.saved else { panic!("expected an agent tab") };
    assert_eq!((saved.dir.as_path(), saved.branch.as_deref()), (tab.dir.as_path(), None));
}

#[test]
fn a_yolo_session_waits_typed_for_enter_and_a_worktree_is_where_the_tab_runs() {
    let fixture = Fixture::new();
    let yolo = Session {
        flags: Flags { level: Some(Level::Yolo), ..Flags::default() },
        ..claude(1, Activity::Idle)
    };
    let spec = MoveSpec { session: yolo, worktree: true, allow_quiet: false };
    let made = Worktree {
        repo: PathBuf::from("/Users/me/code/acme"),
        path: PathBuf::from("/Users/me/.tidewatch/acme-ab12cd/swift-badger"),
        branch: "agent/swift-badger".into(),
    };
    let plan = window_plan(
        &fixture.agents,
        Some(&fixture.profiles),
        &spec,
        (Path::new("/Users/me/code/acme"), false),
        Some(made.clone()),
    )
    .unwrap();
    let tab = &plan.tabs[0];
    assert_eq!(tab.dir, made.path);
    assert_eq!(tab.worktree, Some(made));
    let TabAction::ResumeAgent { how, .. } = &tab.action else { panic!("expected a resume") };
    assert_eq!(
        *how,
        tidewatch_core::resume_mode::Resume::TypeOnly(tidewatch_core::resume_mode::Why::Yolo)
    );
    let SavedTab::Agent(saved) = &tab.saved else { panic!("expected an agent tab") };
    assert_eq!(saved.branch.as_deref(), Some("agent/swift-badger"));
}

#[test]
fn a_gemini_session_without_an_id_resumes_latest() {
    let fixture = Fixture::new();
    let gemini = Session { session_id: None, cli: Cli::Gemini, ..claude(1, Activity::Idle) };
    let spec = MoveSpec { session: gemini, worktree: false, allow_quiet: true };
    let plan =
        window_plan(&fixture.agents, Some(&fixture.profiles), &spec, (Path::new("/a"), true), None)
            .unwrap();
    let TabAction::ResumeAgent { session_id, .. } = &plan.tabs[0].action else { panic!() };
    assert_eq!(session_id, "latest");
    assert!(plan.other);
}

#[test]
fn the_dialog_says_what_will_happen_for_each_kind_of_row() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let mut takeover = Takeover::default();
    let open = |takeover: &mut Takeover, session: Session, worktree: bool| {
        takeover.confirm = Some(Confirm { sessions: vec![session], worktree, batch: false });
        takeover.dialog_view(&env).unwrap()
    };
    let idle = open(&mut takeover, claude(1, Activity::Idle), false);
    assert_eq!(idle.title, "Move session-1 to Tidewatch?");
    assert_eq!(idle.confirm, "Move");
    assert_eq!(
        idle.lines[0],
        "Ends Claude in another terminal. The window stays open at a shell."
            .replace("another terminal", "Terminal")
    );
    assert_eq!(
        idle.lines[1],
        "Opens a tab in ~/code/acme and resumes the same conversation with the same flags."
    );
    assert_eq!(
        idle.lines[2],
        "The Reader shows the whole history, and hooks, notes and PR cards work from here on."
    );
    assert_eq!((idle.notes.len(), idle.yolo_note), (0, None));
    let busy = open(&mut takeover, claude(1, Activity::Working), false);
    assert_eq!(busy.title, "Move session-1 to Tidewatch when it is idle?");
    assert_eq!(busy.confirm, "Move when idle");
    assert!(
        busy.lines[0].starts_with("Waits until it is idle, then ends Claude in Terminal"),
        "{}",
        busy.lines[0]
    );
    let quiet = open(&mut takeover, running(Cli::Codex, 2, Activity::Quiet, Host::Other), false);
    assert_eq!(quiet.notes.len(), 1);
    assert!(quiet.notes[0].starts_with("Its status is not known"));
    let ended = open(&mut takeover, stored(Cli::Claude, ID_B), false);
    assert_eq!(ended.title, "Resume claude · ended 14 min ago in Tidewatch?");
    assert_eq!(ended.confirm, "Resume");
    assert_eq!(ended.lines[0], "Nothing is ended: no process holds this conversation.");
}

#[test]
fn the_worktree_box_is_offered_for_claude_in_a_repo_and_changes_the_second_line() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let session = claude(1, Activity::Idle);
    let mut takeover = takeover_with(vec![session.clone()]);
    takeover.confirm =
        Some(Confirm { sessions: vec![session.clone()], worktree: true, batch: false });
    let without = takeover.dialog_view(&env).unwrap();
    assert!(!without.offer_worktree && !without.worktree, "no repo is known for the folder");
    let mut roots = HashMap::new();
    roots.insert(session.cwd.clone(), Some(session.cwd.clone()));
    takeover.finish_scan(Scanned {
        sessions: vec![session.clone()],
        roots,
        idents: HashMap::new(),
    });
    takeover.confirm =
        Some(Confirm { sessions: vec![session.clone()], worktree: true, batch: false });
    let with = takeover.dialog_view(&env).unwrap();
    assert!(with.offer_worktree && with.worktree);
    assert!(
        with.lines[1].starts_with("Opens a tab in a new worktree of ~/code/acme"),
        "{}",
        with.lines[1]
    );
    let codex = Session { cli: Cli::Codex, ..session.clone() };
    takeover.confirm = Some(Confirm { sessions: vec![codex], worktree: true, batch: false });
    assert!(
        !takeover.dialog_view(&env).unwrap().offer_worktree,
        "only Claude is known to resume from another folder"
    );
}

const NOT_A_REPO: &str =
    "This folder is not a git repo, so it keeps running YOLO here as it did before.";

fn yolo_of(cli: Cli, pid: u32) -> Session {
    Session {
        flags: Flags { level: Some(Level::Yolo), ..Flags::default() },
        ..running(cli, pid, Activity::Idle, Host::Terminal)
    }
}

#[test]
fn a_yolo_session_always_has_a_way_to_move() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let claude_yolo = yolo_of(Cli::Claude, 1);
    let gemini_yolo = yolo_of(Cli::Gemini, 2);
    let cases = [
        ("not a repo", claude_yolo.clone(), false, false, Some(NOT_A_REPO.to_owned())),
        ("repo, box ticked", claude_yolo.clone(), true, true, None),
        (
            "repo, box unticked",
            claude_yolo.clone(),
            true,
            false,
            Some("YOLO only runs in a worktree, so it moves here as Auto.".to_owned()),
        ),
        (
            "repo, no worktree for this CLI",
            gemini_yolo,
            true,
            false,
            Some("YOLO only runs in a worktree, so it moves here as Auto.".to_owned()),
        ),
        ("not yolo", claude(3, Activity::Idle), true, false, None),
    ];
    for (why, session, repo, worktree, want) in cases {
        let mut takeover = Takeover::default();
        if repo {
            takeover.roots.insert(session.cwd.clone(), Some(session.cwd.clone()));
        }
        takeover.confirm =
            Some(Confirm { sessions: vec![session.clone()], worktree, batch: false });
        let view = takeover.dialog_view(&env).unwrap();
        assert_eq!(view.yolo_note, want, "{why}");
    }
}

#[test]
fn pressing_move_on_a_yolo_session_ticks_the_worktree_box_only_where_one_can_be_made() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let cases = [
        ("claude in a repo", yolo_of(Cli::Claude, 1), true, true),
        ("claude not in a repo", yolo_of(Cli::Claude, 2), false, false),
        ("gemini in a repo", yolo_of(Cli::Gemini, 3), true, false),
        ("plain claude in a repo", claude(4, Activity::Idle), true, false),
    ];
    for (why, session, repo, ticked) in cases {
        let mut takeover = takeover_with(vec![session.clone()]);
        if repo {
            takeover.roots.insert(session.cwd.clone(), Some(session.cwd.clone()));
        }
        takeover.press(&env, &key_of(&session));
        assert_eq!(takeover.confirm.as_ref().map(|c| c.worktree), Some(ticked), "{why}");
    }
}

#[test]
fn an_ended_yolo_session_resumes_into_a_worktree_at_yolo_or_one_level_down_never_at_ask() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let ended = Session {
        flags: Flags { level: Some(Level::Yolo), ..Flags::default() },
        ..stored(Cli::Claude, ID)
    };
    let mut takeover = takeover_with(vec![ended.clone()]);
    takeover.roots.insert(ended.cwd.clone(), Some(ended.cwd.clone()));
    assert_eq!(takeover.offer(&env, &ended), Offer::ResumeHere);
    takeover.press(&env, &key_of(&ended));
    assert_eq!(takeover.confirm.as_ref().map(|c| c.worktree), Some(true));
    let specs = takeover.accept();
    let dir = Path::new("/Users/me/code/acme");
    let made = Worktree {
        repo: dir.to_path_buf(),
        path: PathBuf::from("/Users/me/.tidewatch/acme-ab12cd/swift-badger"),
        branch: "agent/swift-badger".into(),
    };
    let levels = [(Some(made), Level::Yolo), (None, Level::Auto)];
    for (worktree, want) in levels {
        let plan = window_plan(
            &fixture.agents,
            Some(&fixture.profiles),
            &specs[0],
            (dir, false),
            worktree,
        )
        .unwrap();
        let SavedTab::Agent(saved) = &plan.tabs[0].saved else { panic!("expected an agent tab") };
        assert_eq!(saved.level, want);
    }
}

#[test]
fn a_lowered_yolo_preview_shows_the_level_it_will_run_at() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let session = yolo_of(Cli::Claude, 1);
    let yolo = preview_command(&env, &session, false).unwrap();
    let lowered = preview_command(&env, &session, true).unwrap();
    assert!(yolo.contains("--dangerously-skip-permissions"), "{yolo}");
    assert!(!lowered.contains("--dangerously-skip-permissions"), "{lowered}");
}

#[test]
fn a_yolo_session_moves_as_it_was_in_a_plain_folder_and_one_level_down_in_a_repo() {
    let fixture = Fixture::new();
    let session = yolo_of(Cli::Claude, 1);
    let spec = MoveSpec { session, worktree: false, allow_quiet: false };
    let dir = Path::new("/Users/me/code/acme");
    let level_of = |other: bool| {
        let plan = window_plan(&fixture.agents, Some(&fixture.profiles), &spec, (dir, other), None)
            .unwrap();
        let SavedTab::Agent(saved) = &plan.tabs[0].saved else { panic!("expected an agent tab") };
        saved.level
    };
    assert_eq!(level_of(true), Level::Yolo);
    assert_eq!(level_of(false), Level::Auto);
}

#[test]
fn the_batch_dialog_lists_every_session_and_takes_yolo_in() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let mut takeover = takeover_with(vec![claude(1, Activity::Idle), claude(2, Activity::Idle)]);
    assert!(takeover.open_batch(&env));
    let view = takeover.dialog_view(&env).unwrap();
    assert_eq!(view.title, "Move 2 idle sessions to Tidewatch?");
    assert_eq!(view.confirm, "Move all");
    assert_eq!(view.items.len(), 2);
    assert!(!view.offer_worktree);
    assert_eq!(view.yolo_note, None);
    assert!(view.notes[0].contains("A YOLO session moves into a new worktree"));
}

#[test]
fn move_all_idle_puts_a_yolo_session_in_a_worktree_only_where_one_can_be_made() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let in_repo = yolo_of(Cli::Claude, 1);
    let gemini = yolo_of(Cli::Gemini, 2);
    let plain_dir = Session { cwd: PathBuf::from("/Users/me/notes"), ..yolo_of(Cli::Claude, 3) };
    let mut takeover = Takeover::default();
    let mut roots = HashMap::new();
    roots.insert(in_repo.cwd.clone(), Some(in_repo.cwd.clone()));
    roots.insert(plain_dir.cwd.clone(), None);
    takeover.finish_scan(Scanned {
        sessions: vec![in_repo, gemini, plain_dir, claude(4, Activity::Idle)],
        roots,
        idents: HashMap::new(),
    });
    assert!(takeover.open_batch(&env));
    let worktrees: Vec<_> = takeover
        .accept()
        .iter()
        .filter_map(|spec| spec.session.running().map(|r| (r.pid, spec.worktree)))
        .collect();
    assert_eq!(worktrees, [(1, true), (2, false), (3, false), (4, false)]);
}

#[test]
fn a_reveal_is_asked_for_once_and_forces_a_scan() {
    let mut takeover = Takeover::default();
    takeover.begin_scan(true, 0.0);
    takeover.finish_scan(scanned(vec![]));
    assert!(!takeover.begin_scan(true, 1.0));
    takeover.ask_to_reveal();
    assert!(takeover.begin_scan(true, 1.5));
    assert!(takeover.take_reveal());
    assert!(!takeover.take_reveal());
}

#[test]
fn the_elsewhere_line_folds_and_opens_and_a_reveal_opens_it() {
    let mut takeover = Takeover::default();
    assert!(!takeover.line_open());
    takeover.toggle_line();
    assert!(takeover.line_open());
    assert!(takeover.take_reveal());
    takeover.toggle_line();
    assert!(!takeover.line_open());
    assert!(!takeover.take_reveal());
    takeover.ask_to_reveal();
    assert!(takeover.line_open());
    takeover.fold_line();
    assert!(!takeover.line_open());
}

#[test]
fn a_scan_that_finds_nothing_running_folds_the_line() {
    let mut takeover = Takeover::default();
    takeover.ask_to_reveal();
    takeover.finish_scan(scanned(vec![claude(1, Activity::Idle)]));
    assert!(takeover.line_open());
    takeover.finish_scan(scanned(vec![]));
    assert!(!takeover.line_open());
    assert!(!takeover.take_reveal());
}

fn titled(id: &str, title: &str) -> Session {
    Session { name: Some(title.to_owned()), ..stored(Cli::Claude, id) }
}

#[test]
fn ended_sessions_are_listed_apart_from_the_running_ones_and_never_alone() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let mixed = takeover_with(vec![claude(1, Activity::Idle), stored(Cli::Claude, ID_B)]);
    let rows = mixed.card_rows(&env).expect("something runs");
    assert_eq!(rows.running.len(), 1);
    assert_eq!(rows.ended.len(), 1);
    assert_eq!(rows.ended[0].status, Status::NotRunning);
    assert_eq!(rows.ended[0].offer, Offer::ResumeHere);

    let only_ended = takeover_with(vec![stored(Cli::Claude, ID_B)]);
    assert_eq!(only_ended.card_rows(&env), None);
}

#[test]
fn an_ended_row_is_named_by_its_title_and_says_when_it_ended() {
    let fixture = Fixture::new();
    let env = fixture.env(&everything);
    let takeover = takeover_with(vec![
        claude(1, Activity::Idle),
        Session { since_ms: Some(env.now_ms - 33 * 60_000), ..titled(ID_B, "Fix login redirect") },
        Session { since_ms: Some(env.now_ms - 40 * 60_000), ..stored(Cli::Claude, ID) },
    ]);
    let ended = takeover.card_rows(&env).expect("something runs").ended;
    let got: Vec<_> = ended.iter().map(|row| (row.name.as_str(), row.host.as_str())).collect();
    assert_eq!(
        got,
        [
            ("Fix login redirect", "ended 33 min ago"),
            ("claude \u{b7} ended 40 min ago", "ended 40 min ago"),
        ]
    );
}

#[test]
fn the_recent_list_opens_and_closes() {
    let mut takeover = Takeover::default();
    assert!(!takeover.recent_open());
    takeover.toggle_recent();
    assert!(takeover.recent_open());
    takeover.toggle_recent();
    assert!(!takeover.recent_open());
}

#[test]
fn what_an_open_tab_held_stays_held_after_the_tab_is_gone() {
    let mut takeover = Takeover::default();
    takeover.note_open(Some(ID_B), Some(Path::new("/store/other.jsonl")));
    takeover.note_open(None, None);
    assert!(takeover.held().ids.contains(ID_B));
    assert!(takeover.held().transcripts.contains(Path::new("/store/other.jsonl")));
}
