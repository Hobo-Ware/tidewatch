use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::Command;

use super::*;
use crate::agents::Level;
use crate::elsewhere::Host;

const OWN: u32 = 424_242;
const START: &str = "Fri Oct 16 13:23:31 2026";
const ID_A: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const ID_B: &str = "79fdd8f6-5176-4a47-a46f-8db10ec660b5";
const ID_C: &str = "0f5e4aa6-cdf3-464c-8123-4ddb654ee653";

struct Fake {
    ps: String,
    cwds: HashMap<u32, PathBuf>,
    sent: RefCell<Vec<u32>>,
    polls_alive_after_signal: Cell<i32>,
    signalled: Cell<bool>,
    start: String,
    can_signal: bool,
    holders: Option<Vec<u32>>,
}

impl Fake {
    fn new(ps: &str) -> Self {
        Self {
            ps: ps.to_owned(),
            cwds: HashMap::new(),
            sent: RefCell::new(Vec::new()),
            polls_alive_after_signal: Cell::new(0),
            signalled: Cell::new(false),
            start: START.to_owned(),
            can_signal: true,
            holders: Some(Vec::new()),
        }
    }
}

impl System for Fake {
    fn ps(&self) -> Option<String> {
        Some(self.ps.clone())
    }

    fn cwd_of(&self, pid: u32) -> Option<PathBuf> {
        self.cwds.get(&pid).cloned()
    }

    fn git_root(&self, dir: &Path) -> Option<PathBuf> {
        Some(dir.to_path_buf())
    }

    fn cwd_holders(&self, _dir: &Path) -> Option<Vec<u32>> {
        self.holders.clone()
    }

    fn terminate(&self, pid: u32) -> bool {
        self.sent.borrow_mut().push(pid);
        self.signalled.set(self.can_signal);
        self.can_signal
    }

    fn alive(&self, _pid: u32) -> Option<String> {
        if !self.signalled.get() {
            return Some(self.start.clone());
        }
        let left = self.polls_alive_after_signal.get();
        self.polls_alive_after_signal.set(left - 1);
        (left > 0).then(|| self.start.clone())
    }
}

