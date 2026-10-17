//! The repo's test command, which the Tests, TestFails, TestPasses and SameBehavior gates run.
//!
//! `<repo>/.tidewatch.toml` with `test = "..."` wins (an empty string switches the gates off for
//! that repo); otherwise the first marker file found picks a conventional command.
//! `test_timeout_secs = N` in the same file changes how long a run may take (default 10 minutes).
//! `checklist = "<path>"` points at the repo's own checklist template for the progress ledger.

use std::fmt::Write as _;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;

const REPO_CONFIG: &str = ".tidewatch.toml";

/// How long a test run may take before it is stopped.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_millis(25);
const PIPE_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
enum Configured {
    Absent,
    Disabled,
    Command(String),
}

#[derive(Deserialize)]
struct RepoConfig {
    test: Option<String>,
    test_timeout_secs: Option<u64>,
    checklist: Option<String>,
}

fn configured(repo: &Path) -> Configured {
    let Ok(text) = std::fs::read_to_string(repo.join(REPO_CONFIG)) else {
        return Configured::Absent;
    };
    match toml::from_str::<RepoConfig>(&text) {
        Ok(RepoConfig { test: Some(test), .. }) if test.trim().is_empty() => Configured::Disabled,
        Ok(RepoConfig { test: Some(test), .. }) => Configured::Command(test.trim().to_owned()),
        Ok(RepoConfig { test: None, .. }) => Configured::Absent,
        Err(e) => {
            eprintln!("tidewatch: {REPO_CONFIG} is invalid ({e}); detecting the test command");
            Configured::Absent
        }
    }
}

/// The command to run in a worktree of `repo`, or `None` when there is none to run.
#[must_use]
pub fn test_command(repo: &Path) -> Option<String> {
    match configured(repo) {
        Configured::Command(command) => Some(command),
        Configured::Disabled => None,
        Configured::Absent => detect(repo),
    }
}

/// How long a test run in `repo` may take: `test_timeout_secs` from `.tidewatch.toml` when it is a
/// positive number, else [`DEFAULT_TIMEOUT`].
#[must_use]
pub fn timeout(repo: &Path) -> Duration {
    std::fs::read_to_string(repo.join(REPO_CONFIG))
        .ok()
        .and_then(|text| toml::from_str::<RepoConfig>(&text).ok())
        .and_then(|config| config.test_timeout_secs)
        .filter(|secs| *secs > 0)
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs)
}

/// The checklist template file `.tidewatch.toml` names, inside `repo`. A path that is absolute or
/// climbs out of the repo is ignored.
#[must_use]
pub fn checklist_path(repo: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(repo.join(REPO_CONFIG)).ok()?;
    let relative = PathBuf::from(toml::from_str::<RepoConfig>(&text).ok()?.checklist?.trim());
    let inside = !relative.as_os_str().is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    inside.then(|| repo.join(relative))
}

fn has(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

fn file_contains(dir: &Path, name: &str, needle: &str) -> bool {
    std::fs::read_to_string(dir.join(name)).is_ok_and(|text| text.contains(needle))
}

fn has_node_test_script(dir: &Path) -> bool {
    let manifest = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let script = manifest.as_ref().and_then(|m| m.pointer("/scripts/test")?.as_str());
    script.is_some_and(|script| !script.trim().is_empty() && !script.contains("no test specified"))
}

fn node_command(dir: &Path) -> &'static str {
    if has(dir, "pnpm-lock.yaml") {
        "pnpm test"
    } else if has(dir, "yarn.lock") {
        "yarn test"
    } else if has(dir, "bun.lock") || has(dir, "bun.lockb") {
        "bun run test"
    } else {
        "npm test"
    }
}

fn makefile_has_test_target(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Makefile"))
        .is_ok_and(|text| text.lines().any(|line| line.starts_with("test:")))
}

