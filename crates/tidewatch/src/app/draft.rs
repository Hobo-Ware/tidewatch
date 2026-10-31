//! The draft tab in the central panel: the new-tab form and what its Start and Esc do.

use tidewatch_core::paths::TidewatchPaths;

use super::Tidewatch;
use super::frame::FrameIntent;
use crate::catalog::Catalog;
use crate::new_tab::{Kind, NewTab};
use crate::new_tab_form::{self, Out};
use crate::pane::Body;
use crate::slash_glue;

fn offer_slash_commands(
    ctx: &egui::Context,
    form: &mut NewTab,
    catalog: &Catalog,
    (slash, files): (&mut slash_glue::Host, &mut crate::files_glue::Host),
    paths: Option<&TidewatchPaths>,
) {
    if form.text().contains('@') {
        let folder = form.repo().to_path_buf();
        form.files = files.files(ctx, &folder);
    }
    let claude = form.selected_agent(catalog).is_some_and(|agent| agent.id == "claude");
    if claude && form.text().starts_with('/') {
        let claude_dir = paths.map(|paths| paths.claude_dir().to_path_buf());
        let folder = form.repo().to_path_buf();
        form.commands = slash.entries(ctx, claude_dir.as_deref(), &folder);
    } else if !form.commands.is_empty() {
        form.commands = std::sync::Arc::default();
    }
}

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
        if form.kind() != Kind::Shell {
            form.insert_files(ctx, self.dev_reader.take_pictures());
        }
        offer_slash_commands(
            ctx,
            form,
            &self.catalog,
            (&mut self.slash, &mut self.files),
            self.paths.as_ref(),
        );
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