fn env(home: &Path) -> ScanEnv {
    ScanEnv {
        home: home.to_path_buf(),
        claude_dir: home.join(".claude"),
        codex_home: None,
        copilot_home: None,
        own_pid: OWN,
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn registry(home: &Path, pid: u32, id: &str, start: &str, status: &str) {
    let text = format!(
        r#"{{"pid":{pid},"sessionId":"{id}","cwd":"/work/acme","startedAt":1792157013549,"procStart":"{start}","kind":"interactive","name":"n{pid}","status":"{status}"}}"#
    );
    write(&home.join(format!(".claude/sessions/{pid}.json")), &text);
}

fn age(path: &Path, seconds: u64) {
    let when = SystemTime::now() - Duration::from_secs(seconds);
    fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

const PS: &str = "  PID  PPID STARTED                      ARGS
90775     1 Wed Oct  7 09:29:20 2026     /Applications/stdusk.app/Contents/MacOS/stdusk
11662 90775 Fri Oct 16 13:20:00 2026     /bin/zsh
19821 11662 Fri Oct 16 13:23:31 2026     claude --permission-mode acceptEdits
25746     1 Wed Oct 14 09:59:03 2026     claude --permission-mode bypassPermissions
";

#[test]
fn a_scan_reads_the_registry_and_the_process_table_together() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 19821, ID_A, START, "idle");
    registry(home.path(), 25746, ID_B, "Wed Oct 14 09:59:03 2026", "busy");
    registry(home.path(), 8173, ID_C, "Sun Oct 4 20:17:36 2026", "idle");
    let scan = scan_running(&Fake::new(PS), &env(home.path()), &AgentTable::builtin()).unwrap();
    let got: Vec<_> = scan
        .sessions
        .iter()
        .map(|s| {
            let r = s.running().unwrap();
            (r.pid, r.host, r.activity, s.flags.level)
        })
        .collect();
    assert_eq!(
        got,
        [
            (19821, Host::Stdusk, Activity::Idle, Some(Level::Edits)),
            (25746, Host::Other, Activity::Working, Some(Level::Yolo)),
        ]
    );
    assert_eq!(scan.held_ids, HashSet::from([ID_A.to_owned(), ID_B.to_owned()]));
}

#[test]
fn only_digit_dot_json_files_of_the_registry_are_read() {
    let home = tempfile::tempdir().unwrap();
    let sessions = home.path().join(".claude/sessions");
    registry(home.path(), 19821, ID_A, START, "idle");
    let text = fs::read_to_string(sessions.join("19821.json")).unwrap();
    for decoy in ["19821.abc.key", "19821.json.key", "notes.json", "19821.json.bak"] {
        write(&sessions.join(decoy), &text.replace("19821", "25746"));
    }
    write(&sessions.join("25746.json"), &text.replace("19821", "99999"));
    write(&sessions.join("5.json"), &format!("{}{}", text, " ".repeat(70_000)));
    let entries = registry_entries(&env(home.path()).claude_dir);
    assert_eq!(entries.iter().map(|e| e.pid).collect::<Vec<_>>(), [19821]);
}

#[test]
fn a_missing_registry_folder_is_no_sessions() {
    let home = tempfile::tempdir().unwrap();
    let scan = scan_running(&Fake::new(PS), &env(home.path()), &AgentTable::builtin()).unwrap();
    assert_eq!(scan.sessions, []);
}

#[test]
fn sessions_of_tidewatchs_own_tabs_are_not_running_elsewhere_but_are_held() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 19821, ID_A, START, "idle");
    let mut own = env(home.path());
    own.own_pid = 90775;
    let scan = scan_running(&Fake::new(PS), &own, &AgentTable::builtin()).unwrap();
    assert_eq!(scan.sessions, []);
    assert!(scan.held_ids.contains(ID_A));
}

fn rollout(home: &Path, start: &str, folder: &str, last_role: &str) -> PathBuf {
    let path =
        home.join(format!(".codex/sessions/2026/10/16/rollout-2026-10-16T15-09-00-{ID_C}.jsonl"));
    let meta = format!(
        r#"{{"timestamp":"{start}","type":"session_meta","payload":{{"id":"{ID_C}","cwd":"{folder}","timestamp":"{start}"}}}}"#
    );
    let message = format!(
        r#"{{"timestamp":"2026-10-16T15:10:00.000Z","type":"response_item","payload":{{"type":"message","role":"{last_role}","content":[{{"type":"{}","text":"hello"}}]}}}}"#,
        if last_role == "user" { "input_text" } else { "output_text" }
    );
    write(&path, &format!("{meta}\n{message}\n"));
    path
}

const CODEX_PS: &str = "11 1 Fri Oct 16 15:09:00 2026 /opt/homebrew/bin/codex --sandbox read-only --ask-for-approval on-request\n";

#[test]
fn a_codex_process_is_found_by_folder_and_start_time_and_idle_after_a_finished_turn() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("acme");
    fs::create_dir_all(&folder).unwrap();
    let file =
        rollout(home.path(), "2026-10-16T15:09:01.000Z", folder.to_str().unwrap(), "assistant");
    age(&file, 60);
    let mut system = Fake::new(CODEX_PS);
    system.cwds.insert(11, folder.clone());
    let scan = scan_running(&system, &env(home.path()), &AgentTable::builtin()).unwrap();
    let session = &scan.sessions[0];
    assert_eq!(session.cli, Cli::Codex);
    assert_eq!(session.session_id.as_deref(), Some(ID_C));
    assert_eq!(session.cwd, folder);
    assert_eq!(session.flags.level, Some(Level::Ask));
    assert_eq!(session.running().unwrap().activity, Activity::Idle);
}

