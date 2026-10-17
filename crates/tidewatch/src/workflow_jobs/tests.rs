use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Instant;

use tidewatch_core::agents::{AgentOverride, Level};
use tidewatch_core::events::TabEvent;
use tidewatch_core::profiles::{Profile, Profiles};

use super::*;

fn git(dir: &std::path::Path, args: &[&str]) {
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

fn throwaway_repo(home: &std::path::Path) -> PathBuf {
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.join(".tidewatch.toml"),
        "test = \"sh check.sh\"\ntest_timeout_secs = 90\n",
    )
    .unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    repo
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

fn open_spec(
    paths: &TidewatchPaths,
    repo: &std::path::Path,
    existing: Option<Worktree>,
) -> OpenSpec {
    OpenSpec {
        paths: paths.clone(),
        repo: repo.to_path_buf(),
        tab: 3,
        tools: Tools::with_path(std::env::var_os("PATH").unwrap_or_default()),
        existing,
        placement: Placement::default(),
    }
}

#[test]
fn opening_makes_the_worktree_and_finds_the_base_the_test_command_and_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = throwaway_repo(&home);
    let paths = TidewatchPaths::new(&home);
    let jobs = Jobs::new();
    open(&jobs, &egui::Context::default(), open_spec(&paths, &repo, None));
    let Done::Opened { tab, result: Ok(opening) } = next(&jobs) else { panic!("expected Opened") };
    assert_eq!(tab, 3);
    assert!(opening.worktree.path.is_dir());
    assert!(opening.worktree.branch.starts_with("agent/"));
    assert_eq!(opening.base, "main");
    assert_eq!(opening.test_command.as_deref(), Some("sh check.sh"));
    assert_eq!(opening.test_timeout, Duration::from_secs(90));
}

#[test]
fn opening_an_existing_worktree_makes_no_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = throwaway_repo(&home);
    let paths = TidewatchPaths::new(&home);
    let existing = worktree::add(&paths, &repo).unwrap();
    let jobs = Jobs::new();
    open(&jobs, &egui::Context::default(), open_spec(&paths, &repo, Some(existing.clone())));
    let Done::Opened { result: Ok(opening), .. } = next(&jobs) else { panic!("expected Opened") };
    assert_eq!(opening.worktree, existing);
    let listed = worktree::list(&repo).unwrap();
    assert_eq!(listed.len(), 2, "the repo and the one worktree");
}

#[test]
fn a_repo_that_cannot_make_a_worktree_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let paths = TidewatchPaths::new(&home);
    let jobs = Jobs::new();
    open(&jobs, &egui::Context::default(), open_spec(&paths, &home.join("not-a-repo"), None));
    let Done::Opened { result, .. } = next(&jobs) else { panic!("expected Opened") };
    assert!(result.is_err());
}

fn reviewer() -> (Agent, Profile) {
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    let agent = profiles.agent_table().get("claude").unwrap().clone();
    let profile = Profile { level: Level::Plan, ..profiles.profile["builder"].clone() };
    (agent, profile)
}

fn launch(generation: u64) -> StageLaunch {
    let (_, profile) = reviewer();
    StageLaunch {
        index: 1,
        attempt: 1,
        generation,
        profile,
        prompt: "Review main..HEAD\nand report".into(),
        divider: "--- Stage 2: Review (claude, Read only) ---".into(),
    }
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

fn stage_done(summary: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"stage_done","arguments":{{"summary":"{summary}"}}}}}}"#
    )
}

