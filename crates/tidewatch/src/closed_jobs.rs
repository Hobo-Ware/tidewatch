//! The blocking work of Recently closed, run on `Jobs` threads: reading the list, noting a tab
//! closed with its worktree kept, and one reopen (branch and worktree back, the repo's setup
//! recipe, the one-tab plan the restore path carries out). The window places the tab when it lands.

use std::ffi::OsString;

use tidewatch_core::closed_tabs::{self, Closed, Tab};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::restore::{Disk, RealDisk, RestoreEnv, WindowPlan, plan_window};
use tidewatch_core::worktree::{self, Worktree};

use crate::jobs::Jobs;
use crate::repo::Workspace;
use crate::worktree_jobs::set_up;

pub(crate) enum Done {
    Loaded(Vec<Closed>),
    Noted,
    Revived(Box<Landed>),
    Refused { session_id: String, text: String },
}

pub(crate) struct Landed {
    pub(crate) closed: Closed,
    pub(crate) workspace: Workspace,
    pub(crate) plan: WindowPlan,
    pub(crate) notes: Vec<String>,
}

pub(crate) struct ReviveSpec {
    pub(crate) paths: TidewatchPaths,
    pub(crate) closed: Closed,
    pub(crate) profiles: Profiles,
    pub(crate) path_var: OsString,
    pub(crate) disk: RealDisk,
}

pub(crate) fn load(jobs: &Jobs<Done>, ctx: &egui::Context, paths: TidewatchPaths, disk: RealDisk) {
    jobs.spawn(ctx, move || Done::Loaded(run_load(&paths, &disk)));
}

fn run_load(paths: &TidewatchPaths, disk: &dyn Disk) -> Vec<Closed> {
    closed_tabs::load(paths)
        .into_iter()
        .filter(|closed| {
            closed
                .session_id
                .as_deref()
                .is_some_and(|id| disk.transcript_exists(&closed.agent, &closed.path, id))
        })
        .collect()
}

pub(crate) fn note_kept(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    worktree: Worktree,
    tab: Tab,
) {
    jobs.spawn(ctx, move || {
        if let Err(e) = worktree::note_kept(&paths, &worktree, &tab) {
            eprintln!("tidewatch: cannot note the closed tab: {e}");
        }
        Done::Noted
    });
}

pub(crate) fn revive(jobs: &Jobs<Done>, ctx: &egui::Context, spec: ReviveSpec) {
    jobs.spawn(ctx, move || run_revive(spec));
}

fn has_profile(profiles: &Profiles, key: &str) -> bool {
    profiles.profile.contains_key(key) || profiles.profile.values().any(|p| p.name == key)
}

fn run_revive(spec: ReviveSpec) -> Done {
    let ReviveSpec { paths, closed, profiles, path_var, disk } = spec;
    let session_id = closed.session_id.clone().unwrap_or_default();
    let refused = |why: String| Done::Refused {
        text: format!("Could not reopen {}: {why}", closed.title),
        session_id: session_id.clone(),
    };
    if !has_profile(&profiles, &closed.profile) {
        return refused(crate::reopen::Why::NoProfile.text().to_owned());
    }
    let revived = match worktree::revive(&paths, &closed) {
        Ok(revived) => revived,
        Err(e) => return refused(e.to_string()),
    };
    let notes = if revived.made_folder { set_up(&paths, &revived.worktree).1 } else { Vec::new() };
    let agents = profiles.agent_table();
    let installed = agents.found_on_path(Some(&path_var));
    let env = RestoreEnv {
        agents: &agents,
        profiles: &profiles,
        installed: &installed,
        resume: ResumeAgents::Auto,
        crash_loop: false,
    };
    let plan = plan_window(&closed.saved_window(), &env, &disk);
    let workspace = Workspace::open_in_main_repo(&closed.repo);
    Done::Revived(Box::new(Landed { closed, workspace, plan, notes }))
}

#[cfg(test)]
mod tests;
