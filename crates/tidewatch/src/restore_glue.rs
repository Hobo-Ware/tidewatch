//! Session restore in the app (PLAN.md, D11): once the profiles are loaded, one job per repo of
//! the window reads the saved session and plans that repo's tabs; this module then carries each
//! plan out into the repo's tab group, in the saved order. A repo opened later in the same window
//! goes through the same job.
//! A shell reopens in its folder at once, an agent is prepared on a job (a fresh token for the tab,
//! the resume command, folder trust) and a workflow run goes back through the workflow host. A
//! tab that cannot come back as it was reopens as a shell with the reason printed above it, or is
//! left out, and the plan's message says which. Generic over the pty, so the whole flow runs on
//! plain data in tests.
//!
//! While the restore is under way the saver stays quiet: a save then would overwrite the saved
//! tabs with a window that has only some of them. `settled` says when every repo is done.

use std::path::{Path, PathBuf};

use tidewatch_core::agents::Level;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::{Prefs, ResumeAgents};
use tidewatch_core::prepare::Prepared;
use tidewatch_core::restore::{RealDisk, TabAction, TabPlan, WindowPlan, restore_workflow};
use tidewatch_core::resume_mode::{Resume, Why};
use tidewatch_core::session::{SavedAgentTab, SavedTab};
use tidewatch_core::worktree::{BRANCH_PREFIX, Leftover, Worktree};

use crate::catalog::Catalog;
use crate::jobs::Jobs;
use crate::leftover_glue::merge;
use crate::pane::{Body, Pane, STARTING_TITLE, finish_open};
use crate::repo::Workspace;
use crate::restore_jobs::{self, Done, ReopenSpec, Reopened, Startup, StartupSpec};
use crate::tabs::Tabs;
use crate::workflow_glue::{self, Env, Start};
use crate::worktree_jobs::without_open;

/// How a pty is made: an agent from a prepared launch, a shell with that launch typed at its
/// prompt, or a plain shell with an optional note printed above it.
pub(crate) struct Spawn<'a, T> {
    pub(crate) agent: &'a dyn Fn(&Path, &Prepared) -> T,
    pub(crate) typed: &'a dyn Fn(&Path, &Prepared) -> T,
    pub(crate) shell: &'a dyn Fn(&Path, Option<&str>) -> T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The profiles are not loaded yet, so there is nothing to plan with.
    Waiting,
    Planning,
    /// Every plan was carried out; some tabs are still being made.
    Settling,
    Settled,
}

/// A repo of the window whose saved tabs are still to be looked up.
struct Seed {
    group: usize,
    dir: PathBuf,
    other: bool,
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
    seeds: Vec<Seed>,
    planning: usize,
    skip: bool,
    app_crashed: bool,
    prefs: Prefs,
    disk: RealDisk,
}

impl Host {
    /// `skip` is the `--no-restore` flag; `disk` is where the CLIs keep their session files;
    /// `app_crashed` says the run before this one never quit cleanly.
    pub(crate) fn new(skip: bool, app_crashed: bool, disk: RealDisk) -> Self {
        Self {
            jobs: Jobs::new(),
            phase: Phase::Waiting,
            seeds: Vec::new(),
            planning: 0,
            skip,
            app_crashed,
            prefs: Prefs::default(),
            disk,
        }
    }

    /// The tabs came from a window that was running them, not from the saved session: there is
    /// nothing to bring back, and the saver may write at once.
    pub(crate) fn mark_settled(&mut self) {
        self.phase = Phase::Settled;
    }

    /// Whether launch restores the last session, as the settings say.
    pub(crate) fn restore_enabled(&self) -> bool {
        self.prefs.restore_session
    }

    pub(crate) fn resume_agents(&self) -> ResumeAgents {
        self.prefs.resume_agents
    }

