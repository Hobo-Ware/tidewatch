//! Recently closed, the window's side: what a closing agent tab leaves in the list, reading the list
//! while the Overview shows, and bringing a tab back (the job makes its worktree, the tab opens
//! through the restore path like a taken-over session).

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use tidewatch_core::closed_tabs::Tab;
use tidewatch_core::restore::RealDisk;

use super::Tidewatch;
use crate::closed_jobs::{self, Done, Landed, ReviveSpec};
use crate::closed_state::{Card, CardAction, fake_card, fake_requested};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

impl Tidewatch {
    pub(super) fn closed_facts(&self, id: u64) -> Option<Tab> {
        let tab = self.tabs.items().iter().find(|tab| tab.id == id)?;
        let agent = tab.pane.agent.as_ref()?;
        tab.pane.worktree.as_ref()?;
        let launched = agent.launched.as_ref()?;
        Some(Tab {
            title: tab.title.clone(),
            agent: agent.run.agent_id.clone(),
            profile: launched.profile_id.clone(),
            level: launched.level,
            session_id: launched.session_id.clone(),
        })
    }

    pub(super) fn note_kept_tab(&self, id: u64, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone() else { return };
        let Some(facts) = self.closed_facts(id) else { return };
        let Some(worktree) =
            self.tabs.items().iter().find(|tab| tab.id == id).and_then(|t| t.pane.worktree.clone())
        else {
            return;
        };
        closed_jobs::note_kept(&self.closed_jobs, ctx, paths, worktree, facts);
    }

    fn open_sessions(&self) -> HashSet<String> {
        self.tabs
            .items()
            .iter()
            .filter_map(|tab| tab.pane.agent.as_ref()?.launched.as_ref()?.session_id.clone())
            .collect()
    }

    pub(super) fn drive_closed(&mut self, ctx: &egui::Context) {
        for done in self.closed_jobs.drain() {
            self.apply_closed_done(done, ctx);
        }
        let shown = self.overview
            && self.settings.is_none()
            && self.welcome.is_none()
            && !self.shell.window_hidden();
        if let Some(paths) = self.paths.clone()
            && self.closed.begin_load(shown)
        {
            closed_jobs::load(&self.closed_jobs, ctx, paths, RealDisk::from_env());
        }
    }

    pub(super) fn closed_card(&self) -> Option<Card> {
        if fake_requested() {
            return Some(fake_card(self.closed.is_open()));
        }
        let open = self.open_sessions();
        self.closed.card(now_secs(), &|id| open.contains(id), &|repo| self.place_label(repo))
    }

    pub(super) fn closed_click(&mut self, action: CardAction, ctx: &egui::Context) {
        match action {
            CardAction::Toggle => self.closed.toggle(),
            CardAction::Reopen(session_id) => self.reopen_closed(&session_id, ctx),
        }
    }

    fn reopen_closed(&mut self, session_id: &str, ctx: &egui::Context) {
        let (Some(paths), Some(profiles)) = (self.paths.clone(), self.catalog.profiles().cloned())
        else {
            self.show_toast("Profiles are still loading", ctx);
            return;
        };
        let Some(closed) = self.closed.press(session_id, now_secs()) else { return };
        let spec = ReviveSpec {
            paths,
            closed,
            profiles,
            path_var: self.catalog.path.clone(),
            disk: RealDisk::from_env(),
        };
        closed_jobs::revive(&self.closed_jobs, ctx, spec);
    }

    fn apply_closed_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Done::Loaded(records) => self.closed.finish_load(records),
            Done::Noted => self.closed.mark_changed(),
            Done::Refused { session_id, text } => {
                self.closed.finished(&session_id);
                self.show_toast(text, ctx);
            }
            Done::Revived(landed) => self.place_revived(*landed, ctx),
        }
    }

    fn place_revived(&mut self, landed: Landed, ctx: &egui::Context) {
        let Landed { closed, workspace, plan, notes } = landed;
        if let Some(id) = &closed.session_id {
            self.closed.finished(id);
        }
        let dir = workspace.dir().to_path_buf();
        let group = match self.group_of_dir(&dir) {
            Some(group) => group,
            None => self.add_workspace(workspace, ctx),
        };
        let now = ctx.input(|i| i.time);
        self.apply_taken_plan(&plan, group, ctx, now);
        if let Some(note) = notes.first() {
            self.show_toast(format!("Started without part of the setup: {note}"), ctx);
        }
        let name = closed.worktree().name().to_owned();
        if let Some(id) = self.open_tab_of_title(group, &name) {
            self.show_reader_of(id, ctx);
        }
    }
}
