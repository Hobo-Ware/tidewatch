use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::agents::Level;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::restore::{Reason, TabAction};
use tidewatch_core::resume_mode::{Resume, Why};
use tidewatch_core::session::{SavedAgentTab, SavedTab, SavedWindow};
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree::{BranchChoice, RemoveMode};

use super::*;

const SESSION_ID: &str = "0c2cbc96-1111-4222-8333-444455556666";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    worktree: Worktree,
    path_var: OsString,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("work").join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add(&paths, &repo).unwrap();
    let transcripts =
        home.join(".claude").join("projects").join(claude_project_dir_name(&worktree.path));
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join(format!("{SESSION_ID}.jsonl")), "{}\n").unwrap();
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fixture { _dir: dir, home, repo, paths, worktree, path_var: bin.into_os_string() }
}

fn profiles() -> Profiles {
    Profiles::seeded(&["claude".to_owned()])
}

fn agent_tab(worktree: &Worktree, level: Level) -> SavedTab {
    SavedTab::Agent(SavedAgentTab {
        dir: worktree.path.clone(),
        branch: Some(worktree.branch.clone()),
        agent: "claude".into(),
        profile: "builder".into(),
        level,
        session_id: Some(SESSION_ID.into()),
        stages: None,
        crashed: false,
    })
}

fn shell_tab(worktree: &Worktree) -> SavedTab {
    SavedTab::Shell { dir: worktree.path.clone(), branch: Some(worktree.branch.clone()) }
}

fn save(f: &Fixture, windows: Vec<SavedWindow>) {
    Session { windows }.save(&f.paths).unwrap();
}

fn window(dir: &Path, tabs: Vec<SavedTab>) -> SavedWindow {
    SavedWindow { dir: dir.to_path_buf(), other: false, active: 1, focus_mode: true, tabs }
}

fn spec(f: &Fixture, skip: bool) -> StartupSpec {
    StartupSpec {
        paths: f.paths.clone(),
        group: 0,
        dir: f.repo.clone(),
        other: false,
        skip,
        app_crashed: false,
        profiles: profiles(),
        path_var: f.path_var.clone(),
        disk: RealDisk::new(&f.home),
    }
}

