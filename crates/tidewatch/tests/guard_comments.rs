//! The comment guard's hooks end to end: the real `tidewatch --guard comments` binary, run the way
//! Claude Code runs a command hook (a shell line, the payload on stdin), with real git for the sweep.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tidewatch_core::comment_flags::TabFiles;
use tidewatch_core::guard_shim::GuardCtx;
use tidewatch_core::guards::{GuardPrefs, Rule};
use tidewatch_core::launch::claude_settings;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;

struct Lab {
    root: tempfile::TempDir,
    paths: TidewatchPaths,
    repo: PathBuf,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl Lab {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(&root.path().join("home"));
        let repo = root.path().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        Self { root, paths, repo }
    }

    fn tab_dir(&self) -> PathBuf {
        self.root.path().join("tab")
    }

    fn settings(&self, change: impl FnOnce(&mut GuardPrefs)) {
        let mut prefs = Prefs::default();
        change(&mut prefs.guards);
        prefs.save(&self.paths).unwrap();
    }

    fn command(&self, which: &str) -> String {
        let ctx = GuardCtx {
            bin: PathBuf::from(env!("CARGO_BIN_EXE_tidewatch")),
            config: self.paths.prefs_file(),
            repo: self.repo.clone(),
            cache_dir: self.root.path().join("cache"),
            tab_dir: self.tab_dir(),
        };
        let settings: Value =
            serde_json::from_str(&claude_settings(None, Some(&ctx.bin)).unwrap()).unwrap();
        let group = &settings["hooks"][which][0];
        group["hooks"][0]["command"].as_str().unwrap().to_owned()
    }

    fn hook(&self, which: &str, payload: &Value) -> Output {
        let ctx = GuardCtx {
            bin: PathBuf::from(env!("CARGO_BIN_EXE_tidewatch")),
            config: self.paths.prefs_file(),
            repo: self.repo.clone(),
            cache_dir: self.root.path().join("cache"),
            tab_dir: self.tab_dir(),
        };
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(self.command(which))
            .current_dir(&self.repo)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .envs(ctx.env(Path::new("/guard-bin")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

fn write_payload(path: &str, content: &str) -> Value {
    json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {"file_path": path, "content": content},
        "tool_response": {"filePath": path, "type": "create"},
        "cwd": "/",
        "session_id": "s1",
    })
}

#[test]
fn a_noisy_write_comes_back_as_exit_two_with_the_report_on_stderr() {
    let lab = Lab::new();
    let output = lab
        .hook("PostToolUse", &write_payload("/w/src/a.rs", "fn a() {}\n// loop over the items\n"));
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("COMMENT GUARD - a.rs: 1 suspect comment(s) in what you just wrote."),
        "{stderr}"
    );
    assert!(stderr.contains("L2  [narration]"), "{stderr}");
    assert_eq!(text(&output.stdout), "");
    assert_eq!(TabFiles::new(&lab.tab_dir()).load().len(), 1);
}

#[test]
fn a_clean_write_a_markdown_file_and_a_rule_that_is_off_are_silent_exit_zero() {
    let lab = Lab::new();
    for payload in [
        write_payload("/w/src/a.rs", "fn a() {}\n"),
        write_payload("/w/notes.md", "// loop over the items\n"),
    ] {
        let output = lab.hook("PostToolUse", &payload);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(text(&output.stderr), "");
    }
    lab.settings(|guards| guards.rules.set(Rule::Comments, false));
    let output = lab.hook("PostToolUse", &write_payload("/w/src/a.rs", "// loop over the items\n"));
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_tab_with_the_switch_off_is_not_checked() {
    let lab = Lab::new();
    TabFiles::new(&lab.tab_dir()).set_off(true).unwrap();
    let output = lab.hook("PostToolUse", &write_payload("/w/src/a.rs", "// loop over the items\n"));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(TabFiles::new(&lab.tab_dir()).load(), vec![]);
}

#[test]
fn the_stop_hook_sweeps_the_branch_once_and_stays_quiet_when_nothing_changed() {
    let lab = Lab::new();
    lab.git(&["init", "-q", "-b", "main"]);
    lab.git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    lab.git(&["switch", "-q", "-c", "agent/work"]);
    std::fs::write(lab.repo.join("a.rs"), "// set a to one\nfn a() {}\n").unwrap();
    let payload = json!({"hook_event_name": "Stop", "stop_hook_active": false, "cwd": lab.repo, "session_id": "s1"});

    let first = lab.hook("Stop", &payload);
    assert_eq!(first.status.code(), Some(2));
    let stderr = text(&first.stderr);
    assert!(
        stderr.starts_with(
            "COMMENT GUARD (branch sweep) - noisy comments still in the diff:\na.rs: 1 suspect"
        ),
        "{stderr}"
    );

    assert_eq!(lab.hook("Stop", &payload).status.code(), Some(0));
    let active = json!({"hook_event_name": "Stop", "stop_hook_active": true, "cwd": lab.repo});
    assert_eq!(lab.hook("Stop", &active).status.code(), Some(0));
}

#[test]
fn a_command_line_that_is_not_an_agent_tab_is_exit_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_tidewatch"))
        .args(["--guard", "comments"])
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}
