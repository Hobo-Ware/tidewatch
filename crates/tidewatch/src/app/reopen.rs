//! Showing the tab of a scheduled item: select it, or bring a closed one back through the restore
//! path with the session and profile its schedule entry remembers.

use std::path::Path;

use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::restore::{RealDisk, RestoreEnv, WindowPlan, plan_window};

use super::Tidewatch;
use crate::coming_up::{Row, coming_up};
use crate::progress_glue::today;
use crate::reopen::{Target, saved_window};
use crate::tab_view::TabView;
use crate::tabs::Action;

impl Tidewatch {
    pub(super) fn open_tab_of(&self, repo: &Path, name: &str) -> Option<u64> {
        self.tabs
            .items()
            .iter()
            .find(|tab| {
                tab.pane
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| worktree.repo == repo && worktree.name() == name)
            })
            .map(|tab| tab.id)
    }

    /// Selects tab `id` and puts its Reader in front.
    pub(super) fn show_reader_of(&mut self, id: u64, ctx: &egui::Context) {
        if let Some(index) = self.tabs.index_of(id) {
            self.dispatch(Action::Select(index), ctx);
        }
        if let Some(agent) = self.tabs.agent_mut(id) {
            agent.reader.set_wants_reader(true);
        }
        if let Some(view) = self.views.get_mut(&id) {
            TabView::back_to_terminal(view);
        }
    }

    /// `TIDEWATCH_DEV_OPEN_SCHEDULED=<tab>`: once the profiles are loaded and the restore is done,
    /// does once what a click on a Coming up card of that tab does.
    pub(super) fn drive_dev_scheduled(&mut self, ctx: &egui::Context) {
        let Some(name) = self.dev_open_scheduled.clone() else { return };
        if self.catalog.profiles().is_none() || !self.restore.settled() {
            return;
        }
        let rows = coming_up(today(), self.progress.index());
        if let Some(row) = rows.iter().find(|row| row.tab == name) {
            self.dev_open_scheduled = None;
            self.open_scheduled(row, ctx);
        }
    }

    pub(super) fn open_scheduled(&mut self, row: &Row, ctx: &egui::Context) {
        if let Some(id) = self.open_tab_of(&row.repo, &row.tab) {
            self.show_reader_of(id, ctx);
            return;
        }
        let Some(group) = self.group_of_dir(&row.repo) else {
            self.show_toast(format!("Open {} first", row.repo.display()), ctx);
            return;
        };
        match self.plan_reopen(row) {
            Ok(plan) => {
                let now = ctx.input(|i| i.time);
                self.apply_taken_plan(&plan, group, ctx, now);
                if let Some(id) = self.open_tab_of_title(group, &row.tab) {
                    self.show_reader_of(id, ctx);
                }
            }
            Err(text) => self.show_toast(text, ctx),
        }
    }

    pub(super) fn open_tab_of_title(&self, group: usize, name: &str) -> Option<u64> {
        self.tabs.in_group(group).find(|(_, tab)| tab.title == name).map(|(_, tab)| tab.id)
    }

    fn plan_reopen(&self, row: &Row) -> Result<WindowPlan, String> {
        let (Some(paths), Some(profiles)) = (&self.paths, self.catalog.profiles()) else {
            return Err("Profiles are still loading".to_owned());
        };
        let target = Target {
            repo: &row.repo,
            tab: &row.tab,
            session_id: row.session_id.as_deref(),
            profile_id: row.profile_id.as_deref(),
        };
        let window = saved_window(paths, profiles, target).map_err(|why| why.text().to_owned())?;
        let agents = profiles.agent_table();
        let installed = agents.found_on_path(Some(&self.catalog.path));
        let env = RestoreEnv {
            agents: &agents,
            profiles,
            installed: &installed,
            resume: ResumeAgents::Auto,
            crash_loop: false,
        };
        Ok(plan_window(&window, &env, &RealDisk::from_env()))
    }
}
