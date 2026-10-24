//! Tabs moved from stdusk, the window's side: the receive runs on a job thread
//! (`crate::from_stdusk`), and what it brings is placed here, one shell tab per pane in the group
//! of its repo. A failure shows a notice and changes nothing; stdusk keeps its window.

use std::path::PathBuf;

use super::Tidewatch;
use crate::from_stdusk::{self, Landing, Place, Taken, Target};
use crate::pane::Pane;
use crate::tabs::Action;

pub(super) type Done = Result<Vec<Taken>, String>;

impl Tidewatch {
    pub(super) fn adopt_from_stdusk(&mut self, socket: PathBuf, ctx: &egui::Context) {
        let fallback = self.workspace().dir().to_path_buf();
        let ctx_for_job = ctx.clone();
        self.stdusk_jobs.spawn(ctx, move || from_stdusk::receive(&ctx_for_job, &socket, &fallback));
    }

    pub(super) fn apply_stdusk_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Ok(taken) => self.place_from_stdusk(taken, ctx),
            Err(reason) => {
                eprintln!("tidewatch: tabs from stdusk not taken: {reason}");
                self.notice = Some(format!(
                    "Moving tabs from stdusk failed: {reason}. stdusk kept its window and its tabs."
                ));
            }
        }
    }

    fn place_from_stdusk(&mut self, taken: Vec<Taken>, ctx: &egui::Context) {
        let landings: Vec<Landing> = taken
            .iter()
            .map(|pane| Landing { place: pane.place.clone(), focused: pane.focused })
            .collect();
        let held: Vec<Place> = if self.welcome.is_some() {
            Vec::new()
        } else {
            self.workspaces.iter().map(Place::of).collect()
        };
        let plan = from_stdusk::plan(&landings, &held);
        let replaces_welcome = self.welcome.is_some();

        let (mut workspaces, panes): (Vec<_>, Vec<_>) =
            taken.into_iter().map(|pane| (Some(pane.workspace), (pane.term, pane.name))).unzip();
        let groups: Vec<usize> = plan
            .new_groups
            .iter()
            .filter_map(|&at| workspaces[at].take())
            .map(|workspace| self.add_workspace(workspace, ctx))
            .collect();
        let mut ids = Vec::with_capacity(panes.len());
        for ((term, name), target) in panes.into_iter().zip(&plan.targets) {
            let group = match *target {
                Target::Existing(group) => group,
                Target::New(made) => groups[made],
            };
            ids.push(self.tabs.push_in(group, name, Pane::shell(term)));
        }
        let front = match plan.focus {
            Some(at) => {
                self.tabs.select_id(ids[at]);
                self.tabs.group_of(ids[at])
            }
            None if replaces_welcome => groups.first().copied(),
            None => None,
        };
        if let Some(group) = front {
            self.dispatch(Action::ShowGroup(group), ctx);
        }
        ctx.request_repaint();
    }
}