/// Picks a conventional command from the files in `dir`. Cargo, Node, Deno, Go, Python, Ruby,
/// Elixir, then a Makefile `test` target, in that order.
#[must_use]
pub fn detect(dir: &Path) -> Option<String> {
    let command = if has(dir, "Cargo.toml") {
        "cargo test"
    } else if has_node_test_script(dir) {
        node_command(dir)
    } else if has(dir, "deno.json") || has(dir, "deno.jsonc") {
        "deno test"
    } else if has(dir, "go.mod") {
        "go test ./..."
    } else if has(dir, "pytest.ini")
        || file_contains(dir, "pyproject.toml", "pytest")
        || file_contains(dir, "setup.cfg", "[tool:pytest]")
    {
        "pytest"
    } else if has(dir, "Gemfile") && (has(dir, "spec") || has(dir, ".rspec")) {
        "bundle exec rspec"
    } else if has(dir, "Gemfile") && has(dir, "test") {
        "bundle exec rake test"
    } else if has(dir, "mix.exs") {
        "mix test"
    } else if makefile_has_test_target(dir) {
        "make test"
    } else {
        return None;
    };
    Some(command.to_owned())
}

/// What a finished test command left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRun {
    /// `-1` when the process was killed by a signal or never started.
    pub exit_code: i32,
    /// Standard output, then standard error.
    pub output: String,
}

/// Runs `command` through `sh -c` in `dir` and waits for it. Blocking: call it from a background job.
#[must_use]
pub fn run_in(dir: &Path, command: &str) -> TestRun {
    match Command::new("sh").arg("-c").arg(command).current_dir(dir).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            TestRun { exit_code: output.status.code().unwrap_or(-1), output: text }
        }
        Err(e) => TestRun { exit_code: -1, output: format!("could not run `{command}`: {e}") },
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receiver
}

fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

