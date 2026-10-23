//! Running `git` and `gh` for delivery and pull request work.
//!
//! Every call blocks until the child exits: call these from a background thread, never the UI
//! thread. Children run with prompts off and stdin closed, so a missing login fails fast instead of
//! waiting for a keypress that nobody can type. They also run in Tidewatch's own environment, not an
//! agent tab's, so the never-push guard (D7) and the outbound guards do not apply to them.
//!
//! `gh` messages were checked against gh 2.98.0, `git` against 2.55.0.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::path_env;

/// How `git` and `gh` are found. The default looks them up on the `PATH` this process has, with the
/// usual install directories in front (an app started from Finder has a bare one).
#[derive(Clone, Debug, Default)]
pub struct Tools {
    path: Option<OsString>,
}

impl Tools {
    #[must_use]
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let current = std::env::var_os("PATH");
        Self { path: Some(path_env::for_this_platform(current.as_deref(), &home)) }
    }

    /// Looks `git` and `gh` up on exactly this `PATH`. Tests put a fake `gh` first on it.
    #[must_use]
    pub fn with_path(path: impl Into<OsString>) -> Self {
        Self { path: Some(path.into()) }
    }

    pub(crate) fn command(&self, program: &str, dir: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(dir)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .env_remove(crate::guard_run::CONFIG_ENV);
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        command
    }

    pub(crate) fn git(&self, dir: &Path) -> Command {
        self.command("git", dir)
    }

    pub(crate) fn gh(&self, dir: &Path) -> Command {
        self.command("gh", dir)
    }
}

/// What went wrong with a `gh` call, in the cases the UI words differently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhError {
    /// `gh` is not installed or not on the `PATH`.
    Missing,
    /// `gh` has no login (`gh auth login`, or `GH_TOKEN`).
    NotAuthenticated,
    /// No git remote of the folder points at a GitHub host.
    NoGithubRemote,
    /// Anything else, with gh's own message.
    Failed(String),
}

impl fmt::Display for GhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("the GitHub CLI (gh) is not installed"),
            Self::NotAuthenticated => f.write_str("gh is not logged in; run `gh auth login`"),
            Self::NoGithubRemote => f.write_str("this repository has no GitHub remote"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for GhError {}

pub(crate) struct Finished {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn finish(mut command: Command) -> std::io::Result<Finished> {
    let out = command.output()?;
    Ok(Finished {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Runs `gh` and returns its stdout, or the failure sorted into a [`GhError`].
pub(crate) fn run_gh(tools: &Tools, dir: &Path, args: &[&str]) -> Result<String, GhError> {
    let mut command = tools.gh(dir);
    command.args(args);
    let finished = finish(command).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => GhError::Missing,
        _ => GhError::Failed(format!("cannot run gh: {e}")),
    })?;
    if finished.success { Ok(finished.stdout) } else { Err(classify_gh(&finished)) }
}

pub(crate) fn classify_gh(finished: &Finished) -> GhError {
    let message = failure_text(finished);
    let lower = message.to_lowercase();
    if lower.contains("none of the git remotes") {
        GhError::NoGithubRemote
    } else if lower.contains("gh auth login")
        || lower.contains("gh_token")
        || lower.contains("not logged in")
        || lower.contains("http 401")
        || lower.contains("bad credentials")
    {
        GhError::NotAuthenticated
    } else {
        GhError::Failed(message)
    }
}

/// gh prints its error to stderr, but `gh api` puts the response body on stdout; take whichever has
/// text, stderr first.
pub(crate) fn failure_text(finished: &Finished) -> String {
    let stderr = finished.stderr.trim();
    let text = if stderr.is_empty() { finished.stdout.trim() } else { stderr };
    if text.is_empty() { "gh failed without a message".to_owned() } else { text.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(stdout: &str, stderr: &str) -> Finished {
        Finished { success: false, stdout: stdout.into(), stderr: stderr.into() }
    }

    #[test]
    fn tidewatch_own_commands_never_carry_the_agent_guard_setting() {
        let tools = Tools::with_path("/usr/bin");
        for command in [tools.git(Path::new("/w")), tools.gh(Path::new("/w"))] {
            let removed = command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new(crate::guard_run::CONFIG_ENV));
            assert_eq!(removed, Some((std::ffi::OsStr::new(crate::guard_run::CONFIG_ENV), None)));
        }
    }

    #[test]
    fn gh_failures_are_sorted() {
        let cases = [
            (
                "none of the git remotes configured for this repository point to a known GitHub host. To tell gh about a new GitHub host, please use `gh auth login`",
                GhError::NoGithubRemote,
            ),
            (
                "To get started with GitHub CLI, please run:  gh auth login\nAlternatively, populate the GH_TOKEN environment variable",
                GhError::NotAuthenticated,
            ),
            ("gh: Bad credentials (HTTP 401)", GhError::NotAuthenticated),
            (
                "GraphQL: Head sha can't be blank (createPullRequest)",
                GhError::Failed("GraphQL: Head sha can't be blank (createPullRequest)".into()),
            ),
        ];
        for (stderr, want) in cases {
            assert_eq!(classify_gh(&failed("", stderr)), want, "stderr {stderr:?}");
        }
    }

    #[test]
    fn an_api_error_body_on_stdout_is_used_when_stderr_is_empty() {
        let body = r#"{"message":"Validation Failed"}"#;
        assert_eq!(classify_gh(&failed(body, "")), GhError::Failed(body.into()));
        assert_eq!(
            classify_gh(&failed("", "")),
            GhError::Failed("gh failed without a message".into())
        );
    }

    #[test]
    fn a_missing_program_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::with_path(dir.path());
        assert_eq!(run_gh(&tools, dir.path(), &["--version"]), Err(GhError::Missing));
    }
}
