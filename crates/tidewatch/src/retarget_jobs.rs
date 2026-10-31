//! Changing what a tab runs, the jobs: find the session, look where it would land, change it.

use std::ffi::OsString;
use std::hash::{BuildHasher as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tidewatch_core::elsewhere::Cli;
use tidewatch_core::elsewhere::{Scope, Session};
use tidewatch_core::elsewhere_scan::{
    EndRequest, Ended, RealSystem, Refusal as EndRefusal, ScanEnv, System, end_session_in,
    scan_in_shells,
};
use tidewatch_core::limited;
use tidewatch_core::listener::Listener;
use tidewatch_core::names;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::StartFrom;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::setup_recipe;
use tidewatch_core::start_base;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, Worktree};

use crate::jobs::Jobs;
use crate::repo::Repo;
use crate::repo_base::RepoFacts;
use crate::restore_jobs::{ReopenSpec, Reopened, run_reopen};
use crate::retarget::{
    AgentPlan, Failure, Folder, Home, Landing, Plan, Source, Start, Subject, Target, home, landing,
    plan,
};
use crate::retarget_copy::{copy_session, find_session};
use crate::retarget_repo::worked_in;
use crate::shell_agents::{Detected, Probe};
use crate::worktree_jobs::set_up;

pub(crate) enum Done {
    Detected(Vec<(u64, Detected)>),
    DetectFailed,
    Looked(Box<Looked>),
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
    Agent {
        opened: Box<Reopened>,
        landing: Landing,
        note: Option<String>,
        /// The tab the change was asked for; the agent runs in another tab when that one is the
        /// pinned main tab, which keeps its shell.
        from: u64,
    },
    Shell {
        tab: u64,
        remembered: Option<(Cli, String)>,
    },
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

/// Where a change would put a tab, looked up for the popover and kept until the session or its
/// folder changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Looked {
    pub(crate) tab: u64,
    pub(crate) key: LookKey,
    pub(crate) folder: Folder,
    /// The files with uncommitted changes in the checkout the work was in, and where it is.
    pub(crate) dirty: Option<(usize, PathBuf)>,
    /// The name the fresh worktree will take, when there is one.
    pub(crate) name: Option<String>,
}

/// What a look was made for: the folder and the session found in the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LookKey {
    pub(crate) cwd: PathBuf,
    pub(crate) session: Option<(Cli, String)>,
}

pub(crate) struct LookSpec {
    pub(crate) subject: Subject,
    pub(crate) key: LookKey,
    pub(crate) paths: TidewatchPaths,
    pub(crate) claude_dir: PathBuf,
}

pub(crate) fn look(jobs: &Jobs<Done>, ctx: &egui::Context, spec: LookSpec) {
    jobs.spawn(ctx, move || Done::Looked(Box::new(run_look(&spec))));
}

fn run_look(spec: &LookSpec) -> Looked {
    let LookSpec { subject, key, paths, claude_dir } = spec;
    let folder = look_folder(paths, claude_dir, subject, &key.cwd, key.session.as_ref());
    let checkout =
        folder.main.as_ref().map(|_| key.cwd.clone()).or_else(|| folder.worked_in.clone());
    let dirty = checkout.and_then(|dir| Some((uncommitted(&dir)?, dir)));
    let name = match home(&key.cwd, subject, &folder) {
        Home::Fresh { repo, .. } => Some(free_name(paths, &repo)),
        Home::Stay(_) => None,
    };
    Looked { tab: subject.tab, key: key.clone(), folder, dirty, name }
}

fn uncommitted(dir: &Path) -> Option<usize> {
    let out = limited::output(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["status", "--porcelain"])
            .env("GIT_OPTIONAL_LOCKS", "0"),
        limited::DEFAULT,
    )
    .ok()
    .filter(|out| out.status.success())?;
    let count = String::from_utf8_lossy(&out.stdout).lines().count();
    (count > 0).then_some(count)
}

fn free_name(paths: &TidewatchPaths, repo: &Path) -> String {
    let taken = worktree::taken_names(paths, repo);
    let seed = std::collections::hash_map::RandomState::new().build_hasher().finish();
    names::pick(seed, |name| taken.contains(name))
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
    pub(crate) start_from: StartFrom,
    /// The name a look promised the fresh worktree.
    pub(crate) name: Option<String>,
    /// A tab id kept for the agent when the pinned main tab is changed: the agent opens in a new
    /// tab and the main tab keeps its shell.
    pub(crate) spare_tab: Option<u64>,
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
    let out = limited::output(
        Command::new("git").arg("-C").arg(dir).args(["symbolic-ref", "--quiet", "--short", "HEAD"]),
        limited::DEFAULT,
    )
    .ok()?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !branch.is_empty()).then_some(branch)
}

fn probe_folder(paths: &TidewatchPaths, dir: &Path) -> Folder {
    let Some(repo) = Repo::discover_main(dir) else { return Folder::default() };
    let branch = current_branch(dir);
    let made_by_tidewatch =
        worktree::is_made_by_tidewatch(paths, &repo.root, dir, branch.as_deref());
    Folder { main: Some(repo.root), branch, made_by_tidewatch, worked_in: None }
}

