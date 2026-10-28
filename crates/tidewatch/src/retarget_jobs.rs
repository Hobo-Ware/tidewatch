//! The blocking work of changing what a tab runs, on `Jobs` threads: the look inside the shell
//! tabs for an agent session, and one change (scan again, look at the folder the session runs in,
//! plan, end the agent with `end_session_in` when it runs, get the new agent ready there). The
//! window swaps the tab's pane and moves it to the repo's group when it lands.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tidewatch_core::elsewhere::Cli;
use tidewatch_core::elsewhere::{Scope, Session};
use tidewatch_core::elsewhere_scan::{
    EndRequest, Ended, RealSystem, Refusal as EndRefusal, ScanEnv, System, end_session_in,
    scan_in_shells,
};
use tidewatch_core::listener::Listener;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree;

use crate::jobs::Jobs;
use crate::repo::Repo;
use crate::restore_jobs::{ReopenSpec, Reopened, run_reopen};
use crate::retarget::{
    AgentPlan, Failure, Folder, Landing, Plan, Source, Start, Subject, Target, is_inside, plan,
};
use crate::shell_agents::{Detected, Probe};

pub(crate) enum Done {
    Detected(Vec<(u64, Detected)>),
    DetectFailed,
    Retargeted(Box<Retargeted>),
    /// Not yet: the change goes ahead when the turn ends or the tab settles.
    Held {
        tab: u64,
    },
    Failed {
        tab: u64,
        failure: Failure,
    },
}

pub(crate) enum Retargeted {
    Agent { opened: Box<Reopened>, landing: Landing, note: Option<String> },
    Shell { tab: u64, remembered: Option<(Cli, String)> },
}

pub(crate) fn detect(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    profiles: Profiles,
    probes: Vec<Probe>,
) {
    jobs.spawn(ctx, move || {
        run_detect(&RealSystem::new(), &ScanEnv::from_env(), &profiles, &probes)
    });
}

fn run_detect(system: &dyn System, env: &ScanEnv, profiles: &Profiles, probes: &[Probe]) -> Done {
    let shells: Vec<u32> = probes.iter().map(|probe| probe.shell_pid).collect();
    let Some(found) = scan_in_shells(system, env, &profiles.agent_table(), &shells) else {
        return Done::DetectFailed;
    };
    let tabs = found
        .iter()
        .filter_map(|(shell, session)| {
            let probe = probes.iter().find(|probe| probe.shell_pid == *shell)?;
            Some((probe.tab, Detected::of(session)?))
        })
        .collect();
    Done::Detected(tabs)
}

pub(crate) struct Request {
    pub(crate) subject: Subject,
    pub(crate) target: Target,
    /// A session this tab ended earlier, to resume when nothing runs in it.
    pub(crate) remembered: Option<(Cli, String)>,
    /// End the agent even while it works on a turn.
    pub(crate) interrupt: bool,
    pub(crate) profiles: Profiles,
    pub(crate) installed: Vec<String>,
    pub(crate) paths: TidewatchPaths,
    pub(crate) path_var: OsString,
    pub(crate) listener: Option<Arc<Listener>>,
    pub(crate) trust: Option<Grant>,
    pub(crate) wait: Duration,
}

pub(crate) fn retarget(jobs: &Jobs<Done>, ctx: &egui::Context, request: Request) {
    jobs.spawn(ctx, move || run_retarget(&RealSystem::new(), &ScanEnv::from_env(), &request));
}

fn stopped(tab: u64, failure: Failure) -> Done {
    if failure.waits() { Done::Held { tab } } else { Done::Failed { tab, failure } }
}

/// `Ok(true)` once the agent is stopped, `Ok(false)` when it was gone before it could be. An agent
/// that ignores the signal is ended with its terminal, which the window drops when the change lands.
fn end_it(
    system: &dyn System,
    env: &ScanEnv,
    request: &Request,
    session: &Session,
    pid: u32,
) -> Result<bool, Failure> {
    let Some(running) = session.running() else { return Ok(false) };
    let ending = EndRequest {
        cli: session.cli,
        pid: running.pid,
        proc_start: running.proc_start.clone(),
        session_id: session.session_id.clone(),
        allow_quiet: true,
        allow_working: request.interrupt,
    };
    let agents = request.profiles.agent_table();
    match end_session_in(Scope::Inside(&[pid]), system, env, &agents, &ending, request.wait) {
        Ended::Exited | Ended::StillRunning | Ended::Refused(EndRefusal::SignalFailed) => Ok(true),
        Ended::Refused(EndRefusal::Working) => Err(Failure::MidTurn),
        Ended::Refused(EndRefusal::ScanFailed) => Err(Failure::ScanFailed),
        Ended::Refused(_) => Ok(false),
    }
}

