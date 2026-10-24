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
use crate::checklist;
use crate::fsutil::write_atomic;
use crate::guard_shim::{GuardCtx, ensure_shims};
use crate::house_style;
use crate::launch::{
    GEMINI_SETTINGS_PATH, HookListener, Launch, LaunchCtx, LaunchError, ensure_no_push_helper,
    gemini_exclude_lines, gemini_project_settings, launch_command, mcp_config_json,
};
use crate::paths::TidewatchPaths;
use crate::profiles::Profile;
use crate::remote::session_name;
use crate::restore::{ResumeError, resume_command};
use crate::trust::TrustCli;
use crate::trust_apply::{Grant, Outcome};

const MCP_CONFIG_FILE: &str = "mcp.json";
pub const PROGRESS_ENV: &str = "TIDEWATCH_PROGRESS";
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
    pub remote: bool,
    pub meeseeks: bool,
    pub path_var: Option<&'a OsStr>,
    pub listener: Option<HookListener>,
    /// With consent (D14): the folder-trust entry to add for this worktree before the agent starts.
    pub trust: Option<&'a Grant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub launch: Launch,
    /// The environment for the pty: `PATH` from the input, overridden by what the launch sets.
    pub env: Vec<(String, String)>,
    pub setup: String,
    /// Things that did not work and were skipped, for the UI to mention.
    pub notes: Vec<String>,
    pub remote: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareError {
    Launch(LaunchError),
    Resume(ResumeError),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(e) => e.fmt(f),
            Self::Resume(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PrepareError {}

impl From<LaunchError> for PrepareError {
    fn from(e: LaunchError) -> Self {
        Self::Launch(e)
    }
}

impl From<ResumeError> for PrepareError {
    fn from(e: ResumeError) -> Self {
        Self::Resume(e)
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
    prepare_with(input, None)
}

/// [`prepare`] for a restored tab: the same files, environment and wiring, but the agent reopens
/// `session_id` (see `restore::resume_command`) and no task is added. `input.session_id` and
/// `input.task` are ignored. A Gemini tab's generated settings are rewritten, because they name
/// the listener of the run that wrote them.
pub fn prepare_resume(input: &PrepareInput, session_id: &str) -> Result<Prepared, PrepareError> {
    prepare_with(input, Some(session_id))
}

fn prepare_with(input: &PrepareInput, resume: Option<&str>) -> Result<Prepared, PrepareError> {
    let wanted_style = Profile { house_style: true, ..input.profile.clone() };
    let profile =
        if input.meeseeks && !input.profile.house_style { &wanted_style } else { input.profile };
    let bare = LaunchCtx { worktree_tab: input.worktree_tab, ..LaunchCtx::default() };
    launch_command(input.agent, profile, &bare)?;

    let mut notes = Vec::new();
    let paths = input.paths;
    let wiring = input.agent.wiring;
    if input.worktree_tab
        && let (Some(grant), Some(cli)) = (input.trust, TrustCli::from_wiring(wiring))
        && let Outcome::Failed(why) = grant.trust_worktree(cli, input.dir)
    {
        notes.push(format!("folder trust: {why}"));
    }

    let mut soft = |what: &str, result: std::io::Result<PathBuf>| match result {
        Ok(path) => Some(path),
        Err(e) => {
            notes.push(format!("{what}: {e}"));
            None
        }
    };

    let house_style =
        profile.house_style.then(|| soft("house style", house_style::ensure(paths))).flatten().map(
            |shared| soft("progress ledger", write_agent_style(input, &shared)).unwrap_or(shared),
        );
    let copilot_instructions_dir =
        house_style.as_ref().filter(|_| wiring == Wiring::Copilot).and_then(|file| {
            let dir = paths.tab_copilot_instructions_dir(input.repo, input.tab);
            let synced = std::fs::read_to_string(file)
                .and_then(|text| house_style::sync_copilot_instructions(&dir, &text));
            soft("Copilot instructions", synced)
        });
    let guard_bin_dir = input
        .worktree_tab
        .then(|| {
            let dir = paths.guard_bin_dir();
            let helper =
                if profile.never_push { ensure_no_push_helper(&dir).map(|_| ()) } else { Ok(()) };
            let shims = helper.and_then(|()| ensure_shims(&dir));
            soft("guard helpers", shims.map(|()| dir))
        })
        .flatten();

    let mcp_path = paths.tab_state_dir(input.repo, input.tab).join(MCP_CONFIG_FILE);
    let mcp_config = input.listener.as_ref().map(|listener| {
        if let Some(json) = mcp_config_json(wiring, listener) {
            soft("MCP config", write_atomic(&mcp_path, &json).map(|()| mcp_path.clone()));
        }
        mcp_path.clone()
    });

    let feed = paths.progress_feed_file(input.repo, input.tab);
    let feed_env = soft("progress feed", touch(&feed).map(|()| feed.clone()))
        .map(|file| (PROGRESS_ENV.to_owned(), file.display().to_string()));

    let ctx = context(input, house_style, copilot_instructions_dir, guard_bin_dir, mcp_config);
    let mut launch = match resume {
        Some(id) => resume_command(input.agent, profile, id, &ctx)?,
        None => launch_command(input.agent, profile, &ctx)?,
    };

    if wiring == Wiring::Gemini
        && input.worktree_tab
        && let Some(settings) = gemini_project_settings(profile, &ctx)
        && let Some(note) = write_gemini_settings(input.dir, &settings, resume.is_some())
    {
        notes.push(note);
    }

    if resume.is_none() {
        launch.args.extend(task_args(wiring, input.task));
    }
    let mut env = merge_env(&ctx.env, &launch.env);
    env.extend(feed_env);
    if input.worktree_tab {
        let recipe = crate::setup_recipe::recipe_for(paths, input.repo);
        for (key, value) in crate::setup_recipe::env_for(&recipe, input.repo) {
            if !env.iter().any(|(set, _)| *set == key) {
                env.push((key, value));
            }
        }
    }
    let remote = ctx.remote.filter(|name| !input.agent.remote_args(name).is_empty());
    Ok(Prepared { launch, env, setup: profile.setup.clone(), notes, remote })
}

fn context(
    input: &PrepareInput,
    house_style: Option<PathBuf>,
    copilot_instructions_dir: Option<PathBuf>,
    guard_bin_dir: Option<PathBuf>,
    mcp_config: Option<PathBuf>,
) -> LaunchCtx {
    LaunchCtx {
        worktree: input.dir.to_path_buf(),
        worktree_tab: input.worktree_tab,
        session_id: input.session_id.clone(),
        remote: input.remote.then(|| remote_name(input)),
        hooks: input.listener.clone(),
        mcp_config,
        house_style,
        copilot_instructions_dir,
        guard_bin_dir,
        push_branch: input.worktree_tab.then(|| checked_out_agent_branch(input.dir)).flatten(),
        guards: input.worktree_tab.then(|| guard_ctx(input.paths, input.repo, input.tab)).flatten(),
        env: input
            .path_var
            .map(|path| vec![("PATH".to_owned(), path.to_string_lossy().into_owned())])
            .unwrap_or_default(),
    }
}

fn guard_ctx(paths: &TidewatchPaths, repo: &Path, tab: &str) -> Option<GuardCtx> {
    Some(GuardCtx {
        bin: std::env::current_exe().ok()?,
        config: paths.prefs_file(),
        repo: repo.to_path_buf(),
        cache_dir: paths.guard_cache_dir(),
        tab_dir: paths.tab_state_dir(repo, tab),
    })
}

fn checked_out_agent_branch(dir: &Path) -> Option<String> {
    let dot_git = dir.join(".git");
    let git_dir = match std::fs::read_to_string(&dot_git) {
        Ok(text) => {
            let linked = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            if linked.is_absolute() { linked } else { dir.join(linked) }
        }
        Err(_) => dot_git,
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    branch.starts_with(crate::worktree::BRANCH_PREFIX).then(|| branch.to_owned())
}

fn remote_name(input: &PrepareInput) -> String {
    let repo = input.repo.file_name().map(|name| name.to_string_lossy());
    let repo = repo.as_deref().unwrap_or_default();
    session_name(repo, input.worktree_tab.then_some(input.tab))
}

/// The house style this agent reads: the user's file with the ledger format and the repo's
/// checklist appended, in the tab's state folder. The user's file is not changed.
fn write_agent_style(input: &PrepareInput, shared: &Path) -> std::io::Result<PathBuf> {
    let paths = input.paths;
    let _ = checklist::ensure(paths);
    let text = house_style::compose_for(
        &std::fs::read_to_string(shared)?,
        &checklist::for_repo(paths, input.repo),
        input.meeseeks,
    );
    let file = paths.tab_house_style_file(input.repo, input.tab);
    write_atomic(&file, &text)?;
    Ok(file)
}

/// The command `prepare` would build, for the new-tab preview: the same flags, with the helper and
/// house-style paths assumed to exist. Writes nothing. Codex reads the house style file when it exists.
pub fn preview(input: &PrepareInput) -> Result<Launch, LaunchError> {
    let paths = input.paths;
    let wiring = input.agent.wiring;
    let house_style =
        input.profile.house_style.then(|| paths.tab_house_style_file(input.repo, input.tab));
    let copilot_dir = (house_style.is_some() && wiring == Wiring::Copilot)
        .then(|| paths.tab_copilot_instructions_dir(input.repo, input.tab));
    let guard = (input.worktree_tab && input.profile.never_push).then(|| paths.guard_bin_dir());
    let mcp = input
        .listener
        .as_ref()
        .map(|_| paths.tab_state_dir(input.repo, input.tab).join(MCP_CONFIG_FILE));
    let ctx = context(input, house_style, copilot_dir, guard, mcp);
    let mut launch = launch_command(input.agent, input.profile, &ctx)?;
    launch.args.extend(task_args(wiring, input.task));
    Ok(launch)
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

fn touch(file: &Path) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(file).map(drop)
}

fn merge_env(base: &[(String, String)], over: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        base.iter().filter(|(k, _)| !over.iter().any(|(ok, _)| ok == k)).cloned().collect();
    env.extend(over.iter().cloned());
    env
}

fn write_gemini_settings(dir: &Path, settings: &str, refresh_generated: bool) -> Option<String> {
    let file = dir.join(GEMINI_SETTINGS_PATH);
    let ours = refresh_generated && !is_tracked(dir, GEMINI_SETTINGS_PATH);
    if file.exists() && !ours {
        return Some(format!(
            "{GEMINI_SETTINGS_PATH} already exists in this repo; Gemini keeps its own settings"
        ));
    }
    if let Err(e) = write_atomic(&file, settings) {
        return Some(format!("Gemini settings: {e}"));
    }
    exclude_from_git(dir, &gemini_exclude_lines()).err().map(|e| format!("git exclude: {e}"))
}

fn is_tracked(dir: &Path, file: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--", file])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Adds `lines` to the repo's `info/exclude` unless they are there. For a linked worktree git keeps
/// that file in the main repo, so the lines apply to every checkout of it.
pub(crate) fn exclude_from_git(dir: &Path, lines: &[&str]) -> std::io::Result<()> {
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

/// A setup line that prints `text` on a line of its own, for a divider in the tab's terminal when a
/// new stage's agent starts in it.
#[must_use]
pub fn banner_line(text: &str) -> String {
    format!("printf '\\n%s\\n' {}", crate::launch::shell_quote(text))
}

/// The line for a plain interactive `shell` that prints `note` first: what a restored tab runs when
/// its agent could not be resumed.
#[must_use]
pub fn note_script(note: &str, shell: &str) -> String {
    format!("{}; exec {} -l -i", banner_line(note), crate::launch::shell_quote(shell))
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
    use crate::trust::is_trusted;
    use crate::trust_apply::ConfigFiles;

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
                remote: false,
                meeseeks: false,
                path_var: Some(OsStr::new("/opt/homebrew/bin:/usr/bin:/bin")),
                listener,
                trust: None,
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
    fn a_meeseeks_reads_its_one_task_block_even_with_the_house_style_off_and_a_plain_agent_never_does()
     {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let mut quiet = profile("claude", Level::Edits);
        quiet.house_style = false;
        let style = fx.paths.tab_house_style_file(Path::new("/work/web"), "brave-otter");
        let input = |meeseeks| PrepareInput {
            paths: &fx.paths,
            agent: table.get("claude").unwrap(),
            profile: &quiet,
            repo: Path::new("/work/web"),
            tab: "brave-otter",
            dir: &fx.dir,
            worktree_tab: true,
            task: "rename it",
            session_id: None,
            remote: false,
            meeseeks,
            path_var: None,
            listener: None,
            trust: None,
        };
        prepare(&input(false)).unwrap();
        assert!(!style.exists(), "a plain agent with the house style off gets no file");
        let prepared = prepare(&input(true)).unwrap();
        let text = std::fs::read_to_string(&style).unwrap();
        assert!(text.contains("## You are a Meeseeks") && text.contains("`meeseeks_done`"));
        assert!(
            prepared.launch.args.iter().any(|arg| arg.contains("house-style")),
            "{:?}",
            prepared.launch.args
        );
        let resumed = prepare_resume(&input(true), SESSION).unwrap();
        assert!(resumed.launch.args.iter().any(|arg| arg.contains("house-style")));
    }

    #[test]
    fn every_agent_is_told_where_to_append_job_progress_and_the_file_exists() {
        let fx = Fixture::new();
        for id in ["claude", "codex", "gemini", "copilot"] {
            let prepared =
                fx.run(id, &profile(id, Level::Edits), true, "fix the bug", None).unwrap();
            let want = fx.paths.progress_feed_file(Path::new("/work/web"), "brave-otter");
            assert_eq!(env_of(&prepared, PROGRESS_ENV), want.to_str(), "{id}");
            assert!(want.is_file(), "{id}");
        }
    }

    #[test]
    fn a_resumed_agent_gets_the_same_feed_and_its_lines_are_kept() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let claude = profile("claude", Level::Edits);
        let feed = fx.paths.progress_feed_file(Path::new("/work/web"), "brave-otter");
        std::fs::create_dir_all(feed.parent().unwrap()).unwrap();
        std::fs::write(&feed, "{\"name\":\"orders\",\"done\":1}\n").unwrap();
        let input = resume_input(&fx, &table, "claude", &claude, None);
        let resumed = prepare_resume(&input, SESSION).unwrap();
        assert_eq!(env_of(&resumed, PROGRESS_ENV), feed.to_str());
        assert_eq!(std::fs::read_to_string(&feed).unwrap(), "{\"name\":\"orders\",\"done\":1}\n");
    }

    #[test]
    fn a_builder_launch_has_the_level_flag_the_house_style_and_the_task_last() {
        let fx = Fixture::new();
        let prepared =
            fx.run("claude", &profile("claude", Level::Edits), true, "fix the bug", None).unwrap();
        assert!(fx.paths.house_style_file().is_file(), "house-style.md is created on first launch");
        let house = fx.paths.tab_house_style_file(Path::new("/work/web"), "brave-otter");
        assert!(house.is_file());
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
    fn without_never_push_there_is_no_push_guard_but_the_outbound_shims_stay() {
        let fx = Fixture::new();
        let mut open = profile("claude", Level::Edits);
        open.never_push = false;
        let prepared = fx.run("claude", &open, true, "", None).unwrap();
        let dir = fx.paths.guard_bin_dir();
        assert_eq!(env_of(&prepared, "GIT_CONFIG_COUNT"), None);
        assert!(!dir.join(crate::launch::NO_PUSH_HELPER_NAME).exists());
        assert!(dir.join("git").is_file() && dir.join("gh").is_file());
        assert_eq!(
            env_of(&prepared, "PATH"),
            Some(format!("{}:/opt/homebrew/bin:/usr/bin:/bin", dir.display()).as_str())
        );
        assert_eq!(
            env_of(&prepared, crate::guard_run::CONFIG_ENV),
            Some(fx.paths.prefs_file().display().to_string().as_str())
        );
        assert_eq!(env_of(&prepared, crate::guard_run::REPO_ENV), Some("/work/web"));
        assert_eq!(
            env_of(&prepared, crate::guard_run::DIR_ENV),
            Some(dir.display().to_string().as_str())
        );
    }

    #[test]
    fn outside_a_worktree_there_is_no_guard_and_path_is_the_given_one() {
        let fx = Fixture::new();
        let other = fx.run("claude", &profile("claude", Level::Edits), false, "", None).unwrap();
        assert_eq!(env_of(&other, "GIT_CONFIG_COUNT"), None);
        assert_eq!(env_of(&other, "PATH"), Some("/opt/homebrew/bin:/usr/bin:/bin"));
        assert_eq!(env_of(&other, crate::guard_run::CONFIG_ENV), None);
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
        let dir = fx.paths.tab_copilot_instructions_dir(Path::new("/work/web"), "brave-otter");
        assert!(dir.join("tidewatch.instructions.md").is_file());
        assert_eq!(
            env_of(&prepared, "COPILOT_CUSTOM_INSTRUCTIONS_DIRS"),
            Some(dir.display().to_string().as_str())
        );
        assert_eq!(&prepared.launch.args[prepared.launch.args.len() - 2..], ["-i", "add tests"]);
    }

    fn agent_style(fx: &Fixture) -> String {
        std::fs::read_to_string(
            fx.paths.tab_house_style_file(Path::new("/work/web"), "brave-otter"),
        )
        .unwrap()
    }

    #[test]
    fn the_agent_is_given_the_house_style_with_the_ledger_format_and_the_default_checklist() {
        let fx = Fixture::new();
        fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        let text = agent_style(&fx);
        assert!(text.starts_with(house_style::DEFAULT_HOUSE_STYLE.trim_end()));
        assert!(text.contains("## Progress ledger format"));
        assert!(text.contains("### 10. Close out"));
        assert!(fx.paths.checklist_file().is_file(), "checklist.md is installed on first launch");
    }

    #[test]
    fn an_edited_house_style_and_checklist_are_used_and_never_overwritten() {
        let fx = Fixture::new();
        std::fs::create_dir_all(fx.paths.config_dir()).unwrap();
        std::fs::write(fx.paths.house_style_file(), "my own style\n").unwrap();
        std::fs::write(fx.paths.checklist_file(), "### 1. Mine\n- [ ] my item\n").unwrap();
        fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        let text = agent_style(&fx);
        assert!(text.starts_with("my own style\n\n## Tidewatch tool guidance"), "{text}");
        assert!(text.contains("`report_job`") && text.contains("`$TIDEWATCH_PROGRESS`"));
        assert!(text.contains("`set_avatar`"));
        assert!(text.contains("\n\n## Progress ledger format"), "{text}");
        assert!(text.contains("### 1. Mine\n- [ ] my item\n"));
        assert!(!text.contains("Close out"));
        assert_eq!(std::fs::read_to_string(fx.paths.house_style_file()).unwrap(), "my own style\n");
        assert_eq!(
            std::fs::read_to_string(fx.paths.checklist_file()).unwrap(),
            "### 1. Mine\n- [ ] my item\n"
        );
    }

    #[test]
    fn a_repos_checklist_file_replaces_the_template_for_its_tabs() {
        let fx = Fixture::new();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join(".tidewatch.toml"), "checklist = \"steps.md\"\n").unwrap();
        std::fs::write(repo.path().join("steps.md"), "### 1. Repo way\n- [ ] repo item\n").unwrap();
        let table = AgentTable::builtin();
        let p = profile("claude", Level::Edits);
        prepare(&PrepareInput {
            paths: &fx.paths,
            agent: table.get("claude").unwrap(),
            profile: &p,
            repo: repo.path(),
            tab: "brave-otter",
            dir: &fx.dir,
            worktree_tab: true,
            task: "",
            session_id: None,
            remote: false,
            meeseeks: false,
            path_var: None,
            listener: None,
            trust: None,
        })
        .unwrap();
        let text =
            std::fs::read_to_string(fx.paths.tab_house_style_file(repo.path(), "brave-otter"))
                .unwrap();
        assert!(text.contains("### 1. Repo way\n- [ ] repo item\n"));
        assert!(!text.contains("Close out"));
    }

    #[test]
    fn every_cli_is_pointed_at_the_text_with_the_ledger_format() {
        let fx = Fixture::new();
        let tab_dir = fx.paths.tab_state_dir(Path::new("/work/web"), "brave-otter");
        let claude = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        let flag = claude.launch.args.iter().position(|a| a == "--append-system-prompt-file");
        let file = std::path::PathBuf::from(&claude.launch.args[flag.unwrap() + 1]);
        assert!(std::fs::read_to_string(file).unwrap().contains("update_progress"));

        let codex = fx.run("codex", &profile("codex", Level::Edits), true, "", None).unwrap();
        let instructions =
            codex.launch.args.iter().find(|a| a.starts_with("developer_instructions="));
        assert!(instructions.is_some_and(|text| text.contains("update_progress")));

        let gemini = fx.run("gemini", &profile("gemini", Level::Edits), true, "", None).unwrap();
        let include = format!("--include-directories={}", tab_dir.display());
        assert!(gemini.launch.args.contains(&include), "{:?}", gemini.launch.args);
        assert!(agent_style(&fx).contains("update_progress"));

        fx.run("copilot", &profile("copilot", Level::Edits), true, "", None).unwrap();
        let copilot = tab_dir.join("copilot").join("tidewatch.instructions.md");
        let text = std::fs::read_to_string(copilot).unwrap();
        assert!(text.starts_with("---\napplyTo: \"**\"\n---\n"));
        assert!(text.contains("update_progress"));
    }

    #[test]
    fn the_preview_shows_the_flags_the_real_launch_gets_for_every_agent() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        for id in ["claude", "codex", "gemini", "copilot"] {
            let p = profile(id, Level::Edits);
            let input = PrepareInput {
                paths: &fx.paths,
                agent: table.get(id).unwrap(),
                profile: &p,
                repo: Path::new("/work/web"),
                tab: "brave-otter",
                dir: &fx.dir,
                worktree_tab: true,
                task: "do it",
                session_id: None,
                remote: false,
                meeseeks: false,
                path_var: None,
                listener: None,
                trust: None,
            };
            let real = prepare(&input).unwrap();
            let shown = preview(&input).unwrap();
            assert_eq!(shown.program, real.launch.program, "{id}");
            assert_eq!(shown.args, real.launch.args, "{id}");
        }
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
    fn status_hooks_and_mcp_are_wired_only_when_a_listener_is_given() {
        let fx = Fixture::new();
        let none = fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        assert!(!none.launch.args.iter().any(|a| a.starts_with("--mcp-config")));
        let at = none.launch.args.iter().position(|a| a == "--settings").unwrap();
        let settings: serde_json::Value = serde_json::from_str(&none.launch.args[at + 1]).unwrap();
        assert!(settings["hooks"]["SessionStart"].is_null(), "no status hooks without a listener");
        assert_eq!(settings["hooks"]["PostToolUse"][0]["hooks"][0]["type"], "command");

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

    fn grant_in(fx: &Fixture, config_home: &Path) -> Grant {
        Grant::new(&fx.paths, ConfigFiles::resolve(config_home, |_| None))
    }

    fn prepare_with_grant(
        fx: &Fixture,
        agent: &str,
        worktree_tab: bool,
        grant: &Grant,
    ) -> Prepared {
        let table = AgentTable::builtin();
        let profile = profile(agent, Level::Edits);
        let mut input = resume_input(fx, &table, agent, &profile, None);
        input.worktree_tab = worktree_tab;
        input.trust = Some(grant);
        prepare(&input).unwrap()
    }

    #[test]
    fn a_worktree_tab_with_consent_is_trusted_before_the_agent_starts() {
        let fx = Fixture::new();
        let config = tempfile::tempdir().unwrap();
        let grant = grant_in(&fx, config.path());
        let prepared = prepare_with_grant(&fx, "claude", true, &grant);
        assert_eq!(prepared.notes, Vec::<String>::new());
        let text = std::fs::read_to_string(config.path().join(".claude.json")).unwrap();
        let folder = fx.dir.canonicalize().unwrap();
        assert!(is_trusted(TrustCli::Claude, &text, &folder.to_string_lossy()).unwrap());
    }

    #[test]
    fn no_trust_entry_for_gemini_a_shared_folder_or_a_tab_without_consent() {
        let fx = Fixture::new();
        let config = tempfile::tempdir().unwrap();
        let grant = grant_in(&fx, config.path());
        prepare_with_grant(&fx, "gemini", true, &grant);
        prepare_with_grant(&fx, "claude", false, &grant);
        fx.run("claude", &profile("claude", Level::Edits), true, "", None).unwrap();
        let names: Vec<_> = std::fs::read_dir(config.path()).unwrap().collect();
        assert_eq!(names.len(), 0, "nothing was written");
    }

    #[test]
    fn a_config_file_that_cannot_be_edited_is_a_note_and_the_agent_still_starts() {
        let fx = Fixture::new();
        let config = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join(".claude.json"), "{ broken").unwrap();
        let grant = grant_in(&fx, config.path());
        let prepared = prepare_with_grant(&fx, "claude", true, &grant);
        assert_eq!(prepared.notes.len(), 1);
        assert!(prepared.notes[0].starts_with("folder trust: "), "{:?}", prepared.notes);
        assert_eq!(
            std::fs::read_to_string(config.path().join(".claude.json")).unwrap(),
            "{ broken"
        );
    }

    const SESSION: &str = "0c2cbc96-1111-4222-8333-444455556666";

    fn resume_input<'a>(
        fx: &'a Fixture,
        table: &'a AgentTable,
        agent: &str,
        profile: &'a Profile,
        listener: Option<HookListener>,
    ) -> PrepareInput<'a> {
        PrepareInput {
            paths: &fx.paths,
            agent: table.get(agent).unwrap(),
            profile,
            repo: Path::new("/work/web"),
            tab: "brave-otter",
            dir: &fx.dir,
            worktree_tab: true,
            task: "ignored task",
            session_id: Some("99999999-8888-7777-6666-555555555555".into()),
            remote: false,
            meeseeks: false,
            path_var: Some(OsStr::new("/opt/homebrew/bin:/usr/bin:/bin")),
            listener,
            trust: None,
        }
    }

    #[test]
    fn a_resumed_tab_gets_its_mcp_config_rewritten_with_the_current_wiring() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let claude = profile("claude", Level::Edits);
        let input = resume_input(&fx, &table, "claude", &claude, Some(listener()));
        let mcp = fx.paths.tab_state_dir(input.repo, input.tab).join("mcp.json");
        std::fs::create_dir_all(mcp.parent().unwrap()).unwrap();
        let old = r#"{"mcpServers":{"tidewatch":{"type":"http","url":"http://127.0.0.1:1/mcp"}}}"#;
        std::fs::write(&mcp, old).unwrap();
        prepare_resume(&input, SESSION).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["tidewatch"]["alwaysLoad"], serde_json::json!(true));
        assert_eq!(
            std::fs::read_to_string(&mcp).unwrap(),
            mcp_config_json(Wiring::Claude, &listener()).unwrap()
        );
    }

    #[test]
    fn a_resumed_tab_gets_the_same_wiring_as_a_fresh_one_and_neither_task_nor_new_session_id() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let claude = profile("claude", Level::Edits);
        let input = resume_input(&fx, &table, "claude", &claude, Some(listener()));
        let fresh = prepare(&input).unwrap();
        let resumed = prepare_resume(&input, SESSION).unwrap();
        assert_eq!(resumed.env, fresh.env);
        assert_eq!(resumed.launch.env, fresh.launch.env);
        assert_eq!(&resumed.launch.args[..2], ["--resume", SESSION]);
        assert!(!resumed.launch.args.iter().any(|a| a == "--session-id" || a == "ignored task"));
        for flag in ["--settings", "--append-system-prompt-file", "--permission-mode"] {
            assert!(resumed.launch.args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(
            resumed.launch.args.iter().any(|a| a.starts_with("--mcp-config=")),
            "{:?}",
            resumed.launch.args
        );
    }

    #[test]
    fn remote_control_names_the_session_after_the_repo_and_the_worktree_fresh_and_resumed() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let name = "web \u{b7} brave-otter";
        let cases: [(&str, bool, &[&str]); 5] = [
            ("claude", true, &["--remote-control", name]),
            ("claude", false, &[]),
            ("copilot", true, &["--remote"]),
            ("codex", true, &[]),
            ("gemini", true, &[]),
        ];
        for (agent, remote, want) in cases {
            let p = profile(agent, Level::Edits);
            let input =
                PrepareInput { remote, ..resume_input(&fx, &table, agent, &p, Some(listener())) };
            for prepared in [prepare(&input).unwrap(), prepare_resume(&input, SESSION).unwrap()] {
                let args = &prepared.launch.args;
                assert_eq!(prepared.remote.is_some(), !want.is_empty(), "{agent} {remote}");
                if let Some(named) = &prepared.remote {
                    assert_eq!(named, name);
                }
                let found =
                    want.is_empty() || args.windows(want.len()).any(|window| window == want);
                assert!(found, "{agent} {remote}: {args:?}");
                let flagged = args.iter().any(|a| a.starts_with("--remote"));
                assert_eq!(flagged, !want.is_empty(), "{agent} {remote}");
            }
        }
    }

    #[test]
    fn a_tab_in_a_plain_folder_is_named_by_the_folder_alone() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let p = profile("claude", Level::Edits);
        let input = PrepareInput {
            remote: true,
            worktree_tab: false,
            repo: Path::new("/work/notes"),
            tab: "tab-4",
            ..resume_input(&fx, &table, "claude", &p, None)
        };
        let prepared = prepare(&input).unwrap();
        let at = prepared.launch.args.iter().position(|a| a == "--remote-control").unwrap();
        assert_eq!(prepared.launch.args[at + 1], "notes");
    }

    #[test]
    fn resuming_yolo_outside_a_worktree_is_refused_like_a_fresh_launch() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let yolo = profile("claude", Level::Yolo);
        let input = PrepareInput {
            worktree_tab: false,
            ..resume_input(&fx, &table, "claude", &yolo, None)
        };
        assert_eq!(
            prepare_resume(&input, SESSION),
            Err(PrepareError::Launch(LaunchError::YoloOutsideWorktree))
        );
    }

    #[test]
    fn resuming_yolo_in_a_worktree_keeps_its_yolo_flags_guard_and_wiring() {
        let fx = Fixture::new();
        let table = AgentTable::builtin();
        let yolo = profile("claude", Level::Yolo);
        let input = resume_input(&fx, &table, "claude", &yolo, Some(listener()));
        let fresh = prepare(&input).unwrap();
        let resumed = prepare_resume(&input, SESSION).unwrap();
        let yolo_args = table.get("claude").unwrap().level_args(Level::Yolo).unwrap();
        assert_eq!(&resumed.launch.args[..2], ["--resume", SESSION]);
        assert_eq!(resumed.launch.args[2..2 + yolo_args.len()], *yolo_args);
        assert_eq!(resumed.env, fresh.env);
        assert_eq!(resumed.launch.env, fresh.launch.env);
        assert!(resumed.launch.args.iter().any(|a| a.starts_with("--mcp-config=")));
    }

    #[test]
    fn a_resumed_gemini_tab_gets_its_generated_settings_rewritten_for_the_new_listener() {
        let fx = Fixture::new();
        let worktree = git_worktree(&fx);
        let fx = Fixture { dir: worktree.clone(), ..fx };
        let table = AgentTable::builtin();
        let gemini = profile("gemini", Level::Edits);
        prepare(&resume_input(&fx, &table, "gemini", &gemini, Some(listener()))).unwrap();
        let newer =
            HookListener { base_url: "http://127.0.0.1:6666".into(), token: "newtok".into() };
        let resumed =
            prepare_resume(&resume_input(&fx, &table, "gemini", &gemini, Some(newer)), SESSION)
                .unwrap();
        assert_eq!(resumed.notes, Vec::<String>::new());
        let settings = std::fs::read_to_string(worktree.join(".gemini/settings.json")).unwrap();
        assert!(settings.contains("6666") && settings.contains("newtok"), "{settings}");
        assert!(!settings.contains("5555"), "{settings}");
        assert_eq!(&resumed.launch.args[..2], ["--resume", "latest"]);
    }

    #[test]
    fn a_resumed_gemini_tab_never_overwrites_a_settings_file_the_repo_tracks() {
        let fx = Fixture::new();
        let worktree = git_worktree(&fx);
        let fx = Fixture { dir: worktree.clone(), ..fx };
        std::fs::create_dir_all(worktree.join(".gemini")).unwrap();
        std::fs::write(worktree.join(".gemini/settings.json"), "{\"theirs\":true}").unwrap();
        git(&worktree, &["add", ".gemini/settings.json"]);
        git(&worktree, &["commit", "-q", "-m", "settings"]);
        let table = AgentTable::builtin();
        let gemini = profile("gemini", Level::Edits);
        let resumed = prepare_resume(
            &resume_input(&fx, &table, "gemini", &gemini, Some(listener())),
            SESSION,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(worktree.join(".gemini/settings.json")).unwrap(),
            "{\"theirs\":true}"
        );
        assert_eq!(resumed.notes.len(), 1);
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
            remote: false,
            meeseeks: false,
            path_var: Some(OsStr::new(&path)),
            listener: None,
            trust: None,
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

        git(&clone, &["checkout", "-q", "-b", "agent/brave-otter"]);
        let input = PrepareInput {
            paths: &fx.paths,
            agent: table.get("claude").unwrap(),
            profile: &profile("claude", Level::Edits),
            repo: Path::new("/work/web"),
            tab: "brave-otter",
            dir: &clone,
            worktree_tab: true,
            task: "",
            session_id: None,
            remote: false,
            meeseeks: false,
            path_var: Some(OsStr::new(&path)),
            listener: None,
            trust: None,
        };
        let prepared = prepare(&input).unwrap();
        let out = Command::new("/bin/sh")
            .args(["-c", "git push -q origin agent/brave-otter"])
            .current_dir(&clone)
            .envs(prepared.env.iter().map(|(k, v)| (k, v)))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "its own branch goes: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let refs = Command::new("git").arg("-C").arg(&remote).arg("for-each-ref").output().unwrap();
        assert!(String::from_utf8_lossy(&refs.stdout).contains("refs/heads/agent/brave-otter"));
    }

    #[test]
    fn the_push_branch_is_the_checked_out_agent_branch_of_a_linked_worktree_only() {
        let dir = tempfile::tempdir().unwrap();
        let linked = dir.path().join("wt");
        let admin = dir.path().join("repo/.git/worktrees/wt");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        let cases = [
            ("ref: refs/heads/agent/brave-otter\n", Some("agent/brave-otter")),
            ("ref: refs/heads/main\n", None),
            ("0123456789abcdef0123456789abcdef01234567\n", None),
        ];
        for (head, want) in cases {
            std::fs::write(admin.join("HEAD"), head).unwrap();
            assert_eq!(checked_out_agent_branch(&linked).as_deref(), want, "{head}");
        }
        assert_eq!(checked_out_agent_branch(&dir.path().join("missing")), None);
    }

    #[test]
    fn a_rust_repos_recipe_points_its_worktree_agents_at_the_main_checkouts_target() {
        use crate::setup_recipe::{BUILD_DIR_VAR, Recipe, Store};
        let fx = Fixture::new();
        let repo = fx.dir.join("main-checkout");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[package]\n").unwrap();
        let table = AgentTable::builtin();
        let prepare_in = |worktree_tab| {
            prepare(&PrepareInput {
                paths: &fx.paths,
                agent: table.get("claude").unwrap(),
                profile: &profile("claude", Level::Edits),
                repo: &repo,
                tab: "brave-otter",
                dir: &fx.dir,
                worktree_tab,
                task: "",
                session_id: None,
                remote: false,
                meeseeks: false,
                path_var: None,
                listener: None,
                trust: None,
            })
            .unwrap()
        };
        assert_eq!(env_of(&prepare_in(true), BUILD_DIR_VAR), None, "off until the recipe asks");
        Store::update(&fx.paths, |store| {
            store.set(&repo, &Recipe { shared_build_dir: true, ..Recipe::default() });
        })
        .unwrap();
        let want = repo.join("target").display().to_string();
        assert_eq!(env_of(&prepare_in(true), BUILD_DIR_VAR), Some(want.as_str()));
        assert_eq!(
            env_of(&prepare_in(false), BUILD_DIR_VAR),
            None,
            "a plain folder tab is left alone"
        );
    }

    #[test]
    fn a_banner_line_prints_the_text_even_with_quotes_and_percent_signs() {
        let text = "--- Stage 2: Review (gemini, Read only) --- it's 100%";
        let out = Command::new("sh").arg("-c").arg(banner_line(text)).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("\n{text}\n"));
    }

    #[test]
    fn a_note_script_prints_the_note_and_then_runs_the_shell() {
        let note = "The agent was not resumed: it's gone, 100% sure.";
        let script = note_script(note, "/bin/sh");
        assert!(script.ends_with("; exec /bin/sh -l -i"), "{script}");
        let echo = script.replace("exec /bin/sh -l -i", "true");
        let out = Command::new("sh").arg("-c").arg(echo).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("\n{note}\n"));
    }

    #[test]
    fn a_banner_runs_before_the_agent_in_the_script() {
        let launch = Launch {
            program: "agent".into(),
            args: Vec::new(),
            env: Vec::new(),
            experimental: false,
        };
        let setup = format!("{}\nnpm ci", banner_line("--- Stage 1 ---"));
        let script = shell_script(&setup, &launch, "/bin/zsh");
        assert!(
            script.starts_with("printf '\\n%s\\n' '--- Stage 1 ---' && npm ci && "),
            "{script}"
        );
    }
}
