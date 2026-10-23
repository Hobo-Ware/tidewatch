//! Opening, closing and answering tabs: the new-tab draft, worktree jobs and the close flow.

use std::hash::{BuildHasher, Hasher, RandomState};

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::worktree::BranchOutcome;

use super::{Closing, Tidewatch};
use crate::close_dialog::{self, Answer, Choice};
use crate::dev_worktree::DevWorktree;
use crate::new_tab::{Group, MeeseeksRequest, NewTab, Request};
use crate::pane::{
    CREATING_TITLE, ClosePolicy, Pane, STARTING_TITLE, agent_tabs_in_folder, finish_open,
};
use crate::pty::{spawn_agent, spawn_shell};
use crate::repo::Workspace;
use crate::tabs::Action;
use crate::workflow_glue::Start;
use crate::worktree_jobs::{self, Done, OpenSpec, Target};

fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish()
}

impl Tidewatch {
    /// Opens a draft tab: the new-tab form, with nothing made until Start. Without a home
    /// directory there are no profiles to pick from, so the tab opens as a plain shell.
    fn new_tab(&mut self, ctx: &egui::Context) {
        self.new_tab_on(ctx, None);
    }

    pub(super) fn new_tab_on(&mut self, ctx: &egui::Context, preselect: Option<&str>) {
        if self.welcome.is_some() {
            self.run_repo_command(super::repos::RepoCommand::Open, ctx);
            return;
        }
        if self.paths.is_none() {
            self.open_tab(ctx, Request::Shell);
            return;
        }
        let state = self.draft_state(preselect);
        self.open_draft(ctx, state);
    }

    pub(super) fn open_draft(&mut self, ctx: &egui::Context, state: NewTab) -> u64 {
        self.bases.refresh(ctx, state.repo(), self.paths.as_ref());
        let title = state.title().to_owned();
        self.tabs.push(title, Pane::draft(state))
    }

    pub(super) fn draft_state(&self, preselect: Option<&str>) -> NewTab {
        let (group, label, dir) = match self.workspace() {
            Workspace::Repo(repo) => (Group::Repo, repo.label.clone(), repo.root.clone()),
            Workspace::Other { dir, label } => {
                (Group::Other, format!("Other: {label}"), dir.clone())
            }
        };
        let running = agent_tabs_in_folder(&self.tabs);
        let last = self.last_profile.as_deref();
        let mut state = NewTab::open(group, label, dir, running, last, random_seed())
            .with_defaults(self.new_tab_prefs.clone());
        state.settle(&self.catalog);
        if let Some(preselect) = preselect {
            state.jump(preselect, &self.catalog);
        }
        state
    }

    /// Turns the tab `tab` into what the draft asked for.
    pub(super) fn start_draft(&mut self, ctx: &egui::Context, tab: u64, request: Request) {
        match &request {
            Request::Agent(agent) => self.last_profile = Some(agent.profile_id.clone()),
            Request::Meeseeks(request) => {
                let agent = request.tabs.first().map(|tab| tab.agent.id.clone());
                self.remember_meeseeks(request.ticks, agent);
                self.show_toast("Caaan do!", ctx);
            }
            Request::Shell | Request::Workflow(_) => {}
        }
        self.open_tab_in(ctx, request, Some(tab));
    }

    fn remember_meeseeks(
        &mut self,
        ticks: tidewatch_core::meeseeks::Criteria,
        agent: Option<String>,
    ) {
        self.new_tab_prefs.meeseeks = ticks;
        self.new_tab_prefs.meeseeks_agent.clone_from(&agent);
        if let Some(paths) = self.paths.as_ref().filter(|_| self.screenshot.is_none())
            && let Err(e) = tidewatch_core::prefs::Prefs::update(paths, |p| {
                p.new_tab.meeseeks = ticks;
                p.new_tab.meeseeks_agent = agent;
            })
        {
            eprintln!("tidewatch: cannot save the Meeseeks choices: {e}");
        }
    }

