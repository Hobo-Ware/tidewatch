//! The trailers guard's PreToolUse hook end to end: the real `tidewatch --guard trailers` binary,
//! run the way Claude Code runs a command hook (the line from the tab's settings, the payload on stdin).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tidewatch_core::guard_shim::GuardCtx;
use tidewatch_core::launch::{LaunchCtx, claude_settings_json};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;

fn hook(payload: &Value, trailers_on: bool) -> Output {
    let root = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(&root.path().join("home"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let mut prefs = Prefs::default();
    prefs.guards.rules.trailers = trailers_on;
    prefs.save(&paths).unwrap();
    let guards = GuardCtx {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_tidewatch")),
        config: paths.prefs_file(),
        repo: root.path().join("web"),
        cache_dir: root.path().join("cache"),
        tab_dir: root.path().join("tab"),
    };
    let ctx = LaunchCtx {
        worktree_tab: true,
        trailers_guard: true,
        guards: Some(guards.clone()),
        ..LaunchCtx::default()
    };
    let settings: Value = serde_json::from_str(&claude_settings_json(&ctx).unwrap()).unwrap();
    let command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs(guards.env(Path::new("/guard-bin")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn bash(command: &str) -> Value {
    json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/" })
}

#[test]
fn a_gh_body_with_a_trailer_is_refused_with_one_line_and_a_clean_one_passes() {
    let bad = bash(
        "gh pr create --title t --body \"$(cat <<'EOF'\nFix.\n\nGenerated with [Claude Code](x)\nEOF\n)\"",
    );
    let refused = hook(&bad, true);
    assert_eq!(refused.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("a Generated with footer"), "{stderr}");

    assert_eq!(hook(&bash("gh pr create --title t --body 'Fix.'"), true).status.code(), Some(0));
    assert_eq!(hook(&bad, false).status.code(), Some(0));
}