#[test]
fn a_stage_gets_a_fresh_token_with_its_generation_a_prompt_and_a_divider() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = throwaway_repo(&home);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add(&paths, &repo).unwrap();
    let ctx = egui::Context::default();
    let (listener, inbox) = Listener::start(paths.clone(), Box::new(|| {})).unwrap();
    let listener = Arc::new(listener);
    let jobs = Jobs::new();
    let spec = |generation| StageSpec {
        paths: paths.clone(),
        tab: 7,
        launch: launch(generation),
        agent: reviewer().0,
        worktree: worktree.clone(),
        path_var: OsString::from("/usr/bin:/bin"),
        listener: Some(Arc::clone(&listener)),
        trust: None,
    };

    launch_stage(&jobs, &ctx, spec(1));
    let Done::StageReady { tab, generation, divider, result: Ok(first) } = next(&jobs) else {
        panic!("expected StageReady")
    };
    assert_eq!((tab, generation), (7, 1));
    assert_eq!(divider, "--- Stage 2: Review (claude, Read only) ---");
    assert!(first.session_id.is_some());
    assert_eq!(first.profile_name, "Builder");
    assert_eq!(first.notes_file, Some(paths.notes_file(&repo, worktree.name())));
    let script = first.prepared.script("/bin/zsh");
    assert!(
        script.starts_with("printf '\\n%s\\n' '--- Stage 2: Review (claude, Read only) ---' && "),
        "{script}"
    );
    assert!(first.prepared.launch.shell_line().ends_with("-- 'Review main..HEAD\nand report'"));
    let mcp_file = paths.tab_state_dir(&repo, worktree.name()).join("mcp.json");
    let token_of = |file: &std::path::Path| {
        let text = std::fs::read_to_string(file).unwrap();
        let (_, after) = text.split_once("Bearer ").expect("the MCP config carries the token");
        after.split('"').next().unwrap().to_owned()
    };
    let first_token = token_of(&mcp_file);

    launch_stage(&jobs, &ctx, spec(2));
    let Done::StageReady { generation, result: Ok(_), .. } = next(&jobs) else {
        panic!("expected StageReady")
    };
    assert_eq!(generation, 2);
    let second_token = token_of(&mcp_file);
    assert_ne!(first_token, second_token);
    assert!(post(&listener.base_url(), &first_token, &stage_done("late")).contains("401"));
    assert!(post(&listener.base_url(), &second_token, &stage_done("now")).contains("200"));
    let message = inbox.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((message.tab_id, message.generation), (7, 2));
    assert_eq!(message.event, TabEvent::StageDone { summary: "now".into() });
}

#[test]
fn a_stage_that_cannot_be_prepared_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = throwaway_repo(&home);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add(&paths, &repo).unwrap();
    let mut profiles = Profiles::seeded(&["claude".to_owned()]);
    profiles.agent.insert(
        "mine".into(),
        AgentOverride { bin: Some("mine".into()), ..AgentOverride::default() },
    );
    let agent = profiles.agent_table().get("mine").unwrap().clone();
    let launch = launch(1);
    let spec = StageSpec {
        paths,
        tab: 1,
        launch,
        agent,
        worktree,
        path_var: OsString::from("/usr/bin"),
        listener: None,
        trust: None,
    };
    let jobs = Jobs::new();
    launch_stage(&jobs, &egui::Context::default(), spec);
    let Done::StageReady { result, .. } = next(&jobs) else { panic!("expected StageReady") };
    assert!(result.is_err());
}

#[test]
fn the_test_job_reports_the_exit_code_and_stops_a_hung_command() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = Jobs::new();
    let ctx = egui::Context::default();
    run_tests(&jobs, &ctx, 2, dir.path().to_path_buf(), "echo ok".into(), Duration::from_secs(30));
    let Done::Tested { tab, run } = next(&jobs) else { panic!("expected Tested") };
    assert_eq!((tab, run.exit_code, run.output.as_str()), (2, 0, "ok\n"));
    run_tests(
        &jobs,
        &ctx,
        2,
        dir.path().to_path_buf(),
        "sleep 30".into(),
        Duration::from_millis(200),
    );
    let Done::Tested { run, .. } = next(&jobs) else { panic!("expected Tested") };
    assert_eq!(run.exit_code, -1);
    assert!(run.output.contains("was stopped"), "{}", run.output);
}
