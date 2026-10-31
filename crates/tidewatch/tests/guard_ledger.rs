//! The ledger guard's hooks end to end: the real `tidewatch --guard ledger` binary, run the way
//! Claude Code runs a command hook (a shell line, the payload on stdin) from the tab's settings.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use tidewatch_core::guard_shim::GuardCtx;
use tidewatch_core::launch::claude_settings;
use tidewatch_core::paths::TidewatchPaths;

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
        std::fs::create_dir_all(root.path().join("tab")).unwrap();
        Self { root, paths, repo }
    }

    fn tab_dir(&self) -> PathBuf {
        self.root.path().join("tab")
    }

    fn ledger(&self) -> PathBuf {
        self.tab_dir().join("progress.md")
    }

    fn ctx(&self) -> GuardCtx {
        GuardCtx {
            bin: PathBuf::from(env!("CARGO_BIN_EXE_tidewatch")),
            config: self.paths.prefs_file(),
            repo: self.repo.clone(),
            cache_dir: self.root.path().join("cache"),
            tab_dir: self.tab_dir(),
        }
    }

    fn command(&self, which: &str, tail: &str) -> String {
        let settings: Value =
            serde_json::from_str(&claude_settings(None, Some(&self.ctx().bin)).unwrap()).unwrap();
        settings["hooks"][which]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group["hooks"][0]["command"].as_str())
            .find(|command| command.ends_with(tail))
            .unwrap()
            .to_owned()
    }

    fn hook(&self, which: &str, tail: &str, payload: &Value) -> Output {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(self.command(which, tail))
            .current_dir(&self.repo)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .envs(self.ctx().env(Path::new("/guard-bin")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn start_turn(&self) {
        let output = self.hook(
            "UserPromptSubmit",
            "ledger",
            &json!({"hook_event_name": "UserPromptSubmit"}),
        );
        assert_eq!(output.status.code(), Some(0));
        assert_eq!((text(&output.stdout), text(&output.stderr)), (String::new(), String::new()));
        std::thread::sleep(Duration::from_millis(20));
    }

    fn transcript(&self, lines: &[&str]) -> PathBuf {
        let file = self.root.path().join("transcript.jsonl");
        std::fs::write(&file, lines.join("\n")).unwrap();
        file
    }

    fn stop(&self, transcript: &Path, active: bool) -> Output {
        self.hook(
            "Stop",
            "ledger --stop",
            &json!({
                "hook_event_name": "Stop",
                "stop_hook_active": active,
                "transcript_path": transcript,
                "cwd": self.repo,
            }),
        )
    }
}

const USER: &str = r#"{"type":"user","message":{"role":"user","content":"plan the rollout"}}"#;
const CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Bash","input":{}}]}}"#;
const RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"1","content":"ok"}]}}"#;
const SAY: &str =
    r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Nov 1: switch it off"}]}}"#;

#[test]
fn a_turn_with_tools_and_no_ledger_is_sent_back_once_and_the_ledger_ends_the_asking() {
    let lab = Lab::new();
    lab.start_turn();
    let transcript = lab.transcript(&[USER, CALL, RESULT, SAY]);

    let first = lab.stop(&transcript, false);
    assert_eq!(first.status.code(), Some(2));
    let stderr = text(&first.stderr);
    assert_eq!(stderr.trim_end(), tidewatch_core::guard_ledger::MESSAGE);
    assert_eq!(text(&first.stdout), "");

    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0), "never twice for a turn");
    assert_eq!(lab.stop(&transcript, true).status.code(), Some(0));

    lab.start_turn();
    std::fs::write(lab.ledger(), "# Goal\ng\n").unwrap();
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0), "written this turn");
}

#[test]
fn a_ledger_older_than_the_turn_counts_as_not_written() {
    let lab = Lab::new();
    std::fs::write(lab.ledger(), "# Goal\ng\n").unwrap();
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(lab.ledger()).unwrap().set_modified(old).unwrap();
    lab.start_turn();
    let transcript = lab.transcript(&[USER, CALL, RESULT, SAY]);
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(2));
}

#[test]
fn a_turn_without_tool_calls_and_a_tab_without_the_environment_are_left_alone() {
    let lab = Lab::new();
    lab.start_turn();
    let chat = lab.transcript(&[USER, SAY]);
    assert_eq!(lab.stop(&chat, false).status.code(), Some(0));

    for tail in [&["--guard", "ledger", "--stop"][..], &["--guard", "ledger"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_tidewatch"))
            .args(tail)
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
    }
}

#[test]
fn a_ledger_written_this_turn_that_still_says_working_is_asked_for_its_final_state_once() {
    let lab = Lab::new();
    lab.start_turn();
    std::fs::write(lab.ledger(), "# Goal\ng\n\n## Status\nstate: working\n").unwrap();
    let transcript = lab.transcript(&[USER, CALL, RESULT, SAY]);

    let first = lab.stop(&transcript, false);
    assert_eq!(first.status.code(), Some(2));
    let stderr = text(&first.stderr);
    assert_eq!(stderr.trim_end(), tidewatch_core::guard_ledger::MESSAGE);

    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0), "never twice for a turn");
    assert_eq!(lab.stop(&transcript, true).status.code(), Some(0));

    lab.start_turn();
    std::fs::write(lab.ledger(), "# Goal\ng\n\n## Status\nstate: waiting\n").unwrap();
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0), "a stopped state ends it");
}

#[test]
fn the_guard_leaves_one_record_per_ask_for_the_reader() {
    let lab = Lab::new();
    lab.start_turn();
    let transcript = lab.transcript(&[USER, CALL, RESULT, SAY]);
    let file = lab.tab_dir().join("tidewatch-sent.jsonl");
    assert_eq!(tidewatch_core::sent::load(&file), []);

    assert_eq!(lab.stop(&transcript, false).status.code(), Some(2));
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0));

    let records = tidewatch_core::sent::load(&file);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason, tidewatch_core::sent::Reason::LedgerGuard);
    assert!(records[0].text.starts_with("Before you stop: "), "{}", records[0].text);
    assert_eq!(records[0].text, tidewatch_core::guard_ledger::MESSAGE);
}

#[test]
fn the_final_state_ask_leaves_its_own_record() {
    let lab = Lab::new();
    lab.start_turn();
    std::fs::write(lab.ledger(), "# Goal\ng\n\n## Status\nstate: working\n").unwrap();
    let transcript = lab.transcript(&[USER, CALL, RESULT, SAY]);
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(2));
    assert_eq!(lab.stop(&transcript, false).status.code(), Some(0));

    let records = tidewatch_core::sent::load(&lab.tab_dir().join("tidewatch-sent.jsonl"));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason, tidewatch_core::sent::Reason::LedgerGuard);
    assert_eq!(records[0].because, tidewatch_core::sent::because::LEDGER_FINAL);
    assert_eq!(records[0].text, tidewatch_core::guard_ledger::MESSAGE);
}
