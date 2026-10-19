//! The commit a new worktree starts from: by default the remote default branch (`origin/main`),
//! fetched fresh first, falling back to the copy already on the disk when the fetch fails.
//!
//! Every function shells out to `git` and blocks: call them from a background thread.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const REMOTE: &str = "origin";
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

/// What `prepare` settled on: the start point to pass to `git worktree add` (`None` is the current
/// commit) and, when the remote could not be reached, what to tell the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub base: Option<String>,
    pub note: Option<String>,
}

#[must_use]
pub fn remote_ref(branch: &str) -> String {
    format!("{REMOTE}/{branch}")
}

/// The default branch among the remote's `branches` (`main`, no `origin/`): the one `origin/HEAD`
/// names, else `main`, else `master`. `None` without any of them.
#[must_use]
pub fn pick_remote_default(origin_head: Option<&str>, branches: &[String]) -> Option<String> {
    let named = origin_head.map(|head| head.strip_prefix("origin/").unwrap_or(head));
    named
        .into_iter()
        .chain(["main", "master"])
        .find(|candidate| branches.iter().any(|branch| branch == candidate))
        .map(str::to_owned)
}

fn git(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn output_of(mut command: Command) -> Option<String> {
    let out = command.output().ok().filter(|out| out.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[must_use]
pub fn remote_branches(repo: &Path) -> Vec<String> {
    let mut command = git(repo);
    command.args(["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin"]);
    output_of(command)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("origin/"))
        .filter(|name| *name != "HEAD")
        .map(str::to_owned)
        .collect()
}

fn origin_head(repo: &Path) -> Option<String> {
    let mut command = git(repo);
    command.args(["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]);
    output_of(command).filter(|name| !name.is_empty())
}

/// The remote's default branch as the repo knows it, without the remote's name.
#[must_use]
pub fn remote_default(repo: &Path) -> Option<String> {
    pick_remote_default(origin_head(repo).as_deref(), &remote_branches(repo))
}

/// Settles the start point. A base that is a branch of `origin` is fetched first (quietly, for at
/// most `timeout`); when that fails the copy of it already here is used, and failing that the
/// current commit, with a note that says so.
#[must_use]
pub fn prepare(repo: &Path, base: Option<&str>, timeout: Duration) -> Start {
    let Some(base) = base else { return Start { base: None, note: None } };
    let Some(branch) = base.strip_prefix("origin/") else {
        return Start { base: Some(base.to_owned()), note: None };
    };
    let Err(reason) = fetch(repo, branch, timeout) else {
        return Start { base: Some(base.to_owned()), note: None };
    };
    if resolves(repo, base) {
        let note = format!(
            "Could not fetch {base} ({reason}), so this started from the copy fetched earlier."
        );
        return Start { base: Some(base.to_owned()), note: Some(note) };
    }
    let note =
        format!("Could not fetch {base} ({reason}), so this started from the current branch.");
    Start { base: None, note: Some(note) }
}

fn resolves(repo: &Path, base: &str) -> bool {
    let mut command = git(repo);
    command.args(["rev-parse", "--verify", "--quiet"]).arg(format!("{base}^{{commit}}"));
    output_of(command).is_some()
}

fn fetch(repo: &Path, branch: &str, timeout: Duration) -> Result<(), String> {
    let mut command = git(repo);
    command.args(["fetch", "--quiet", "--no-tags", REMOTE, "--", branch]);
    run_within(command, timeout)
}

fn run_within(mut command: Command, timeout: Duration) -> Result<(), String> {
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("cannot run git: {e}"))?;
    let Some(mut stderr) = child.stderr.take() else { return Err("no output".to_owned()) };
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer in {} s", timeout.as_secs().max(1)));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let said = reader.join().unwrap_or_default();
    if status.success() { Ok(()) } else { Err(reason_of(&said)) }
}

fn reason_of(stderr: &str) -> String {
    let mut lines = stderr.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines
        .clone()
        .rev()
        .find(|line| line.starts_with("fatal:") || line.starts_with("error:"))
        .or_else(|| lines.next_back())
        .unwrap_or("git fetch failed");
    let line =
        line.strip_prefix("fatal: ").or_else(|| line.strip_prefix("error: ")).unwrap_or(line);
    line.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_default_is_what_origin_head_names_else_main_else_master() {
        let cases = [
            (Some("origin/trunk"), &["main", "trunk"][..], Some("trunk")),
            (Some("origin/trunk"), &["main"], Some("main")),
            (None, &["master", "dev"], Some("master")),
            (None, &["main", "master"], Some("main")),
            (Some("trunk"), &["trunk"], Some("trunk")),
            (None, &["dev"], None),
            (Some("origin/main"), &[], None),
        ];
        for (head, branches, want) in cases {
            assert_eq!(
                pick_remote_default(head, &names(branches)).as_deref(),
                want,
                "{head:?} {branches:?}"
            );
        }
    }

    #[test]
    fn only_a_branch_of_origin_is_fetched() {
        let nowhere = Path::new("/nonexistent/repo");
        assert_eq!(prepare(nowhere, None, FETCH_TIMEOUT), Start { base: None, note: None });
        assert_eq!(
            prepare(nowhere, Some("feature/x"), FETCH_TIMEOUT),
            Start { base: Some("feature/x".into()), note: None }
        );
    }

    #[test]
    fn a_fetch_that_runs_too_long_is_stopped() {
        let mut command = Command::new("sleep");
        command.arg("5");
        let started = Instant::now();
        let error = run_within(command, Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(error, "no answer in 1 s");
    }

    #[test]
    fn the_last_fatal_line_git_said_is_the_reason() {
        let said = "fatal: 'x' does not appear to be a git repository\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n";
        assert_eq!(reason_of(said), "Could not read from remote repository.");
        assert_eq!(reason_of("remote: busy\nstill going"), "still going");
        assert_eq!(reason_of(""), "git fetch failed");
        assert_eq!(reason_of(&"x".repeat(300)).len(), 120);
    }
}
