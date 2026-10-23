//! The outbound guards end to end: the real shims, the real `tidewatch --guard` binary, real git in a
//! throwaway repo and a fake `gh` that logs its calls. The real `gh` never runs.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tidewatch_core::guard_shim::GuardCtx;
use tidewatch_core::guards::{GuardPrefs, Rule};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;

const FAKE_GH: &str = r#"#!/bin/sh
echo "$*" >> "$FAKE_GH_LOG"
case "$*" in
  *"--json visibility"*) cat "$FAKE_GH_VISIBILITY"; exit 0 ;;
esac
exit 0
"#;

const RECORDER: &str = "#!/bin/sh\necho \"$*\" >> \"$FAKE_GUARD_LOG\"\nexit 0\n";

const EM: &str = "\u{2014}";

struct Lab {
    root: tempfile::TempDir,
    paths: TidewatchPaths,
    repo: PathBuf,
    shims: PathBuf,
    fake_bin: PathBuf,
}

fn executable(file: &Path, content: &str) {
    let source = file.with_extension("src");
    fs::write(&source, content).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Command::new("cp").arg(&source).arg(file).status().unwrap().success());
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn guard_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tidewatch"))
}

impl Lab {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(&root.path().join("home"));
        let repo = root.path().join("web");
        let shims = root.path().join("shims");
        let fake_bin = root.path().join("fake-bin");
        for dir in [&repo, &shims, &fake_bin, &paths.config_dir().to_path_buf()] {
            fs::create_dir_all(dir).unwrap();
        }
        for (name, content) in tidewatch_core::guard_shim::shims() {
            executable(&shims.join(name), &content);
        }
        executable(&fake_bin.join("gh"), FAKE_GH);
        fs::write(root.path().join("visibility"), "PUBLIC").unwrap();
        let lab = Self { root, paths, repo, shims, fake_bin };
        lab.plain("git", &["init", "-q"]);
        lab.plain("git", &["remote", "add", "origin", "https://github.com/acme/web.git"]);
        lab
    }

    fn path(&self, with_shims: bool) -> String {
        let mut dirs = Vec::new();
        if with_shims {
            dirs.push(self.shims.clone());
        }
        dirs.push(self.fake_bin.clone());
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        std::env::join_paths(dirs).unwrap().to_string_lossy().into_owned()
    }

    fn base(&self, program: &Path, path: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repo)
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.root.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .env("FAKE_GH_LOG", self.gh_log())
            .env("FAKE_GUARD_LOG", self.root.path().join("guard.log"))
            .env("FAKE_GH_VISIBILITY", self.root.path().join("visibility"));
        command
    }

    fn gh_log(&self) -> PathBuf {
        self.root.path().join("gh.log")
    }

    fn gh_calls(&self) -> Vec<String> {
        fs::read_to_string(self.gh_log()).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    fn plain(&self, tool: &str, args: &[&str]) -> Output {
        let output = self.base(Path::new(tool), &self.path(false)).args(args).output().unwrap();
        assert!(output.status.success(), "{tool} {args:?}: {}", text(&output.stderr));
        output
    }

    fn guarded(&self, tool: &str, bin: &Path) -> Command {
        let ctx = GuardCtx {
            bin: bin.to_path_buf(),
            config: self.paths.prefs_file(),
            repo: self.repo.clone(),
            cache_dir: self.paths.guard_cache_dir(),
            tab_dir: self.root.path().join("tab"),
        };
        let mut command = self.base(&self.shims.join(tool), &self.path(true));
        command.envs(ctx.env(&self.shims));
        command
    }

    fn run(&self, tool: &str, args: &[&str]) -> Output {
        self.guarded(tool, &guard_binary()).args(args).output().unwrap()
    }

    fn run_with_stdin(&self, tool: &str, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .guarded(tool, &guard_binary())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn settings(&self, change: impl FnOnce(&mut GuardPrefs)) {
        let mut prefs = Prefs::default();
        change(&mut prefs.guards);
        prefs.save(&self.paths).unwrap();
    }

    fn commits(&self) -> Vec<String> {
        let log = Command::new("git")
            .current_dir(&self.repo)
            .args(["log", "--format=%B", "--no-color"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        text(&log.stdout).lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()
    }
}

fn refused(output: &Output, rule: &str) {
    assert_eq!(output.status.code(), Some(1), "stderr: {}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(stderr.contains(&format!("tidewatch guard [{rule}]: ")), "{stderr}");
    assert!(!stderr.contains(EM), "{stderr}");
}

fn passed(output: &Output) {
    assert!(output.status.success(), "stderr: {}", text(&output.stderr));
}

#[test]
fn a_dashed_commit_message_is_refused_and_a_clean_one_passes() {
    let lab = Lab::new();
    let dashed = format!("fix: a {EM} b");
    let blocked = lab.run("git", &["commit", "--allow-empty", "-m", &dashed]);
    refused(&blocked, "dashes");
    assert!(text(&blocked.stderr).contains("line 1, column 8"), "{}", text(&blocked.stderr));
    assert_eq!(lab.commits(), Vec::<String>::new());

    passed(&lab.run("git", &["commit", "--allow-empty", "-m", "fix: a - b"]));
    assert_eq!(lab.commits(), ["fix: a - b"]);
}

#[test]
fn a_message_file_and_a_message_on_stdin_are_checked_too() {
    let lab = Lab::new();
    let file = lab.root.path().join("msg.txt");
    fs::write(&file, format!("fix: x\n\nbecause {EM} reasons\n")).unwrap();
    refused(&lab.run("git", &["commit", "--allow-empty", "-F", file.to_str().unwrap()]), "dashes");

    let piped = lab.run_with_stdin(
        "git",
        &["commit", "--allow-empty", "-F", "-"],
        &format!("fix: y\n\nco {EM} op\n"),
    );
    refused(&piped, "dashes");
    assert_eq!(lab.commits(), Vec::<String>::new());

    passed(&lab.run_with_stdin(
        "git",
        &["commit", "--allow-empty", "-F", "-"],
        "fix: z\n\nall clean\n",
    ));
    assert_eq!(lab.commits(), ["fix: z", "all clean"]);
}

#[test]
fn trailers_and_footers_are_refused() {
    let lab = Lab::new();
    refused(
        &lab.run("git", &["commit", "--allow-empty", "-m", "fix: x\n\nCo-Authored-By: A <a@b.c>"]),
        "trailers",
    );
    refused(
        &lab.run(
            "gh",
            &["pr", "comment", "3", "-b", "done\n\n\u{1F916} Generated with Claude Code"],
        ),
        "trailers",
    );
    assert_eq!(lab.gh_calls(), Vec::<String>::new());
}

#[test]
fn a_switch_in_settings_applies_to_the_next_call_and_a_repo_can_answer_for_itself() {
    let lab = Lab::new();
    let dashed = format!("fix: a {EM} b");
    lab.settings(|guards| guards.rules.set(Rule::Dashes, false));
    passed(&lab.run("git", &["commit", "--allow-empty", "-m", &dashed]));

    lab.settings(|guards| guards.set_rule(Some(&lab.repo), Rule::Dashes, true));
    refused(&lab.run("git", &["commit", "--allow-empty", "-m", &dashed]), "dashes");

    lab.settings(|guards| {
        guards.set_rule(None, Rule::Dashes, true);
        guards.set_rule(Some(Path::new("/some/other/repo")), Rule::Dashes, false);
    });
    refused(&lab.run("git", &["commit", "--allow-empty", "-m", &dashed]), "dashes");
}

#[test]
fn github_text_is_checked_before_gh_runs_and_a_clean_call_reaches_it() {
    let lab = Lab::new();
    let title = format!("feat: add x {EM} y");
    refused(&lab.run("gh", &["pr", "create", "--title", &title, "--body", "ok"]), "dashes");
    refused(&lab.run("gh", &["issue", "comment", "4", "--body", &format!("a {EM} b")]), "dashes");
    let body = lab.root.path().join("body.md");
    fs::write(&body, format!("text {EM} more")).unwrap();
    refused(&lab.run("gh", &["pr", "edit", "4", "--body-file", body.to_str().unwrap()]), "dashes");
    assert_eq!(lab.gh_calls(), Vec::<String>::new());

    passed(&lab.run("gh", &["pr", "create", "--title", "feat: add x", "--body", "fine"]));
    assert_eq!(lab.gh_calls(), ["pr create --title feat: add x --body fine"]);
}

#[test]
fn drafts_and_failed_reruns_follow_their_switches() {
    let lab = Lab::new();
    passed(&lab.run("gh", &["pr", "create", "--title", "t", "--body", "b"]));

    lab.settings(|guards| guards.rules.set(Rule::DraftPrs, true));
    refused(&lab.run("gh", &["pr", "create", "--title", "t", "--body", "b"]), "draft_prs");
    passed(&lab.run("gh", &["pr", "create", "--draft", "--title", "t", "--body", "b"]));

    lab.settings(|_| {});
    refused(&lab.run("gh", &["run", "rerun", "99"]), "rerun_failed");
    passed(&lab.run("gh", &["run", "rerun", "99", "--failed"]));
    lab.settings(|guards| guards.rules.set(Rule::RerunFailed, false));
    passed(&lab.run("gh", &["run", "rerun", "99"]));
    assert_eq!(lab.gh_calls().len(), 4);
}

#[test]
fn private_names_are_refused_in_a_public_repo_only_and_visibility_is_asked_once() {
    let lab = Lab::new();
    lab.settings(|guards| guards.private_repos = vec!["acme/vault".to_owned()]);
    let args = ["pr", "comment", "3", "-R", "acme/web", "-b", "see acme/vault#9"];

    refused(&lab.run("gh", &args), "private_names");
    refused(&lab.run("gh", &args), "private_names");
    let asked = lab.gh_calls().iter().filter(|call| call.contains("--json visibility")).count();
    assert_eq!(asked, 1, "the second call reads the cache: {:?}", lab.gh_calls());

    refused(
        &lab.run("git", &["commit", "--allow-empty", "-m", "mention acme/vault"]),
        "private_names",
    );

    fs::remove_dir_all(lab.paths.guard_cache_dir()).unwrap();
    fs::write(lab.root.path().join("visibility"), "PRIVATE").unwrap();
    passed(&lab.run("gh", &args));
    passed(&lab.run("git", &["commit", "--allow-empty", "-m", "mention acme/vault"]));
}

#[test]
fn no_private_names_means_no_visibility_lookup_at_all() {
    let lab = Lab::new();
    passed(&lab.run("gh", &["pr", "comment", "3", "-R", "acme/web", "-b", "see acme/vault#9"]));
    assert_eq!(lab.gh_calls().len(), 1);
}

#[test]
fn outside_an_agent_tab_the_shims_do_nothing_extra() {
    let lab = Lab::new();
    let dashed = format!("fix: a {EM} b");
    let mut command = lab.base(&lab.shims.join("git"), &lab.path(true));
    let output = command.args(["commit", "--allow-empty", "-m", &dashed]).output().unwrap();
    passed(&output);
    assert_eq!(lab.commits(), [dashed]);
}

#[test]
fn a_shim_asks_the_guard_only_for_calls_that_can_matter() {
    let lab = Lab::new();
    let recorder = lab.root.path().join("recorder");
    executable(&recorder, RECORDER);
    let run = |tool: &str, args: &[&str]| {
        passed(&lab.guarded(tool, &recorder).args(args).output().unwrap());
    };
    run("git", &["status"]);
    run("git", &["rev-parse", "--git-dir"]);
    run("gh", &["repo", "view"]);
    run("gh", &["api", "user"]);
    assert!(!lab.root.path().join("guard.log").exists());
    run("git", &["commit", "-m", "x"]);
    run("gh", &["pr", "list"]);
    run("gh", &["issue", "list"]);
    run("gh", &["run", "list"]);
    let log = fs::read_to_string(lab.root.path().join("guard.log")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            "--guard git commit -m x",
            "--guard gh pr list",
            "--guard gh issue list",
            "--guard gh run list"
        ]
    );
}

#[test]
fn a_missing_guard_binary_falls_back_to_the_real_tool() {
    let lab = Lab::new();
    let dashed = format!("fix: a {EM} b");
    let output = lab
        .guarded("git", Path::new("/nonexistent/tidewatch"))
        .args(["commit", "--allow-empty", "-m", &dashed])
        .output()
        .unwrap();
    passed(&output);
    assert_eq!(lab.commits(), [dashed]);
}

#[test]
fn the_real_tool_is_found_past_the_shim_directory_whatever_it_looks_like_on_path() {
    let lab = Lab::new();
    let alias = lab.root.path().join("alias");
    std::os::unix::fs::symlink(&lab.shims, &alias).unwrap();
    let path = format!("{}/:{}:{}", lab.shims.display(), alias.display(), lab.path(false));
    let output = lab.base(&lab.shims.join("git"), &path).arg("--version").output().unwrap();
    passed(&output);
    assert!(text(&output.stdout).starts_with("git version"), "{}", text(&output.stdout));
}

#[test]
fn with_no_real_tool_on_path_the_shim_says_so() {
    let lab = Lab::new();
    let output = lab
        .base(&lab.shims.join("git"), &lab.shims.display().to_string())
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(127));
    assert!(text(&output.stderr).contains("no real git on PATH"), "{}", text(&output.stderr));
}

#[test]
fn the_flag_without_an_agent_environment_refuses_to_guess() {
    let output = Command::new(guard_binary())
        .args(["--guard", "git", "status"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
