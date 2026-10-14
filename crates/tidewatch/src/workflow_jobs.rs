//! The blocking work behind a workflow tab, run on `Jobs` threads: making the worktree and finding
//! the base branch and the test command, getting a stage's agent ready (a fresh token for the tab,
//! the files its CLI reads, the shell line), and running the test command with its timeout. Each
//! function starts one job and returns at once; its `Done` arrives through `Jobs::drain`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tidewatch_core::agents::Agent;
use tidewatch_core::delivery::detect_base;
use tidewatch_core::listener::{Listener, TabRegistration};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prepare::{PrepareInput, banner_line, prepare};
use tidewatch_core::session_id;
use tidewatch_core::test_command::{self, TestRun, run_in_timeout};
use tidewatch_core::tools::Tools;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, Worktree};

use crate::jobs::Jobs;
use crate::pane::Started;
use crate::workflow_run::StageLaunch;

const FALLBACK_BASE: &str = "main";

/// What a workflow tab needs to know before its first stage starts.
pub(crate) struct Opening {
    pub(crate) worktree: Worktree,
    pub(crate) base: String,
    pub(crate) test_command: Option<String>,
    pub(crate) test_timeout: Duration,
}

pub(crate) enum Done {
    Opened { tab: u64, result: Result<Opening, String> },
    StageReady { tab: u64, generation: u64, divider: String, result: Result<Box<Started>, String> },
    Tested { tab: u64, run: TestRun },
}

pub(crate) struct OpenSpec {
    pub(crate) paths: TidewatchPaths,
    pub(crate) repo: PathBuf,
    pub(crate) tab: u64,
    pub(crate) tools: Tools,
    /// A worktree that already exists (a run that is resumed). `None` makes a new one.
    pub(crate) existing: Option<Worktree>,
}

/// Makes the worktree when there is none, then looks up the base branch, the repo's test command and
/// how long it may run.
pub(crate) fn open(jobs: &Jobs<Done>, ctx: &egui::Context, spec: OpenSpec) {
    jobs.spawn(ctx, move || Done::Opened { tab: spec.tab, result: run_open(&spec) });
}

fn run_open(spec: &OpenSpec) -> Result<Opening, String> {
    let worktree = match &spec.existing {
        Some(worktree) => worktree.clone(),
        None => worktree::add(&spec.paths, &spec.repo).map_err(|e| e.to_string())?,
    };
    Ok(Opening {
        base: detect_base(&spec.tools, &worktree.path).unwrap_or_else(|| FALLBACK_BASE.to_owned()),
        test_command: test_command::test_command(&spec.repo),
        test_timeout: test_command::timeout(&spec.repo),
        worktree,
    })
}

pub(crate) struct StageSpec {
    pub(crate) paths: TidewatchPaths,
    pub(crate) tab: u64,
    pub(crate) launch: StageLaunch,
    pub(crate) agent: Agent,
    pub(crate) worktree: Worktree,
    pub(crate) path_var: OsString,
    /// Where the agent reports; `None` when the listener is down.
    pub(crate) listener: Option<Arc<Listener>>,
    /// Set after the user agreed to folder trust (D14).
    pub(crate) trust: Option<Grant>,
}

/// Registers the tab with the launch's generation (which revokes the previous stage's token) and
/// gets the stage's agent ready in the tab's worktree.
pub(crate) fn launch_stage(jobs: &Jobs<Done>, ctx: &egui::Context, spec: StageSpec) {
    jobs.spawn(ctx, move || Done::StageReady {
        tab: spec.tab,
        generation: spec.launch.generation,
        divider: spec.launch.divider.clone(),
        result: run_stage(&spec).map(Box::new),
    });
}

fn run_stage(spec: &StageSpec) -> Result<Started, String> {
    let launch = &spec.launch;
    let tab_key = spec.worktree.name().to_owned();
    let repo = &spec.worktree.repo;
    let hooks = spec.listener.as_ref().map(|listener| {
        listener.register_tab(TabRegistration {
            tab_id: spec.tab,
            agent: spec.agent.id.clone(),
            repo: repo.clone(),
            tab: tab_key.clone(),
            generation: launch.generation,
        })
    });
    let session_id = spec.agent.session_id_flag.is_some().then(session_id::new);
    let mut prepared = prepare(&PrepareInput {
        paths: &spec.paths,
        agent: &spec.agent,
        profile: &launch.profile,
        repo,
        tab: &tab_key,
        dir: &spec.worktree.path,
        worktree_tab: true,
        task: &launch.prompt,
        session_id: session_id.clone(),
        path_var: Some(&spec.path_var),
        listener: hooks,
        trust: spec.trust.as_ref(),
    })
    .map_err(|e| e.to_string())?;
    prepared.setup = format!("{}\n{}", banner_line(&launch.divider), prepared.setup);
    Ok(Started {
        profile_name: launch.profile.name.clone(),
        profile_id: launch.profile.name.clone(),
        level: launch.profile.level,
        agent_id: spec.agent.id.clone(),
        notes_file: Some(spec.paths.notes_file(repo, &tab_key)),
        session_id,
        awaiting_prompt: false,
        prepared,
    })
}

/// Runs the repo's test command in the worktree and stops it after `timeout`.
pub(crate) fn run_tests(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    dir: PathBuf,
    command: String,
    timeout: Duration,
) {
    jobs.spawn(ctx, move || Done::Tested { tab, run: run_in_timeout(&dir, &command, timeout) });
}

#[cfg(test)]
mod tests;
