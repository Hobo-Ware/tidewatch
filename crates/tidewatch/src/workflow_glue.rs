//! The seams between workflow tabs and the rest of the app: opening a tab for a workflow (or for a
//! saved run), handing the listener's messages to the tab's run, carrying out what the run asks for
//! (launch the next stage's agent, run the tests), applying what the jobs finished and keeping each
//! tab's status in step with the run. Generic over the pty so the whole flow runs on plain data in
//! tests; `app.rs` only calls these.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use tidewatch_core::events::TabMessage;
use tidewatch_core::listener::Listener;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::profiles::Profile;
use tidewatch_core::runner::{RunContext, RunEvent, Runner};
use tidewatch_core::tools::Tools;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::workflows::Workflow;
use tidewatch_core::worktree::Worktree;

use crate::agent_run::Change;
use crate::catalog::Catalog;
use crate::jobs::Jobs;
use crate::pane::{Body, CREATING_TITLE, Pane, relaunch_stage};
use crate::reader::Reader;
use crate::tabs::Tabs;
use crate::workflow_jobs::{self, Done, OpenSpec, StageSpec};
use crate::workflow_run::{Exec, TabRun};
use tidewatch_core::status::AgentStatus;

/// What the glue needs from the app each time it is called.
pub(crate) struct Env<'a> {
    pub(crate) paths: &'a TidewatchPaths,
    pub(crate) catalog: &'a Catalog,
    pub(crate) listener: Option<Arc<Listener>>,
    pub(crate) tools: Tools,
    /// Set after the user agreed to folder trust (D14).
    pub(crate) trust: Option<Grant>,
}

/// How a pty is made: an agent from a prepared launch, or a plain shell.
pub(crate) struct Spawners<'a, T> {
    pub(crate) agent: &'a dyn Fn(&Path, &Prepared) -> T,
    pub(crate) shell: &'a dyn Fn(&Path) -> T,
}

/// What the new-tab dialog hands over for a workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowRequest {
    pub(crate) workflow: Workflow,
    pub(crate) profiles: BTreeMap<String, Profile>,
    pub(crate) task: String,
    /// The agent of the first stage, for the tab's logo while the worktree is made.
    pub(crate) first_agent: String,
}

pub(crate) enum Start {
    New(Box<WorkflowRequest>),
    /// A saved run (`stages.json`) in a worktree that already exists.
    Resume(Box<Runner>),
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    pending: HashMap<u64, Start>,
}

fn tab_run<T>(tabs: &mut Tabs<Pane<T>>, id: u64) -> Option<&mut TabRun> {
    tabs.get_mut(id)?.pane.agent.as_mut()?.workflow.as_deref_mut()
}

/// Hands a listener message to the run of the tab it belongs to. `None` when the message must be
/// dropped altogether: it comes from the agent of a stage that was replaced since. Otherwise what
/// the run asks for (usually nothing).
pub(crate) fn feed<T>(tabs: &mut Tabs<Pane<T>>, message: &TabMessage) -> Option<Vec<Exec>> {
    let Some(run) = tab_run(tabs, message.tab_id) else { return Some(Vec::new()) };
    run.accepts(message.generation).then(|| run.message(message))
}

/// A process that failed (a status change to `Failed`) fails the stage of a workflow tab.
pub(crate) fn failures<T>(
    tabs: &mut Tabs<Pane<T>>,
    changes: &[(u64, Change)],
) -> Vec<(u64, Vec<Exec>)> {
    changes
        .iter()
        .filter_map(|(tab, change)| {
            let AgentStatus::Failed(reason) = &change.to else { return None };
            let execs = tab_run(tabs, *tab)?.agent_failed(reason.clone());
            (!execs.is_empty()).then_some((*tab, execs))
        })
        .collect()
}

