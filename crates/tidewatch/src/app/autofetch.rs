//! The window's side of background fetching: the open repos to the host each frame, and a fresh
//! look at the History of every tab of a repo that was just fetched.

use std::path::Path;
use std::time::Instant;

use tidewatch_core::prefs::GitPrefs;

use super::Tidewatch;
use crate::autofetch_glue;
use crate::prefs_save::update_in_background;

impl Tidewatch {
    pub(super) fn drive_autofetch(&mut self, ctx: &egui::Context) {
        let roots = autofetch_glue::roots(&self.workspaces);
        for root in self.autofetch.drive(ctx, &roots, Instant::now()) {
            self.reload_history_of(&root);
        }
    }

    fn reload_history_of(&mut self, root: &Path) {
        for (id, view) in &mut self.views {
            let in_repo = self
                .tabs
                .group_of(*id)
                .and_then(|group| self.workspaces.get(group))
                .is_some_and(|workspace| workspace.dir() == root);
            if in_repo {
                view.reload_history();
            }
        }
    }

    pub(super) fn set_git_prefs(&mut self, prefs: GitPrefs) {
        self.autofetch.set_prefs(prefs, Instant::now());
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.git = prefs);
        }
    }

    pub(super) fn fetch_failure(&self) -> Option<String> {
        self.autofetch.failure_note(self.workspace().dir())
    }
}