fn next(jobs: &Jobs<Done>) -> Done {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(done) = jobs.drain().into_iter().next() {
            return done;
        }
        assert!(Instant::now() < deadline, "the job never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn run(spec: &StartupSpec) -> Startup {
    let jobs = Jobs::new();
    startup(&jobs, &egui::Context::default(), clone_spec(spec));
    let Done::Planned(startup) = next(&jobs) else { panic!("expected Planned") };
    *startup
}

fn clone_spec(spec: &StartupSpec) -> StartupSpec {
    StartupSpec {
        paths: spec.paths.clone(),
        group: spec.group,
        dir: spec.dir.clone(),
        other: spec.other,
        skip: spec.skip,
        app_crashed: spec.app_crashed,
        profiles: spec.profiles.clone(),
        path_var: spec.path_var.clone(),
        disk: spec.disk.clone(),
    }
}

#[test]
fn the_saved_window_is_planned_against_the_disk_and_its_worktrees_are_not_leftovers() {
    let f = fixture();
    save(&f, vec![window(&f.repo, vec![SavedTab::Main, agent_tab(&f.worktree, Level::Edits)])]);
    let started = run(&spec(&f, false));
    let plan = started.plan.expect("a plan");
    assert_eq!(plan.tabs.len(), 2);
    assert!(
        matches!(&plan.tabs[1].action, TabAction::ResumeAgent { session_id, .. } if session_id == SESSION_ID),
        "{:?}",
        plan.tabs[1].action
    );
    assert!(plan.focus_mode);
    assert_eq!(started.leftovers, []);
    assert_eq!(started.prefs, Prefs::default());
}

#[test]
fn the_resume_setting_and_a_crashed_last_run_decide_how_a_saved_agent_comes_back() {
    let cases = [
        (ResumeAgents::Auto, false, Ok(Resume::Run)),
        (ResumeAgents::Auto, true, Ok(Resume::TypeOnly(Why::AppCrashed))),
        (ResumeAgents::Prefill, false, Ok(Resume::TypeOnly(Why::Chosen))),
        (ResumeAgents::Off, false, Err(Reason::ResumeOff)),
    ];
    for (mode, app_crashed, want) in cases {
        let f = fixture();
        save(&f, vec![window(&f.repo, vec![SavedTab::Main, agent_tab(&f.worktree, Level::Edits)])]);
        Prefs { resume_agents: mode, ..Prefs::default() }.save(&f.paths).unwrap();
        let mut s = spec(&f, false);
        s.app_crashed = app_crashed;
        let plan = run(&s).plan.expect("a plan");
        let got = match &plan.tabs[1].action {
            TabAction::ResumeAgent { how, .. } => Ok(*how),
            TabAction::ShellInstead(reason) => Err(reason.clone()),
            other => panic!("{other:?}"),
        };
        assert_eq!(got, want, "{mode:?} {app_crashed}");
    }
}

#[test]
fn a_worktree_no_saved_or_open_tab_owns_is_still_a_leftover() {
    let f = fixture();
    let stray = worktree::add(&f.paths, &f.repo).unwrap();
    save(&f, vec![window(&f.repo, vec![SavedTab::Main, shell_tab(&f.worktree)])]);
    let started = run(&spec(&f, false));
    let found: Vec<&Path> = started.leftovers.iter().map(|l| l.path.as_path()).collect();
    assert_eq!(found, [stray.path.as_path()]);
    assert_eq!(started.leftovers[0].worktree(), Some(stray));
}

#[test]
fn another_windows_saved_tabs_own_their_worktrees_too() {
    let f = fixture();
    let other_repo = f.home.join("work").join("api");
    save(
        &f,
        vec![
            window(&f.repo, vec![SavedTab::Main]),
            window(&other_repo, vec![SavedTab::Main, shell_tab(&f.worktree)]),
        ],
    );
    assert_eq!(run(&spec(&f, false)).leftovers, []);
}

#[test]
fn without_restoring_the_saved_tabs_own_nothing_and_there_is_no_plan() {
    let f = fixture();
    save(&f, vec![window(&f.repo, vec![SavedTab::Main, shell_tab(&f.worktree)])]);
    let started = run(&spec(&f, true));
    assert!(started.plan.is_none());
    assert_eq!(started.leftovers.len(), 1, "the flag leaves the old tab's worktree to clean up");
}

#[test]
fn the_settings_switch_turns_restoring_off_the_same_way() {
    let f = fixture();
    save(&f, vec![window(&f.repo, vec![SavedTab::Main, shell_tab(&f.worktree)])]);
    Prefs { restore_session: false, ..Prefs::default() }.save(&f.paths).unwrap();
    let started = run(&spec(&f, false));
    assert!(started.plan.is_none());
    assert!(!started.prefs.restore_session);
    assert_eq!(started.leftovers.len(), 1);
}

#[test]
fn a_session_with_nothing_for_this_window_has_no_plan() {
    let f = fixture();
    assert!(run(&spec(&f, false)).plan.is_none());
    save(&f, vec![window(&f.home.join("work").join("api"), vec![SavedTab::Main])]);
    assert!(run(&spec(&f, false)).plan.is_none());
}

#[test]
fn a_removed_worktree_is_skipped_and_an_uninstalled_agent_falls_back_to_a_shell() {
    let f = fixture();
    let gone = worktree::add(&f.paths, &f.repo).unwrap();
    worktree::remove(&f.paths, &gone, BranchChoice::Delete, RemoveMode::Safe).unwrap();
    save(
        &f,
        vec![window(
            &f.repo,
            vec![SavedTab::Main, shell_tab(&gone), agent_tab(&f.worktree, Level::Edits)],
        )],
    );
    let mut s = spec(&f, false);
    s.path_var = OsString::from("/nonexistent");
    let plan = run(&s).plan.unwrap();
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].reason, Reason::WorktreeGone);
    assert_eq!(plan.tabs[1].action, TabAction::ShellInstead(Reason::AgentMissing("claude".into())));
}