impl Host {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), pending: HashMap::new() }
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.pending.remove(&tab);
    }

    /// Adds the tab for a workflow and starts making its worktree (or looking at the existing one).
    /// The first stage starts when the job is back.
    pub(crate) fn open<T>(
        &mut self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        repo: &Path,
        existing: Option<Worktree>,
        start: Start,
        now: f64,
    ) -> u64 {
        let (name, agent) = match &start {
            Start::New(request) => (request.workflow.name.clone(), request.first_agent.clone()),
            Start::Resume(runner) => (
                runner.workflow_name().to_owned(),
                runner.profile(runner.position()).map(|p| p.agent.clone()).unwrap_or_default(),
            ),
        };
        let tab = tabs.push(CREATING_TITLE.to_owned(), Pane::creating_agent(name, agent, now));
        self.pending.insert(tab, start);
        let spec = OpenSpec {
            paths: env.paths.clone(),
            repo: repo.to_path_buf(),
            tab,
            tools: env.tools.clone(),
            existing,
        };
        workflow_jobs::open(&self.jobs, ctx, spec);
        tab
    }

    /// One pass per frame: carries out what the run asked for (`asked`), applies the jobs that
    /// finished (which may ask for more) and sets every workflow tab's status from its run. Returns
    /// the toasts to show.
    pub(crate) fn tick<T>(
        &mut self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawners<T>,
        asked: Vec<(u64, Vec<Exec>)>,
        now: f64,
    ) -> Vec<String> {
        let mut toasts = Vec::new();
        for (tab, execs) in asked {
            self.carry_out(ctx, tabs, env, tab, execs);
        }
        for done in self.jobs.drain() {
            self.apply(ctx, tabs, env, spawn, done, now, &mut toasts);
        }
        hold_statuses(tabs);
        toasts
    }

    fn apply<T>(
        &mut self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawners<T>,
        done: Done,
        now: f64,
        toasts: &mut Vec<String>,
    ) {
        match done {
            Done::Opened { tab, result } => self.opened(ctx, tabs, env, spawn, tab, result, now),
            Done::StageReady { tab, generation, divider, result } => {
                let Some(run) = tab_run(tabs, tab) else { return };
                if run.generation() != generation {
                    return;
                }
                let execs = match result {
                    Ok(started) => {
                        let dir = tabs
                            .get_mut(tab)
                            .and_then(|t| t.pane.worktree.as_ref().map(|w| w.path.clone()));
                        let Some(dir) = dir else { return };
                        let notes = relaunch_stage(
                            tabs,
                            tab,
                            *started,
                            &dir,
                            divider,
                            now,
                            |dir, prepared| (spawn.agent)(dir, prepared),
                        );
                        if let Some(note) = notes.first() {
                            toasts.push(format!("Started without part of the setup: {note}"));
                        }
                        Vec::new()
                    }
                    Err(reason) => {
                        tab_run(tabs, tab).map(|run| run.launch_failed(reason)).unwrap_or_default()
                    }
                };
                self.carry_out(ctx, tabs, env, tab, execs);
            }
            Done::Tested { tab, run: tested } => {
                let Some(run) = tab_run(tabs, tab) else { return };
                let execs = run.event(RunEvent::TestResult {
                    exit_code: tested.exit_code,
                    output: tested.output,
                });
                self.carry_out(ctx, tabs, env, tab, execs);
            }
        }
    }

    fn opened<T>(
        &mut self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        spawn: &Spawners<T>,
        tab: u64,
        result: Result<workflow_jobs::Opening, String>,
        now: f64,
    ) {
        let Some(start) = self.pending.remove(&tab) else { return };
        let Some(found) = tabs.get_mut(tab) else { return };
        let opening = match result {
            Ok(opening) => opening,
            Err(message) => {
                fail_tab(&mut found.pane, message);
                return;
            }
        };
        let worktree = opening.worktree;
        let (runner, resuming) = match start {
            Start::Resume(runner) => (*runner, true),
            Start::New(request) => {
                let context = RunContext {
                    task: request.task.clone(),
                    base: opening.base,
                    branch: worktree.branch.clone(),
                    test_command: opening.test_command,
                };
                match Runner::new(&request.workflow, &request.profiles, context) {
                    Ok(runner) => (runner, false),
                    Err(problems) => {
                        fail_tab(&mut found.pane, problems.join("; "));
                        worktree.name().clone_into(&mut found.title);
                        found.pane.worktree = Some(worktree);
                        return;
                    }
                }
            }
        };
        let stages_file = env.paths.stages_file(&worktree.repo, worktree.name());
        let mut run = TabRun::new(runner, Some(stages_file), opening.test_timeout);
        let execs = if resuming { run.resume() } else { run.start() };
        worktree.name().clone_into(&mut found.title);
        let Some(agent) = found.pane.agent.as_mut() else { return };
        agent.reader = Reader::for_agent(&agent.run.agent_id, None, worktree.path.clone());
        agent.workflow = Some(Box::new(run));
        if !execs.iter().any(|exec| matches!(exec, Exec::Launch(_))) {
            let notes_file = env.paths.notes_file(&worktree.repo, worktree.name());
            agent.run.started(Some(notes_file), now);
            found.pane.body = Body::Shell((spawn.shell)(&worktree.path));
        }
        found.pane.worktree = Some(worktree);
        self.carry_out(ctx, tabs, env, tab, execs);
    }

    /// Starts the jobs the run asked for. A launch that cannot even be set up (an agent that is not
    /// in the table) goes back to the run as a failed agent.
    fn carry_out<T>(
        &self,
        ctx: &egui::Context,
        tabs: &mut Tabs<Pane<T>>,
        env: &Env,
        tab: u64,
        execs: Vec<Exec>,
    ) {
        let mut queue = std::collections::VecDeque::from(execs);
        while let Some(exec) = queue.pop_front() {
            let Some(found) = tabs.get_mut(tab) else { return };
            let Some(worktree) = found.pane.worktree.clone() else { return };
            match exec {
                Exec::Launch(launch) => {
                    let Some(agent) = env.catalog.agent(&launch.profile.agent).cloned() else {
                        let reason = format!("unknown agent \"{}\"", launch.profile.agent);
                        if let Some(run) = tab_run(tabs, tab) {
                            queue.extend(run.launch_failed(reason));
                        }
                        continue;
                    };
                    workflow_jobs::launch_stage(
                        &self.jobs,
                        ctx,
                        StageSpec {
                            paths: env.paths.clone(),
                            tab,
                            launch,
                            agent,
                            worktree,
                            path_var: env.catalog.path.clone(),
                            listener: env.listener.clone(),
                            trust: env.trust.clone(),
                        },
                    );
                }
                Exec::RunTests { command, timeout } => {
                    workflow_jobs::run_tests(&self.jobs, ctx, tab, worktree.path, command, timeout);
                }
            }
        }
    }
}

fn fail_tab<T>(pane: &mut Pane<T>, message: String) {
    if let Some(agent) = &mut pane.agent {
        agent.run.fail(message.clone());
    }
    pane.body = Body::Failed(message);
}

/// Sets each workflow tab's status: held by the run while the runner has the next move, the agent's
/// own while a stage works.
fn hold_statuses<T>(tabs: &mut Tabs<Pane<T>>) {
    for tab in tabs.items_mut() {
        let Some(agent) = tab.pane.agent.as_mut() else { continue };
        let Some(run) = agent.workflow.as_ref() else { continue };
        match run.hold() {
            Some(hold) => agent.run.hold(Some(hold.status), hold.detail),
            None => agent.run.hold(None, None),
        }
    }
}

#[cfg(test)]
mod tests;
