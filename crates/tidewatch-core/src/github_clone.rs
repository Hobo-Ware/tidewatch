//! Getting a GitHub repo onto the disk: find a checkout that is already known, else clone into
//! `<parent>/<repo>`. `clone_from` takes the clone address, so tests clone from a local bare repo.
//!
//! `find_checkout`, `plan` and the clones run `git` (and maybe `gh`) and block: call them off the UI thread.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use crate::github_link::GithubRepo;
use crate::tools::Tools;

const FALLBACK_PARENT: &str = "Git";
const POLL: Duration = Duration::from_millis(100);

/// The folder new clones go in: next to the repo opened last, else `~/Git`.
#[must_use]
pub fn default_parent(last_repo: Option<&Path>, home: &Path) -> PathBuf {
    last_repo
        .and_then(Path::parent)
        .filter(|parent| parent.parent().is_some())
        .map_or_else(|| home.join(FALLBACK_PARENT), Path::to_path_buf)
}

#[must_use]
pub fn target_of(parent: &Path, repo: &GithubRepo) -> PathBuf {
    parent.join(&repo.name)
}

/// What opening a link comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    /// A checkout of the repo is on the disk: open it.
    Open(PathBuf),
    Clone {
        target: PathBuf,
    },
    /// The folder a clone would go in is taken by something else.
    Refuse(String),
}

/// The `owner/name` of a folder's `origin` remote, when it is a GitHub one.
#[must_use]
pub fn origin_of(tools: &Tools, dir: &Path) -> Option<GithubRepo> {
    if !dir.is_dir() {
        return None;
    }
    let mut command = tools.git(dir);
    command.args(["remote", "get-url", "origin"]);
    let done = tools.finish(command).ok().filter(|done| done.success)?;
    GithubRepo::parse(done.stdout.trim())
}

/// The first of `known` whose `origin` is `repo`.
#[must_use]
pub fn find_checkout(tools: &Tools, repo: &GithubRepo, known: &[PathBuf]) -> Option<PathBuf> {
    known
        .iter()
        .find(|dir| origin_of(tools, dir).is_some_and(|origin| origin.same_repo(repo)))
        .cloned()
}