fn look_folder(
    paths: &TidewatchPaths,
    claude_dir: &Path,
    subject: &Subject,
    cwd: &Path,
    session: Option<&(Cli, String)>,
) -> Folder {
    if subject.worktree.is_some() && crate::retarget::is_inside(cwd, &subject.dir) {
        return Folder::default();
    }
    let folder = probe_folder(paths, cwd);
    if folder.main.is_some() {
        return folder;
    }
    let id = session.filter(|(cli, _)| *cli == Cli::Claude).map(|(_, id)| id.as_str());
    Folder { worked_in: worked_in(claude_dir, cwd, id), ..folder }
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
    let known = match (&session, &stored) {
        (Some(session), _) => session.session_id.clone().map(|id| (session.cli, id)),
        (None, Some((cli, id))) => Some((*cli, id.clone())),
        (None, None) => None,
    };
    let folder = look_folder(&request.paths, &env.claude_dir, subject, &cwd, known.as_ref());
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
            let AgentPlan { agent: cli, profile_id, profile, start, home, note } = *agent;
            let settled = settle(env, request, home, start, note, &cli.id);
            let (session_id, fresh) = match settled.start {
                Start::Resume(id) => (id, false),
                Start::Fresh => (String::new(), true),
            };
            let tab = match (&settled.made, subject.pinned) {
                (Some(_), true) => request.spare_tab.unwrap_or(subject.tab),
                _ => subject.tab,
            };
            let mut opened = run_reopen(&ReopenSpec {
                paths: request.paths.clone(),
                tab,
                repo: settled.landing.repo.clone(),
                worktree: settled.landing.worktree.clone(),
                dir: settled.landing.dir.clone(),
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
            if let (Some(made), Ok(done)) = (&settled.made, &mut opened.result) {
                done.base_note.clone_from(&made.base_note);
                if let Some(started) = done.started.as_mut() {
                    started.prepared.notes.extend(made.setup_notes.clone());
                    started.prepared.notes.extend(
                        setup_recipe::install_through_links(&made.recipe).map(str::to_owned),
                    );
                    started.prepared.setup =
                        setup_recipe::with_generator(&made.recipe, &started.prepared.setup);
                }
            }
            Done::Retargeted(Box::new(Retargeted::Agent {
                opened: Box::new(opened),
                landing: settled.landing,
                note: settled.note,
                from: subject.tab,
            }))
        }
    })
}

struct Made {
    recipe: setup_recipe::Recipe,
    setup_notes: Vec<String>,
    base_note: Option<String>,
}

struct Settled {
    landing: Landing,
    start: Start,
    note: Option<String>,
    made: Option<Made>,
}

fn join_notes(first: Option<String>, second: &str) -> String {
    first.map_or_else(|| second.to_owned(), |first| format!("{first} {second}"))
}

fn make_worktree(request: &Request, repo: &Path) -> Result<(Worktree, Made), String> {
    let base = RepoFacts::start_facts(repo).default_base(request.start_from);
    let started = start_base::prepare(repo, base.as_deref(), start_base::FETCH_TIMEOUT);
    let name = request.name.clone().unwrap_or_else(|| free_name(&request.paths, repo));
    let made = worktree::add_free(&request.paths, repo, &name, started.base.as_deref())
        .map_err(|e| e.to_string())?;
    let (recipe, setup_notes) = set_up(&request.paths, &made);
    Ok((made, Made { recipe, setup_notes, base_note: started.note }))
}

/// Where the agent runs and what it starts with. A fresh home makes the worktree and, for Claude,
/// copies the conversation to the worktree's project folder; when either cannot be done the tab
/// stays in its folder, or starts a new session, and the note says so.
fn settle(
    env: &ScanEnv,
    request: &Request,
    home: Home,
    start: Start,
    note: Option<String>,
    agent_id: &str,
) -> Settled {
    let (repo, from) = match home {
        Home::Stay(landing) => return Settled { landing, start, note, made: None },
        Home::Fresh { repo, from } => (repo, from),
    };
    let (made, extra) = match make_worktree(request, &repo) {
        Ok(made) => made,
        Err(e) => {
            let landing = landing(&from, &request.subject, &Folder::default());
            let said =
                format!("A new branch could not be made ({e}), so the tab stays in its folder.");
            let note = Some(join_notes(note, &said));
            return Settled { landing, start, note, made: None };
        }
    };
    let (start, note) = match (&start, agent_id) {
        (Start::Resume(id), "claude") => {
            match find_session(&env.claude_dir, &from, id)
                .ok_or_else(|| "its file was not found".to_owned())
                .and_then(|file| copy_session(&env.claude_dir, &file, &made.path))
            {
                Ok(()) => (start, note),
                Err(e) => {
                    let said = format!(
                        "The conversation could not be copied ({e}), so a new session started. The old one stays on disk."
                    );
                    (Start::Fresh, Some(join_notes(note, &said)))
                }
            }
        }
        _ => (start, note),
    };
    let landing = Landing { dir: made.path.clone(), worktree: Some(made), repo };
    Settled { landing, start, note, made: Some(extra) }
}

#[cfg(test)]
mod tests;