    fn summon(&mut self, ctx: &egui::Context, request: MeeseeksRequest, into: Option<u64>) {
        let mut draft = into;
        for agent in request.tabs {
            self.open_tab_in(ctx, Request::Agent(Box::new(agent)), draft.take());
        }
    }

    pub(super) fn open_tab(&mut self, ctx: &egui::Context, request: Request) {
        self.open_tab_in(ctx, request, None);
    }

    /// The tab a new tab's pane goes into: the draft that asked for it, else one at the end.
    fn claim_tab(&mut self, into: Option<u64>, title: &str, pane: Pane<PtyTerm>) -> u64 {
        match into.and_then(|id| self.tabs.get_mut(id)) {
            Some(found) => {
                title.clone_into(&mut found.title);
                found.pane = pane;
                let id = found.id;
                self.tabs.select_id(id);
                id
            }
            None => self.tabs.push(title.to_owned(), pane),
        }
    }

    /// In a git repo a new tab is a fresh worktree; in the Other group it is a plain shell, or an
    /// agent in the folder itself. An agent tab is prepared on a job thread first. `into` is the
    /// draft tab the new tab takes the place of.
    fn open_tab_in(&mut self, ctx: &egui::Context, request: Request, into: Option<u64>) {
        let agent = match request {
            Request::Shell => None,
            Request::Agent(agent) => Some(agent),
            Request::Meeseeks(request) => {
                self.summon(ctx, *request, into);
                return;
            }
            Request::Workflow(request) => {
                self.open_workflow(ctx, Start::New(request), None, into);
                return;
            }
        };
        let now = ctx.input(|i| i.time);
        let pane = match &agent {
            Some(request) => {
                Pane::creating_agent(request.profile.name.clone(), request.agent.id.clone(), now)
            }
            None => Pane::creating(),
        };
        let listener = self.hooks.listener();
        let trust = self.trust.grant();
        let group = self.tabs.active_group();
        let placement = agent.as_ref().map(|agent| agent.placement.clone()).unwrap_or_default();
        match (&self.workspaces[group], &self.paths) {
            (Workspace::Repo(repo), Some(paths)) => {
                let (repo, paths) = (repo.root.clone(), paths.clone());
                let tab = self.claim_tab(into, CREATING_TITLE, pane);
                let target = Target::Worktree { repo, placement };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec { paths, target, agent, path_var, tab, listener, trust };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            (workspace, Some(paths)) if agent.is_some() => {
                let dir = workspace.dir().to_path_buf();
                let paths = paths.clone();
                let tab = self.claim_tab(into, STARTING_TITLE, pane);
                let target = Target::Folder { dir };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec { paths, target, agent, path_var, tab, listener, trust };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            _ => {
                let dir = self.workspace().dir().to_path_buf();
                if let Some(draft) = into {
                    self.tabs.remove_id(draft);
                    self.forget_tab(draft);
                }
                self.tabs.apply(Action::New, || Pane::shell(spawn_shell(ctx, &dir)));
            }
        }
    }

    pub(super) fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        let Some(tab) = self.tabs.items().get(index) else { return };
        if tab.pinned {
            return;
        }
        let id = tab.id;
        match tab.pane.close_policy() {
            ClosePolicy::Now => {
                self.tabs.apply(Action::Close(index), || unreachable!());
                self.forget_tab(id);
            }
            ClosePolicy::Wait => {}
            ClosePolicy::Confirm(worktree) => {
                if self.closing.is_none() {
                    self.closing = Some(Closing::Inspecting(id));
                    worktree_jobs::inspect(&self.jobs, ctx, id, worktree);
                }
            }
        }
    }

    pub(super) fn forget_tab(&mut self, id: u64) {
        self.hooks.close_tab(id);
        self.notifier.forget(id);
        self.parking.forget(id);
        self.flow.forget(id);
        self.pr.forget(id);
        self.runs.forget(id);
        self.meeseeks.forget(id);
        self.done_check.forget(id);
    }

    pub(super) fn answer_close(&mut self, answer: Answer, ctx: &egui::Context) {
        let Some(Closing::Asking { tab, .. }) = self.closing.take() else { return };
        match answer {
            Answer::Cancel => {}
            Answer::Pick(Choice::KeepWorktree) => {
                self.tabs.remove_id(tab);
                self.forget_tab(tab);
            }
            Answer::Pick(Choice::Remove { branch, mode }) => {
                let Some(paths) = self.paths.clone() else { return };
                let Some((shell, worktree)) =
                    self.tabs.get_mut(tab).and_then(|t| t.pane.begin_remove())
                else {
                    return;
                };
                let trust = self.trust.grant();
                worktree_jobs::remove(
                    &self.jobs, ctx, paths, tab, shell, worktree, branch, mode, trust,
                );
            }
        }
    }

    pub(super) fn apply_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Done::Opened { tab, mut result } => {
                let created = result.is_ok();
                let base_note = result.as_mut().ok().and_then(|opened| opened.base_note.take());
                let now = ctx.input(|i| i.time);
                let notes =
                    finish_open(&mut self.tabs, tab, result, now, |dir, prepared| match prepared {
                        Some(prepared) => spawn_agent(ctx, dir, prepared),
                        None => spawn_shell(ctx, dir),
                    });
                if let Some(note) = notes.first() {
                    self.show_toast(format!("Started without part of the setup: {note}"), ctx);
                }
                if let Some(note) = base_note {
                    match self.tabs.get_mut(tab).and_then(|tab| tab.pane.agent.as_mut()) {
                        Some(agent) => agent.base_note = Some(note),
                        None => self.show_toast(note, ctx),
                    }
                }
                if created
                    && self.dev.is_some_and(DevWorktree::closes_after_opening)
                    && let Some(index) = self.tabs.index_of(tab)
                {
                    self.close_tab(index, ctx);
                }
            }
            Done::Inspected { tab, worktree, inspection } => {
                if !matches!(self.closing, Some(Closing::Inspecting(id)) if id == tab) {
                    return;
                }
                self.closing = Some(Closing::Asking { tab, worktree, inspection });
                if self.dev.is_some_and(DevWorktree::answers_the_dialog)
                    && let Some(choice) = close_dialog::choices(&inspection).get(1).copied()
                {
                    self.answer_close(Answer::Pick(choice), ctx);
                }
            }
            Done::Removed { tab, result } => match result {
                Ok(outcome) => {
                    self.tabs.remove_id(tab);
                    self.forget_tab(tab);
                    if let BranchOutcome::DeleteFailed(message) = outcome {
                        eprintln!("tidewatch: worktree removed, branch kept: {message}");
                    }
                }
                Err(message) => {
                    if let Some(tab) = self.tabs.get_mut(tab) {
                        tab.pane.remove_failed(message);
                    }
                }
            },
        }
    }

    pub(super) fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        if matches!(action, Action::Move(..) | Action::MoveActive(_)) {
            self.tabs.apply(action, || unreachable!());
            return;
        }
        if self.dispatch_parking(action, ctx) {
            return;
        }
        self.settings = None;
        self.overview = false;
        match action {
            Action::New if self.welcome.is_some() => {
                self.run_repo_command(super::repos::RepoCommand::Open, ctx);
            }
            Action::New => self.new_tab(ctx),
            Action::CloseActive => self.close_tab(self.tabs.active(), ctx),
            Action::Close(index) => self.close_tab(index, ctx),
            Action::Select(_)
            | Action::Move(..)
            | Action::MoveActive(_)
            | Action::SelectNth(_)
            | Action::NextGroup
            | Action::PrevGroup
            | Action::ShowGroup(_)
            | Action::Park(_)
            | Action::ParkActive
            | Action::ToggleKeep(_) => self.tabs.apply(action, || unreachable!()),
        }
    }
}
