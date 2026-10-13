//! Session restore in the app (PLAN.md, D11): once the profiles are loaded, one job reads the
//! saved session and plans every tab; this module then carries the plan out, in the saved order.
//! A shell reopens in its folder at once, an agent is prepared on a job (a fresh token for the tab,
//! the resume command, folder trust) and a workflow run goes back through the workflow host. A
//! tab that cannot come back as it was reopens as a shell with the reason printed above it, or is
//! left out, and the plan's message says which. Generic over the pty, so the whole flow runs on
//! plain data in tests.
//!
//! While the restore is under way the saver stays quiet: a save then would overwrite the saved
//! tabs with a window that has only some of them. `settled` says when it is done.

use std::path::Path;

use tidewatch_core::agents::Level;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::restore::{RealDisk, TabAction, TabPlan, WindowPlan, restore_workflow};
use tidewatch_core::session::{SavedAgentTab, SavedTab};
use tidewatch_core::worktree::{BRANCH_PREFIX, Leftover, Worktree};

use crate::catalog::Catalog;
use crate::jobs::Jobs;
use crate::pane::{Body, Pane, STARTING_TITLE, finish_open};
use crate::repo::Workspace;
use crate::restore_jobs::{self, Done, ReopenSpec, Reopened, Startup, StartupSpec};
use crate::tabs::{Action, Tabs};
use crate::workflow_glue::{self, Env, Start};
use crate::worktree_jobs::without_open;

/// How a pty is made: an agent from a prepared launch, or a plain shell with an optional note
/// printed above it.
pub(crate) struct Spawn<'a, T> {
    pub(crate) agent: &'a dyn Fn(&Path, &Prepared) -> T,
    pub(crate) shell: &'a dyn Fn(&Path, Option<&str>) -> T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The profiles are not loaded yet, so there is nothing to plan with.
    Waiting,
    Planning,
    /// The plan was carried out; some tabs are still being made.
    Settling,
    Settled,
}

/// What a frame's restore work asks the app to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) toasts: Vec<String>,
    /// The plan's message about tabs that were left out or came back as shells; stays until dismissed.
    pub(crate) notice: Option<String>,
    /// The worktrees nothing owns, once known.
    pub(crate) leftovers: Option<Vec<Leftover>>,
    /// Focus mode as saved, when the window was still untouched.
    pub(crate) focus: Option<bool>,
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    phase: Phase,
    skip: bool,
    prefs: Prefs,
    disk: RealDisk,
}

impl Host {
    /// `skip` is the `--no-restore` flag; `disk` is where the CLIs keep their session files.
    pub(crate) fn new(skip: bool, disk: RealDisk) -> Self {
        Self { jobs: Jobs::new(), phase: Phase::Waiting, skip, prefs: Prefs::default(), disk }
    }

    /// True until the saved tabs are back (or there were none to bring back).
    pub(crate) fn settled(&self) -> bool {
        self.phase == Phase::Settled
    }

    /// Starts the one startup job, the first time the profiles are known. Later calls do nothing.
    pub(crate) fn begin(
        &mut self,
        ctx: &egui::Context,
        paths: &TidewatchPaths,
        workspace: &Workspace,
        catalog: &Catalog,
    ) {
        let (Phase::Waiting, Some(profiles)) = (self.phase, catalog.profiles()) else { return };
        self.phase = Phase::Planning;
        restore_jobs::startup(
            &self.jobs,
            ctx,
            StartupSpec {
                paths: paths.clone(),
                dir: workspace.dir().to_path_buf(),
                other: matches!(workspace, Workspace::Other { .. }),
                skip: self.skip,
                profiles: profiles.clone(),
                path_var: catalog.path.clone(),
                disk: self.disk.clone(),
            },
        );
    }

    /// One pass per frame: applies what the jobs finished.
    pub(crate) fn tick<T>(
        &mut self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawn<T>,
        runs: &mut workflow_glue::Host,
        now: f64,
    ) -> Report {
        let mut report = Report::default();
        for done in self.jobs.drain() {
            match done {
                Done::Planned(startup) => {
                    let Startup { repaired, leftovers, plan, prefs } = *startup;
                    if repaired > 0 {
                        eprintln!(
                            "tidewatch: repaired {repaired} worktree(s) after the repo moved"
                        );
                    }
                    self.prefs = prefs;
                    report.leftovers = Some(without_open(leftovers, tabs));
                    if let Some(plan) = plan {
                        let applied = self.apply(&plan, ctx, tabs, env, spawn, runs, now);
                        report.toasts.extend(applied.toasts);
                        report.notice = applied.notice;
                        report.focus = applied.focus;
                    }
                    self.phase = Phase::Settling;
                }
                Done::Reopened(done) => reopened(*done, tabs, spawn, now, &mut report.toasts),
            }
        }
        if self.phase == Phase::Settling && !tabs.items().iter().any(|t| making(&t.pane)) {
            self.phase = Phase::Settled;
        }
        report
    }

    /// Opens the planned tabs in their saved order, then selects the saved active tab and takes the
    /// saved focus mode, unless the user already opened tabs of their own meanwhile.
    pub(crate) fn apply<T>(
        &self,
        plan: &WindowPlan,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawn<T>,
        runs: &mut workflow_glue::Host,
        now: f64,
    ) -> Report {
        let untouched = tabs.items().len() == 1;
        let current = tabs.items()[tabs.active()].id;
        let mut report = Report::default();
        let ids: Vec<u64> = plan
            .tabs
            .iter()
            .map(|tab| {
                let opened = Opening { plan, tab, ctx, env, now };
                self.open_one(&opened, tabs, spawn, runs, &mut report.toasts)
            })
            .collect();
        let selected = if untouched { ids.get(plan.active).copied() } else { Some(current) };
        if let Some(index) = selected.and_then(|id| tabs.index_of(id)) {
            tabs.apply(Action::Select(index), || unreachable!());
        }
        if untouched {
            report.focus = Some(plan.focus_mode);
        }
        report.notice = plan.toast();
        report
    }

