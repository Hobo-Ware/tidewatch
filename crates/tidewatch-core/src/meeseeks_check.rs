//! The checks a Meeseeks has to pass, run off the UI thread: the commit check, the tests, the clean
//! tree, the pull request and the poof. Each says what it found when it passes, or what went wrong
//! in words for the agent.

use std::path::Path;

use crate::delivery::{self, DeliveryPlan};
use crate::meeseeks_log::pr_label_of;
use crate::paths::TidewatchPaths;
use crate::test_command;
use crate::tools::{Tools, finish};
use crate::worktree::{self, BranchChoice, RemoveMode, Worktree};

fn changed_files(tools: &Tools, dir: &Path) -> Result<usize, String> {
    let mut command = tools.git(dir);
    command.args(["status", "--porcelain"]);
    let out = finish(command).map_err(|e| format!("cannot run git: {e}"))?;
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

/// A check that passed: the one line it says, and the pull request it opened when it was the hand off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub detail: String,
    pub pr_url: Option<String>,
}

fn said(detail: impl Into<String>) -> Checked {
    Checked { detail: detail.into(), pr_url: None }
}

pub fn change(tools: &Tools, dir: &Path, branch: &str) -> Result<Checked, String> {
    clean(tools, dir)?;
    let plan = DeliveryPlan::build(tools, dir, branch, None).map_err(|e| e.to_string())?;
    match plan.commits.len() {
        0 => Err(format!("no commit ahead of {}", plan.base)),
        1 => Ok(said(format!("1 commit ahead of {}", plan.base))),
        n => Ok(said(format!("{n} commits ahead of {}", plan.base))),
    }
}

pub fn tests(repo: &Path, dir: &Path) -> Result<Checked, String> {
    let Some(command) = test_command::test_command(repo) else {
        return Ok(said("No test command for this repo"));
    };
    let run = test_command::run_in_timeout(dir, &command, test_command::timeout(repo));
    if run.exit_code == 0 {
        return Ok(said(format!("{command} passed")));
    }
    Err(format!("exit {} from `{command}`\n{}", run.exit_code, run.output))
}

pub fn proof(tools: &Tools, dir: &Path) -> Result<Checked, String> {
    clean(tools, dir).map(|()| said("Worktree clean"))
}

pub fn hand_off(tools: &Tools, dir: &Path, branch: &str, summary: &str) -> Result<Checked, String> {
    let plan = DeliveryPlan::build(tools, dir, branch, Some(summary)).map_err(|e| e.to_string())?;
    let url = delivery::deliver(tools, &plan, &plan.title, &plan.body)
        .map(|delivered| delivered.url)
        .map_err(|e| e.to_string())?;
    let detail = pr_label_of(&url).map_or_else(
        || "Opened the pull request".to_owned(),
        |label| format!("Opened {label} against {}", plan.base),
    );
    Ok(Checked { detail, pr_url: Some(url) })
}

pub fn poof(paths: &TidewatchPaths, worktree: &Worktree) -> Result<(), String> {
    worktree::remove(paths, worktree, BranchChoice::Keep, RemoveMode::Safe)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
