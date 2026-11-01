use super::*;

const NOW: i64 = 1_793_000_000_000;

const INPUT: &str = r#"{"workspace":{"project_dir":"@PROJECT@","current_dir":"@PROJECT@"},"cost":{"total_cost_usd":20.51},"rate_limits":{"five_hour":{"used_percentage":31,"resets_at":1793007800}}}"#;

struct Lab {
    root: tempfile::TempDir,
}

impl Lab {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for dir in ["claude", "project", "tab"] {
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

    fn env(&self) -> Env {
        Env {
            claude_dir: self.path("claude"),
            cwd: Some(self.path("project")),
            nested: false,
            now_ms: NOW,
        }
    }

    fn owner(&self, script: &str) {
        let file = self.path("owner.sh");
        std::fs::write(&file, script).unwrap();
        std::fs::write(
            self.path("claude/settings.json"),
            format!(r#"{{"statusLine":{{"type":"command","command":"sh {}"}}}}"#, file.display()),
        )
        .unwrap();
    }

    fn run(&self) -> Outcome {
        run(&self.path("tab"), self.input().as_bytes(), &self.env())
    }

    fn saved(&self) -> Option<Reading> {
        statusline::load(&plan_limits_file_in(&self.path("tab")))
    }
}

#[test]
fn the_numbers_are_saved_and_the_owners_output_and_exit_code_pass_through() {
    let lab = Lab::new();
    lab.owner("cat >/dev/null; printf 'owner footer'; exit 3");
    assert_eq!(lab.run(), Outcome { stdout: b"owner footer".to_vec(), code: 3 });
    let saved = lab.saved().unwrap();
    assert_eq!(saved.cost_usd, Some(20.51));
    assert_eq!(saved.limits.five_hour.map(|window| window.used_pct), Some(31.0));
    assert_eq!(saved.read_ms, NOW);
}

#[test]
fn the_owners_command_gets_the_same_input_and_the_same_working_folder() {
    let lab = Lab::new();
    lab.owner("cat; echo; pwd");
    let outcome = run(&lab.path("tab"), lab.input().as_bytes(), &Env { cwd: None, ..lab.env() });
    let text = String::from_utf8(outcome.stdout).unwrap();
    assert!(text.starts_with(&lab.input()), "{text}");
    let here = std::env::current_dir().unwrap();
    assert_eq!(text.lines().last().map(Path::new), Some(here.as_path()));
}

#[test]
fn with_no_owner_command_it_prints_nothing_and_still_saves() {
    let lab = Lab::new();
    assert_eq!(lab.run(), Outcome { stdout: Vec::new(), code: 0 });
    assert_eq!(lab.saved().map(|reading| reading.cost_usd), Some(Some(20.51)));
}

#[test]
fn input_that_is_not_json_is_not_saved_but_the_owners_command_still_runs() {
    let lab = Lab::new();
    lab.owner("cat");
    let outcome = run(&lab.path("tab"), b"garbage", &lab.env());
    assert_eq!(outcome, Outcome { stdout: b"garbage".to_vec(), code: 0 });
    assert_eq!(lab.saved(), None);
}

#[test]
fn an_unwritable_tab_folder_never_stops_the_owners_command() {
    let lab = Lab::new();
    lab.owner("cat >/dev/null; printf ok");
    let blocked = lab.path("tab/not-a-folder");
    std::fs::write(&blocked, "x").unwrap();
    let outcome = run(&blocked, lab.input().as_bytes(), &lab.env());
    assert_eq!(outcome, Outcome { stdout: b"ok".to_vec(), code: 0 });
}

#[test]
fn a_run_started_by_another_run_does_not_run_the_owners_command_again() {
    let lab = Lab::new();
    lab.owner("printf again");
    let outcome = run(&lab.path("tab"), lab.input().as_bytes(), &Env { nested: true, ..lab.env() });
    assert_eq!(outcome, Outcome { stdout: Vec::new(), code: 0 });
}

#[test]
fn the_project_setting_wins_over_the_user_setting_and_a_later_input_keeps_the_saved_window() {
    let lab = Lab::new();
    lab.owner("printf user");
    let project = lab.path("project/.claude");
    std::fs::create_dir_all(&project).unwrap();
    let local = lab.path("project-owner.sh");
    std::fs::write(&local, "printf project").unwrap();
    std::fs::write(
        project.join("settings.json"),
        format!(r#"{{"statusLine":{{"command":"sh {}"}}}}"#, local.display()),
    )
    .unwrap();
    assert_eq!(lab.run().stdout, b"project");
    let early = r#"{"cost":{"total_cost_usd":21.0}}"#;
    let _ = run(&lab.path("tab"), early.as_bytes(), &Env { now_ms: NOW + 1000, ..lab.env() });
    let saved = lab.saved().unwrap();
    assert_eq!(saved.cost_usd, Some(21.0));
    assert_eq!(saved.limits.five_hour.map(|window| window.used_pct), Some(31.0));
    assert_eq!(saved.read_ms, NOW + 1000);
}

#[test]
fn the_project_folder_is_read_from_the_input_with_the_working_folder_as_the_fallback() {
    let cases = [
        (r#"{"workspace":{"project_dir":"/a","current_dir":"/b"}}"#, Some("/a")),
        (r#"{"workspace":{"current_dir":"/b"},"cwd":"/c"}"#, Some("/b")),
        (r#"{"cwd":"/c"}"#, Some("/c")),
        (r#"{"workspace":{"project_dir":""}}"#, None),
        ("nope", None),
    ];
    for (input, want) in cases {
        assert_eq!(project_dir(input).as_deref(), want.map(Path::new), "{input}");
    }
}
