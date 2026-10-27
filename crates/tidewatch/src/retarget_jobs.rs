//! The blocking work of changing what a tab runs, on `Jobs` threads: the look inside the shell
//! tabs for an agent session, and one change (scan again, plan, end the agent with `end_session_in`
//! when it runs, get the new agent ready). The window swaps the tab's pane when it lands.

use std::ffi::OsString;
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

use crate::jobs::Jobs;
use crate::restore_jobs::{ReopenSpec, Reopened, run_reopen};
use crate::retarget::{Plan, Refusal, Source, Start, Subject, Target, plan};
use crate::shell_agents::{Detected, Probe};

pub(crate) enum Done {
    Detected(Vec<(u64, Detected)>),
    DetectFailed,
    Retargeted(Box<Retargeted>),
    Refused { tab: u64, refusal: Refusal },
}

pub(crate) enum Retargeted {
    Agent(Reopened),
    Shell { tab: u64, cli: Cli, session_id: String },
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

fn refused(tab: u64, refusal: Refusal) -> Done {
    Done::Refused { tab, refusal }
}

fn end_refusal(refusal: EndRefusal) -> Refusal {
    match refusal {
        EndRefusal::Gone | EndRefusal::Stdusk | EndRefusal::NotConfirmed => Refusal::Gone,
        EndRefusal::Working => Refusal::MidTurn,
        EndRefusal::SignalFailed => Refusal::SignalFailed,
        EndRefusal::ScanFailed => Refusal::ScanFailed,
    }
}

fn end_it(
    system: &dyn System,
    env: &ScanEnv,
    request: &Request,
    session: &Session,
) -> Result<(), Refusal> {
    let running = session.running().ok_or(Refusal::NothingRunning)?;
    let ending = EndRequest {
        cli: session.cli,
        pid: running.pid,
        proc_start: running.proc_start.clone(),
        session_id: session.session_id.clone(),
        allow_quiet: true,
        allow_working: request.interrupt,
    };
    let agents = request.profiles.agent_table();
    let shells = [request.subject.shell_pid];
    match end_session_in(Scope::Inside(&shells), system, env, &agents, &ending, request.wait) {
        Ended::Exited => Ok(()),
        Ended::StillRunning => Err(Refusal::WillNotEnd { secs: request.wait.as_secs() }),
        Ended::Refused(refusal) => Err(end_refusal(refusal)),
    }
}

fn run_retarget(system: &dyn System, env: &ScanEnv, request: &Request) -> Done {
    let subject = &request.subject;
    let agents = request.profiles.agent_table();
    let Some(scanned) = scan_in_shells(system, env, &agents, &[subject.shell_pid]) else {
        return refused(subject.tab, Refusal::ScanFailed);
    };
    let session =
        scanned.into_iter().find(|(shell, _)| *shell == subject.shell_pid).map(|(_, s)| s);
    let stored = request.remembered.clone().or_else(|| {
        let id = subject.known_session_id.clone()?;
        Some((subject.cli?, id))
    });
    let source = match (&session, &stored) {
        (Some(session), _) => Source::Running(session),
        (None, Some((cli, id))) => Source::Stored { cli: *cli, session_id: id },
        (None, None) => Source::Nothing,
    };
    let planned = plan(
        &request.target,
        source,
        subject,
        (&request.profiles, &agents, &request.installed),
        request.interrupt,
    );
    let planned = match planned {
        Ok(planned) => planned,
        Err(refusal) => return refused(subject.tab, refusal),
    };
    if let Some(session) = &session
        && let Err(refusal) = end_it(system, env, request, session)
    {
        return refused(subject.tab, refusal);
    }
    match planned {
        Plan::Shell { cli, session_id } => {
            Done::Retargeted(Box::new(Retargeted::Shell { tab: subject.tab, cli, session_id }))
        }
        Plan::Agent(agent) => {
            let (session_id, fresh) = match agent.start {
                Start::Resume(id) => (id, false),
                Start::Fresh => (String::new(), true),
            };
            let reopened = run_reopen(&ReopenSpec {
                paths: request.paths.clone(),
                tab: subject.tab,
                repo: subject.repo.clone(),
                worktree: subject.worktree.clone(),
                dir: agent.cwd,
                agent: agent.agent,
                profile_id: agent.profile_id,
                profile: agent.profile,
                session_id,
                fresh,
                remote: subject.remote,
                meeseeks: None,
                typed: None,
                path_var: request.path_var.clone(),
                listener: request.listener.clone(),
                trust: request.trust.clone(),
            });
            Done::Retargeted(Box::new(Retargeted::Agent(reopened)))
        }
    }
}

#[cfg(test)]
mod tests;