    /// Takes the new mode and writes it, off the UI thread. It applies to the next restore.
    pub(crate) fn set_resume_agents(&mut self, paths: &TidewatchPaths, mode: ResumeAgents) {
        self.prefs.resume_agents = mode;
        restore_jobs::save_resume_agents(paths.clone(), mode);
    }

    /// This run was started with `--no-restore`.
    pub(crate) fn skipped_by_flag(&self) -> bool {
        self.skip
    }

    /// Takes the new setting and writes it, off the UI thread.
    pub(crate) fn set_restore_enabled(&mut self, paths: &TidewatchPaths, on: bool) {
        self.prefs.restore_session = on;
        restore_jobs::save_restore_switch(paths.clone(), on);
    }

    /// True until the saved tabs are back (or there were none to bring back).
    pub(crate) fn settled(&self) -> bool {
        self.phase == Phase::Settled
    }

    /// Adds a repo of the window, in tab group `group`, to the ones whose saved tabs are to be
    /// brought back. Nothing is looked up until `begin`. The window counts as unsettled again, so
    /// the saver leaves the repo's saved tabs alone until they are back.
    pub(crate) fn enqueue(&mut self, group: usize, workspace: &Workspace) {
        self.seeds.push(Seed {
            group,
            dir: workspace.dir().to_path_buf(),
            other: matches!(workspace, Workspace::Other { .. }),
        });
        if matches!(self.phase, Phase::Settling | Phase::Settled) {
            self.phase = Phase::Waiting;
        }
    }

    /// Starts one startup job for each repo waiting, once the profiles are known.
    pub(crate) fn begin(&mut self, ctx: &egui::Context, paths: &TidewatchPaths, catalog: &Catalog) {
        let Some(profiles) = catalog.profiles().filter(|_| !self.seeds.is_empty()) else { return };
        self.phase = Phase::Planning;
        for seed in self.seeds.drain(..) {
            self.planning += 1;
            restore_jobs::startup(
                &self.jobs,
                ctx,
                StartupSpec {
                    paths: paths.clone(),
                    group: seed.group,
                    dir: seed.dir,
                    other: seed.other,
                    skip: self.skip,
                    app_crashed: self.app_crashed,
                    profiles: profiles.clone(),
                    path_var: catalog.path.clone(),
                    disk: self.disk.clone(),
                },
            );
        }
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
                    let Startup { group, repaired, leftovers, plan, prefs } = *startup;
                    self.planning -= 1;
                    if repaired > 0 {
                        eprintln!(
                            "tidewatch: repaired {repaired} worktree(s) after the repo moved"
                        );
                    }
                    self.prefs = prefs;
                    merge(
                        report.leftovers.get_or_insert_with(Vec::new),
                        without_open(leftovers, tabs),
                    );
                    if let Some(plan) = plan {
                        let applied = self.apply(&plan, group, ctx, tabs, env, spawn, runs, now);
                        report.toasts.extend(applied.toasts);
                        report.notice = join_notices(report.notice.take(), applied.notice);
                        report.focus = report.focus.or(applied.focus);
                    }
                }
                Done::Reopened(done) => reopened(*done, tabs, spawn, now, &mut report.toasts),
            }
        }
        if self.planning == 0
            && self.seeds.is_empty()
            && matches!(self.phase, Phase::Planning | Phase::Settling)
        {
            let busy = tabs.items().iter().any(|t| making(&t.pane));
            self.phase = if busy { Phase::Settling } else { Phase::Settled };
        }
        report
    }

    /// Opens the planned tabs of a repo in their saved order, in its tab group, then selects the
    /// saved active tab, unless the user already opened tabs of their own there meanwhile. A repo
    /// that is not in front only remembers it, and a restored tab never takes the window from the
    /// one the user is on. The saved focus mode is taken from the first repo, the one in front at
    /// launch, and from no other.
    pub(crate) fn apply<T>(
        &self,
        plan: &WindowPlan,
        group: usize,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawn<T>,
        runs: &mut workflow_glue::Host,
        now: f64,
    ) -> Report {
        let untouched = tabs.in_group(group).count() == 1;
        let current = tabs.items()[tabs.active()].id;
        let mut report = Report::default();
        let ids: Vec<u64> = plan
            .tabs
            .iter()
            .map(|tab| {
                let opened = Opening { plan, group, tab, ctx, env, now };
                self.open_one(&opened, tabs, spawn, runs, &mut report.toasts)
            })
            .collect();
        let selected = if untouched { ids.get(plan.active).copied() } else { None };
        tabs.select_id(selected.unwrap_or(current));
        if untouched && group == 0 {
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
        let Opening { plan, group, tab, ctx, env, now } = *opening;
        match &tab.action {
            _ if matches!(tab.saved, SavedTab::Main) => tabs.main_id(group).unwrap_or_default(),
            TabAction::Shell => push_shell(tabs, spawn, (plan, group), tab, None),
            TabAction::ShellInstead(_) => {
                push_shell(tabs, spawn, (plan, group), tab, tab.note().as_deref())
            }
            TabAction::ResumeAgent { agent, profile, session_id, how } => {
                let Some(agent) = env.catalog.agent(agent).cloned() else {
                    let note = format!("The agent was not resumed: `{agent}` is not configured.");
                    return push_shell(tabs, spawn, (plan, group), tab, Some(&note));
                };
                let SavedTab::Agent(saved) = &tab.saved else {
                    return push_shell(tabs, spawn, (plan, group), tab, None);
                };
                let title = tab.worktree.as_ref().map_or(STARTING_TITLE, |w| w.name()).to_owned();
                let pane = Pane::creating_agent(profile.name.clone(), agent.id.clone(), now);
                let id = tabs.push_in(group, title, pane);
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
                        typed: match how {
                            Resume::Run => None,
                            Resume::TypeOnly(why) => Some(*why),
                        },
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
                        group,
                        env,
                        &plan.dir,
                        Some(worktree),
                        Start::Resume(Box::new(runner)),
                        now,
                    );
                }
                let note = "The workflow was not resumed: its saved state is unreadable.";
                toasts.push(note.to_owned());
                push_shell(tabs, spawn, (plan, group), tab, Some(note))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Opening<'a> {
    plan: &'a WindowPlan,
    group: usize,
    tab: &'a TabPlan,
    ctx: &'a egui::Context,
    env: &'a Env<'a>,
    now: f64,
}

fn join_notices(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}\n{second}")),
        (first, second) => first.or(second),
    }
}