#[test]
fn a_codex_turn_that_is_not_known_to_be_finished_stays_quiet() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("acme");
    fs::create_dir_all(&folder).unwrap();
    let mut system = Fake::new(CODEX_PS);
    system.cwds.insert(11, folder.clone());
    let cases = [("assistant", 0), ("user", 60)];
    for (role, seconds) in cases {
        let file = rollout(home.path(), "2026-10-16T15:09:01.000Z", folder.to_str().unwrap(), role);
        age(&file, seconds);
        let scan = scan_running(&system, &env(home.path()), &AgentTable::builtin()).unwrap();
        assert_eq!(
            scan.sessions[0].running().unwrap().activity,
            Activity::Quiet,
            "{role} {seconds}"
        );
    }
}

#[test]
fn a_codex_rollout_from_before_the_process_started_is_not_its_session() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("acme");
    fs::create_dir_all(&folder).unwrap();
    rollout(home.path(), "2026-10-16T09:00:00.000Z", folder.to_str().unwrap(), "assistant");
    let mut system = Fake::new(CODEX_PS);
    system.cwds.insert(11, folder);
    let scan = scan_running(&system, &env(home.path()), &AgentTable::builtin()).unwrap();
    assert_eq!(scan.sessions[0].session_id, None);
}

#[test]
fn a_process_whose_folder_is_unknown_is_left_out() {
    let home = tempfile::tempdir().unwrap();
    let scan =
        scan_running(&Fake::new(CODEX_PS), &env(home.path()), &AgentTable::builtin()).unwrap();
    assert_eq!(scan.sessions, []);
}

#[test]
fn a_copilot_process_is_matched_to_its_session_by_the_lock_file() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join(".copilot/session-state");
    write(&state.join(format!("{ID_A}/inuse.77.lock")), "");
    write(&state.join(format!("{ID_B}/inuse.78.lock")), "");
    let mut system =
        Fake::new("77 1 Fri Oct 16 15:09:00 2026 /usr/local/bin/copilot --allow-all-tools\n");
    system.cwds.insert(77, PathBuf::from("/work/acme"));
    let scan = scan_running(&system, &env(home.path()), &AgentTable::builtin()).unwrap();
    assert_eq!(scan.sessions[0].session_id.as_deref(), Some(ID_A));
    assert_eq!(scan.sessions[0].flags.level, Some(Level::Auto));
}

#[test]
fn a_gemini_process_uses_the_latest_chat_of_its_folder() {
    let home = tempfile::tempdir().unwrap();
    let chats = home.path().join(".gemini/tmp/acme/chats");
    write(&chats.join("session-2026-10-16T10-00-aaaaaaaa.jsonl"), "{}\n");
    let newest = chats.join("session-2026-10-16T12-00-bbbbbbbb.jsonl");
    write(&newest, "{}\n");
    age(&chats.join("session-2026-10-16T10-00-aaaaaaaa.jsonl"), 3600);
    assert_eq!(newest_gemini_chat(home.path(), Path::new("/work/acme")), Some(newest));
    assert_eq!(newest_gemini_chat(home.path(), Path::new("/work/other")), None);
}

#[test]
fn recent_sessions_of_a_folder_come_newest_first_without_the_ones_held() {
    let home = tempfile::tempdir().unwrap();
    let folder = Path::new("/work/acme");
    let projects = home.path().join(".claude/projects").join(claude_project_dir_name(folder));
    let ids = [
        "aaaaaaaa-0000-4000-8000-000000000001",
        "aaaaaaaa-0000-4000-8000-000000000002",
        "aaaaaaaa-0000-4000-8000-000000000003",
        "aaaaaaaa-0000-4000-8000-000000000004",
        "aaaaaaaa-0000-4000-8000-000000000005",
    ];
    for (i, id) in ids.iter().enumerate() {
        let file = projects.join(format!("{id}.jsonl"));
        write(&file, "{}\n");
        age(&file, 3600 * (i as u64 + 1));
    }
    write(&projects.join("notes.txt"), "x");
    age(&projects.join(format!("{}.jsonl", ids[4])), 40 * 86_400);
    let held = Held { ids: HashSet::from([ids[0].to_owned()]), ..Held::default() };
    let got = recent_sessions(
        &env(home.path()),
        &[folder.to_path_buf()],
        &held,
        &TitleCache::default(),
        3,
    );
    let seen: Vec<_> = got.iter().map(|s| s.session_id.clone().unwrap()).collect();
    assert_eq!(seen, [ids[1], ids[2], ids[3]]);
    assert!(got.iter().all(|s| s.presence == Presence::NotRunning && s.cli == Cli::Claude));
    assert!(got[0].since_ms > got[1].since_ms);
}

