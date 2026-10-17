//! The draft tab in the central panel: the new-tab form and what its Start and Esc do.

use super::Tidewatch;
use super::frame::FrameIntent;
use crate::new_tab_form::{self, Out};
use crate::pane::Body;

impl Tidewatch {
    pub(super) fn active_is_draft(&self) -> bool {
        matches!(self.tabs.items()[self.tabs.active()].pane.body, Body::Draft(_))
    }

    pub(super) fn show_draft(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        ctx: &egui::Context,
        frame: &FrameIntent,
    ) {
        let index = self.tabs.active();
        let tab = &mut self.tabs.items_mut()[index];
        let (id, title, pane) = (tab.id, &mut tab.title, &mut tab.pane);
        let Body::Draft(form) = &mut pane.body else { return };
        let facts = self.bases.facts(form.repo());
        let out = new_tab_form::show(ui, area, form, &self.catalog, facts, !frame.keys.modal_open);
        if title != form.title() {
            form.title().clone_into(title);
        }
        match out {
            Some(Out::Start(request)) => self.start_draft(ctx, id, request),
            Some(Out::Close) => self.close_tab(index, ctx),
            None => {}
        }
    }
}
