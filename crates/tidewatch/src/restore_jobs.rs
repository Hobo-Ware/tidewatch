//! The blocking work behind session restore, run on `Jobs` threads: reading the saved session,
//! repairing a moved repo, planning every tab against the disk and scanning for leftover
//! worktrees (one job at startup), and getting one resumed agent ready (a fresh token for the tab,
//! the files its CLI reads, folder trust for the worktree, the resume command).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use tidewatch_core::agents::Agent;
use tidewatch_core::listener::{Listener, TabRegistration};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;
use tidewatch_core::prepare::{PrepareInput, prepare_resume};
use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::restore::{self, RealDisk, RestoreEnv, WindowPlan, plan_window};
use tidewatch_core::session::Session;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, Leftover, Worktree};

use crate::jobs::Jobs;
use crate::pane::{OpenFailure, Opened, Started};
use crate::session_save;

pub(crate) enum Done {
    Planned(Box<Startup>),
    Reopened(Box<Reopened>),
}

pub(crate) struct StartupSpec {
    pub(crate) paths: TidewatchPaths,
    /// The tab group of the repo in the window.
    pub(crate) group: usize,
    /// The repo root, or the folder in the Other group.
    pub(crate) dir: PathBuf,
    pub(crate) other: bool,
    /// `--no-restore`: read nothing back, whatever the settings say.
    pub(crate) skip: bool,
    pub(crate) profiles: Profiles,
    pub(crate) path_var: OsString,
    pub(crate) disk: RealDisk,
}

pub(crate) struct Startup {
    pub(crate) group: usize,
    pub(crate) repaired: usize,
    /// Worktree folders no open tab and no saved tab owns. Empty in the Other group.
    pub(crate) leftovers: Vec<Leftover>,
    /// `None` when restoring is off or the session has nothing for this window.
    pub(crate) plan: Option<WindowPlan>,
    pub(crate) prefs: Prefs,
}

/// Repairs worktrees after a moved repo, plans the saved window and scans for leftovers.
pub(crate) fn startup(jobs: &Jobs<Done>, ctx: &egui::Context, spec: StartupSpec) {
    jobs.spawn(ctx, move || Done::Planned(Box::new(run_startup(&spec))));
}

fn run_startup(spec: &StartupSpec) -> Startup {
    let paths = &spec.paths;
    let prefs = Prefs::load(paths);
    let mut session = Session::load(paths);
    let mut repaired = 0;
    if !spec.other {
        let known = session.window(&spec.dir).is_some();
        repaired += restore::repair_moved(paths, &mut session, &spec.dir).len();
        if !known && session.window(&spec.dir).is_some() {
            session
                .save(paths)
                .unwrap_or_else(|e| eprintln!("tidewatch: cannot save the session: {e}"));
        }
        repaired += worktree::repair(paths, &spec.dir).len();
    }
    let plan = (prefs.restore_session && !spec.skip)
        .then(|| session.window(&spec.dir))
        .flatten()
        .map(|window| {
            let agents = spec.profiles.agent_table();
            let installed = agents.found_on_path(Some(&spec.path_var));
            let env =
                RestoreEnv { agents: &agents, profiles: &spec.profiles, installed: &installed };
            plan_window(window, &env, &spec.disk)
        });
    let mut owned: Vec<PathBuf> = session
        .windows
        .iter()
        .filter(|window| window.dir != spec.dir)
        .flat_map(session_save::worktree_dirs)
        .collect();
    if let Some(plan) = &plan {
        owned.extend(
            plan.tabs.iter().filter_map(|tab| tab.worktree.as_ref().map(|w| w.path.clone())),
        );
    }
    let leftovers = if spec.other { Vec::new() } else { worktree::leftovers(paths, &owned) };
    Startup { group: spec.group, repaired, leftovers, plan, prefs }
}

/// Writes the restore switch to the settings file off the UI thread, leaving the rest of the file
/// as it is on disk; a failure goes to stderr.
pub(crate) fn save_restore_switch(paths: TidewatchPaths, on: bool) {
    std::thread::spawn(move || {
        if let Err(e) = Prefs::update(&paths, |prefs| prefs.restore_session = on) {
            eprintln!("tidewatch: cannot save settings.toml: {e}");
        }
    });
}

pub(crate) struct ReopenSpec {
    pub(crate) paths: TidewatchPaths,
    pub(crate) tab: u64,
    /// The repo root, or the folder in the Other group.
    pub(crate) repo: PathBuf,
    pub(crate) worktree: Option<Worktree>,
    pub(crate) dir: PathBuf,
    pub(crate) agent: Agent,
    /// The `profiles.toml` key the tab was saved with.
    pub(crate) profile_id: String,
    /// The profile at the level the tab ran at.
    pub(crate) profile: Profile,
    pub(crate) session_id: String,
    pub(crate) path_var: OsString,
    /// Where the agent reports; `None` when the listener is down.
    pub(crate) listener: Option<Arc<Listener>>,
    /// Set after the user agreed to folder trust (D14): the worktree gets its entry again.
    pub(crate) trust: Option<Grant>,
}

pub(crate) struct Reopened {
    pub(crate) tab: u64,
    pub(crate) result: Result<Opened, OpenFailure>,
    /// Printed above the plain shell that stands in when the agent could not be prepared.
    pub(crate) note: Option<String>,
}

pub(crate) fn reopen(jobs: &Jobs<Done>, ctx: &egui::Context, spec: ReopenSpec) {
    jobs.spawn(ctx, move || Done::Reopened(Box::new(run_reopen(&spec))));
}

fn run_reopen(spec: &ReopenSpec) -> Reopened {
    let tab_key = spec
        .worktree
        .as_ref()
        .map_or_else(|| format!("tab-{}", spec.tab), |worktree| worktree.name().to_owned());
    let hooks = spec.listener.as_ref().map(|listener| {
        listener.register_tab(TabRegistration {
            tab_id: spec.tab,
            agent: spec.agent.id.clone(),
            repo: spec.repo.clone(),
            tab: tab_key.clone(),
            generation: 0,
        })
    });
    let prepared = prepare_resume(
        &PrepareInput {
            paths: &spec.paths,
            agent: &spec.agent,
            profile: &spec.profile,
            repo: &spec.repo,
            tab: &tab_key,
            dir: &spec.dir,
            worktree_tab: spec.worktree.is_some(),
            task: "",
            session_id: None,
            path_var: Some(&spec.path_var),
            listener: hooks,
            trust: spec.trust.as_ref(),
        },
        &spec.session_id,
    );
    let (started, note) = match prepared {
        Ok(prepared) => {
            let started = Started {
                profile_name: spec.profile.name.clone(),
                profile_id: spec.profile_id.clone(),
                level: spec.profile.level,
                agent_id: spec.agent.id.clone(),
                notes_file: Some(spec.paths.notes_file(&spec.repo, &tab_key)),
                session_id: Some(spec.session_id.clone()),
                prepared,
            };
            (Some(Box::new(started)), None)
        }
        Err(e) => {
            if let Some(listener) = &spec.listener {
                listener.unregister_tab(spec.tab);
            }
            (None, Some(format!("The agent was not resumed: {e}.")))
        }
    };
    Reopened {
        tab: spec.tab,
        result: Ok(Opened { worktree: spec.worktree.clone(), dir: spec.dir.clone(), started }),
        note,
    }
}

#[cfg(test)]
mod tests;