fn claude_file(home: &Path, id: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(claude_project_dir_name(Path::new("/work/acme")))
        .join(format!("{id}.jsonl"))
}

fn mode_record(mode: &str) -> String {
    format!("{{\"type\":\"permission-mode\",\"permissionMode\":\"{mode}\"}}\n")
}

#[test]
fn an_ended_claude_session_reads_the_level_it_last_ran_at_from_its_transcript() {
    let cases = [
        (mode_record("bypassPermissions"), Some(Level::Yolo)),
        (format!("{}{}", mode_record("bypassPermissions"), mode_record("auto")), Some(Level::Auto)),
        (mode_record("default"), Some(Level::Ask)),
        ("{}\n".to_owned(), None),
    ];
    for (text, want) in cases {
        let home = tempfile::tempdir().unwrap();
        write(&claude_file(home.path(), ID_A), &text);
        let got = recent_sessions(
            &env(home.path()),
            &[PathBuf::from("/work/acme")],
            &Held::default(),
            &TitleCache::default(),
            1,
        );
        assert_eq!(got[0].flags.level, want, "{text}");
    }
}

#[test]
fn a_running_claude_session_takes_its_level_from_its_transcript_over_its_launch_flags() {
    let cases = [
        ("claude --permission-mode bypassPermissions", mode_record("auto"), Some(Level::Auto)),
        ("claude", mode_record("bypassPermissions"), Some(Level::Yolo)),
        ("claude --permission-mode acceptEdits", "{}\n".to_owned(), Some(Level::Edits)),
        ("claude", "{}\n".to_owned(), None),
    ];
    for (command, transcript, want) in cases {
        let home = tempfile::tempdir().unwrap();
        registry(home.path(), 19821, ID_A, START, "idle");
        write(&claude_file(home.path(), ID_A), &transcript);
        let ps = format!("19821 1 {START} {command}\n");
        let scan =
            scan_running(&Fake::new(&ps), &env(home.path()), &AgentTable::builtin()).unwrap();
        assert_eq!(scan.sessions[0].flags.level, want, "{command} {transcript}");
    }
}

#[test]
fn recent_codex_gemini_and_copilot_sessions_are_matched_to_the_folder() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("acme");
    fs::create_dir_all(&folder).unwrap();
    rollout(home.path(), "2026-10-16T15:09:01.000Z", folder.to_str().unwrap(), "assistant");
    let elsewhere = home.path().join(".codex/sessions/2026/10/16/rollout-2026-10-16T16-00-00-1c2cbc96-1111-4222-8333-444455556666.jsonl");
    write(
        &elsewhere,
        &format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{}/other\"}}}}\n",
            home.path().display()
        ),
    );
    let chat = home.path().join(".gemini/tmp/acme/chats/session-2026-10-16T12-00-bbbbbbbb.jsonl");
    write(&chat, &format!("{{\"sessionId\":\"{ID_B}\"}}\n"));
    let state = home.path().join(".copilot/session-state");
    write(
        &state.join(format!("{ID_A}/workspace.yaml")),
        &format!("id: {ID_A}\ncwd: {}\n", folder.display()),
    );
    write(
        &state.join("1c2cbc96-1111-4222-8333-444455556666/workspace.yaml"),
        "cwd: /somewhere/else\n",
    );
    let got =
        recent_sessions(&env(home.path()), &[folder], &Held::default(), &TitleCache::default(), 3);
    let mut seen: Vec<_> = got.iter().map(|s| (s.cli, s.session_id.clone().unwrap())).collect();
    seen.sort_by_key(|(cli, _)| Cli::ALL.iter().position(|each| each == cli));
    assert_eq!(
        seen,
        [
            (Cli::Codex, ID_C.to_owned()),
            (Cli::Gemini, ID_B.to_owned()),
            (Cli::Copilot, ID_A.to_owned()),
        ]
    );
}

