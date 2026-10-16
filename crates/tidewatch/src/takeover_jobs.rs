//! The blocking work of take-over, run on `Jobs` threads: the scan that fills the card and one move
//! (make the worktree if asked, end the session, look up the repo of its folder). The window places
//! the tab when a move lands.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tidewatch_core::agents::AgentTable;
use tidewatch_core::elsewhere::Key;
use tidewatch_core::elsewhere_scan::{
    EndRequest, Ended, RealSystem, Refusal, ScanEnv, System, end_session, recent_sessions,
    scan_running,
};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree::{self, BranchChoice, RemoveMode, Worktree};

use crate::jobs::Jobs;
use crate::repo::Workspace;
use crate::takeover::{MoveSpec, RECENT_PER_FOLDER, RECENT_ROWS, Scanned, order};

pub(crate) enum Done {
    Scanned(Box<Scanned>),
    ScanFailed,
    Moved(Box<Landed>),
    Failed(Failure),
}

pub(crate) struct Landed {
    pub(crate) spec: MoveSpec,
    pub(crate) workspace: Workspace,
    pub(crate) worktree: Option<Worktree>,
}

pub(crate) struct Failure {
    pub(crate) key: Option<Key>,
    pub(crate) name: String,
    pub(crate) why: String,
}

pub(crate) struct ScanSpec {
    pub(crate) agents: AgentTable,
    pub(crate) folders: Vec<PathBuf>,
    pub(crate) held: HashSet<String>,
    pub(crate) roots: HashMap<PathBuf, Option<PathBuf>>,
}

pub(crate) fn scan(jobs: &Jobs<Done>, ctx: &egui::Context, spec: ScanSpec) {
    jobs.spawn(ctx, move || run_scan(&RealSystem::new(), &ScanEnv::from_env(), &spec));
}

fn run_scan(system: &dyn System, env: &ScanEnv, spec: &ScanSpec) -> Done {
    let Some(found) = scan_running(system, env, &spec.agents) else { return Done::ScanFailed };
    let mut held = spec.held.clone();
    held.extend(found.held_ids);
    held.extend(found.sessions.iter().filter_map(|session| session.session_id.clone()));
    let mut recent = recent_sessions(env, &spec.folders, &held, RECENT_PER_FOLDER);
    order(&mut recent);
    recent.truncate(RECENT_ROWS);
    let mut sessions = found.sessions;
    sessions.extend(recent);
    order(&mut sessions);
    let mut roots = HashMap::new();
    for session in &sessions {
        if !spec.roots.contains_key(&session.cwd) && !roots.contains_key(&session.cwd) {
            roots.insert(session.cwd.clone(), system.git_root(&session.cwd));
        }
    }
    Done::Scanned(Box::new(Scanned { sessions, roots }))
}

pub(crate) struct MoveJob {
    pub(crate) spec: MoveSpec,
    pub(crate) agents: AgentTable,
    pub(crate) paths: Option<TidewatchPaths>,
    pub(crate) root: Option<PathBuf>,
    pub(crate) wait: Duration,
}

pub(crate) fn start_move(jobs: &Jobs<Done>, ctx: &egui::Context, job: MoveJob) {
    jobs.spawn(ctx, move || run_move(&RealSystem::new(), &ScanEnv::from_env(), &job));
}

fn refusal_text(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::Gone => "it is not running as it was; nothing was ended",
        Refusal::Working => "it started working again; it was left running",
        Refusal::NotConfirmed => "its status is not known; it was left running",
        Refusal::Stdusk => "it runs in stdusk; use stdusk's Move tabs to Tidewatch",
        Refusal::SignalFailed => "the signal could not be sent; it was left running",
        Refusal::ScanFailed => "the process list could not be read; nothing was ended",
    }
}

fn run_move(system: &dyn System, env: &ScanEnv, job: &MoveJob) -> Done {
    let MoveJob { spec, agents, paths, root, wait } = job;
    let session = &spec.session;
    let failed = |why: String| {
        Done::Failed(Failure { key: session.key(), name: session.display_name(now_ms()), why })
    };
    let made = if spec.worktree {
        let (Some(paths), Some(root)) = (paths, root) else {
            return failed("a worktree needs a git repo".to_owned());
        };
        match worktree::add(paths, root) {
            Ok(made) => Some(made),
            Err(e) => return failed(format!("the worktree could not be made ({e})")),
        }
    } else {
        None
    };
    if let Some(running) = session.running() {
        let request = EndRequest {
            cli: session.cli,
            pid: running.pid,
            proc_start: running.proc_start.clone(),
            session_id: session.session_id.clone(),
            allow_quiet: spec.allow_quiet,
        };
        let ended = end_session(system, env, agents, &request, *wait);
        let why = match ended {
            Ended::Exited => None,
            Ended::StillRunning => {
                Some(format!("it did not exit within {} s and was left running", wait.as_secs()))
            }
            Ended::Refused(refusal) => Some(refusal_text(refusal).to_owned()),
        };
        if let Some(why) = why {
            if let (Some(made), Some(paths)) = (&made, paths) {
                let _ = worktree::remove(paths, made, BranchChoice::Delete, RemoveMode::Safe);
            }
            return failed(why);
        }
    }
    let folder: &Path = made.as_ref().map_or(&session.cwd, |made| &made.repo);
    Done::Moved(Box::new(Landed {
        spec: spec.clone(),
        workspace: Workspace::open(folder),
        worktree: made,
    }))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests;
