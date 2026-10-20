//! The checks a Meeseeks has to pass, run off the UI thread: the commit check, the tests, the clean
//! tree, the pull request and the poof. Each says `Ok` or what went wrong in words for the agent.

use std::path::Path;

use crate::delivery::{self, DeliveryPlan};
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

/// The worktree is clean and the branch has at least one commit the base does not have.
pub fn change(tools: &Tools, dir: &Path, branch: &str) -> Result<(), String> {
    clean(tools, dir)?;
    let plan = DeliveryPlan::build(tools, dir, branch, None).map_err(|e| e.to_string())?;
    if plan.commits.is_empty() {
        return Err(format!("no commit ahead of {}", plan.base));
    }
    Ok(())
}

/// The repo's test command passes in the worktree. A repo with none passes.
pub fn tests(repo: &Path, dir: &Path) -> Result<(), String> {
    let Some(command) = test_command::test_command(repo) else { return Ok(()) };
    let run = test_command::run_in_timeout(dir, &command, test_command::timeout(repo));
    if run.exit_code == 0 {
        return Ok(());
    }
    Err(format!("exit {} from `{command}`\n{}", run.exit_code, run.output))
}

/// The tests and the agent left nothing loose behind.
pub fn proof(tools: &Tools, dir: &Path) -> Result<(), String> {
    clean(tools, dir)
}

/// Pushes the branch and opens the pull request through the delivery path, never the agent. The
/// summary is the title. Returns the pull request's address.
pub fn hand_off(tools: &Tools, dir: &Path, branch: &str, summary: &str) -> Result<String, String> {
    let plan = DeliveryPlan::build(tools, dir, branch, Some(summary)).map_err(|e| e.to_string())?;
    delivery::deliver(tools, &plan, &plan.title, &plan.body)
        .map(|delivered| delivered.url)
        .map_err(|e| e.to_string())
}

/// Removes the worktree folder and keeps the branch. A dirty tree is refused and nothing changes.
pub fn poof(paths: &TidewatchPaths, worktree: &Worktree) -> Result<(), String> {
    worktree::remove(paths, worktree, BranchChoice::Keep, RemoveMode::Safe)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