#[test]
fn a_moved_repo_is_repaired_and_its_window_is_saved_under_the_new_folder() {
    let f = fixture();
    let old = f.repo.clone();
    save(&f, vec![window(&old, vec![SavedTab::Main, agent_tab(&f.worktree, Level::Edits)])]);
    let moved = f.home.join("elsewhere").join("web");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&old, &moved).unwrap();
    let mut s = spec(&f, false);
    s.dir = moved.clone();
    let started = run(&s);
    assert!(started.repaired >= 1);
    let plan = started.plan.expect("the moved window is planned");
    assert!(matches!(plan.tabs[1].action, TabAction::ResumeAgent { .. }));
    let saved = Session::load(&f.paths);
    assert!(saved.window(&moved).is_some());
    assert!(saved.window(&old).is_none(), "the old entry does not linger");
}

#[test]
fn the_other_group_has_no_leftover_scan() {
    let f = fixture();
    let mut s = spec(&f, false);
    s.dir = f.home.join("work");
    s.other = true;
    assert_eq!(run(&s).leftovers, []);
}

fn reopen_spec(f: &Fixture, level: Level, listener: Option<Arc<Listener>>) -> ReopenSpec {
    let profiles = profiles();
    let agent = profiles.agent_table().get("claude").unwrap().clone();
    ReopenSpec {
        paths: f.paths.clone(),
        tab: 9,
        repo: f.repo.clone(),
        worktree: Some(f.worktree.clone()),
        dir: f.worktree.path.clone(),
        agent,
        profile_id: "builder".into(),
        profile: Profile { level, ..profiles.profile["builder"].clone() },
        session_id: SESSION_ID.into(),
        typed: None,
        path_var: OsString::from("/usr/bin:/bin"),
        listener,
        trust: None,
    }
}

fn reopened(spec: ReopenSpec) -> Reopened {
    let jobs = Jobs::new();
    reopen(&jobs, &egui::Context::default(), spec);
    let Done::Reopened(done) = next(&jobs) else { panic!("expected Reopened") };
    *done
}

fn post(base: &str, token: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(base.trim_start_matches("http://")).unwrap();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: tidewatch\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn a_resumed_agent_gets_the_resume_command_the_same_wiring_and_its_old_session_id() {
    let f = fixture();
    let done = reopened(reopen_spec(&f, Level::Edits, None));
    assert_eq!(done.tab, 9);
    assert_eq!(done.note, None);
    let opened = done.result.ok().expect("opened");
    assert_eq!(opened.worktree.as_ref(), Some(&f.worktree));
    assert_eq!(opened.dir, f.worktree.path);
    let started = opened.started.expect("an agent to start");
    assert_eq!(started.session_id.as_deref(), Some(SESSION_ID));
    assert_eq!((started.profile_id.as_str(), started.level), ("builder", Level::Edits));
    assert_eq!(started.notes_file, Some(f.paths.notes_file(&f.repo, f.worktree.name())));
    let line = started.prepared.launch.shell_line();
    assert!(line.starts_with(&format!("claude --resume {SESSION_ID} ")), "{line}");
    assert!(line.contains("--permission-mode acceptEdits"), "{line}");
    assert!(!line.contains("--session-id"), "{line}");
    let env = &started.prepared.env;
    assert!(env.iter().any(|(k, _)| k == "GIT_CONFIG_COUNT"), "the push guard is still on");
}

#[test]
fn a_resumed_tab_registers_with_the_listener_and_its_token_works() {
    let f = fixture();
    let (listener, inbox) = Listener::start(f.paths.clone(), Box::new(|| {})).unwrap();
    let listener = Arc::new(listener);
    let done = reopened(reopen_spec(&f, Level::Edits, Some(Arc::clone(&listener))));
    assert!(done.result.ok().and_then(|o| o.started).is_some());
    let mcp = f.paths.tab_state_dir(&f.repo, f.worktree.name()).join("mcp.json");
    let text = std::fs::read_to_string(mcp).expect("the MCP config is written for the resume");
    let (_, after) = text.split_once("Bearer ").expect("it carries the token");
    let token = after.split('"').next().unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"stage_done","arguments":{"summary":"ok"}}}"#;
    assert!(post(&listener.base_url(), token, body).contains("200"));
    let message = inbox.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((message.tab_id, message.generation), (9, 0));
}

