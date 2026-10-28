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
    Unreadable { path: PathBuf },
    Removed { path: PathBuf, result: Result<BranchOutcome, String> },
    Trashed { path: PathBuf, result: Result<(), String> },
}

pub(crate) fn inspect(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    worktree: Worktree,
) {
    jobs.spawn(ctx, move || {
        let safety = worktree::inspect(&paths, &worktree, Owner::Nobody);
        let path = worktree.path;
        if matches!(safety, Safety::Unknown(_)) && worktree::is_unreadable(&path) {
            Done::Unreadable { path }
        } else {
            Done::Inspected { safety, path }
        }
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
        let Removal { worktree, branch, mode, spare_unique } = removal;
        let terms = Terms::new(branch, mode).why(Why::Leftover).spare_unique(spare_unique);
        let result = worktree::remove_with(&paths, &worktree, &terms).map_err(|e| e.to_string());
        if result.is_ok()
            && let Some(grant) = &trust
        {
            let _ = grant.revoke(Some(&worktree.path));
        }
        Done::Removed { path: worktree.path, result }
    });
}

pub(crate) fn trash(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    path: PathBuf,
    known: Option<Worktree>,
) {
    jobs.spawn(ctx, move || {
        let result = worktree::discard_unreadable(
            &paths,
            &path,
            known.as_ref(),
            crate::trash::move_to_trash,
        )
        .map_err(|e| e.to_string());
        Done::Trashed { path, result }
    });
}

#[cfg(test)]
mod tests;
