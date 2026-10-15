//! The status bar panel: gathers what the bar shows from the window and draws it below the content.

use egui::{Frame, Panel};
use stdusk_egui::colors;

use super::Tidewatch;
use crate::commands::Platform;
use crate::reader::Shown;
use crate::status_bar::{self, Bar, Screen, Workspace};
use crate::{agent_frame, reader_glue};

impl Tidewatch {
    pub(super) fn show_status_bar(&self, ui: &mut egui::Ui, active_id: u64) {
        if self.welcome.is_some() {
            return;
        }
        let bar = self.status_bar(active_id);
        Panel::bottom("statusbar")
            .exact_size(status_bar::HEIGHT)
            .frame(Frame::new().fill(colors::titlebar()))
            .show(ui, |ui| status_bar::show(ui, ui.max_rect(), &bar));
    }

    fn status_bar(&self, active_id: u64) -> Bar {
        let counts = status_bar::counts(&agent_frame::tones(&self.tabs));
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        let (screen, place) = if self.settings.is_some() {
            (Screen::Settings, None)
        } else if self.overview {
            (Screen::Overview, None)
        } else {
            let workspace = Workspace {
                focus: self.focus,
                history: self.side_layout() && self.panes_showing(),
                file_open: self.file_view_showing(active_id),
                reader: reader_glue::shown(pane).map(|shown| shown == Shown::Reader),
            };
            let place = pane.agent.as_ref().and(pane.worktree.as_ref()).map(|worktree| {
                let base = self.bases.get(self.workspace().dir());
                status_bar::place(&worktree.branch, base)
            });
            (Screen::Workspace(workspace), place)
        };
        Bar { counts, place, hints: status_bar::hints(screen, Platform::CURRENT) }
    }
}