/// Where a link leads: a known checkout, else the folder `<parent>/<repo>` when it is free (missing
/// or empty) or already a checkout of the repo, else a refusal that says why.
#[must_use]
pub fn plan(tools: &Tools, repo: &GithubRepo, parent: &Path, known: &[PathBuf]) -> Landing {
    if let Some(dir) = find_checkout(tools, repo, known) {
        return Landing::Open(dir);
    }
    let target = target_of(parent, repo);
    if !target.exists() {
        return Landing::Clone { target };
    }
    if !target.is_dir() {
        return Landing::Refuse(format!("{} is a file, not a folder", target.display()));
    }
    if origin_of(tools, &target).is_some_and(|origin| origin.same_repo(repo)) {
        return Landing::Open(target);
    }
    if std::fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_none()) {
        return Landing::Clone { target };
    }
    Landing::Refuse(format!("{} exists and is not a checkout of {}", target.display(), repo.slug()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloneError {
    NoGit,
    Cancelled,
    Failed(String),
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGit => f.write_str("git is not installed"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CloneError {}

/// `git clone <url> <target>`, calling `progress` with each line git prints (`Receiving objects:
/// 42%`). `cancel` set stops it and removes what it began. Prompts are off, so a repo that asks for a
/// login fails instead of waiting.
pub fn clone_from(
    tools: &Tools,
    url: &str,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<(), CloneError> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| CloneError::Failed(e.to_string()))?;
    let mut command = tools.git(parent);
    command.args(["clone", "--progress", "--", url]).arg(target);
    run(command, target, cancel, progress).map_err(|e| match e {
        Run::Missing => CloneError::NoGit,
        Run::Cancelled => CloneError::Cancelled,
        Run::Failed(lines) => CloneError::Failed(explain(&lines)),
    })
}

/// Clones `repo` over HTTPS with git; when git fails and `gh` is installed, once more with
/// `gh repo clone`, which carries the user's GitHub login for a private repo.
pub fn clone_repo(
    tools: &Tools,
    repo: &GithubRepo,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<(), CloneError> {
    clone_with_gh_fallback(tools, &repo.clone_url(), repo, target, cancel, progress)
}

pub fn clone_with_gh_fallback(
    tools: &Tools,
    url: &str,
    repo: &GithubRepo,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<(), CloneError> {
    let first = clone_from(tools, url, target, cancel, progress);
    let Err(CloneError::Failed(git_said)) = first else { return first };
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut command = tools.gh(parent);
    command.args(["repo", "clone", &repo.slug()]).arg(target).args(["--", "--progress"]);
    match run(command, target, cancel, progress) {
        Ok(()) => Ok(()),
        Err(Run::Missing) => Err(CloneError::Failed(git_said)),
        Err(Run::Cancelled) => Err(CloneError::Cancelled),
        Err(Run::Failed(lines)) => Err(CloneError::Failed(explain(&lines))),
    }
}

enum Run {
    Missing,
    Cancelled,
    Failed(Vec<String>),
}

fn run(
    mut command: std::process::Command,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<(), Run> {
    let existed = target.exists();
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Run::Missing,
        _ => Run::Failed(vec![e.to_string()]),
    })?;
    let (sender, lines) = channel::<String>();
    if let Some(mut stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut line = Vec::new();
            let mut byte = [0_u8; 1];
            while stderr.read(&mut byte).is_ok_and(|n| n == 1) {
                if matches!(byte[0], b'\r' | b'\n') {
                    if !line.is_empty() {
                        let _ = sender.send(String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                } else {
                    line.push(byte[0]);
                }
            }
            if !line.is_empty() {
                let _ = sender.send(String::from_utf8_lossy(&line).into_owned());
            }
        });
    }
    let mut said = Vec::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            if !existed {
                let _ = std::fs::remove_dir_all(target);
            }
            return Err(Run::Cancelled);
        }
        match lines.recv_timeout(POLL) {
            Ok(line) => {
                progress(&line);
                said.push(line);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| Run::Failed(vec![e.to_string()]))?;
    if status.success() {
        return Ok(());
    }
    if !existed {
        let _ = std::fs::remove_dir_all(target);
    }
    Err(Run::Failed(said))
}

fn explain(lines: &[String]) -> String {
    let all = lines.join("\n");
    let lower = all.to_lowercase();
    if lower.contains("terminal prompts disabled")
        || lower.contains("authentication failed")
        || lower.contains("repository not found")
    {
        return "GitHub did not let us in. A private repo needs `gh auth login` first.".to_owned();
    }
    let reason = lines
        .iter()
        .rev()
        .find(|line| line.starts_with("fatal:") || line.starts_with("error:"))
        .or(lines.last());
    reason.map_or_else(|| "git clone failed".to_owned(), |line| line.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_go_next_to_the_repo_opened_last_else_in_git_under_home() {
        let home = Path::new("/Users/me");
        let cases = [
            (Some("/Users/me/code/web"), "/Users/me/code"),
            (Some("/work/web"), "/work"),
            (Some("/web"), "/Users/me/Git"),
            (Some("/"), "/Users/me/Git"),
            (None, "/Users/me/Git"),
        ];
        for (last, want) in cases {
            assert_eq!(default_parent(last.map(Path::new), home), Path::new(want), "{last:?}");
        }
    }

    #[test]
    fn the_target_is_the_repo_name_under_the_parent() {
        let repo = GithubRepo::parse("github.com/acme/web").unwrap();
        assert_eq!(target_of(Path::new("/x/code"), &repo), Path::new("/x/code/web"));
    }

    #[test]
    fn a_failed_clone_says_the_fatal_line_or_that_the_login_is_missing() {
        let lines = |text: &[&str]| text.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            explain(&lines(&["Cloning into 'web'...", "fatal: repository 'x' does not exist"])),
            "fatal: repository 'x' does not exist"
        );
        assert!(
            explain(&lines(&["fatal: could not read Username: terminal prompts disabled"]))
                .contains("gh auth login")
        );
        assert_eq!(explain(&lines(&["warning: odd"])), "warning: odd");
        assert_eq!(explain(&[]), "git clone failed");
    }
}