fn describe(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 120 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

/// [`run_in`] that gives up after `timeout`: the command and everything it started are killed, and
/// the run is a failure (exit code `-1`) whose output ends with a line saying so. Blocking: call it
/// from a background job.
#[must_use]
pub fn run_in_timeout(dir: &Path, command: &str, timeout: Duration) -> TestRun {
    let spawned = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            return TestRun { exit_code: -1, output: format!("could not run `{command}`: {e}") };
        }
    };
    let streams = (child.stdout.take().map(drain), child.stderr.take().map(drain));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                kill_group(child.id());
                let _ = child.kill();
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(_) => break None,
        }
    };
    let mut output = String::new();
    for stream in [streams.0, streams.1].into_iter().flatten() {
        output.push_str(&stream.recv_timeout(PIPE_GRACE).unwrap_or_default());
    }
    if timed_out {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        let _ = writeln!(
            output,
            "[tidewatch: the test command ran longer than {} and was stopped]",
            describe(timeout)
        );
        return TestRun { exit_code: -1, output };
    }
    TestRun { exit_code: status.and_then(|s| s.code()).unwrap_or(-1), output }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Files<'a> = &'a [(&'a str, &'a str)];

    fn repo_with(files: Files) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    #[test]
    fn detection_table() {
        let cases: &[(&str, Files, Option<&str>)] = &[
            ("cargo", &[("Cargo.toml", "")], Some("cargo test")),
            ("npm", &[("package.json", r#"{"scripts":{"test":"vitest run"}}"#)], Some("npm test")),
            (
                "pnpm",
                &[("package.json", r#"{"scripts":{"test":"vitest"}}"#), ("pnpm-lock.yaml", "")],
                Some("pnpm test"),
            ),
            (
                "yarn",
                &[("package.json", r#"{"scripts":{"test":"jest"}}"#), ("yarn.lock", "")],
                Some("yarn test"),
            ),
            (
                "bun",
                &[("package.json", r#"{"scripts":{"test":"bun test"}}"#), ("bun.lockb", "")],
                Some("bun run test"),
            ),
            (
                "node without a test script",
                &[("package.json", r#"{"scripts":{"build":"tsc"}}"#)],
                None,
            ),
            ("node without scripts", &[("package.json", "{}")], None),
            ("node with broken json", &[("package.json", "{")], None),
            (
                "npm init placeholder",
                &[(
                    "package.json",
                    r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
                )],
                None,
            ),
            ("deno", &[("deno.json", "{}")], Some("deno test")),
            ("go", &[("go.mod", "module x")], Some("go test ./...")),
            ("pytest.ini", &[("pytest.ini", "")], Some("pytest")),
            (
                "pyproject with pytest",
                &[("pyproject.toml", "[tool.pytest.ini_options]")],
                Some("pytest"),
            ),
            ("pyproject without pytest", &[("pyproject.toml", "[project]\nname = \"x\"")], None),
            ("rspec", &[("Gemfile", ""), ("spec/a_spec.rb", "")], Some("bundle exec rspec")),
            ("minitest", &[("Gemfile", ""), ("test/a_test.rb", "")], Some("bundle exec rake test")),
            ("gemfile alone", &[("Gemfile", "")], None),
            ("mix", &[("mix.exs", "")], Some("mix test")),
            ("make", &[("Makefile", "build:\n\tcc\ntest:\n\t./t\n")], Some("make test")),
            ("make without a test target", &[("Makefile", "build:\n\tcc\n")], None),
            (
                "cargo beats node",
                &[("Cargo.toml", ""), ("package.json", r#"{"scripts":{"test":"x"}}"#)],
                Some("cargo test"),
            ),
            ("nothing", &[("README.md", "")], None),
        ];
        for (name, files, expected) in cases {
            let dir = repo_with(files);
            assert_eq!(detect(dir.path()).as_deref(), *expected, "{name}");
        }
    }

    #[test]
    fn repo_config_wins_over_detection() {
        let dir = repo_with(&[("Cargo.toml", ""), (".tidewatch.toml", "test = \"just ci\"\n")]);
        assert_eq!(test_command(dir.path()).as_deref(), Some("just ci"));
    }

    #[test]
    fn repo_config_is_trimmed_and_ignores_other_keys() {
        let dir =
            repo_with(&[(".tidewatch.toml", "house_style = false\ntest = \"  make check \"\n")]);
        assert_eq!(test_command(dir.path()).as_deref(), Some("make check"));
    }

    #[test]
    fn an_empty_test_value_switches_the_gates_off() {
        let dir = repo_with(&[("Cargo.toml", ""), (".tidewatch.toml", "test = \"\"\n")]);
        assert_eq!(test_command(dir.path()), None);
        let blank = repo_with(&[("Cargo.toml", ""), (".tidewatch.toml", "test = \"  \"\n")]);
        assert_eq!(test_command(blank.path()), None);
    }

    #[test]
    fn a_config_without_test_or_with_bad_toml_falls_back_to_detection() {
        let cases = ["house_style = false\n", "test = \n[[", ""];
        for text in cases {
            let dir = repo_with(&[("go.mod", "module x"), (".tidewatch.toml", text)]);
            assert_eq!(test_command(dir.path()).as_deref(), Some("go test ./..."), "{text:?}");
        }
    }

    #[test]
    fn no_config_and_no_markers_is_no_command() {
        let dir = repo_with(&[]);
        assert_eq!(test_command(dir.path()), None);
    }

    #[test]
    fn run_reports_real_exit_codes_and_both_streams() {
        let dir = tempfile::tempdir().unwrap();
        let ok = run_in(dir.path(), "echo out; echo err 1>&2; exit 0");
        assert_eq!((ok.exit_code, ok.output.as_str()), (0, "out\nerr\n"));
        let failed = run_in(dir.path(), "echo boom; exit 3");
        assert_eq!((failed.exit_code, failed.output.as_str()), (3, "boom\n"));
        let missing = run_in(dir.path(), "definitely-not-a-command-xyz");
        assert_eq!(missing.exit_code, 127);
    }

    #[test]
    fn run_happens_in_the_given_directory() {
        let dir = repo_with(&[("marker.txt", "here")]);
        let run = run_in(dir.path(), "cat marker.txt");
        assert_eq!((run.exit_code, run.output.as_str()), (0, "here"));
    }

    #[test]
    fn a_missing_directory_is_a_failed_run_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let run = run_in(&dir.path().join("gone"), "true");
        assert_eq!(run.exit_code, -1);
        assert!(run.output.contains("could not run"), "{}", run.output);
    }

    #[test]
    fn a_run_with_a_timeout_reports_like_run_in_when_it_finishes_in_time() {
        let dir = tempfile::tempdir().unwrap();
        let ok = run_in_timeout(dir.path(), "echo out; echo err 1>&2", Duration::from_secs(30));
        assert_eq!((ok.exit_code, ok.output.as_str()), (0, "out\nerr\n"));
        let failed = run_in_timeout(dir.path(), "echo boom; exit 3", Duration::from_secs(30));
        assert_eq!((failed.exit_code, failed.output.as_str()), (3, "boom\n"));
    }

    #[test]
    fn a_hung_command_and_its_children_are_killed_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let run = run_in_timeout(
            dir.path(),
            "echo before; sleep 30 & sleep 30; echo after",
            Duration::from_millis(300),
        );
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        assert_eq!(run.exit_code, -1);
        assert!(run.output.starts_with("before\n"), "{}", run.output);
        assert!(run.output.contains("longer than 0 s and was stopped"), "{}", run.output);
        assert!(!run.output.contains("after\n"), "{}", run.output);
    }

    #[test]
    fn the_timeout_is_ten_minutes_unless_the_repo_says_otherwise() {
        let none = repo_with(&[]);
        assert_eq!(timeout(none.path()), DEFAULT_TIMEOUT);
        let set = repo_with(&[(".tidewatch.toml", "test_timeout_secs = 90\n")]);
        assert_eq!(timeout(set.path()), Duration::from_secs(90));
        for text in ["test_timeout_secs = 0\n", "test_timeout_secs = \"x\"\n", "[["] {
            let bad = repo_with(&[(".tidewatch.toml", text)]);
            assert_eq!(timeout(bad.path()), DEFAULT_TIMEOUT, "{text:?}");
        }
    }

    #[test]
    fn the_checklist_is_a_path_inside_the_repo() {
        let cases = [
            ("checklist = \"docs/checklist.md\"\n", Some("docs/checklist.md")),
            ("checklist = \" docs/checklist.md \"\n", Some("docs/checklist.md")),
            ("test = \"make check\"\nchecklist = \"c.md\"\n", Some("c.md")),
            ("checklist = \"./c.md\"\n", Some("./c.md")),
            ("checklist = \"/etc/passwd\"\n", None),
            ("checklist = \"../c.md\"\n", None),
            ("checklist = \"a/../../c.md\"\n", None),
            ("checklist = \"\"\n", None),
            ("checklist = 3\n", None),
            ("test = \"x\"\n", None),
            ("[[", None),
        ];
        for (text, want) in cases {
            let repo = repo_with(&[(".tidewatch.toml", text)]);
            assert_eq!(
                checklist_path(repo.path()),
                want.map(|path| repo.path().join(path)),
                "{text:?}"
            );
        }
        assert_eq!(checklist_path(repo_with(&[]).path()), None);
    }

    #[test]
    fn a_missing_directory_is_a_failed_run_with_a_timeout_too() {
        let dir = tempfile::tempdir().unwrap();
        let run = run_in_timeout(&dir.path().join("gone"), "true", Duration::from_secs(5));
        assert_eq!(run.exit_code, -1);
        assert!(run.output.contains("could not run"), "{}", run.output);
    }

    #[test]
    fn timeouts_are_worded_in_seconds_or_whole_minutes() {
        assert_eq!(describe(Duration::from_secs(90)), "90 s");
        assert_eq!(describe(Duration::from_secs(600)), "10 min");
        assert_eq!(describe(Duration::from_millis(300)), "0 s");
    }
}