fn making<T>(pane: &Pane<T>) -> bool {
    matches!(pane.body, Body::Creating)
}

fn push_shell<T>(
    tabs: &mut Tabs<Pane<T>>,
    spawn: &Spawn<T>,
    (plan, group): (&WindowPlan, usize),
    tab: &TabPlan,
    note: Option<&str>,
) -> u64 {
    let shell = (spawn.shell)(&tab.dir, note);
    let title = tab.saved.label(&plan.dir);
    let pane = Pane { body: Body::Shell(shell), worktree: tab.worktree.clone(), agent: None };
    tabs.push_in(group, title, pane)
}

/// A reopened agent starts in its tab, or the tab becomes the plain shell the job asked for.
fn reopened<T>(
    done: Reopened,
    tabs: &mut Tabs<Pane<T>>,
    spawn: &Spawn<T>,
    now: f64,
    toasts: &mut Vec<String>,
) {
    let Reopened { tab, result, note, typed } = done;
    let notes = finish_open(tabs, tab, result, now, |dir, prepared| match (prepared, typed) {
        (Some(prepared), Some(_)) => (spawn.typed)(dir, prepared),
        (Some(prepared), None) => (spawn.agent)(dir, prepared),
        (None, _) => (spawn.shell)(dir, note.as_deref()),
    });
    if let Some(agent) = tabs.get_mut(tab).and_then(|tab| tab.pane.agent.as_mut()) {
        agent.resume_note = typed.map(Why::note);
    }
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
        crashed: false,
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
