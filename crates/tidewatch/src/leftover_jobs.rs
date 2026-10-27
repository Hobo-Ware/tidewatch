//! The git work behind the leftover worktrees dialog, run on `Jobs` threads: counting what each
//! worktree would lose, and removing one (with its folder-trust entries, like closing a tab does).

use std::path::PathBuf;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, BranchOutcome, Terms, Why, Worktree};
use tidewatch_core::worktree_safety::{Owner, Safety};

use crate::jobs::Jobs;
use crate::leftovers::Removal;

pub(crate) enum Done {
    Inspected { path: PathBuf, safety: Safety },
    Removed { path: PathBuf, result: Result<BranchOutcome, String> },
}

pub(crate) fn inspect(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    worktree: Worktree,
) {
    jobs.spawn(ctx, move || Done::Inspected {
        safety: worktree::inspect(&paths, &worktree, Owner::Nobody),
        path: worktree.path,
    });
}

pub(crate) fn remove(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    removal: Removal,
    trust: Option<Grant>,
) {
    jobs.spawn(ctx, move || {
        let Removal { worktree, branch, mode } = removal;
        let terms = Terms::new(branch, mode).why(Why::Leftover);
        let result = worktree::remove_with(&paths, &worktree, &terms).map_err(|e| e.to_string());
        if result.is_ok()
            && let Some(grant) = &trust
        {
            let _ = grant.revoke(Some(&worktree.path));
        }
        Done::Removed { path: worktree.path, result }
    });
}

#[cfg(test)]
mod tests;