#[test]
fn with_consent_the_resumed_worktree_gets_its_folder_trust_entry_again() {
    use tidewatch_core::trust::{TrustCli, is_trusted};
    use tidewatch_core::trust_apply::ConfigFiles;
    let f = fixture();
    let claude_json = f.home.join(".claude.json");
    std::fs::write(&claude_json, "{\n  \"numStartups\": 1\n}\n").unwrap();
    let grant = Grant::new(&f.paths, ConfigFiles::resolve(&f.home, |_| None));
    let mut s = reopen_spec(&f, Level::Edits, None);
    s.trust = Some(grant);
    assert!(reopened(s).result.ok().and_then(|o| o.started).is_some());
    let folder = f.worktree.path.canonicalize().unwrap().to_string_lossy().into_owned();
    let text = std::fs::read_to_string(&claude_json).unwrap();
    assert!(is_trusted(TrustCli::Claude, &text, &folder).unwrap(), "{text}");
}

#[test]
fn without_consent_a_resume_touches_no_cli_file() {
    let f = fixture();
    assert!(reopened(reopen_spec(&f, Level::Edits, None)).result.ok().is_some());
    assert!(!f.home.join(".claude.json").exists());
}

#[test]
fn an_agent_that_cannot_be_prepared_comes_back_as_a_shell_with_the_reason() {
    let f = fixture();
    let (listener, _inbox) = Listener::start(f.paths.clone(), Box::new(|| {})).unwrap();
    let done = reopened(reopen_spec(&f, Level::Yolo, Some(Arc::new(listener))));
    let note = done.note.expect("a note");
    assert!(note.starts_with("The agent was not resumed: "), "{note}");
    let opened = done.result.ok().expect("the worktree still opens");
    assert!(opened.started.is_none());
    assert_eq!(opened.worktree.as_ref(), Some(&f.worktree));
}

#[test]
fn an_agent_in_a_plain_folder_resumes_without_a_worktree_and_without_the_guard() {
    let f = fixture();
    let folder = f.home.join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    let mut s = reopen_spec(&f, Level::Edits, None);
    s.worktree = None;
    s.dir = folder.clone();
    s.repo = folder.clone();
    let opened = reopened(s).result.ok().expect("opened");
    assert_eq!(opened.worktree, None);
    let started = opened.started.expect("an agent");
    assert!(!started.prepared.env.iter().any(|(k, _)| k == "GIT_CONFIG_COUNT"));
    assert!(started.prepared.launch.shell_line().starts_with("claude --resume "));
}

#[test]
fn a_reopened_agent_remembers_whether_its_command_is_typed_or_run() {
    let f = fixture();
    for typed in [None, Some(Why::AppCrashed)] {
        let mut spec = reopen_spec(&f, Level::Edits, None);
        spec.typed = typed;
        let done = reopened(spec);
        assert_eq!(done.typed, typed);
        let Ok(opened) = done.result else { panic!("opened") };
        let started = opened.started.expect("the agent was prepared");
        assert!(started.prepared.launch.shell_line().contains(SESSION_ID));
    }
}
