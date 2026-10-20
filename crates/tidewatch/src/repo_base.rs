//! What a repo says about its next worktree - the checked-out branch, the default branch, the local branches and the names already taken - looked up off the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::start_base;
use tidewatch_core::test_command;
use tidewatch_core::worktree::{local_branches, taken_names};

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

/// `origin/HEAD` as `origin/main`, or `None` without a remote or when it was never set.
fn origin_head(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !name.is_empty()).then_some(name)
}

/// The default branch among the local `branches`: the one `origin/HEAD` names, else `main`, else
/// `master`. `None` when the repo has none of them, so the form falls back to the checked-out branch.
pub(crate) fn pick_default_branch(
    origin_head: Option<&str>,
    branches: &[String],
) -> Option<String> {
    let named = origin_head.map(|head| head.strip_prefix("origin/").unwrap_or(head));
    named
        .into_iter()
        .chain(["main", "master"])
        .find(|candidate| branches.iter().any(|branch| branch == candidate))
        .map(str::to_owned)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RepoFacts {
    pub(crate) checked_out: Option<String>,
    pub(crate) default_branch: Option<String>,
    /// The remote's default branch (`main`, no `origin/`), when the repo has an `origin` with one.
    pub(crate) remote_default: Option<String>,
    pub(crate) branches: Vec<String>,
    pub(crate) taken: HashSet<String>,
    pub(crate) test_command: Option<String>,
}

impl RepoFacts {
    fn look(repo: &Path, paths: Option<&TidewatchPaths>) -> Self {
        let branches = local_branches(repo);
        Self {
            checked_out: checked_out(repo),
            default_branch: pick_default_branch(origin_head(repo).as_deref(), &branches),
            remote_default: start_base::remote_default(repo),
            branches,
            taken: paths.map(|paths| taken_names(paths, repo)).unwrap_or_default(),
            test_command: test_command::test_command(repo),
        }
    }
}

pub(crate) struct Bases {
    jobs: Jobs<(PathBuf, RepoFacts)>,
    known: HashMap<PathBuf, RepoFacts>,
}

impl Bases {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), known: HashMap::new() }
    }

    /// Looks `repo` up again, for a repo that may have switched branches or gained worktrees.
    pub(crate) fn refresh(&self, ctx: &egui::Context, repo: &Path, paths: Option<&TidewatchPaths>) {
        let (repo, paths) = (repo.to_path_buf(), paths.cloned());
        self.jobs.spawn(ctx, move || {
            let facts = RepoFacts::look(&repo, paths.as_ref());
            (repo, facts)
        });
    }

    pub(crate) fn drain(&mut self) -> bool {
        let done = self.jobs.drain();
        let any = !done.is_empty();
        self.known.extend(done);
        any
    }

    pub(crate) fn get(&self, repo: &Path) -> Option<&str> {
        self.known.get(repo)?.checked_out.as_deref()
    }

    pub(crate) fn facts(&self, repo: &Path) -> Option<&RepoFacts> {
        self.known.get(repo)
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
        bases.refresh(&egui::Context::default(), dir.path(), None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !bases.drain() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(bases.get(dir.path()), Some("main"));
    }

    #[test]
    fn a_look_names_the_branches_and_what_a_new_worktree_cannot_be_called() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "trunk"]);
        git(
            &repo,
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
        git(&repo, &["branch", "feat/x"]);
        git(&repo, &["branch", "agent/old"]);
        let paths = TidewatchPaths::new(&home);
        std::fs::create_dir_all(paths.worktree_dir(&repo, "busy")).unwrap();
        let facts = RepoFacts::look(&repo, Some(&paths));
        assert_eq!(facts.checked_out.as_deref(), Some("trunk"));
        let mut branches = facts.branches.clone();
        branches.sort();
        assert_eq!(branches, ["agent/old", "feat/x", "trunk"]);
        let mut taken: Vec<&str> = facts.taken.iter().map(String::as_str).collect();
        taken.sort_unstable();
        assert_eq!(taken, ["busy", "old"]);
        assert_eq!(RepoFacts::look(&repo, None).taken, HashSet::new());
    }

    #[test]
    fn the_default_branch_is_what_origin_names_else_main_else_master() {
        let list = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        let cases = [
            (Some("origin/trunk"), list(&["main", "trunk"]), Some("trunk")),
            (Some("origin/trunk"), list(&["main", "master"]), Some("main")),
            (Some("origin/trunk"), list(&["master"]), Some("master")),
            (None, list(&["master", "main"]), Some("main")),
            (None, list(&["feat/x", "master"]), Some("master")),
            (None, list(&["feat/x", "trunk"]), None),
            (Some("origin/main"), list(&[]), None),
            (Some("main"), list(&["main"]), Some("main")),
            (Some("origin/release/1.0"), list(&["release/1.0", "main"]), Some("release/1.0")),
        ];
        for (head, branches, want) in cases {
            assert_eq!(
                pick_default_branch(head, &branches).as_deref(),
                want,
                "{head:?} {branches:?}"
            );
        }
    }

    #[test]
    fn a_look_finds_the_default_branch_through_origin_head_and_without_a_remote() {
        let dir = tempfile::tempdir().unwrap();
        let commit = [
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ];
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "trunk"]);
        git(&origin, &commit);
        let clone = dir.path().join("clone");
        let status =
            Command::new("git").arg("clone").arg("-q").arg(&origin).arg(&clone).status().unwrap();
        assert!(status.success());
        git(&clone, &["checkout", "-q", "-b", "work"]);
        git(&clone, &["branch", "main"]);
        let facts = RepoFacts::look(&clone, None);
        assert_eq!(origin_head(&clone).as_deref(), Some("origin/trunk"));
        assert_eq!(facts.checked_out.as_deref(), Some("work"));
        assert_eq!(facts.default_branch.as_deref(), Some("trunk"));
        let alone = dir.path().join("alone");
        std::fs::create_dir_all(&alone).unwrap();
        git(&alone, &["init", "-q", "-b", "master"]);
        git(&alone, &commit);
        assert_eq!(origin_head(&alone), None);
        assert_eq!(RepoFacts::look(&alone, None).default_branch.as_deref(), Some("master"));
    }
}
