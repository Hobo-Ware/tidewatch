//! Live end-to-end check of the listener against a real agent CLI, run by hand:
//!
//! `cargo run -p tidewatch-core --example live_check -- <claude|gemini|copilot|codex> <git repo> [prompt]`
//!
//! Starts the listener, registers one tab, builds the launch with `launch_command` (hooks and MCP
//! wired, never YOLO), runs the CLI headless with a 120 second limit and prints every event that
//! arrives. `LIVE_LEVEL=plan|ask|edits|auto` picks the permission level (default edits, never YOLO).
//! Codex runs as `codex exec`, which has no `--ask-for-approval`, so that pair is dropped; point
//! `CODEX_HOME` at a throwaway folder. Use a throwaway repo: the agent may write files in it.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use tidewatch_core::agents::{AgentTable, ArgList, Level, Wiring};
use tidewatch_core::events::TabEvent;
use tidewatch_core::launch::{
    GeminiSettingsFiles, LaunchCtx, copilot_plugin_files, ensure_no_push_helper,
    gemini_system_settings, launch_command, mcp_config_json,
};
use tidewatch_core::listener::{Listener, TabRegistration};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::profiles::Profile;

const LIMIT: Duration = Duration::from_secs(120);
const DEFAULT_PROMPT: &str = "call the tidewatch stage_done tool with summary hello";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [agent_id, repo, rest @ ..] = args.as_slice() else {
        eprintln!("usage: live_check <claude|gemini|copilot|codex> <git repo> [prompt]");
        std::process::exit(2);
    };
    let prompt = rest.first().map_or(DEFAULT_PROMPT, String::as_str);
    let repo = PathBuf::from(repo);
    let home = tempfile::tempdir().expect("a temp home");

    let (listener, events) =
        Listener::start(TidewatchPaths::new(home.path()), Box::new(|| {})).expect("loopback bind");
    let hook = listener.register_tab(TabRegistration {
        tab_id: 1,
        agent: agent_id.clone(),
        repo: repo.clone(),
        tab: "live".into(),
        generation: 0,
    });
    println!("listener {} token {}...", hook.base_url, &hook.token[..6]);

    let table = AgentTable::builtin();
    let agent = table.get(agent_id).expect("a built-in agent id");
    let profile = Profile {
        name: "live".into(),
        agent: agent_id.clone(),
        model: (agent_id == "claude").then(|| "haiku".to_owned()),
        effort: None,
        level: level_from_env(),
        setup: String::new(),
        never_push: true,
        extra_args: ArgList::default(),
        house_style: false,
    };
    let mcp_file = home.path().join("mcp.json");
    let mcp_json = mcp_config_json(agent.wiring, &hook);
    if let Some(json) = &mcp_json {
        std::fs::write(&mcp_file, json).expect("write the MCP config");
    }
    let guard_dir = home.path().join("guard");
    std::fs::create_dir_all(&guard_dir).expect("create the guard dir");
    ensure_no_push_helper(&guard_dir).expect("write the no-push helper");
    let mut ctx = LaunchCtx {
        worktree: repo.clone(),
        guard_bin_dir: Some(guard_dir),
        worktree_tab: true,
        hooks: Some(hook.clone()),
        mcp_config: Some(mcp_file),
        env: std::env::vars().collect(),
        ..LaunchCtx::default()
    };
    if agent.wiring == Wiring::Gemini {
        let settings = gemini_system_settings(&profile, &ctx, None).expect("gemini settings");
        let file = home.path().join("gemini-settings.json");
        std::fs::write(&file, settings).expect("write settings");
        ctx.gemini_settings = Some(GeminiSettingsFiles {
            settings: file,
            defaults: home.path().join("system-defaults.json"),
        });
    }
    if agent.wiring == Wiring::Copilot {
        let plugin = home.path().join("copilot-plugin");
        for (name, text) in copilot_plugin_files(&hook) {
            let file = plugin.join(name);
            std::fs::create_dir_all(file.parent().expect("a parent")).expect("create the plugin");
            std::fs::write(file, text).expect("write the plugin");
        }
        ctx.copilot_plugin_dir = Some(plugin);
    }

    let launch = launch_command(agent, &profile, &ctx).expect("a launch");
    let mut args = if agent.wiring == Wiring::Codex {
        codex_exec_args(&launch.args, prompt)
    } else {
        let mut args = launch.args.clone();
        args.extend(["-p".into(), prompt.into()]);
        args
    };
    if agent.wiring == Wiring::Copilot {
        args.extend(["--no-auto-update".into(), "-s".into()]);
    }
    println!(
        "running: {} {}",
        launch.program,
        launch.args.join(" ").replace(&hook.token, "<token>")
    );
    let mut child = Command::new(&launch.program)
        .args(&args)
        .envs(launch.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the agent CLI");

    let started = Instant::now();
    let mut seen = Vec::new();
    let exit = loop {
        drain(&events, &mut seen, started);
        match child.try_wait().expect("poll the child") {
            Some(status) => break status.code(),
            None if started.elapsed() > LIMIT => {
                let _ = child.kill();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    std::thread::sleep(Duration::from_millis(300));
    drain(&events, &mut seen, started);
    let output = child.wait_with_output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    println!(
        "exit {exit:?}, {} events, agent said: {:?}",
        seen.len(),
        output.unwrap_or_default().trim()
    );

    let hooks = seen.iter().filter(|e| matches!(e, TabEvent::Hook(_))).count();
    let stage_done =
        seen.iter().any(|e| matches!(e, TabEvent::StageDone { summary } if summary == "hello"));
    println!("RESULT hook events: {hooks}, stage_done hello: {stage_done}");
    let _ = std::io::stdout().flush();
}

fn codex_exec_args(launch_args: &[String], prompt: &str) -> Vec<String> {
    let mut args = vec!["exec".to_owned()];
    let mut rest = launch_args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--ask-for-approval" {
            rest.next();
        } else {
            args.push(arg.clone());
        }
    }
    args.extend(["--".to_owned(), prompt.to_owned()]);
    args
}

fn level_from_env() -> Level {
    match std::env::var("LIVE_LEVEL").as_deref() {
        Ok("plan") => Level::Plan,
        Ok("ask") => Level::Ask,
        Ok("auto") => Level::Auto,
        _ => Level::Edits,
    }
}

fn drain(
    events: &std::sync::mpsc::Receiver<tidewatch_core::events::TabMessage>,
    seen: &mut Vec<TabEvent>,
    started: Instant,
) {
    loop {
        match events.recv_timeout(Duration::from_millis(10)) {
            Ok(message) => {
                println!(
                    "[{:>5.1}s] tab {} {:?}",
                    started.elapsed().as_secs_f32(),
                    message.tab_id,
                    message.event
                );
                seen.push(message.event);
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
        }
    }
}
