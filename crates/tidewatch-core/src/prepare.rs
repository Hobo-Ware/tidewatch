//! Gets an agent tab ready to start: the files its CLI reads, the never-push helper, the command and
//! the environment, and the one shell line that runs setup and then the agent.
//!
//! Blocks on disk and one `git` call, so run it off the UI thread. Missing extras (house style, the
//! guard helper, Gemini settings) never stop a launch; each one adds a note the UI can show. Only
//! what `launch_command` refuses stops it.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agents::{Agent, Wiring};
use crate::fsutil::write_atomic;
use crate::house_style;
use crate::launch::{
    GEMINI_SETTINGS_PATH, HookListener, Launch, LaunchCtx, LaunchError, ensure_no_push_helper,
    gemini_exclude_lines, gemini_project_settings, launch_command, mcp_config_json,
};
use crate::paths::TidewatchPaths;
use crate::profiles::Profile;

const MCP_CONFIG_FILE: &str = "mcp.json";
const FALLBACK_NOTICE: &str =
    "Tidewatch: setup failed or the agent could not start. Here is a shell.";

pub struct PrepareInput<'a> {
    pub paths: &'a TidewatchPaths,
    pub agent: &'a Agent,
    pub profile: &'a Profile,
    /// The repo, or the folder for a tab in Other. Keys the tab's state directory with `tab`.
    pub repo: &'a Path,
    pub tab: &'a str,
    /// Where the agent runs: the worktree, or the folder in Other.
    pub dir: &'a Path,
    /// An agent worktree tab. Only these get YOLO, the never-push guard and generated project files.
    pub worktree_tab: bool,
    pub task: &'a str,
    pub session_id: Option<String>,
    pub path_var: Option<&'a OsStr>,
    pub listener: Option<HookListener>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub launch: Launch,
    /// The environment for the pty: `PATH` from the input, overridden by what the launch sets.
    pub env: Vec<(String, String)>,
    pub setup: String,
    /// Things that did not work and were skipped, for the UI to mention.
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareError {
    Launch(LaunchError),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PrepareError {}

impl From<LaunchError> for PrepareError {
    fn from(e: LaunchError) -> Self {
        Self::Launch(e)
    }
}

impl Prepared {
    /// The line to run in the user's `shell` as `-c`: the setup commands, then the agent in place of
    /// the shell, so the pty ends with the agent and its exit code is the agent's. When setup fails or
    /// the agent cannot start, the tab falls back to an interactive shell with the error above it.
    #[must_use]
    pub fn script(&self, shell: &str) -> String {
        shell_script(&self.setup, &self.launch, shell)
    }
}

pub fn prepare(input: &PrepareInput) -> Result<Prepared, PrepareError> {
    let bare = LaunchCtx { worktree_tab: input.worktree_tab, ..LaunchCtx::default() };
    launch_command(input.agent, input.profile, &bare)?;

    let mut notes = Vec::new();
    let paths = input.paths;
    let wiring = input.agent.wiring;
    let mut soft = |what: &str, result: std::io::Result<PathBuf>| match result {
        Ok(path) => Some(path),
        Err(e) => {
            notes.push(format!("{what}: {e}"));
            None
        }
    };

    let house_style = input
        .profile
        .house_style
        .then(|| soft("house style", house_style::ensure(paths)))
        .flatten();
    let copilot_instructions_dir = (house_style.is_some() && wiring == Wiring::Copilot)
        .then(|| soft("Copilot instructions", house_style::sync_copilot_instructions(paths)))
        .flatten();
    let guard_bin_dir = (input.worktree_tab && input.profile.never_push)
        .then(|| {
            let dir = paths.guard_bin_dir();
            soft("push guard helper", ensure_no_push_helper(&dir).map(|_| dir))
        })
        .flatten();

    let mcp_path = paths.tab_state_dir(input.repo, input.tab).join(MCP_CONFIG_FILE);
    let mcp_config = input.listener.as_ref().map(|listener| {
        if let Some(json) = mcp_config_json(wiring, listener) {
            soft("MCP config", write_atomic(&mcp_path, &json).map(|()| mcp_path.clone()));
        }
        mcp_path.clone()
    });

    let ctx = LaunchCtx {
        worktree: input.dir.to_path_buf(),
        worktree_tab: input.worktree_tab,
        session_id: input.session_id.clone(),
        hooks: input.listener.clone(),
        mcp_config,
        house_style,
        copilot_instructions_dir,
        guard_bin_dir,
        env: input
            .path_var
            .map(|path| vec![("PATH".to_owned(), path.to_string_lossy().into_owned())])
            .unwrap_or_default(),
    };
    let mut launch = launch_command(input.agent, input.profile, &ctx)?;

    if wiring == Wiring::Gemini
        && input.worktree_tab
        && let Some(settings) = gemini_project_settings(input.profile, &ctx)
        && let Some(note) = write_gemini_settings(input.dir, &settings)
    {
        notes.push(note);
    }

    launch.args.extend(task_args(wiring, input.task));
    let env = merge_env(&ctx.env, &launch.env);
    Ok(Prepared { launch, env, setup: input.profile.setup.clone(), notes })
}

/// How each CLI takes its first prompt. Copilot and Gemini have a flag that starts interactive mode
/// with it; Claude and Codex take a positional prompt, after `--` so list-valued flags do not swallow
/// it. A custom agent gets it as a plain trailing argument. An empty task adds nothing.
#[must_use]
pub fn task_args(wiring: Wiring, task: &str) -> Vec<String> {
    let task = task.trim();
    if task.is_empty() {
        return Vec::new();
    }
    let task = task.to_owned();
    match wiring {
        Wiring::Claude | Wiring::Codex => vec!["--".to_owned(), task],
        Wiring::Gemini => vec!["--prompt-interactive".to_owned(), task],
        Wiring::Copilot => vec!["-i".to_owned(), task],
        Wiring::Custom => vec![task],
    }
}

fn merge_env(base: &[(String, String)], over: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        base.iter().filter(|(k, _)| !over.iter().any(|(ok, _)| ok == k)).cloned().collect();
    env.extend(over.iter().cloned());
    env
}

fn write_gemini_settings(dir: &Path, settings: &str) -> Option<String> {
    let file = dir.join(GEMINI_SETTINGS_PATH);
    if file.exists() {
        return Some(format!(
            "{GEMINI_SETTINGS_PATH} already exists in this repo; Gemini keeps its own settings"
        ));
    }
    if let Err(e) = write_atomic(&file, settings) {
        return Some(format!("Gemini settings: {e}"));
    }
    exclude_from_git(dir, &gemini_exclude_lines()).err().map(|e| format!("git exclude: {e}"))
}

/// Adds `lines` to the repo's `info/exclude` unless they are there. For a linked worktree git keeps
/// that file in the main repo, so the lines apply to every checkout of it.
fn exclude_from_git(dir: &Path, lines: &[&str]) -> std::io::Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_owned()));
    }
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let file =
        if Path::new(&printed).is_absolute() { PathBuf::from(printed) } else { dir.join(printed) };
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let missing: Vec<&str> =
        lines.iter().copied().filter(|line| !current.lines().any(|l| l == *line)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    write_atomic(&file, &text)
}

