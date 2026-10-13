//! The leftover worktrees in the app: the list the startup scan found, the dialog that looks at
//! each one and removes them, and the jobs behind it. `app.rs` only draws the chip and passes the
//! dialog its frame.

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::Leftover;

use crate::jobs::Jobs;
use crate::leftover_jobs::{self, Done};
use crate::leftover_view::{self, Answer};
use crate::leftovers::Cleanup;

pub(crate) struct Host {
    leftovers: Vec<Leftover>,
    jobs: Jobs<Done>,
    dialog: Option<Cleanup>,
    removed: usize,
}

impl Host {
    pub(crate) fn new() -> Self {
        Self { leftovers: Vec::new(), jobs: Jobs::new(), dialog: None, removed: 0 }
    }

    pub(crate) fn set(&mut self, leftovers: Vec<Leftover>) {
        self.leftovers = leftovers;
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
        for worktree in to_inspect {
            leftover_jobs::inspect(&self.jobs, ctx, worktree);
        }
        self.removed = 0;
        self.dialog = Some(cleanup);
    }

    /// Applies finished jobs. Returns the text for a toast, when a removal round ended.
    pub(crate) fn tick(&mut self) -> Option<String> {
        let mut finished_removal = false;
        for done in self.jobs.drain() {
            match done {
                Done::Inspected { path, inspection } => {
                    if let Some(dialog) = &mut self.dialog {
                        dialog.inspected(&path, inspection);
                    }
                }
                Done::Removed { path, result } => {
                    finished_removal = true;
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
            format!("Removed {n} leftover worktree{}", if n == 1 { "" } else { "s" })
        })
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
            Answer::Discard(path) => dialog.discard_removal(path).into_iter().collect(),
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
