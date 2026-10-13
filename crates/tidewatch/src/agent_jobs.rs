//! Profiles and agent probing as background jobs: load or seed `profiles.toml`, look each CLI up on
//! `PATH` and ask it for its version, save edited profiles. Each function starts one job and returns
//! at once; its `Done` arrives through `Jobs::drain`.

use std::ffi::OsString;

use tidewatch_core::agents::{find_on_path, probe_version_in};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use crate::catalog::Availability;
use crate::jobs::Jobs;

pub(crate) enum Done {
    Loaded { profiles: Profiles, workflows: Workflows },
    Probed { agent: String, availability: Availability },
    Saved { result: Result<(), String>, profiles: Profiles },
    WorkflowsSaved { result: Result<(), String>, workflows: Workflows },
}

pub(crate) fn load(jobs: &Jobs<Done>, ctx: &egui::Context, paths: TidewatchPaths, path: OsString) {
    jobs.spawn(ctx, move || Done::Loaded {
        profiles: Profiles::load_or_seed(&paths, Some(&path)),
        workflows: Workflows::load_or_seed(&paths, Some(&path)),
    });
}

pub(crate) fn probe(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    agent: String,
    bin: String,
    path: OsString,
) {
    jobs.spawn(ctx, move || {
        let availability = availability_of(&bin, &path);
        Done::Probed { agent, availability }
    });
}

pub(crate) fn save(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    profiles: Profiles,
) {
    jobs.spawn(ctx, move || {
        let result = profiles.save(&paths.profiles_file()).map_err(|e| e.to_string());
        Done::Saved { result, profiles }
    });
}

pub(crate) fn save_workflows(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    workflows: Workflows,
) {
    jobs.spawn(ctx, move || {
        let result = workflows.save(&paths.workflows_file()).map_err(|e| e.to_string());
        Done::WorkflowsSaved { result, workflows }
    });
}

fn availability_of(bin: &str, path: &OsString) -> Availability {
    match find_on_path(bin, Some(path)) {
        None => Availability::Missing,
        Some(_) => Availability::Found(probe_version_in(bin, Some(path))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    fn next(jobs: &Jobs<Done>) -> Done {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(done) = jobs.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fake_cli(dir: &std::path::Path, name: &str, version: &str) {
        let file = dir.join(name);
        std::fs::write(&file, format!("#!/bin/sh\necho '{name} {version}'\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn first_launch_seeds_profiles_for_the_agents_on_the_path_and_writes_the_file() {
        let home = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        fake_cli(bin.path(), "claude", "2.1.296");
        let paths = TidewatchPaths::new(home.path());
        let jobs = Jobs::new();
        load(&jobs, &egui::Context::default(), paths.clone(), bin.path().into());
        let Done::Loaded { profiles, workflows } = next(&jobs) else { panic!("expected Loaded") };
        let ids: Vec<&str> = profiles.profile.keys().map(String::as_str).collect();
        assert_eq!(ids, ["builder", "builder-yolo", "reviewer"]);
        assert!(paths.profiles_file().is_file());
        let names: Vec<&str> = workflows.workflow.keys().map(String::as_str).collect();
        assert_eq!(names, ["fix", "pr", "ship"]);
        assert!(paths.workflows_file().is_file());
    }

    #[test]
    fn a_probe_reports_the_version_or_that_the_cli_is_missing() {
        let bin = tempfile::tempdir().unwrap();
        fake_cli(bin.path(), "fakeagent", "9.8.7");
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        probe(&jobs, &ctx, "fake".into(), "fakeagent".into(), bin.path().into());
        let Done::Probed { agent, availability } = next(&jobs) else { panic!("expected Probed") };
        assert_eq!(
            (agent.as_str(), availability),
            ("fake", Availability::Found(Some("9.8.7".into())))
        );
        probe(&jobs, &ctx, "gone".into(), "no-such-cli".into(), bin.path().into());
        let Done::Probed { availability, .. } = next(&jobs) else { panic!("expected Probed") };
        assert_eq!(availability, Availability::Missing);
    }

    #[test]
    fn saving_writes_profiles_toml_and_hands_the_profiles_back() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let jobs = Jobs::new();
        save(&jobs, &egui::Context::default(), paths.clone(), profiles.clone());
        let Done::Saved { result, profiles: back } = next(&jobs) else { panic!("expected Saved") };
        assert_eq!(result, Ok(()));
        assert_eq!(back, profiles);
        assert_eq!(Profiles::load(&paths.profiles_file()), profiles);
    }

    #[test]
    fn saving_workflows_writes_workflows_toml_and_hands_them_back() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut workflows = Workflows::defaults();
        workflows.workflow.remove("fix");
        let jobs = Jobs::new();
        save_workflows(&jobs, &egui::Context::default(), paths.clone(), workflows.clone());
        let Done::WorkflowsSaved { result, workflows: back } = next(&jobs) else {
            panic!("expected WorkflowsSaved")
        };
        assert_eq!(result, Ok(()));
        assert_eq!(back, workflows);
        assert_eq!(Workflows::load(&paths.workflows_file()), workflows);
    }
}
