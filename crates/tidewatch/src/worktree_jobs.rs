//! The git work behind worktree tabs, run on `Jobs` threads. Each function starts one job and
//! returns at once; its `Done` arrives later through `Jobs::drain`.

use std::path::PathBuf;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree::{
    self, BranchChoice, BranchOutcome, Inspection, Leftover, RemoveMode, Worktree,
};

use crate::jobs::Jobs;
use crate::pane::Pane;

pub(crate) enum Done {
    Created { tab: u64, result: Result<Worktree, String> },
    Inspected { tab: u64, worktree: Worktree, inspection: Inspection },
    Removed { tab: u64, result: Result<BranchOutcome, String> },
    Startup { repaired: usize, leftovers: Vec<Leftover> },
}

pub(crate) fn create(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    repo: PathBuf,
    tab: u64,
) {
    jobs.spawn(ctx, move || Done::Created {
        tab,
        result: worktree::add(&paths, &repo).map_err(|e| e.to_string()),
    });
}

pub(crate) fn inspect(jobs: &Jobs<Done>, ctx: &egui::Context, tab: u64, worktree: Worktree) {
    jobs.spawn(ctx, move || {
        let inspection = worktree::inspect(&worktree);
        Done::Inspected { tab, worktree, inspection }
    });
}

/// Stops the shell first (on this thread, not the UI's) so nothing is running inside the folder
/// git is about to delete.
pub(crate) fn remove<T: Send + 'static>(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    tab: u64,
    shell: Option<T>,
    worktree: Worktree,
    branch: BranchChoice,
    mode: RemoveMode,
) {
    jobs.spawn(ctx, move || {
        drop(shell);
        Done::Removed {
            tab,
            result: worktree::remove(&paths, &worktree, branch, mode).map_err(|e| e.to_string()),
        }
    });
}

pub(crate) fn startup(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    repo: PathBuf,
) {
    jobs.spawn(ctx, move || {
        let repaired = worktree::repair(&paths, &repo).len();
        Done::Startup { repaired, leftovers: worktree::leftovers(&paths, &[]) }
    });
}

/// Leftovers scanned while a tab was being created can include that tab's own folder.
pub(crate) fn without_open<T>(
    leftovers: Vec<Leftover>,
    tabs: &crate::tabs::Tabs<Pane<T>>,
) -> Vec<Leftover> {
    leftovers
        .into_iter()
        .filter(|l| {
            !tabs.items().iter().any(|t| t.pane.worktree.as_ref().is_some_and(|w| w.path == l.path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::{Body, ClosePolicy, finish_create};
    use crate::tabs::Tabs;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn leftover(path: &str) -> Leftover {
        Leftover { path: PathBuf::from(path), branch: None }
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

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

    #[test]
    fn leftovers_skip_folders_that_open_tabs_own() {
        let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));
        let owned = Worktree {
            repo: "/r".into(),
            path: "/root/r-1/brave-otter".into(),
            branch: "agent/brave-otter".into(),
        };
        tabs.push("brave-otter".into(), Pane { body: Body::Shell(1), worktree: Some(owned) });
        let kept = without_open(
            vec![leftover("/root/r-1/brave-otter"), leftover("/root/r-1/calm-comet")],
            &tabs,
        );
        assert_eq!(kept, [leftover("/root/r-1/calm-comet")]);
    }

    #[test]
    fn opening_and_closing_a_worktree_tab_leaves_no_folder_and_no_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let paths = TidewatchPaths::new(&home);
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));

        let id = tabs.push("new worktree".into(), Pane::creating());
        create(&jobs, &ctx, paths.clone(), repo.clone(), id);
        let Done::Created { tab, result } = next(&jobs) else { panic!("expected Created") };
        finish_create(&mut tabs, tab, result, |_| 1);
        let worktree = tabs.get_mut(id).unwrap().pane.worktree.clone().expect("worktree tab");
        assert!(worktree.path.is_dir());
        assert!(git(&repo, &["worktree", "list"]).contains(worktree.name()));

        let policy = tabs.get_mut(id).unwrap().pane.close_policy();
        assert_eq!(policy, ClosePolicy::Confirm(worktree.clone()));
        inspect(&jobs, &ctx, id, worktree.clone());
        let Done::Inspected { inspection, .. } = next(&jobs) else { panic!("expected Inspected") };
        assert_eq!(inspection, Inspection::default());

        let (shell, to_remove) = tabs.get_mut(id).unwrap().pane.begin_remove().unwrap();
        remove(
            &jobs,
            &ctx,
            paths.clone(),
            id,
            shell,
            to_remove,
            BranchChoice::Delete,
            RemoveMode::Safe,
        );
        let Done::Removed { tab, result } = next(&jobs) else { panic!("expected Removed") };
        assert_eq!(result, Ok(BranchOutcome::Deleted));
        tabs.remove_id(tab);

        assert_eq!(tabs.items().len(), 1);
        assert!(!worktree.path.exists());
        assert!(!paths.repo_dir(&repo).exists());
        assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
        assert_eq!(git(&repo, &["branch", "--list", "agent/*"]).trim(), "");
    }

    #[test]
    fn a_dirty_worktree_stays_until_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let paths = TidewatchPaths::new(&home);
        let ctx = egui::Context::default();
        let jobs = Jobs::new();

        create(&jobs, &ctx, paths.clone(), repo.clone(), 1);
        let Done::Created { result: Ok(worktree), .. } = next(&jobs) else {
            panic!("expected a created worktree")
        };
        std::fs::write(worktree.path.join("wip.txt"), "x").unwrap();

        inspect(&jobs, &ctx, 1, worktree.clone());
        let Done::Inspected { inspection, .. } = next(&jobs) else { panic!("expected Inspected") };
        assert_eq!(inspection.changed, 1);

        remove::<u8>(
            &jobs,
            &ctx,
            paths.clone(),
            1,
            None,
            worktree.clone(),
            BranchChoice::Keep,
            RemoveMode::Safe,
        );
        let Done::Removed { result, .. } = next(&jobs) else { panic!("expected Removed") };
        assert_eq!(result, Err("1 file with uncommitted changes".to_owned()));
        assert!(worktree.path.join("wip.txt").is_file());

        remove::<u8>(
            &jobs,
            &ctx,
            paths,
            1,
            None,
            worktree.clone(),
            BranchChoice::Keep,
            RemoveMode::Discard,
        );
        let Done::Removed { result, .. } = next(&jobs) else { panic!("expected Removed") };
        assert_eq!(result, Ok(BranchOutcome::Kept));
        assert!(!worktree.path.exists());
    }
}
