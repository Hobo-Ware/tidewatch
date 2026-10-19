//! The repo menu's two items: opening another repo (a link or a folder picker), and closing the repo on
//! show one tab at a time.

use std::path::PathBuf;

use super::Tidewatch;
use crate::jobs::Jobs;
use crate::repo_close::{RepoClose, Step};
use crate::session_save::{WindowView, saved_window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepoCommand {
    Open,
    Close,
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
            RepoCommand::Open => self.open_repo_dialog(""),
            RepoCommand::Close => self.begin_close_repo(ctx),
        }
    }

    pub(super) fn pick_repo(&mut self, ctx: &egui::Context) {
        self.picker.start(ctx);
    }

    pub(super) fn drain_picked(&mut self, ctx: &egui::Context) {
        for path in self.picker.drain() {
            self.open_workspace(path, ctx);
        }
    }

    fn begin_close_repo(&mut self, ctx: &egui::Context) {
        if self.workspaces.len() < 2 || self.repo_close.is_some() || self.closing.is_some() {
            return;
        }
        if !self.restore.settled() {
            self.show_toast("Tabs are still coming back, try again in a moment", ctx);
            return;
        }
        self.repo_close = Some(RepoClose::new(self.workspace().dir().to_path_buf()));
        self.drive_repo_close(ctx);
    }

    /// Closes the repo's tabs through the same path as closing a tab, until a dialog is open or a
    /// tab is being removed. A tab that stays (kept, cancelled, failed or busy) ends the close.
    pub(super) fn drive_repo_close(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.repo_close.take() else { return };
        while self.closing.is_none() {
            let Some(group) = self.group_of_dir(&state.dir) else { return };
            match state.next(group, &self.tabs) {
                Step::Close(index) => self.close_tab(index, ctx),
                Step::Done => return self.finish_repo_close(group),
                Step::Busy(title) => {
                    let text = format!("{title} is still starting, so the repo stays open");
                    return self.show_toast(text, ctx);
                }
                Step::Stop => return,
                Step::Wait => break,
            }
        }
        self.repo_close = Some(state);
    }

    fn finish_repo_close(&mut self, group: usize) {
        let view = WindowView::of(group, &self.workspaces[group], self.focus);
        let window = saved_window(&view, &self.tabs);
        let gone = self.tabs.remove_group(group);
        if gone.is_empty() {
            return;
        }
        self.saver.retire(window);
        for id in gone {
            self.forget_tab(id);
        }
        self.workspaces.remove(group);
        self.last_used.record(&super::workspaces::remembered(&self.workspaces, &self.tabs));
    }
}
