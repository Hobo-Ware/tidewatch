//! The repo's test command, which the Tests, TestFails, TestPasses and SameBehavior gates run.
//!
//! `.tidewatch.toml` with `test = "..."` wins (an empty string switches the gates off for that
//! repo); otherwise the first marker file found picks a conventional command. Only the root is
//! looked at, so a monorepo gets its root's toolchain. Every pick carries the file that decided it
//! ([`Detected::why`]) so the UI can show a wrong guess. A tab reads its own worktree ([`resolve`]).
//! `test_timeout_secs = N` in the same file changes how long a run may take (default 10 minutes).
//! `checklist = "<path>"` points at the repo's own checklist template for the progress ledger.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::limited;

const REPO_CONFIG: &str = ".tidewatch.toml";

/// How long a test run may take before it is stopped.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// A test command and what it was picked from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub command: String,
    /// The file or setting that decided it, for example `bun.lock` or `.tidewatch.toml`.
    pub why: String,
}

impl Detected {
    fn new(command: impl Into<String>, why: impl Into<String>) -> Self {
        Self { command: command.into(), why: why.into() }
    }

    /// `bun test (bun.lock)`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{} ({})", self.command, self.why)
    }

    fn program(&self) -> &str {
        self.command.split_whitespace().next().unwrap_or_default()
    }
}

/// What a tab's tests gate does: run a command, skip with a reason, or has nothing to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    Run(Detected),
    Skip(String),
    None,
}

/// What a tooltip says when no command was found.
pub const MISSING_TIP: &str =
    "No test command found. Set test = \"...\" in .tidewatch.toml at the repo root.";

/// What a tooltip says about the command the gates run: where it came from and how to change it.
#[must_use]
pub fn tip(command: &str, why: &str) -> String {
    let source = format!("Runs {command} ({why}).");
    if why == REPO_CONFIG {
        source
    } else {
        format!("{source} Wrong command? Set test = \"...\" in {REPO_CONFIG} at the repo root.")
    }
}

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

/// The folder whose `.tidewatch.toml` applies to `worktree`: its own, else the main checkout's.
fn config_dir<'a>(worktree: &'a Path, repo: &'a Path) -> &'a Path {
    if worktree.join(REPO_CONFIG).is_file() { worktree } else { repo }
}

/// The command to run in `repo` and why, or `None` when there is none to run. Looks only at `repo`
/// itself: the tests gate uses [`resolve`] with the tab's worktree.
#[must_use]
pub fn plan(repo: &Path) -> Option<Detected> {
    match configured(repo) {
        Configured::Command(command) => Some(Detected::new(command, REPO_CONFIG)),
        Configured::Disabled => None,
        Configured::Absent => detect_with_reason(repo),
    }
}

/// The command to run in `repo`, or `None` when there is none to run.
#[must_use]
pub fn test_command(repo: &Path) -> Option<String> {
    plan(repo).map(|plan| plan.command)
}

/// What the tests gate does in `worktree` (a tab's own checkout of `repo`): the worktree's
/// `.tidewatch.toml`, else `repo`'s, else the first detected command whose program is on `PATH`.
#[must_use]
pub fn resolve(worktree: &Path, repo: &Path) -> Gate {
    resolve_with(worktree, repo, program_on_path)
}

fn resolve_with(worktree: &Path, repo: &Path, available: impl Fn(&str) -> bool) -> Gate {
    match configured(config_dir(worktree, repo)) {
        Configured::Command(command) => Gate::Run(Detected::new(command, REPO_CONFIG)),
        Configured::Disabled => Gate::None,
        Configured::Absent => {
            let found = candidates(worktree);
            let mut missing: Vec<&str> = Vec::new();
            for detected in &found {
                if available(detected.program()) {
                    return Gate::Run(detected.clone());
                }
                if !missing.contains(&detected.program()) {
                    missing.push(detected.program());
                }
            }
            if missing.is_empty() {
                Gate::None
            } else {
                Gate::Skip(format!(
                    "skipped: no runnable test command ({} not installed)",
                    missing.join(", ")
                ))
            }
        }
    }
}