    fn open_one<T>(
        &self,
        opening: &Opening,
        tabs: &mut Tabs<Pane<T>>,
        spawn: &Spawn<T>,
        runs: &mut workflow_glue::Host,
        toasts: &mut Vec<String>,
    ) -> u64 {
        let Opening { plan, tab, ctx, env, now } = *opening;
        match &tab.action {
            _ if matches!(tab.saved, SavedTab::Main) => tabs.items()[0].id,
            TabAction::Shell => push_shell(tabs, spawn, plan, tab, None),
            TabAction::ShellInstead(_) => push_shell(tabs, spawn, plan, tab, tab.note().as_deref()),
            TabAction::ResumeAgent { agent, profile, session_id } => {
                let Some(agent) = env.catalog.agent(agent).cloned() else {
                    let note = format!("The agent was not resumed: `{agent}` is not configured.");
                    return push_shell(tabs, spawn, plan, tab, Some(&note));
                };
                let SavedTab::Agent(saved) = &tab.saved else {
                    return push_shell(tabs, spawn, plan, tab, None);
                };
                let title = tab.worktree.as_ref().map_or(STARTING_TITLE, |w| w.name()).to_owned();
                let pane = Pane::creating_agent(profile.name.clone(), agent.id.clone(), now);
                let id = tabs.push(title, pane);
                restore_jobs::reopen(
                    &self.jobs,
                    ctx,
                    ReopenSpec {
                        paths: env.paths.clone(),
                        tab: id,
                        repo: plan.dir.clone(),
                        worktree: tab.worktree.clone(),
                        dir: tab.dir.clone(),
                        agent,
                        profile_id: saved.profile.clone(),
                        profile: (**profile).clone(),
                        session_id: session_id.clone(),
                        path_var: env.catalog.path.clone(),
                        listener: env.listener.clone(),
                        trust: env.trust.clone(),
                    },
                );
                id
            }
            TabAction::ResumeWorkflow { stages } => {
                if let (Some((runner, _)), Some(worktree)) =
                    (restore_workflow(stages), tab.worktree.clone())
                {
                    return runs.open(
                        ctx,
                        tabs,
                        env,
                        &plan.dir,
                        Some(worktree),
                        Start::Resume(Box::new(runner)),
                        now,
                    );
                }
                let note = "The workflow was not resumed: its saved state is unreadable.";
                toasts.push(note.to_owned());
                push_shell(tabs, spawn, plan, tab, Some(note))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Opening<'a> {
    plan: &'a WindowPlan,
    tab: &'a TabPlan,
    ctx: &'a egui::Context,
    env: &'a Env<'a>,
    now: f64,
}

fn making<T>(pane: &Pane<T>) -> bool {
    matches!(pane.body, Body::Creating)
}

fn push_shell<T>(
    tabs: &mut Tabs<Pane<T>>,
    spawn: &Spawn<T>,
    plan: &WindowPlan,
    tab: &TabPlan,
    note: Option<&str>,
) -> u64 {
    let shell = (spawn.shell)(&tab.dir, note);
    let title = tab.saved.label(&plan.dir);
    tabs.push(title, Pane { body: Body::Shell(shell), worktree: tab.worktree.clone(), agent: None })
}

/// A reopened agent starts in its tab, or the tab becomes the plain shell the job asked for.
fn reopened<T>(
    done: Reopened,
    tabs: &mut Tabs<Pane<T>>,
    spawn: &Spawn<T>,
    now: f64,
    toasts: &mut Vec<String>,
) {
    let Reopened { tab, result, note } = done;
    let notes = finish_open(tabs, tab, result, now, |dir, prepared| match prepared {
        Some(prepared) => (spawn.agent)(dir, prepared),
        None => (spawn.shell)(dir, note.as_deref()),
    });
    if let Some(note) = notes.first() {
        toasts.push(format!("Started without part of the setup: {note}"));
    }
}

/// The plan of one saved workflow run, for the `TIDEWATCH_DEV_RESUME` switch: the same path a
/// restart takes, for a single tab.
pub(crate) fn dev_resume_plan(paths: &TidewatchPaths, repo: &Path, name: &str) -> WindowPlan {
    let worktree = Worktree {
        repo: repo.to_path_buf(),
        path: paths.worktree_dir(repo, name),
        branch: format!("{BRANCH_PREFIX}{name}"),
    };
    let stages = paths.stages_file(repo, name);
    let saved = SavedTab::Agent(SavedAgentTab {
        dir: worktree.path.clone(),
        branch: Some(worktree.branch.clone()),
        agent: String::new(),
        profile: String::new(),
        level: Level::Plan,
        session_id: None,
        stages: Some(stages.clone()),
    });
    WindowPlan {
        dir: repo.to_path_buf(),
        other: false,
        focus_mode: false,
        active: 0,
        tabs: vec![TabPlan {
            saved,
            dir: worktree.path.clone(),
            worktree: Some(worktree),
            action: TabAction::ResumeWorkflow { stages },
        }],
        skipped: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
