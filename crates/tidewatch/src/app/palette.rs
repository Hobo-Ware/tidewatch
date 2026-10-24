//! The command palette in the app: opening it, what it lists, and running what it picked.

use super::Tidewatch;
use super::repos::RepoCommand;
use crate::commands::Platform;
use crate::new_tab_workflow;
use crate::palette::{self, Palette, Sources, TabRow, Target};
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
        let workflows = match (self.workspace(), self.catalog.workflows()) {
            (Workspace::Repo(_), Some(workflows)) => new_tab_workflow::ordered(workflows)
                .into_iter()
                .map(|workflow| (workflow.id.clone(), workflow.name.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let group = self.tabs.active_group();
        let active = self.tabs.in_group(group).position(|(i, _)| i == self.tabs.active());
        Sources {
            tabs: self
                .tabs
                .in_group(group)
                .map(|(_, tab)| {
                    TabRow::new(
                        crate::tab_title::label(tab.pane.agent.as_ref(), &tab.title),
                        tab.park.is_some(),
                    )
                })
                .collect(),
            active: active.unwrap_or_default(),
            repos: self.workspaces.iter().map(Workspace::chip_label).collect(),
            repo: group,
            profiles,
            workflows,
            build_workflow_blocked: self.build_workflow_blocked(),
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
            Target::Tab(index) => {
                let group = self.tabs.active_group();
                let picked = self.tabs.in_group(group).nth(index).map(|(i, _)| i);
                if let Some(i) = picked {
                    self.dispatch(Action::Select(i), ctx);
                }
            }
            Target::Repo(group) => self.dispatch(Action::ShowGroup(group), ctx),
            Target::OpenRepo => self.run_repo_command(RepoCommand::Open, ctx),
            Target::CloseRepo => self.run_repo_command(RepoCommand::Close, ctx),
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