fn program_on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |file: &Path| {
        file.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return runnable(Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| runnable(&dir.join(program))))
}

/// How long a test run in `worktree` may take, read from the same `.tidewatch.toml` as
/// [`resolve`]: `test_timeout_secs` when it is a positive number, else [`DEFAULT_TIMEOUT`].
#[must_use]
pub fn timeout_in(worktree: &Path, repo: &Path) -> Duration {
    timeout(config_dir(worktree, repo))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Js {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl Js {
    const LOCKFILES: [(&'static str, Js); 5] = [
        ("bun.lock", Js::Bun),
        ("bun.lockb", Js::Bun),
        ("pnpm-lock.yaml", Js::Pnpm),
        ("yarn.lock", Js::Yarn),
        ("package-lock.json", Js::Npm),
    ];

    fn script_command(self) -> &'static str {
        match self {
            Js::Npm => "npm test",
            Js::Pnpm => "pnpm test",
            Js::Yarn => "yarn test",
            Js::Bun => "bun run test",
        }
    }

    fn from_package_manager(field: &str) -> Option<Self> {
        match field.split('@').next()? {
            "npm" => Some(Js::Npm),
            "pnpm" => Some(Js::Pnpm),
            "yarn" => Some(Js::Yarn),
            "bun" => Some(Js::Bun),
            _ => None,
        }
    }
}

struct Package {
    manager: Option<Js>,
    has_test_script: bool,
}

fn read_package(dir: &Path) -> Option<Package> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let manifest = serde_json::from_str::<serde_json::Value>(&text).ok();
    let script = manifest.as_ref().and_then(|m| m.pointer("/scripts/test")?.as_str());
    Some(Package {
        manager: manifest
            .as_ref()
            .and_then(|m| m.get("packageManager")?.as_str())
            .and_then(Js::from_package_manager),
        has_test_script: script.is_some_and(|script| {
            !script.trim().is_empty() && !script.contains("no test specified")
        }),
    })
}

fn js_runner(dir: &Path, package: &Package) -> (Js, String) {
    if let Some(js) = package.manager {
        return (js, "package.json packageManager".to_owned());
    }
    Js::LOCKFILES
        .iter()
        .find(|(file, _)| has(dir, file))
        .map_or((Js::Npm, "package.json".to_owned()), |(file, js)| (*js, (*file).to_owned()))
}

const SKIPPED_DIRS: [&str; 7] =
    ["node_modules", ".git", "target", "dist", "build", "vendor", ".next"];
const MAX_TEST_SCAN_DEPTH: usize = 6;
const MAX_TEST_SCAN_DIRS: usize = 500;

fn is_bun_test_file(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else { return false };
    ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"].contains(&extension)
        && [".test", ".spec", "_test", "_spec"].iter().any(|mark| stem.ends_with(mark))
}

fn has_bun_test_files(root: &Path) -> bool {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_TEST_SCAN_DIRS {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_file() && is_bun_test_file(&name) {
                return true;
            }
            if kind.is_dir()
                && depth < MAX_TEST_SCAN_DEPTH
                && !SKIPPED_DIRS.contains(&name.as_str())
            {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    false
}

fn node_detected(dir: &Path, package: &Package) -> Option<Detected> {
    let (js, why) = js_runner(dir, package);
    if package.has_test_script {
        Some(Detected::new(js.script_command(), why))
    } else if js == Js::Bun && has_bun_test_files(dir) {
        Some(Detected::new("bun test", format!("{why}, test files")))
    } else {
        None
    }
}

struct DenoConfig {
    file: &'static str,
    has_test_task: bool,
}

fn without_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut last = ' ';
            for next in chars.by_ref() {
                if last == '*' && next == '/' {
                    break;
                }
                last = next;
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn deno_config(dir: &Path) -> Option<DenoConfig> {
    let file = ["deno.json", "deno.jsonc"].into_iter().find(|name| has(dir, name))?;
    let text = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
    let text = without_jsonc_comments(&text);
    let has_test_task = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(config) => config.pointer("/tasks/test").is_some(),
        Err(_) => text.contains("\"test\""),
    };
    Some(DenoConfig { file, has_test_task })
}

fn deno_detected(config: &DenoConfig) -> Detected {
    if config.has_test_task {
        Detected::new("deno task test", format!("{} test task", config.file))
    } else {
        Detected::new("deno test", config.file)
    }
}

fn js_candidates(dir: &Path) -> Vec<Detected> {
    let package = read_package(dir);
    let deno = deno_config(dir);
    let mut found = Vec::new();
    if let Some(package) = &package {
        found.extend(node_detected(dir, package));
    } else if deno.is_some()
        && let Some((file, js)) = Js::LOCKFILES.iter().find(|(file, _)| has(dir, file))
    {
        let command = if *js == Js::Bun { "bun test" } else { js.script_command() };
        found.push(Detected::new(command, *file));
    }
    if let Some(deno) = &deno
        && package.as_ref().is_none_or(|_| deno.has_test_task)
    {
        found.push(deno_detected(deno));
    }
    found
}

fn makefile_has_test_target(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Makefile"))
        .is_ok_and(|text| text.lines().any(|line| line.starts_with("test:")))
}

/// Picks a conventional command from the files in `dir`. Cargo, then JavaScript (the package
/// manager's lockfile or `packageManager` field picks npm, pnpm, yarn or bun before Deno is
/// considered; Deno only without a `package.json`, or when `deno.json` has the `test` task and
/// the package has no test script), Go, Python, Ruby, Elixir, then a Makefile `test` target, in
/// that order.
#[must_use]
pub fn detect(dir: &Path) -> Option<String> {
    detect_with_reason(dir).map(|detected| detected.command)
}

/// [`detect`], with the file that decided it.
#[must_use]
pub fn detect_with_reason(dir: &Path) -> Option<Detected> {
    candidates(dir).into_iter().next()
}

/// Every conventional command whose markers are in `dir`, best first.
#[must_use]
pub fn candidates(dir: &Path) -> Vec<Detected> {
    let mut found = Vec::new();
    if has(dir, "Cargo.toml") {
        found.push(Detected::new("cargo test", "Cargo.toml"));
    }
    found.extend(js_candidates(dir));
    if has(dir, "go.mod") {
        found.push(Detected::new("go test ./...", "go.mod"));
    }
    if has(dir, "pytest.ini") {
        found.push(Detected::new("pytest", "pytest.ini"));
    } else if file_contains(dir, "pyproject.toml", "pytest") {
        found.push(Detected::new("pytest", "pyproject.toml"));
    } else if file_contains(dir, "setup.cfg", "[tool:pytest]") {
        found.push(Detected::new("pytest", "setup.cfg"));
    }
    if has(dir, "Gemfile") && (has(dir, "spec") || has(dir, ".rspec")) {
        found.push(Detected::new("bundle exec rspec", "Gemfile, spec"));
    } else if has(dir, "Gemfile") && has(dir, "test") {
        found.push(Detected::new("bundle exec rake test", "Gemfile, test"));
    }
    if has(dir, "mix.exs") {
        found.push(Detected::new("mix test", "mix.exs"));
    }
    if makefile_has_test_target(dir) {
        found.push(Detected::new("make test", "Makefile"));
    }
    found
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

/// [`run_in`] that gives up after `timeout`: the command and everything it started are killed, and
/// the run is a failure (exit code `-1`) whose output ends with a line saying so. Blocking: call it
/// from a background job.
#[must_use]
pub fn run_in_timeout(dir: &Path, command: &str, timeout: Duration) -> TestRun {
    let mut shell = Command::new("sh");
    shell.arg("-c").arg(command).current_dir(dir);
    let (out, ending) = match limited::run_with(
        &mut shell,
        timeout,
        &limited::Options::labeled("the test command"),
    ) {
        Ok(limited::Ended::Exited(out)) => (out, None),
        Ok(limited::Ended::Stopped(out)) => {
            (out, Some(format!("ran longer than {} and was stopped", limited::describe(timeout))))
        }
        Ok(limited::Ended::Cancelled(out)) => (out, Some("was cancelled".to_owned())),
        Err(e) => {
            return TestRun { exit_code: -1, output: format!("could not run `{command}`: {e}") };
        }
    };
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    if let Some(ending) = ending {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        let _ = writeln!(output, "[tidewatch: the test command {ending}]");
        return TestRun { exit_code: -1, output };
    }
    TestRun { exit_code: out.status.code().unwrap_or(-1), output }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    type Files<'a> = &'a [(&'a str, &'a str)];
    type Case<'a> = (&'a str, Files<'a>, Option<(&'a str, &'a str)>);

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
    fn detection_says_which_file_decided() {
        const TEST: &str = r#"{"scripts":{"test":"vitest"}}"#;
        const NO_TEST: &str = r#"{"scripts":{"build":"tsc"}}"#;
        const DENO_TASK: &str = r#"{"tasks":{"test":"deno test -A"}}"#;
        let cases: &[Case] = &[
            (
                "bun.lock",
                &[("package.json", TEST), ("bun.lock", "")],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "bun.lockb",
                &[("package.json", TEST), ("bun.lockb", "")],
                Some(("bun run test", "bun.lockb")),
            ),
            (
                "pnpm",
                &[("package.json", TEST), ("pnpm-lock.yaml", "")],
                Some(("pnpm test", "pnpm-lock.yaml")),
            ),
            (
                "yarn",
                &[("package.json", TEST), ("yarn.lock", "")],
                Some(("yarn test", "yarn.lock")),
            ),
            (
                "npm",
                &[("package.json", TEST), ("package-lock.json", "")],
                Some(("npm test", "package-lock.json")),
            ),
            ("no lockfile", &[("package.json", TEST)], Some(("npm test", "package.json"))),
            (
                "packageManager beats the lockfile",
                &[
                    ("package.json", r#"{"packageManager":"pnpm@9.1.0","scripts":{"test":"x"}}"#),
                    ("bun.lock", ""),
                ],
                Some(("pnpm test", "package.json packageManager")),
            ),
            (
                "packageManager bun",
                &[
                    ("package.json", r#"{"packageManager":"bun@1.2.0","scripts":{"test":"x"}}"#),
                    ("package-lock.json", ""),
                ],
                Some(("bun run test", "package.json packageManager")),
            ),
            (
                "unknown packageManager falls back to the lockfile",
                &[
                    ("package.json", r#"{"packageManager":"deno@2","scripts":{"test":"x"}}"#),
                    ("yarn.lock", ""),
                ],
                Some(("yarn test", "yarn.lock")),
            ),
            (
                "bun lock beats the other lockfiles",
                &[("package.json", TEST), ("package-lock.json", ""), ("bun.lock", "")],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "bun without a test script runs bun test when test files exist",
                &[("package.json", NO_TEST), ("bun.lock", ""), ("src/math.test.ts", "")],
                Some(("bun test", "bun.lock, test files")),
            ),
            (
                "bun test files in a nested folder",
                &[("package.json", "{}"), ("bun.lock", ""), ("packages/a/src/b_spec.js", "")],
                Some(("bun test", "bun.lock, test files")),
            ),
            (
                "bun without a test script or test files",
                &[("package.json", NO_TEST), ("bun.lock", ""), ("src/index.ts", "")],
                None,
            ),
            (
                "test files under node_modules do not count",
                &[("package.json", NO_TEST), ("bun.lock", ""), ("node_modules/x/a.test.js", "")],
                None,
            ),
            (
                "npm without a test script ignores test files",
                &[("package.json", NO_TEST), ("package-lock.json", ""), ("a.test.ts", "")],
                None,
            ),
            ("deno alone", &[("deno.json", "{}")], Some(("deno test", "deno.json"))),
            ("deno jsonc", &[("deno.jsonc", "{}")], Some(("deno test", "deno.jsonc"))),
            (
                "deno with a test task",
                &[("deno.json", DENO_TASK)],
                Some(("deno task test", "deno.json test task")),
            ),
            (
                "deno jsonc with comments and a trailing comma",
                &[(
                    "deno.jsonc",
                    "{\n  // run everything\n  \"tasks\": { \"test\": \"deno test\", /* x */ },\n}\n",
                )],
                Some(("deno task test", "deno.jsonc test task")),
            ),
            (
                "a test string inside a comment is not a task",
                &[("deno.jsonc", "{ \"name\": \"a//b\" // \"test\"\n}")],
                Some(("deno test", "deno.jsonc")),
            ),
            (
                "bun project with a leftover deno.json",
                &[("package.json", TEST), ("bun.lock", ""), ("deno.json", "{}")],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "bun project without a test script and a plain deno.json",
                &[
                    ("package.json", NO_TEST),
                    ("bun.lock", ""),
                    ("deno.json", "{}"),
                    ("a.test.ts", ""),
                ],
                Some(("bun test", "bun.lock, test files")),
            ),
            (
                "bun project without a test script, test files or a deno test task",
                &[("package.json", NO_TEST), ("bun.lock", ""), ("deno.json", "{}")],
                None,
            ),
            (
                "deno test task beats a package.json with no test script",
                &[("package.json", NO_TEST), ("bun.lock", ""), ("deno.json", DENO_TASK)],
                Some(("deno task test", "deno.json test task")),
            ),
            (
                "package.json test script beats a deno test task",
                &[("package.json", TEST), ("bun.lock", ""), ("deno.json", DENO_TASK)],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "the repo root decides over a nested deno project",
                &[("package.json", TEST), ("bun.lock", ""), ("packages/api/deno.json", DENO_TASK)],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "the repo root decides over a nested package",
                &[("deno.json", "{}"), ("web/package.json", TEST), ("web/bun.lock", "")],
                Some(("deno test", "deno.json")),
            ),
            (
                "a node project with no test script falls through to later markers",
                &[("package.json", NO_TEST), ("deno.json", "{}"), ("go.mod", "module x")],
                Some(("go test ./...", "go.mod")),
            ),
            ("cargo", &[("Cargo.toml", "")], Some(("cargo test", "Cargo.toml"))),
            ("go", &[("go.mod", "module x")], Some(("go test ./...", "go.mod"))),
            ("pytest.ini", &[("pytest.ini", "")], Some(("pytest", "pytest.ini"))),
            (
                "pyproject",
                &[("pyproject.toml", "[tool.pytest.ini_options]")],
                Some(("pytest", "pyproject.toml")),
            ),
            ("setup.cfg", &[("setup.cfg", "[tool:pytest]")], Some(("pytest", "setup.cfg"))),
            ("make", &[("Makefile", "test:\n\t./t\n")], Some(("make test", "Makefile"))),
        ];
        for (name, files, expected) in cases {
            let dir = repo_with(files);
            let found = detect_with_reason(dir.path());
            let got = found.as_ref().map(|d| (d.command.as_str(), d.why.as_str()));
            assert_eq!(got, *expected, "{name}");
        }
    }

    #[test]
    fn a_lockfile_decides_the_javascript_runtime_before_deno_json() {
        const TEST: &str = r#"{"scripts":{"test":"vitest"}}"#;
        const DENO_TASK: &str = r#"{"tasks":{"test":"deno test -A"}}"#;
        let cases: &[Case] = &[
            (
                "bun.lock with deno.json",
                &[("package.json", TEST), ("bun.lock", ""), ("deno.json", DENO_TASK)],
                Some(("bun run test", "bun.lock")),
            ),
            (
                "bun.lockb with deno.jsonc",
                &[("package.json", TEST), ("bun.lockb", ""), ("deno.jsonc", DENO_TASK)],
                Some(("bun run test", "bun.lockb")),
            ),
            (
                "pnpm-lock with deno.json",
                &[("package.json", TEST), ("pnpm-lock.yaml", ""), ("deno.json", DENO_TASK)],
                Some(("pnpm test", "pnpm-lock.yaml")),
            ),
            (
                "yarn.lock with deno.json",
                &[("package.json", TEST), ("yarn.lock", ""), ("deno.json", DENO_TASK)],
                Some(("yarn test", "yarn.lock")),
            ),
            (
                "package-lock with deno.json",
                &[("package.json", TEST), ("package-lock.json", ""), ("deno.json", DENO_TASK)],
                Some(("npm test", "package-lock.json")),
            ),
            (
                "bun.lock beside a deno.json and no package.json",
                &[("bun.lock", ""), ("deno.json", DENO_TASK)],
                Some(("bun test", "bun.lock")),
            ),
            (
                "pnpm-lock beside a deno.json and no package.json",
                &[("pnpm-lock.yaml", ""), ("deno.json", "{}")],
                Some(("pnpm test", "pnpm-lock.yaml")),
            ),
            (
                "bun lock with test files beats a deno test task",
                &[
                    ("package.json", "{}"),
                    ("bun.lock", ""),
                    ("deno.json", DENO_TASK),
                    ("a.test.ts", ""),
                ],
                Some(("bun test", "bun.lock, test files")),
            ),
        ];
        for (name, files, expected) in cases {
            let dir = repo_with(files);
            let found = detect_with_reason(dir.path());
            let got = found.as_ref().map(|d| (d.command.as_str(), d.why.as_str()));
            assert_eq!(got, *expected, "{name}");
        }
    }

    fn only(programs: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |program| programs.contains(&program)
    }

    fn gate(worktree: Files, repo: Files, installed: &'static [&'static str]) -> Gate {
        let (worktree, repo) = (repo_with(worktree), repo_with(repo));
        resolve_with(worktree.path(), repo.path(), only(installed))
    }

    fn run(command: &str, why: &str) -> Gate {
        Gate::Run(Detected::new(command, why))
    }

    #[test]
    fn a_missing_runtime_for_the_lockfile_falls_back_to_deno() {
        const FILES: Files = &[("bun.lock", ""), ("deno.json", r#"{"tasks":{"test":"x"}}"#)];
        assert_eq!(gate(FILES, &[], &["bun", "deno"]), run("bun test", "bun.lock"));
        assert_eq!(gate(FILES, &[], &["deno"]), run("deno task test", "deno.json test task"));
    }

    #[test]
    fn the_worktree_decides_not_the_main_checkout() {
        const BUN: Files = &[("package.json", r#"{"scripts":{"test":"x"}}"#), ("bun.lock", "")];
        const DENO: Files = &[("deno.json", r#"{"tasks":{"test":"deno test"}}"#)];
        let all = &["bun", "deno", "cargo"];
        assert_eq!(gate(BUN, DENO, all), run("bun run test", "bun.lock"));
        assert_eq!(
            gate(DENO, BUN, all),
            run("deno task test", "deno.json test task"),
            "the worktree wins in both directions"
        );
        assert_eq!(gate(&[], DENO, all), Gate::None, "a main checkout's markers are never read");
        assert_eq!(gate(&[("Cargo.toml", "")], BUN, all), run("cargo test", "Cargo.toml"));
    }

    #[test]
    fn the_worktrees_config_wins_and_the_main_checkouts_is_only_a_fallback() {
        let cases: &[(&str, Files, Files, Gate)] = &[
            (
                "worktree config",
                &[(".tidewatch.toml", "test = \"just ci\"\n")],
                &[(".tidewatch.toml", "test = \"make old\"\n")],
                run("just ci", REPO_CONFIG),
            ),
            (
                "main checkout config when the worktree has none",
                &[("Cargo.toml", "")],
                &[(".tidewatch.toml", "test = \"make old\"\n")],
                run("make old", REPO_CONFIG),
            ),
            (
                "a worktree config that switches tests off",
                &[(".tidewatch.toml", "test = \"\"\n"), ("Cargo.toml", "")],
                &[(".tidewatch.toml", "test = \"make old\"\n")],
                Gate::None,
            ),
            (
                "a worktree config without test detects from the worktree",
                &[(".tidewatch.toml", "house_style = false\n"), ("Cargo.toml", "")],
                &[(".tidewatch.toml", "test = \"make old\"\n")],
                run("cargo test", "Cargo.toml"),
            ),
        ];
        for (name, worktree, repo, want) in cases {
            assert_eq!(gate(worktree, repo, &["cargo"]), *want, "{name}");
        }
    }

    #[test]
    fn a_command_whose_program_is_missing_falls_through_to_the_next_marker() {
        const FILES: Files = &[
            ("package.json", r#"{"scripts":{"test":"x"}}"#),
            ("pnpm-lock.yaml", ""),
            ("deno.json", r#"{"tasks":{"test":"deno test"}}"#),
            ("go.mod", "module x"),
            ("Makefile", "test:\n\t./t\n"),
        ];
        let cases: &[(&str, &'static [&'static str], Gate)] = &[
            ("pnpm installed", &["pnpm", "deno", "go", "make"], run("pnpm test", "pnpm-lock.yaml")),
            (
                "no pnpm falls to deno",
                &["deno", "go", "make"],
                run("deno task test", "deno.json test task"),
            ),
            ("no pnpm or deno falls to go", &["go", "make"], run("go test ./...", "go.mod")),
            ("only make", &["make"], run("make test", "Makefile")),
        ];
        for (name, installed, want) in cases {
            assert_eq!(gate(FILES, &[], installed), *want, "{name}");
        }
    }

    #[test]
    fn nothing_runnable_is_a_skip_that_names_the_missing_programs() {
        let deno: Files = &[("deno.json", r#"{"tasks":{"test":"deno test"}}"#)];
        assert_eq!(
            gate(deno, &[], &[]),
            Gate::Skip("skipped: no runnable test command (deno not installed)".to_owned())
        );
        let both: Files = &[("deno.json", "{}"), ("go.mod", "module x")];
        assert_eq!(
            gate(both, &[], &["cargo"]),
            Gate::Skip("skipped: no runnable test command (deno, go not installed)".to_owned())
        );
        assert_eq!(gate(&[("README.md", "")], &[], &[]), Gate::None);
    }

    #[test]
    fn a_configured_command_is_never_passed_over_for_a_missing_program() {
        let configured: Files = &[(".tidewatch.toml", "test = \"deno task ci\"\n")];
        assert_eq!(gate(configured, &[], &[]), run("deno task ci", REPO_CONFIG));
    }

    #[test]
    fn the_timeout_comes_from_the_worktrees_config_before_the_main_checkouts() {
        let worktree = repo_with(&[(".tidewatch.toml", "test_timeout_secs = 30\n")]);
        let repo = repo_with(&[(".tidewatch.toml", "test_timeout_secs = 90\n")]);
        let bare = repo_with(&[]);
        assert_eq!(timeout_in(worktree.path(), repo.path()), Duration::from_secs(30));
        assert_eq!(timeout_in(bare.path(), repo.path()), Duration::from_secs(90));
        assert_eq!(timeout_in(bare.path(), bare.path()), DEFAULT_TIMEOUT);
    }

    #[test]
    fn programs_are_found_on_path_only_when_executable() {
        use std::os::unix::fs::PermissionsExt;
        let bin = repo_with(&[("tool", "#!/bin/sh\n"), ("plain", "data")]);
        std::fs::set_permissions(bin.path().join("tool"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let tool = bin.path().join("tool");
        assert!(program_on_path(tool.to_str().unwrap()));
        assert!(!program_on_path(bin.path().join("plain").to_str().unwrap()));
        assert!(!program_on_path(bin.path().join("gone").to_str().unwrap()));
        assert!(program_on_path("sh"));
        assert!(!program_on_path("definitely-not-a-program-xyz"));
    }

    #[test]
    fn a_detection_reads_as_command_and_reason() {
        let dir = repo_with(&[("package.json", "{}"), ("bun.lock", ""), ("a.test.ts", "")]);
        assert_eq!(
            detect_with_reason(dir.path()).unwrap().describe(),
            "bun test (bun.lock, test files)"
        );
    }

    #[test]
    fn the_repo_config_is_the_reason_when_it_sets_the_command() {
        let dir = repo_with(&[("deno.json", "{}"), (".tidewatch.toml", "test = \"bun test\"\n")]);
        assert_eq!(plan(dir.path()).unwrap().describe(), "bun test (.tidewatch.toml)");
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
        assert!(run.output.contains("longer than 300 ms and was stopped"), "{}", run.output);
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
}
