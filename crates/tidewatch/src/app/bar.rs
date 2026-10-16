//! The status bar panel: gathers what the bar shows from the window and draws it below the content.

use egui::{Frame, Panel};

use super::Tidewatch;
use crate::brand;
use crate::commands::Platform;
use crate::reader::Shown;
use crate::status_bar::{self, Bar, Input, Screen, Workspace};
use crate::{agent_frame, reader_glue};

impl Tidewatch {
    pub(super) fn show_status_bar(&mut self, ui: &mut egui::Ui, active_id: u64) {
        let bar = self.status_bar(active_id);
        let open_review = Panel::bottom("statusbar")
            .exact_size(status_bar::HEIGHT)
            .frame(Frame::new().fill(brand::SIDEBAR))
            .show(ui, |ui| status_bar::show(ui, ui.max_rect(), &bar))
            .inner;
        if open_review {
            self.open_review();
        }
    }

    fn input_in_tab(&self) -> Option<Input> {
        if self.awaiting_workflow_task() {
            Some(Input::Task)
        } else if self.awaiting_first_prompt() {
            Some(Input::Prompt)
        } else {
            None
        }
    }

    fn status_bar(&self, active_id: u64) -> Bar {
        let counts = status_bar::counts(&agent_frame::tones(&self.tabs));
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        let mut review = None;
        let (screen, place) = if self.welcome.is_some() {
            (Screen::Welcome, None)
        } else if self.settings.is_some() {
            (Screen::Settings, None)
        } else if self.overview {
            (Screen::Overview, None)
        } else {
            let workspace = Workspace {
                focus: self.focus,
                history: self.side_layout() && self.panes_showing(),
                file_open: self.file_view_showing(active_id),
                reader: reader_glue::shown(pane).map(|shown| shown == Shown::Reader),
                input: self.input_in_tab(),
            };
            review = self.review_chip(active_id);
            let place = pane.agent.as_ref().and(pane.worktree.as_ref()).map(|worktree| {
                let base = self.bases.get(self.workspace().dir());
                status_bar::place(&worktree.branch, base)
            });
            (Screen::Workspace(workspace), place)
        };
        Bar {
            counts,
            place,
            review,
            note_hint: status_bar::note_hint(screen),
            hints: status_bar::hints(screen, Platform::CURRENT),
            pr: self.pr_bar_item(active_id).filter(|_| matches!(screen, Screen::Workspace(_))),
        }
    }
}
