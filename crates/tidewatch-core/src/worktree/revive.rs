//! Bringing a closed tab's worktree back: its branch at the logged commit and its folder at the
//! old path. Nothing is overwritten or deleted: a branch that exists stays where it is, and a
//! folder that is already a worktree of the repo is used as it is.

use std::fmt;
use std::path::PathBuf;

use super::{
    LOCK_REASON, Worktree, belongs_to, branch_exists, git, has_commits, object_exists, registry,
    roll_back, run, serial,
};
use crate::closed_tabs::Closed;
use crate::paths::TidewatchPaths;

#[derive(Debug, PartialEq, Eq)]
pub enum ReviveError {
    RepoGone(PathBuf),
    /// The log holds no commit to put the branch on.
    NoCommit,
    /// The logged commit is no longer in the repository.
    MissingCommit(String),
    /// The old path holds something that is not a worktree of this repo.
    PathTaken(PathBuf),
    Git(String),
}

impl fmt::Display for ReviveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepoGone(repo) => write!(f, "the repository {} is gone", repo.display()),
            Self::NoCommit => f.write_str("the log holds no commit for its branch"),
            Self::MissingCommit(sha) => {
                write!(f, "commit {sha} is no longer in the repository")
            }
            Self::PathTaken(path) => {
                write!(f, "{} is taken by something else", path.display())
            }
            Self::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ReviveError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revived {
    pub worktree: Worktree,
    pub made_branch: bool,
    /// The folder was made now. A worktree that was already there is left alone.
    pub made_folder: bool,
}

fn is_commit_id(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Recreates the branch at the logged commit when it is gone, then adds the worktree at the old
/// path. Blocks on git: call it from a background thread.
pub fn revive(paths: &TidewatchPaths, closed: &Closed) -> Result<Revived, ReviveError> {
    let _one_at_a_time = serial();
    let worktree = closed.worktree();
    if !worktree.repo.is_dir() || !has_commits(&worktree.repo) {
        return Err(ReviveError::RepoGone(worktree.repo));
    }
    if worktree.path.symlink_metadata().is_ok() {
        let ours =
            worktree.path.join(".git").is_file() && belongs_to(&worktree.repo, &worktree.path);
        return if ours {
            Ok(Revived { worktree, made_branch: false, made_folder: false })
        } else {
            Err(ReviveError::PathTaken(worktree.path))
        };
    }
    let branch_existed = branch_exists(&worktree.repo, &worktree.branch);
    if !branch_existed {
        let commit = closed
            .sha
            .as_deref()
            .or(closed.head.as_deref())
            .filter(|id| is_commit_id(id))
            .ok_or(ReviveError::NoCommit)?;
        if !object_exists(&worktree.repo, commit) {
            return Err(ReviveError::MissingCommit(commit.to_owned()));
        }
        let mut branch = git(&worktree.repo);
        branch.args(["branch", "--no-track", &worktree.branch, commit]);
        run(branch).map_err(|e| ReviveError::Git(e.to_string()))?;
    }
    let mut add = git(&worktree.repo);
    add.args(["worktree", "add", "--lock", "--reason", LOCK_REASON])
        .arg(&worktree.path)
        .arg(&worktree.branch);
    if let Err(error) = run(add) {
        roll_back(&worktree, branch_existed, false);
        return Err(ReviveError::Git(error.to_string()));
    }
    if let Err(e) = registry::record(paths, &worktree) {
        roll_back(&worktree, branch_existed, false);
        return Err(ReviveError::Git(format!("cannot record the worktree in the registry: {e}")));
    }
    Ok(Revived { worktree, made_branch: !branch_existed, made_folder: true })
}
