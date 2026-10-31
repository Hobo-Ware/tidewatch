//! The checks a Meeseeks has to pass, run off the UI thread: the commit check, the tests, the clean
//! tree, the pull request and the poof. Each says what it found when it passes, or what went wrong
//! in words for the agent.

use std::path::Path;

use crate::delivery::{self, Ahead};
use crate::history;
use crate::limited;
use crate::meeseeks_log::pr_label_of;
use crate::paths::TidewatchPaths;
use crate::pr_ask;
use crate::test_command::{self, Gate};
use crate::tools::{GhError, Tools, run_gh};
use crate::worktree::{self, BranchChoice, RemoveMode, Terms, Why, Worktree};
use crate::worktree_safety::Owner;

fn changed_files(tools: &Tools, dir: &Path) -> Result<usize, String> {
    let mut command = tools.git(dir);
    command.args(["status", "--porcelain"]);
    let out = tools.finish(command).map_err(|e| limited::failure("git", &e))?;
    if !out.success {
        return Err(out.stderr.trim().to_owned());
    }
    Ok(out.stdout.lines().filter(|line| !line.trim().is_empty()).count())
}

fn clean(tools: &Tools, dir: &Path) -> Result<(), String> {
    match changed_files(tools, dir)? {
        0 => Ok(()),
        1 => Err("1 file with uncommitted changes".to_owned()),
        n => Err(format!("{n} files with uncommitted changes")),
    }
}

/// A check that passed: the one line it says, and the pull request it found when it was the hand off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub detail: String,
    pub pr_url: Option<String>,
}

#[must_use]
pub fn head(tools: &Tools, dir: &Path) -> Option<String> {
    head_or_stopped(tools, dir).ok().flatten()
}

fn head_or_stopped(tools: &Tools, dir: &Path) -> Result<Option<String>, String> {
    let mut command = tools.git(dir);
    command.args(["rev-parse", "HEAD"]);
    match tools.finish(command) {
        Ok(out) if out.success => {
            Ok(Some(out.stdout.trim().to_owned()).filter(|sha| !sha.is_empty()))
        }
        Err(e) if limited::is_stopped(&e) => Err(e.to_string()),
        _ => Ok(None),
    }
}

/// Whether the worktree differs from where the task started: files changed, or a commit made.
pub fn edited(tools: &Tools, dir: &Path, started_at: Option<&str>) -> Result<bool, String> {
    let status = history::working_status(dir).map_err(|e| format!("{e:#}"))?;
    if !status.all().is_empty() {
        return Ok(true);
    }
    Ok(started_at.is_some_and(|start| head(tools, dir).is_some_and(|now| now != start)))
}

fn said(detail: impl Into<String>) -> Checked {
    Checked { detail: detail.into(), pr_url: None }
}

pub fn change(tools: &Tools, dir: &Path, branch: &str) -> Result<Checked, String> {
    clean(tools, dir)?;
    let plan = Ahead::build(tools, dir, branch).map_err(|e| e.to_string())?;
    let ahead = match plan.commits.len() {
        0 => Ahead::build(tools, dir, "HEAD").map_err(|e| e.to_string())?.commits.len(),
        n => n,
    };
    match ahead {
        0 => Err(format!("no commit ahead of {}", plan.base)),
        1 => Ok(said(format!("1 commit ahead of {}", plan.base))),
        n => Ok(said(format!("{n} commits ahead of {}", plan.base))),
    }
}

pub fn tests(repo: &Path, dir: &Path) -> Result<Checked, String> {
    let command = match test_command::resolve(dir, repo) {
        Gate::Run(detected) => detected.command,
        Gate::Skip(why) => return Ok(said(why)),
        Gate::None => return Ok(said("No test command for this repo")),
    };
    let run = test_command::run_in_timeout(dir, &command, test_command::timeout_in(dir, repo));
    if run.exit_code == 0 {
        return Ok(said(format!("{command} passed")));
    }
    Err(format!("exit {} from `{command}`\n{}", run.exit_code, run.output))
}

pub fn proof(tools: &Tools, dir: &Path) -> Result<Checked, String> {
    clean(tools, dir).map(|()| said("Worktree clean"))
}

/// The open pull request whose head is the commit this worktree is on: the agent pushed its work
/// and opened it.
fn pull_request_at_head(tools: &Tools, dir: &Path) -> Result<Option<String>, String> {
    let Some(sha) = head_or_stopped(tools, dir)? else { return Ok(None) };
    let json = match run_gh(
        tools,
        dir,
        &["pr", "list", "--state", "open", "--json", "url,headRefOid", "--limit", "100"],
    ) {
        Ok(json) => json,
        Err(GhError::Stopped(why)) => return Err(why),
        Err(_) => return Ok(None),
    };
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else { return Ok(None) };
    Ok(list
        .iter()
        .find(|pr| pr["headRefOid"].as_str() == Some(sha.as_str()))
        .and_then(|pr| pr["url"].as_str())
        .map(str::to_owned))
}

pub fn hand_off(tools: &Tools, dir: &Path, branch: &str) -> Result<Checked, String> {
    let Some(url) = pull_request_at_head(tools, dir)? else {
        return Err(pr_ask::meeseeks_fix(branch, delivery::detect_base(tools, dir).as_deref()));
    };
    let detail = pr_label_of(&url)
        .map_or_else(|| "Found the pull request".to_owned(), |label| format!("Found {label}"));
    Ok(Checked { detail, pr_url: Some(url) })
}

pub fn poof(paths: &TidewatchPaths, worktree: &Worktree) -> Result<(), String> {
    let terms =
        Terms::new(BranchChoice::Keep, RemoveMode::Safe).why(Why::Poofed).owner(Owner::ThisWindow);
    worktree::remove_with(paths, worktree, &terms).map(|_| ()).map_err(|e| e.to_string())
}
