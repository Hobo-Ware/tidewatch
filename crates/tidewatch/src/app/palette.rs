//! The command palette in the app: opening it, what it lists, and running what it picked.

use super::Tidewatch;
use crate::commands::Platform;
use crate::new_tab_workflow;
use crate::palette::{self, Palette, Sources, Target};
use crate::palette_view::{self, Outcome};
use crate::repo::Workspace;
use crate::tabs::Action;

impl Tidewatch {
    pub(super) fn toggle_palette(&mut self) {
        if self.palette.take().is_none() {
            self.open_palette("");
        }
    }

    pub(super) fn open_palette(&mut self, query: &str) {
        self.palette = Some(Palette::open(query));
    }

    fn palette_sources(&self) -> Sources {
        let profiles = self
            .catalog
            .entries()
            .into_iter()
            .map(|(id, profile, _)| (id.to_owned(), profile.name.clone()))
            .collect();
        let workflows = match (&self.workspace, self.catalog.workflows()) {
            (Workspace::Repo(_), Some(workflows)) => new_tab_workflow::ordered(workflows)
                .into_iter()
                .map(|workflow| (workflow.id.clone(), workflow.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        Sources {
            tabs: self.tabs.items().iter().map(|tab| tab.title.clone()).collect(),
            active: self.tabs.active(),
            profiles,
            workflows,
        }
    }

    pub(super) fn show_palette(&mut self, ctx: &egui::Context) {
        let Some(mut open) = self.palette.take() else { return };
        let items = palette::items(&self.palette_sources(), Platform::CURRENT);
        match palette_view::show(ctx, &mut open, &items) {
            Some(Outcome::Close) => {}
            Some(Outcome::Run(target)) => self.run_target(target, ctx),
            None => self.palette = Some(open),
        }
    }

    fn run_target(&mut self, target: Target, ctx: &egui::Context) {
        match target {
            Target::Command(command) => self.run_command(command, ctx),
            Target::Tab(index) => self.dispatch(Action::Select(index), ctx),
            Target::Profile(id) => self.open_new_tab_dialog(ctx, Some(&id)),
            Target::Workflow(id) => self.open_new_tab_dialog(ctx, Some(&format!("workflow:{id}"))),
        }
    }

    fn open_new_tab_dialog(&mut self, ctx: &egui::Context, preselect: Option<&str>) {
        self.settings = None;
        self.overview = false;
        self.new_tab_on(ctx, preselect);
    }
}
