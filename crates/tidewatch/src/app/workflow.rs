//! The window's side of the workflow profiles: the repos to the host each frame, the confirm-once
//! card for the repo on show, and the choices made in Settings.

use std::path::{Path, PathBuf};

use egui::Rect;
use tidewatch_core::workflow_profile::Workflow;

use super::Tidewatch;
use crate::repo::Workspace;
use crate::workflow_view::{self, Answer};

impl Tidewatch {
    pub(super) fn drive_workflow(&mut self, ctx: &egui::Context) {
        let repos: Vec<&Path> = self
            .workspaces
            .iter()
            .filter_map(|workspace| match workspace {
                Workspace::Repo(repo) => Some(repo.root.as_path()),
                Workspace::Other { .. } => None,
            })
            .collect();
        self.workflow.sync(&repos, ctx);
        self.workflow.drive();
    }

    fn workflow_repo(&self) -> Option<(PathBuf, String)> {
        match self.workspaces.get(self.tabs.active_group())? {
            Workspace::Repo(repo) => Some((repo.root.clone(), repo.label.clone())),
            Workspace::Other { .. } => None,
        }
    }

    pub(super) fn show_workflow_card(&mut self, ctx: &egui::Context, column: Rect) {
        let workspace = !self.overview && self.settings.is_none() && self.welcome.is_none();
        let Some((repo, label)) = self.workflow_repo().filter(|_| workspace) else { return };
        let Some(detected) = self.workflow.proposal(&repo) else { return };
        let Some(answer) = workflow_view::card(ctx, &label, detected, column) else { return };
        match answer {
            Answer::Accept => {
                self.workflow.accept(&repo);
                self.show_toast(format!("Saved the workflow for {label}"), ctx);
            }
            Answer::Change => {
                self.workflow.dismiss(&repo);
                self.open_updates_settings();
            }
            Answer::NotNow => self.workflow.dismiss(&repo),
        }
    }

    pub(super) fn workflow_view(&self) -> Option<workflow_view::View> {
        let (path, repo) = self.workflow_repo()?;
        Some(workflow_view::View {
            workflow: self.workflow.profile(&path),
            detected: self.workflow.detected(&path).cloned(),
            confirmed: self.workflow.confirmed(&path),
            path,
            repo,
        })
    }

    pub(super) fn set_workflow(&mut self, repo: &Path, workflow: Workflow) {
        self.workflow.set(repo, workflow);
    }
}