fn recent(cli: Cli, id: &str, at: i64) -> Recent {
    Recent {
        cli,
        id: Some(id.to_owned()),
        at,
        file: Some(PathBuf::from(format!("/store/{cli:?}/{id}.jsonl"))),
    }
}

fn ids_of(list: &[Recent]) -> Vec<&str> {
    list.iter().filter_map(|recent| recent.id.as_deref()).collect()
}

type Case<'a> = (&'a str, &'a [&'a str], &'a [&'a str], Vec<&'a str>);

#[test]
fn the_recent_list_leaves_out_what_is_held_by_id_or_by_transcript() {
    let list = || {
        vec![
            recent(Cli::Claude, "a", 900),
            recent(Cli::Claude, "b", 800),
            recent(Cli::Codex, "c", 700),
            recent(Cli::Claude, "d", 600),
        ]
    };
    let file_b = "/store/Claude/b.jsonl";
    let table: [Case; 4] = [
        ("nothing held", &[], &[], vec!["a", "b", "c", "d"]),
        ("held by id", &["a"], &[], vec!["b", "c", "d"]),
        ("held by transcript", &[], &[file_b], vec!["a", "c", "d"]),
        ("held by both", &["a"], &[file_b], vec!["c", "d"]),
    ];
    for (name, ids, files, want) in table {
        let ids = ids.iter().map(|id| (*id).to_owned()).collect();
        let files = files.iter().map(PathBuf::from).collect();
        assert_eq!(ids_of(&newest_unheld(list(), &ids, &files, 0, 10)), want, "{name}");
    }
}

#[test]
fn the_recent_list_is_newest_first_across_clis_and_cut_to_the_count_after_leaving_out() {
    let list = vec![
        recent(Cli::Claude, "old", 100),
        recent(Cli::Gemini, "newest", 900),
        recent(Cli::Claude, "held", 800),
        recent(Cli::Codex, "next", 700),
        recent(Cli::Claude, "ancient", 5),
    ];
    let held = HashSet::from(["held".to_owned()]);
    let got = newest_unheld(list, &held, &HashSet::new(), 50, 2);
    assert_eq!(ids_of(&got), ["newest", "next"], "the held one does not use up a place");
}

#[test]
fn a_transcript_is_held_through_a_symlinked_path() {
    let home = tempfile::tempdir().unwrap();
    let real = home.path().join("real");
    fs::create_dir_all(&real).unwrap();
    let file = real.join(format!("{ID_A}.jsonl"));
    write(&file, "{}\n");
    let link = home.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut held = Held::default();
    held.add_transcript(&link.join(format!("{ID_A}.jsonl")));
    let list =
        vec![Recent { cli: Cli::Claude, id: Some("other".to_owned()), at: 5, file: Some(file) }];
    assert_eq!(newest_unheld(list, &HashSet::new(), &held.resolved(), 0, 10), []);
}

#[test]
fn recent_claude_sessions_carry_their_transcript_title() {
    let home = tempfile::tempdir().unwrap();
    let folder = Path::new("/work/acme");
    let projects = home.path().join(".claude/projects").join(claude_project_dir_name(folder));
    write(
        &projects.join(format!("{ID_A}.jsonl")),
        "{\"type\":\"ai-title\",\"aiTitle\":\"Fix login redirect\"}\n{\"type\":\"custom-title\",\"customTitle\":\"Login work\"}\n",
    );
    write(&projects.join(format!("{ID_B}.jsonl")), "{}\n");
    age(&projects.join(format!("{ID_B}.jsonl")), 3600);
    let got = recent_sessions(
        &env(home.path()),
        &[folder.to_path_buf()],
        &Held::default(),
        &TitleCache::default(),
        2,
    );
    let names: Vec<_> = got.iter().map(|s| s.name.as_deref()).collect();
    assert_eq!(names, [Some("Login work"), None]);
}

