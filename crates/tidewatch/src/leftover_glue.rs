//! The leftover worktrees in the app: the list the startup scan found, the dialog that looks at
//! each one and removes them, and the jobs behind it. `app.rs` only draws the chip and passes the
//! dialog its frame.

use std::path::Path;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{BranchOutcome, Leftover};

use crate::jobs::Jobs;
use crate::leftover_jobs::{self, Done};
use crate::leftover_view::{self, Answer};
use crate::leftovers::Cleanup;

/// `found` joined into `known` by path, sorted by path.
pub(crate) fn merge(known: &mut Vec<Leftover>, found: Vec<Leftover>) {
    for leftover in found {
        if !known.iter().any(|k| k.path == leftover.path) {
            known.push(leftover);
        }
    }
    known.sort_by(|a, b| a.path.cmp(&b.path));
}

fn kept_note(commits: usize, branch: Option<String>) -> String {
    let noun = if commits == 1 { "commit" } else { "commits" };
    let note = format!("branch kept: {commits} {noun} only here");
    match branch {
        Some(branch) => format!("{note} ({branch})"),
        None => note,
    }
}

pub(crate) struct Host {
    paths: Option<TidewatchPaths>,
    leftovers: Vec<Leftover>,
    jobs: Jobs<Done>,
    dialog: Option<Cleanup>,
    removed: usize,
    restores: Vec<String>,
    kept: Vec<String>,
}

impl Host {
    pub(crate) fn new(paths: Option<TidewatchPaths>) -> Self {
        Self {
            paths,
            leftovers: Vec::new(),
            jobs: Jobs::new(),
            dialog: None,
            removed: 0,
            restores: Vec::new(),
            kept: Vec::new(),
        }
    }

    /// Adds what a startup scan found. Each repo of the window scans, and every scan sees the
    /// whole tree, so the same worktree can arrive twice.
    pub(crate) fn add(&mut self, found: Vec<Leftover>) {
        merge(&mut self.leftovers, found);
    }

    pub(crate) fn count(&self) -> usize {
        self.leftovers.len()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The chip in the tab strip, when there is anything left over. A click opens the dialog.
    pub(crate) fn chip(&mut self, ui: &mut egui::Ui) {
        if self.leftovers.is_empty() {
            return;
        }
        ui.add_space(4.0);
        if leftover_view::chip(ui, &self.leftovers) {
            self.open(ui.ctx());
        }
        ui.add_space(8.0);
    }

    pub(crate) fn open(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let (cleanup, to_inspect) = Cleanup::open(&self.leftovers);
        if let Some(paths) = &self.paths {
            for worktree in to_inspect {
                leftover_jobs::inspect(&self.jobs, ctx, paths.clone(), worktree);
            }
        }
        self.removed = 0;
        self.restores.clear();
        self.kept.clear();
        self.dialog = Some(cleanup);
    }

    /// Applies finished jobs. Returns the text for a toast, when a removal round ended.
    pub(crate) fn tick(&mut self) -> Option<String> {
        let mut finished_removal = false;
        for done in self.jobs.drain() {
            match done {
                Done::Inspected { path, safety } => {
                    if let Some(dialog) = &mut self.dialog {
                        dialog.inspected(&path, safety);
                    }
                }
                Done::Unreadable { path } => {
                    if let Some(dialog) = &mut self.dialog {
                        dialog.unreadable(&path);
                    }
                }
                Done::Trashed { path, result } => {
                    finished_removal = true;
                    let gone = match &mut self.dialog {
                        Some(dialog) => dialog.removed(&path, result.map(|()| BranchOutcome::Kept)),
                        None => result.is_ok(),
                    };
                    if gone {
                        self.leftovers.retain(|l| l.path != path);
                        self.removed += 1;
                        self.kept.push("the unreadable folder is in the Trash".to_owned());
                    }
                }
                Done::Removed { path, result } => {
                    finished_removal = true;
                    match &result {
                        Ok(BranchOutcome::Deleted { restore: Some(command) }) => {
                            self.restores.push(command.clone());
                        }
                        Ok(BranchOutcome::KeptCommits { commits }) => {
                            self.kept.push(kept_note(*commits, self.branch_of(&path)));
                        }
                        _ => {}
                    }
                    let gone = match &mut self.dialog {
                        Some(dialog) => dialog.removed(&path, result),
                        None => result.is_ok(),
                    };
                    if gone {
                        self.leftovers.retain(|l| l.path != path);
                        self.removed += 1;
                    }
                }
            }
        }
        let settled = self.dialog.as_ref().is_none_or(|dialog| !dialog.busy());
        (finished_removal && settled && self.removed > 0).then(|| {
            let n = std::mem::take(&mut self.removed);
            let removed = format!("Removed {n} leftover worktree{}", if n == 1 { "" } else { "s" });
            let removed = match std::mem::take(&mut self.restores).as_slice() {
                [] => removed,
                [command] => format!("{removed}. To bring the branch back: {command}"),
                _ => format!(
                    "{removed}. To bring a branch back, take its name and commit from .state/removals.log: git branch <name> <sha>"
                ),
            };
            match std::mem::take(&mut self.kept).as_slice() {
                [] => removed,
                notes => format!("{removed}. {}", notes.join(". ")),
            }
        })
    }

    fn branch_of(&self, path: &Path) -> Option<String> {
        self.leftovers.iter().find(|l| l.path == path)?.branch.clone()
    }

    /// Draws the dialog while it is open and starts what it was asked to do.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
        trust: Option<&Grant>,
    ) {
        let Some(dialog) = &self.dialog else { return };
        if let Some(answer) = leftover_view::dialog(ctx, dialog) {
            self.answer(ctx, paths, trust, &answer);
        }
    }

    /// For `TIDEWATCH_DEV_LEFTOVERS=clean`: presses Clean up once every worktree has been looked at.
    /// True once it was pressed.
    pub(crate) fn press_clean_up(
        &mut self,
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
    ) -> bool {
        let ready = self.dialog.as_ref().is_some_and(|d| !d.busy() && d.clean_count() > 0);
        if ready {
            self.answer(ctx, paths, None, &Answer::CleanUp);
        }
        ready
    }

    /// Closes the dialog, or starts the removals the answer stands for.
    pub(crate) fn answer(
        &mut self,
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
        trust: Option<&Grant>,
        answer: &Answer,
    ) {
        let Some(dialog) = &mut self.dialog else { return };
        let removals = match answer {
            Answer::Close => {
                self.dialog = None;
                return;
            }
            Answer::CleanUp => dialog.clean_removals(),
            Answer::Discard(path, branch) => {
                dialog.discard_removal(path, *branch).into_iter().collect()
            }
            Answer::Trash(path) => {
                let (Some((target, known)), Some(paths)) = (dialog.trash_target(path), paths)
                else {
                    return;
                };
                dialog.removing(&target);
                leftover_jobs::trash(&self.jobs, ctx, paths.clone(), target, known);
                return;
            }
        };
        let Some(paths) = paths else { return };
        for removal in removals {
            dialog.removing(&removal.worktree.path);
            leftover_jobs::remove(&self.jobs, ctx, paths.clone(), removal, trust.cloned());
        }
    }
}

#[cfg(test)]
mod tests;