/// See `Prepared::script`. Setup lines are chained with `&&`, so the first failure stops the rest.
/// The agent is looked up before `exec`: zsh ends the whole script when `exec` cannot find its
/// program, which would skip the shell fallback.
#[must_use]
pub fn shell_script(setup: &str, launch: &Launch, shell: &str) -> String {
    let found =
        format!("command -v {} >/dev/null 2>&1", crate::launch::shell_quote(&launch.program));
    let run = format!("{found} && exec {}", launch.shell_line());
    let setup: Vec<&str> = setup.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let head = if setup.is_empty() { run } else { format!("{} && {run}", setup.join(" && ")) };
    let quoted_shell = crate::launch::shell_quote(shell);
    let notice = crate::launch::shell_quote(FALLBACK_NOTICE);
    format!("{head}; echo; echo {notice}; exec {quoted_shell} -l -i")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentTable, ArgList, Level};
    use crate::launch::{NO_PUSH_MESSAGE, mcp_config_json};

    fn profile(agent: &str, level: Level) -> Profile {
        Profile {
            name: "p".into(),
            agent: agent.into(),
            model: None,
            effort: None,
            level,
            setup: String::new(),
            never_push: true,
            extra_args: ArgList::default(),
            house_style: true,
        }
    }

    struct Fixture {
        _home: tempfile::TempDir,
        paths: TidewatchPaths,
        dir: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let home_path = home.path().canonicalize().unwrap();
            let dir = home_path.join("wt");
            std::fs::create_dir_all(&dir).unwrap();
            Self { paths: TidewatchPaths::new(&home_path), dir, _home: home }
        }

        fn run(
            &self,
            agent: &str,
            profile: &Profile,
            worktree_tab: bool,
            task: &str,
            listener: Option<HookListener>,
        ) -> Result<Prepared, PrepareError> {
            let table = AgentTable::builtin();
            prepare(&PrepareInput {
                paths: &self.paths,
                agent: table.get(agent).unwrap(),
                profile,
                repo: Path::new("/work/web"),
                tab: "brave-otter",
                dir: &self.dir,
                worktree_tab,
                task,
                session_id: None,
                path_var: Some(OsStr::new("/opt/homebrew/bin:/usr/bin:/bin")),
                listener,
            })
        }
    }

    fn listener() -> HookListener {
        HookListener { base_url: "http://127.0.0.1:5555".into(), token: "tok".into() }
    }

    fn env_of<'a>(prepared: &'a Prepared, key: &str) -> Option<&'a str> {
        prepared.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_builder_launch_has_the_level_flag_the_house_style_and_the_task_last() {
        let fx = Fixture::new();
        let prepared =
            fx.run("claude", &profile("claude", Level::Edits), true, "fix the bug", None).unwrap();
        let house = fx.paths.house_style_file();
        assert!(house.is_file(), "house-style.md is created on first launch");
        let args = &prepared.launch.args;
        assert_eq!(&args[..2], ["--permission-mode", "acceptEdits"]);
        let flag = args.iter().position(|a| a == "--append-system-prompt-file").unwrap();
        assert_eq!(args[flag + 1], house.display().to_string());
        assert_eq!(&args[args.len() - 2..], ["--", "fix the bug"]);
        assert_eq!(prepared.notes, Vec::<String>::new());
    }

    #[test]
    fn the_guard_helper_is_written_and_path_puts_it_first_in_front_of_the_given_path() {
        let fx = Fixture::new();
        let prepared = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        let helper = fx.paths.guard_bin_dir().join(crate::launch::NO_PUSH_HELPER_NAME);
        assert!(helper.is_file());
        let path = env_of(&prepared, "PATH").unwrap();
        assert_eq!(
            path,
            format!("{}:/opt/homebrew/bin:/usr/bin:/bin", fx.paths.guard_bin_dir().display())
        );
        assert_eq!(env_of(&prepared, "GIT_CONFIG_COUNT"), Some("1"));
    }

    #[test]
    fn without_never_push_or_outside_a_worktree_there_is_no_guard_and_path_is_the_given_one() {
        let fx = Fixture::new();
        let mut open = profile("claude", Level::Edits);
        open.never_push = false;
        let prepared = fx.run("claude", &open, true, "", None).unwrap();
        assert_eq!(env_of(&prepared, "GIT_CONFIG_COUNT"), None);
        assert_eq!(env_of(&prepared, "PATH"), Some("/opt/homebrew/bin:/usr/bin:/bin"));
        assert!(!fx.paths.guard_bin_dir().exists());

        let other = fx.run("claude", &profile("claude", Level::Edits), false, "", None).unwrap();
        assert_eq!(env_of(&other, "GIT_CONFIG_COUNT"), None);
        assert!(!fx.paths.guard_bin_dir().exists());
    }

    #[test]
    fn yolo_outside_a_worktree_is_refused_before_anything_is_written() {
        let fx = Fixture::new();
        let result = fx.run("claude", &profile("claude", Level::Yolo), false, "", None);
        assert_eq!(result, Err(PrepareError::Launch(LaunchError::YoloOutsideWorktree)));
        assert!(!fx.paths.config_dir().exists());
        assert!(!fx.paths.state_dir().exists());
        let allowed = fx.run("claude", &profile("claude", Level::Yolo), true, "", None).unwrap();
        assert!(allowed.launch.args.contains(&"--dangerously-skip-permissions".to_owned()));
    }

    #[test]
    fn house_style_off_adds_no_file_and_no_flag() {
        let fx = Fixture::new();
        let mut plain = profile("claude", Level::Edits);
        plain.house_style = false;
        let prepared = fx.run("claude", &plain, true, "", None).unwrap();
        assert!(!prepared.launch.args.contains(&"--append-system-prompt-file".to_owned()));
        assert!(!fx.paths.house_style_file().exists());
    }

    #[test]
    fn a_house_style_that_cannot_be_written_becomes_a_note_not_a_failure() {
        let fx = Fixture::new();
        std::fs::create_dir_all(fx.paths.config_dir().parent().unwrap()).unwrap();
        std::fs::write(fx.paths.config_dir(), "a file where the folder should be").unwrap();
        let prepared = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        assert!(!prepared.launch.args.contains(&"--append-system-prompt-file".to_owned()));
        assert_eq!(prepared.notes.len(), 1);
        assert!(prepared.notes[0].starts_with("house style:"), "{:?}", prepared.notes);
    }

    #[test]
    fn copilot_gets_the_derived_instructions_dir_and_takes_its_task_with_i() {
        let fx = Fixture::new();
        let prepared =
            fx.run("copilot", &profile("copilot", Level::Edits), true, "add tests", None).unwrap();
        let dir = fx.paths.copilot_instructions_dir();
        assert!(dir.join("tidewatch.instructions.md").is_file());
        assert_eq!(
            env_of(&prepared, "COPILOT_CUSTOM_INSTRUCTIONS_DIRS"),
            Some(dir.display().to_string().as_str())
        );
        assert_eq!(&prepared.launch.args[prepared.launch.args.len() - 2..], ["-i", "add tests"]);
    }

    #[test]
    fn each_cli_takes_its_task_the_way_it_documents() {
        let cases = [
            (Wiring::Claude, vec!["--", "do it"]),
            (Wiring::Codex, vec!["--", "do it"]),
            (Wiring::Gemini, vec!["--prompt-interactive", "do it"]),
            (Wiring::Copilot, vec!["-i", "do it"]),
            (Wiring::Custom, vec!["do it"]),
        ];
        for (wiring, want) in cases {
            assert_eq!(task_args(wiring, "  do it \n"), want, "{wiring:?}");
            assert_eq!(task_args(wiring, "  \n"), Vec::<String>::new(), "{wiring:?} empty");
        }
    }

    #[test]
    fn hooks_and_mcp_are_wired_only_when_a_listener_is_given() {
        let fx = Fixture::new();
        let none = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        assert!(
            !none.launch.args.iter().any(|a| a == "--settings" || a.starts_with("--mcp-config"))
        );

        let wired =
            fx.run("claude", &profile("claude", Level::Edits), true, "", Some(listener())).unwrap();
        let mcp = fx.paths.tab_state_dir(Path::new("/work/web"), "brave-otter").join("mcp.json");
        assert_eq!(
            std::fs::read_to_string(&mcp).unwrap(),
            mcp_config_json(Wiring::Claude, &listener()).unwrap()
        );
        assert!(wired.launch.args.contains(&format!("--mcp-config={}", mcp.display())));
        assert!(wired.launch.args.contains(&"--settings".to_owned()));
    }

    fn git(dir: &Path, args: &[&str]) {
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

    fn git_worktree(fx: &Fixture) -> PathBuf {
        let main = fx.dir.parent().unwrap().join("main");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let worktree = fx.dir.parent().unwrap().join("linked");
        git(&main, &["worktree", "add", "-q", "-b", "agent/x", worktree.to_str().unwrap()]);
        worktree
    }

    #[test]
    fn gemini_settings_are_written_once_and_excluded_from_git() {
        let fx = Fixture::new();
        let worktree = git_worktree(&fx);
        let fx = Fixture { dir: worktree.clone(), ..fx };
        let first = fx.run("gemini", &profile("gemini", Level::Edits), true, "", None).unwrap();
        assert_eq!(first.notes, Vec::<String>::new());
        let settings = std::fs::read_to_string(worktree.join(".gemini/settings.json")).unwrap();
        assert!(settings.contains("house-style.md"), "{settings}");
        assert_eq!(env_of(&first, "GEMINI_CLI_TRUST_WORKSPACE"), Some("true"));

        let status = Command::new("git")
            .arg("-C")
            .arg(&worktree)
            .args(["status", "--short"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout), "");

        std::fs::remove_file(worktree.join(".gemini/settings.json")).unwrap();
        fx.run("gemini", &profile("gemini", Level::Edits), true, "", None).unwrap();
        let exclude = Command::new("git")
            .arg("-C")
            .arg(&worktree)
            .args(["rev-parse", "--git-path", "info/exclude"])
            .output()
            .unwrap();
        let exclude = String::from_utf8_lossy(&exclude.stdout).trim().to_owned();
        let text = std::fs::read_to_string(exclude).unwrap();
        assert_eq!(text.lines().filter(|l| *l == "/.gemini/settings.json").count(), 1);
    }

    #[test]
    fn a_repo_that_has_its_own_gemini_settings_keeps_them() {
        let fx = Fixture::new();
        let worktree = git_worktree(&fx);
        let fx = Fixture { dir: worktree.clone(), ..fx };
        std::fs::create_dir_all(worktree.join(".gemini")).unwrap();
        std::fs::write(worktree.join(".gemini/settings.json"), "{\"theirs\":true}").unwrap();
        let prepared = fx.run("gemini", &profile("gemini", Level::Edits), true, "", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(worktree.join(".gemini/settings.json")).unwrap(),
            "{\"theirs\":true}"
        );
        assert_eq!(prepared.notes.len(), 1);
        assert!(prepared.notes[0].contains("already exists"));
    }

    #[test]
    fn nothing_is_written_into_a_folder_that_is_not_a_worktree_tab() {
        let fx = Fixture::new();
        fx.run("gemini", &profile("gemini", Level::Edits), false, "", None).unwrap();
        assert!(!fx.dir.join(".gemini").exists());
    }

    #[test]
    fn the_script_runs_setup_then_replaces_the_shell_with_the_agent_and_falls_back_to_a_shell() {
        let fx = Fixture::new();
        let mut with_setup = profile("claude", Level::Edits);
        with_setup.setup = "npm ci\n\n  npm run build  ".into();
        let prepared = fx.run("claude", &with_setup, true, "it's done", None).unwrap();
        let script = prepared.script("/bin/zsh");
        assert!(
            script.starts_with(
                "npm ci && npm run build && command -v claude >/dev/null 2>&1 && exec claude --permission-mode acceptEdits"
            ),
            "{script}"
        );
        assert!(script.contains("-- 'it'\\''s done'"), "{script}");
        assert!(script.ends_with("; echo; echo 'Tidewatch: setup failed or the agent could not start. Here is a shell.'; exec /bin/zsh -l -i"), "{script}");

        let plain = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        assert!(
            plain
                .script("/bin/zsh")
                .starts_with("command -v claude >/dev/null 2>&1 && exec claude ")
        );
    }

    #[test]
    fn a_failing_setup_never_starts_the_agent_and_lands_in_a_shell() {
        let launch = Launch {
            program: "echo".into(),
            args: vec!["AGENT-RAN".into()],
            env: Vec::new(),
            experimental: false,
        };
        let script =
            shell_script("false", &launch, "/bin/sh").replace("exec /bin/sh -l -i", "echo SHELL");
        let out = Command::new("/bin/sh").args(["-c", &script]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!text.contains("AGENT-RAN"), "{text}");
        assert!(text.contains("SHELL"), "{text}");
        let script = shell_script("true", &launch, "/bin/sh");
        let out = Command::new("/bin/sh").args(["-c", &script]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "AGENT-RAN\n");
    }

    #[test]
    fn a_missing_agent_binary_falls_through_to_the_shell_in_zsh_and_bash() {
        let launch = Launch {
            program: "tidewatch-no-such-agent".into(),
            args: Vec::new(),
            env: Vec::new(),
            experimental: false,
        };
        for shell in ["/bin/zsh", "/bin/bash"] {
            if !Path::new(shell).exists() {
                continue;
            }
            let script =
                shell_script("", &launch, shell).replace("-l -i", "-c 'echo FELL-THROUGH'");
            let out = Command::new(shell).args(["-i", "-c", &script]).output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("FELL-THROUGH"),
                "{shell}: {text} / {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn a_push_through_the_prepared_environment_is_refused_with_the_tidewatch_message() {
        if !Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success()) {
            return;
        }
        let fx = Fixture::new();
        let base = fx.dir.parent().unwrap();
        let remote = base.join("remote.git");
        git(base, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let clone = base.join("clone");
        git(base, &["clone", "-q", remote.to_str().unwrap(), clone.to_str().unwrap()]);
        git(&clone, &["commit", "-q", "--allow-empty", "-m", "work"]);
        let fx = Fixture { dir: clone.clone(), ..fx };
        let path = std::env::var("PATH").unwrap();
        let table = AgentTable::builtin();
        let prepared = prepare(&PrepareInput {
            paths: &fx.paths,
            agent: table.get("claude").unwrap(),
            profile: &profile("claude", Level::Edits),
            repo: Path::new("/work/web"),
            tab: "brave-otter",
            dir: &clone,
            worktree_tab: true,
            task: "",
            session_id: None,
            path_var: Some(OsStr::new(&path)),
            listener: None,
        })
        .unwrap();
        for push in ["git push origin main", "git push --no-verify origin main"] {
            let out = Command::new("/bin/sh")
                .args(["-c", push])
                .current_dir(&clone)
                .envs(prepared.env.iter().map(|(k, v)| (k, v)))
                .output()
                .unwrap();
            assert!(!out.status.success(), "{push} must fail");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains(NO_PUSH_MESSAGE), "{push}: {stderr}");
        }
        let refs = Command::new("git").arg("-C").arg(&remote).arg("for-each-ref").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&refs.stdout), "");
    }
}