#[test]
fn a_title_is_read_again_only_after_its_transcript_changed() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("t.jsonl");
    write(&file, "{\"type\":\"ai-title\",\"aiTitle\":\"First\"}\n");
    let cache = TitleCache::default();
    assert_eq!(cache.title_of(&file).as_deref(), Some("First"));
    let stamp = fs::metadata(&file).unwrap();
    write(&file, "{\"type\":\"ai-title\",\"aiTitle\":\"Other\"}\n");
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(stamp.modified().unwrap())
        .unwrap();
    assert_eq!(cache.title_of(&file).as_deref(), Some("First"), "same size and time: not read");
    write(&file, "{\"type\":\"custom-title\",\"customTitle\":\"Renamed\"}\n");
    assert_eq!(cache.title_of(&file).as_deref(), Some("Renamed"), "changed: read again");
    assert_eq!(cache.clone().title_of(&home.path().join("gone.jsonl")), None);
}

fn request() -> EndRequest {
    EndRequest {
        cli: Cli::Claude,
        pid: 19821,
        proc_start: START.to_owned(),
        session_id: Some(ID_A.to_owned()),
        allow_quiet: false,
    }
}

fn end(home: &Path, system: &Fake, request: &EndRequest) -> Ended {
    end_session(system, &env(home), &AgentTable::builtin(), request, Duration::from_millis(400))
}

#[test]
fn an_idle_session_is_sent_sigterm_and_waited_for() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 25746, ID_A, START, "idle");
    let system =
        Fake::new("25746 1 Fri Oct 16 13:23:31 2026 claude --permission-mode acceptEdits\n");
    system.polls_alive_after_signal.set(2);
    let request = EndRequest { pid: 25746, ..request() };
    assert_eq!(end(home.path(), &system, &request), Ended::Exited);
    assert_eq!(*system.sent.borrow(), [25746]);
}

#[test]
fn a_process_that_does_not_exit_in_time_is_left_running_and_never_killed_harder() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 25746, ID_A, START, "idle");
    let system = Fake::new("25746 1 Fri Oct 16 13:23:31 2026 claude\n");
    system.polls_alive_after_signal.set(i32::MAX);
    let request = EndRequest { pid: 25746, ..request() };
    assert_eq!(end(home.path(), &system, &request), Ended::StillRunning);
    assert_eq!(*system.sent.borrow(), [25746]);
}

#[test]
fn nothing_is_signalled_when_the_scan_does_not_find_the_same_session() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 25746, ID_A, START, "idle");
    let ps = "25746 1 Fri Oct 16 13:23:31 2026 claude\n";
    let base = EndRequest { pid: 25746, ..request() };
    let cases = [
        (
            "another start time",
            EndRequest { proc_start: "Mon Oct 12 18:01:24 2026".into(), ..base.clone() },
        ),
        ("another session", EndRequest { session_id: Some(ID_B.into()), ..base.clone() }),
        ("another pid", EndRequest { pid: 999, ..base.clone() }),
        ("another cli", EndRequest { cli: Cli::Codex, ..base.clone() }),
    ];
    for (why, request) in cases {
        let system = Fake::new(ps);
        assert_eq!(end(home.path(), &system, &request), Ended::Refused(Refusal::Gone), "{why}");
        assert_eq!(*system.sent.borrow(), Vec::<u32>::new(), "{why}");
    }
}

