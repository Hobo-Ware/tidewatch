//! The checked-out branch each repo's new worktrees start from, looked up off the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::jobs::Jobs;

/// The checked-out branch, or `None` for a detached HEAD or a folder that is not a repo.
fn checked_out(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !name.is_empty()).then_some(name)
}

pub(crate) struct Bases {
    jobs: Jobs<(PathBuf, Option<String>)>,
    known: HashMap<PathBuf, Option<String>>,
}

impl Bases {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), known: HashMap::new() }
    }

    /// Looks the branch of `repo` up again, for a repo that may have switched branches.
    pub(crate) fn refresh(&self, ctx: &egui::Context, repo: &Path) {
        let repo = repo.to_path_buf();
        self.jobs.spawn(ctx, move || {
            let base = checked_out(&repo);
            (repo, base)
        });
    }

    pub(crate) fn drain(&mut self) -> bool {
        let done = self.jobs.drain();
        let any = !done.is_empty();
        self.known.extend(done);
        any
    }

    pub(crate) fn get(&self, repo: &Path) -> Option<&str> {
        self.known.get(repo)?.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
        assert!(status.success(), "{args:?}");
    }

    #[test]
    fn a_repo_names_its_checked_out_branch_and_a_detached_head_or_plain_folder_names_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(checked_out(dir.path()), None);
        git(dir.path(), &["init", "-q", "-b", "trunk"]);
        git(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        assert_eq!(checked_out(dir.path()).as_deref(), Some("trunk"));
        git(dir.path(), &["checkout", "-q", "--detach"]);
        assert_eq!(checked_out(dir.path()), None);
    }

    #[test]
    fn a_lookup_comes_back_through_the_job() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        let mut bases = Bases::new();
        assert_eq!(bases.get(dir.path()), None);
        bases.refresh(&egui::Context::default(), dir.path());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !bases.drain() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(bases.get(dir.path()), Some("main"));
    }
}
