//! The repo menu's folder picker, which opens another repo.

use std::path::PathBuf;

use super::Tidewatch;
use crate::jobs::Jobs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepoCommand {
    Open,
}

/// The folder dialog, on a job thread so the window keeps drawing; one at a time.
pub(super) struct Picker {
    jobs: Jobs<Option<PathBuf>>,
    open: bool,
}

impl Picker {
    pub(super) fn new() -> Self {
        Self { jobs: Jobs::new(), open: false }
    }

    fn start(&mut self, ctx: &egui::Context) {
        if !self.open {
            self.open = true;
            self.jobs.spawn(ctx, || rfd::FileDialog::new().set_title("Open a repo").pick_folder());
        }
    }

    fn drain(&mut self) -> Vec<PathBuf> {
        let answers = self.jobs.drain();
        self.open &= answers.is_empty();
        answers.into_iter().flatten().collect()
    }
}

impl Tidewatch {
    pub(super) fn run_repo_command(&mut self, command: RepoCommand, ctx: &egui::Context) {
        match command {
            RepoCommand::Open => self.pick_repo(ctx),
        }
    }

    fn pick_repo(&mut self, ctx: &egui::Context) {
        self.picker.start(ctx);
    }

    pub(super) fn drain_picked(&mut self, ctx: &egui::Context) {
        for path in self.picker.drain() {
            self.open_workspace(path, ctx);
        }
    }
}
