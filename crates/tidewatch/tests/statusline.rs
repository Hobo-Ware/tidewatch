//! The status line command end to end: the real `tidewatch --statusline` binary, run the way Claude
//! Code runs it (the shell line from the tab's settings file, the input on stdin), with a fake owner
//! status line command in a throwaway Claude config folder.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tidewatch_core::launch::{LaunchCtx, StatusLineCtx, claude_settings_json};
use tidewatch_core::statusline;

const INPUT: &str = r#"{"session_id":"s1","version":"2.1.316","workspace":{"project_dir":"@PROJECT@","current_dir":"@PROJECT@"},"cost":{"total_cost_usd":20.51,"total_duration_ms":4000},"rate_limits":{"five_hour":{"used_percentage":31,"resets_at":1793007800},"seven_day":{"used_percentage":84.2,"resets_at":1793260000}}}"#;

struct Lab {
    root: tempfile::TempDir,
}

impl Lab {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for dir in ["claude", "project", "tab", "home"] {
            std::fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn input(&self) -> String {
        INPUT.replace("@PROJECT@", &self.path("project").display().to_string())
    }

    fn owner(&self, script: &str) {
        let file = self.path("owner.sh");
        std::fs::write(&file, script).unwrap();
        std::fs::write(
            self.path("claude/settings.json"),
            json!({"statusLine": {"type": "command", "command": format!("sh {}", file.display())}})
                .to_string(),
        )
        .unwrap();
    }

    fn line(&self) -> String {
        let ctx = LaunchCtx {
            status_line: Some(StatusLineCtx {
                bin: PathBuf::from(env!("CARGO_BIN_EXE_tidewatch")),
                tab_dir: self.path("tab"),
                padding: None,
            }),
            ..LaunchCtx::default()
        };
        let settings: Value = serde_json::from_str(&claude_settings_json(&ctx).unwrap()).unwrap();
        settings["statusLine"]["command"].as_str().unwrap().to_owned()
    }

    fn run(&self, input: &str) -> Output {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(self.line())
            .current_dir(self.path("project"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", self.path("home"))
            .env("CLAUDE_CONFIG_DIR", self.path("claude"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn saved(&self) -> Option<statusline::Reading> {
        statusline::load(&tidewatch_core::paths::plan_limits_file_in(&self.path("tab")))
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_numbers_are_saved_and_the_owners_footer_comes_through_with_its_exit_code() {
    let lab = Lab::new();
    lab.owner("input=$(cat); printf 'footer: %s' \"${#input}\"; exit 4");
    let input = lab.input();
    let output = lab.run(&input);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(text(&output.stdout), format!("footer: {}", input.len()));
    let saved = lab.saved().unwrap();
    assert_eq!(saved.cost_usd, Some(20.51));
    assert_eq!(
        saved.limits.five_hour.map(|w| (w.used_pct, w.resets_at_ms)),
        Some((31.0, Some(1_793_007_800_000)))
    );
    assert_eq!(saved.limits.week.map(|w| w.used_pct), Some(84.2));
}

#[test]
fn the_owners_command_runs_in_the_same_folder() {
    let lab = Lab::new();
    lab.owner("cat >/dev/null; pwd");
    let output = lab.run(&lab.input());
    let here = std::fs::canonicalize(lab.path("project")).unwrap();
    assert_eq!(text(&output.stdout).trim_end(), here.display().to_string());
}

#[test]
fn with_no_status_line_of_the_owners_it_prints_nothing_and_still_saves() {
    let lab = Lab::new();
    let output = lab.run(&lab.input());
    assert_eq!(
        (output.status.code(), text(&output.stdout), text(&output.stderr)),
        (Some(0), String::new(), String::new())
    );
    assert_eq!(lab.saved().map(|r| r.cost_usd), Some(Some(20.51)));
}

#[test]
fn input_that_is_not_json_saves_nothing_and_still_runs_the_owners_command() {
    let lab = Lab::new();
    lab.owner("cat");
    let output = lab.run("not json at all");
    assert_eq!(
        (output.status.code(), text(&output.stdout)),
        (Some(0), "not json at all".to_owned())
    );
    assert_eq!(lab.saved(), None);
}

#[test]
fn a_tab_folder_that_cannot_be_written_never_stops_the_footer() {
    let lab = Lab::new();
    lab.owner("cat >/dev/null; printf ok");
    std::fs::remove_dir_all(lab.path("tab")).unwrap();
    std::fs::write(lab.path("tab"), "a file where the folder should be").unwrap();
    let output = lab.run(&lab.input());
    assert_eq!((output.status.code(), text(&output.stdout)), (Some(0), "ok".to_owned()));
}

#[test]
fn a_status_line_without_the_tab_folder_is_refused_with_a_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_tidewatch"))
        .arg("--statusline")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("--statusline needs"), "{}", text(&output.stderr));
}
