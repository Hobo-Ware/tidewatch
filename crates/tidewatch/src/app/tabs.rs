//! Opening, closing and answering tabs: the new-tab dialog, worktree jobs and the close flow.

use tidewatch_core::worktree::BranchOutcome;

use super::{Closing, Tidewatch};
use crate::close_dialog::{self, Answer, Choice};
use crate::dev_worktree::DevWorktree;
use crate::new_tab::{Group, NewTab, Request};
use crate::new_tab_dialog::Outcome;
use crate::pane::{
    CREATING_TITLE, ClosePolicy, Pane, STARTING_TITLE, agent_tabs_in_folder, finish_open,
};
use crate::pty::{spawn_agent, spawn_shell};
use crate::repo::Workspace;
use crate::tabs::Action;
use crate::workflow_glue::Start;
use crate::worktree_jobs::{self, Done, OpenSpec, Target};

impl Tidewatch {
    /// Opens the new-tab dialog. Without a home directory there are no profiles to pick from, so
    /// the tab opens as a plain shell.
    fn new_tab(&mut self, ctx: &egui::Context) {
        self.new_tab_on(ctx, None);
    }

    pub(super) fn new_tab_on(&mut self, ctx: &egui::Context, preselect: Option<&str>) {
        if self.new_tab.is_some() {
            return;
        }
        if self.welcome.is_some() {
            self.run_repo_command(super::repos::RepoCommand::Open, ctx);
            return;
        }
        if self.paths.is_none() {
            self.open_tab(ctx, Request::Shell);
            return;
        }
        if !self.catalog.is_loaded() {
            self.show_toast("Profiles are still loading", ctx);
            return;
        }
        let state = self.dialog_state(preselect);
        self.bases.refresh(ctx, state.repo());
        self.new_tab = Some(state);
    }

    pub(super) fn dialog_state(&self, preselect: Option<&str>) -> NewTab {
        let (group, label, dir) = match self.workspace() {
            Workspace::Repo(repo) => (Group::Repo, repo.label.clone(), repo.root.clone()),
            Workspace::Other { dir, label } => {
                (Group::Other, format!("Other: {label}"), dir.clone())
            }
        };
        let base = self.bases.get(&dir).map(str::to_owned);
        let running = agent_tabs_in_folder(&self.tabs);
        let mut state = NewTab::open(group, label, dir, running, self.last_profile.as_deref());
        state.set_base(base);
        if let Some(preselect) = preselect {
            state.jump(preselect, &self.catalog);
        }
        state
    }

    /// In a git repo a new tab is a fresh worktree; in the Other group it is a plain shell, or an
    /// agent in the folder itself. An agent tab is prepared on a job thread first.
    pub(super) fn open_tab(&mut self, ctx: &egui::Context, request: Request) {
        let agent = match request {
            Request::Shell => None,
            Request::Agent(agent) => Some(agent),
            Request::Workflow(request) => {
                self.open_workflow(ctx, Start::New(request), None);
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
        match (&self.workspaces[group], &self.paths) {
            (Workspace::Repo(repo), Some(paths)) => {
                let tab = self.tabs.push(CREATING_TITLE.to_owned(), pane);
                let target = Target::Worktree { repo: repo.root.clone() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec {
                    paths: paths.clone(),
                    target,
                    agent,
                    path_var,
                    tab,
                    listener,
                    trust,
                };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            (workspace, Some(paths)) if agent.is_some() => {
                let tab = self.tabs.push(STARTING_TITLE.to_owned(), pane);
                let target = Target::Folder { dir: workspace.dir().to_path_buf() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec {
                    paths: paths.clone(),
                    target,
                    agent,
                    path_var,
                    tab,
                    listener,
                    trust,
                };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            _ => {
                let dir = self.workspace().dir().to_path_buf();
                self.tabs.apply(Action::New, || Pane::shell(spawn_shell(ctx, &dir)));
            }
        }
    }

    pub(super) fn answer_new_tab(&mut self, outcome: Outcome, ctx: &egui::Context) {
        self.new_tab = None;
        let Outcome::Start(request) = outcome else { return };
        if let Request::Agent(agent) = &request {
            self.last_profile = Some(agent.profile_id.clone());
        }
        self.open_tab(ctx, request);
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
        self.flow.forget(id);
        self.runs.forget(id);
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
            Done::Opened { tab, result } => {
                let created = result.is_ok();
                let now = ctx.input(|i| i.time);
                let notes =
                    finish_open(&mut self.tabs, tab, result, now, |dir, prepared| match prepared {
                        Some(prepared) => spawn_agent(ctx, dir, prepared),
                        None => spawn_shell(ctx, dir),
                    });
                if let Some(note) = notes.first() {
                    self.show_toast(format!("Started without part of the setup: {note}"), ctx);
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
            | Action::ShowGroup(_) => self.tabs.apply(action, || unreachable!()),
        }
    }
}