fn current_branch(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !branch.is_empty()).then_some(branch)
}

fn probe_folder(paths: &TidewatchPaths, dir: &Path) -> Folder {
    let Some(repo) = Repo::discover_main(dir) else { return Folder::default() };
    let branch = current_branch(dir);
    let made_by_tidewatch =
        worktree::is_made_by_tidewatch(paths, &repo.root, dir, branch.as_deref());
    Folder { main: Some(repo.root), branch, made_by_tidewatch }
}

fn run_retarget(system: &dyn System, env: &ScanEnv, request: &Request) -> Done {
    (0..3)
        .find_map(|_| attempt(system, env, request))
        .unwrap_or(Done::Failed { tab: request.subject.tab, failure: Failure::Changed })
}

/// One go at the change; `None` when the agent was gone before it could be stopped, so the next
/// go looks again.
fn attempt(system: &dyn System, env: &ScanEnv, request: &Request) -> Option<Done> {
    let subject = &request.subject;
    let agents = request.profiles.agent_table();
    let session = match subject.shell_pid {
        Some(pid) => {
            let Some(scanned) = scan_in_shells(system, env, &agents, &[pid]) else {
                return Some(Done::Failed { tab: subject.tab, failure: Failure::ScanFailed });
            };
            scanned.into_iter().find(|(shell, _)| *shell == pid).map(|(_, session)| session)
        }
        None => None,
    };
    let stored = request.remembered.clone().or_else(|| {
        let id = subject.known_session_id.clone()?;
        Some((subject.cli?, id))
    });
    let source = match (&session, &stored) {
        (Some(session), _) => Source::Running(session),
        (None, Some((cli, id))) => Source::Stored { cli: *cli, session_id: id },
        (None, None) => Source::Nothing,
    };
    let cwd = session.as_ref().map_or_else(|| subject.dir.clone(), |found| found.cwd.clone());
    let folder = if is_inside(&cwd, &subject.dir) {
        Folder::default()
    } else {
        probe_folder(&request.paths, &cwd)
    };
    let planned = match plan(
        &request.target,
        source,
        subject,
        &folder,
        (&request.profiles, &agents, &request.installed),
        request.interrupt,
    ) {
        Ok(planned) => planned,
        Err(failure) => return Some(stopped(subject.tab, failure)),
    };
    if let (Some(session), Some(pid)) = (&session, subject.shell_pid) {
        match end_it(system, env, request, session, pid) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(failure) => return Some(stopped(subject.tab, failure)),
        }
    }
    Some(match planned {
        Plan::Shell { remembered } => {
            Done::Retargeted(Box::new(Retargeted::Shell { tab: subject.tab, remembered }))
        }
        Plan::Agent(agent) => {
            let AgentPlan { agent: cli, profile_id, profile, start, landing, note } = *agent;
            let (session_id, fresh) = match start {
                Start::Resume(id) => (id, false),
                Start::Fresh => (String::new(), true),
            };
            let opened = run_reopen(&ReopenSpec {
                paths: request.paths.clone(),
                tab: subject.tab,
                repo: landing.repo.clone(),
                worktree: landing.worktree.clone(),
                dir: landing.dir.clone(),
                agent: cli,
                profile_id,
                profile,
                session_id,
                fresh,
                remote: subject.remote,
                yolo_anywhere: true,
                meeseeks: None,
                typed: None,
                path_var: request.path_var.clone(),
                listener: request.listener.clone(),
                trust: request.trust.clone(),
            });
            Done::Retargeted(Box::new(Retargeted::Agent {
                opened: Box::new(opened),
                landing,
                note,
            }))
        }
    })
}

#[cfg(test)]
mod tests;