#[test]
fn a_session_that_started_working_again_is_not_interrupted() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 25746, ID_A, START, "busy");
    let system = Fake::new("25746 1 Fri Oct 16 13:23:31 2026 claude\n");
    let request = EndRequest { pid: 25746, ..request() };
    assert_eq!(end(home.path(), &system, &request), Ended::Refused(Refusal::Working));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn a_session_in_stdusk_is_never_signalled() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 19821, ID_A, START, "idle");
    let system = Fake::new(PS);
    assert_eq!(end(home.path(), &system, &request()), Ended::Refused(Refusal::Stdusk));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
}

#[test]
fn a_session_of_unknown_status_needs_the_person_to_have_agreed() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("acme");
    fs::create_dir_all(&folder).unwrap();
    let mut system = Fake::new(CODEX_PS);
    system.cwds.insert(11, folder);
    let base = EndRequest {
        cli: Cli::Codex,
        pid: 11,
        proc_start: "Fri Oct 16 15:09:00 2026".into(),
        session_id: None,
        allow_quiet: false,
    };
    assert_eq!(end(home.path(), &system, &base), Ended::Refused(Refusal::NotConfirmed));
    assert_eq!(*system.sent.borrow(), Vec::<u32>::new());
    system.polls_alive_after_signal.set(0);
    let agreed = EndRequest { allow_quiet: true, ..base };
    assert_eq!(end(home.path(), &system, &agreed), Ended::Exited);
    assert_eq!(*system.sent.borrow(), [11]);
}

#[test]
fn a_signal_that_cannot_be_sent_is_reported() {
    let home = tempfile::tempdir().unwrap();
    registry(home.path(), 25746, ID_A, START, "idle");
    let mut system = Fake::new("25746 1 Fri Oct 16 13:23:31 2026 claude\n");
    system.can_signal = false;
    let request = EndRequest { pid: 25746, ..request() };
    assert_eq!(end(home.path(), &system, &request), Ended::Refused(Refusal::SignalFailed));
}

#[test]
fn lsof_output_names_the_folder() {
    assert_eq!(
        parse_lsof_cwd("p4242\nfcwd\nn/Users/me/Git/acme\n"),
        Some(PathBuf::from("/Users/me/Git/acme"))
    );
    assert_eq!(parse_lsof_cwd("p4242\n"), None);
}

#[test]
fn the_real_ps_lists_this_test_with_a_start_time_that_matches_the_alive_check() {
    let system = RealSystem::new();
    let table = ProcessTable::new(parse_ps(&system.ps().expect("ps runs")));
    let me = table.get(std::process::id()).expect("this process is in the table");
    assert_eq!(system.alive(std::process::id()).as_deref(), Some(me.started.as_str()));
    let here = std::env::current_dir().unwrap().canonicalize().unwrap();
    let cwd = system.cwd_of(std::process::id()).map(|dir| dir.canonicalize().unwrap());
    assert_eq!(cwd, Some(here));
}

#[test]
fn a_child_the_test_started_is_ended_by_sigterm_through_the_real_system() {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let system = RealSystem::new();
    let start = system.alive(pid).expect("the child runs");
    registry(home.path(), pid, ID_A, &start, "idle");
    let request = EndRequest { pid, proc_start: start, ..request() };
    let reaper = std::thread::spawn(move || child.wait().unwrap());
    let outcome = end_session(
        &system,
        &env(home.path()),
        &AgentTable::builtin(),
        &request,
        Duration::from_secs(5),
    );
    assert_eq!(outcome, Ended::Exited);
    let status = reaper.join().unwrap();
    assert!(!status.success());
    assert_eq!(system.alive(pid), None);
}

#[test]
fn lsof_output_gives_every_process_whose_cwd_is_in_the_folder_once() {
    let out = "p100\nn/work/web\np200\nn/work/web/src\np300\nn/work/webby\np400\nn/\np500\nn/private/work/web\n";
    let forms = [PathBuf::from("/work/web"), PathBuf::from("/private/work/web")];
    assert_eq!(parse_lsof_holders(out, &forms), [100, 200, 500]);
    assert_eq!(parse_lsof_holders("", &forms), Vec::<u32>::new());
    assert_eq!(parse_lsof_holders("n/work/web\n", &forms), Vec::<u32>::new());
}
