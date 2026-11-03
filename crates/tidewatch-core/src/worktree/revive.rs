//! Bringing a closed tab's worktree back: its branch at the logged commit and its folder at the
//! old path. Nothing is overwritten or deleted: a branch that exists stays where it is, a folder
//! that is already a worktree of the repo is used as it is, and a folder whose record git pruned
//! gets a new record with its files kept.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    LOCK_REASON, Worktree, belongs_to, branch_exists, common_dir, git, has_commits, object_exists,
    registry, roll_back, run, serial,
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
    let relink = worktree.path.symlink_metadata().is_ok();
    if relink {
        if worktree.path.join(".git").is_file() && belongs_to(&worktree.repo, &worktree.path) {
            return Ok(Revived { worktree, made_branch: false, made_folder: false });
        }
        if !lost_its_record(&worktree) {
            return Err(ReviveError::PathTaken(worktree.path));
        }
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
    let added = if relink { relink_folder(&worktree) } else { add(&worktree, false) };
    if let Err(error) = added {
        if !relink {
            roll_back(&worktree, branch_existed, false);
        }
        return Err(error);
    }
    if let Err(e) = registry::record(paths, &worktree) {
        roll_back(&worktree, branch_existed, false);
        return Err(ReviveError::Git(format!("cannot record the worktree in the registry: {e}")));
    }
    Ok(Revived { worktree, made_branch: !branch_existed, made_folder: !relink })
}

fn add(worktree: &Worktree, no_checkout: bool) -> Result<(), ReviveError> {
    let mut add = git(&worktree.repo);
    add.args(["worktree", "add", "--lock", "--reason", LOCK_REASON]);
    if no_checkout {
        add.arg("--no-checkout");
    }
    add.arg(&worktree.path).arg(&worktree.branch);
    run(add).map(drop).map_err(|e| ReviveError::Git(e.to_string()))
}

fn lost_its_record(worktree: &Worktree) -> bool {
    let Ok(link) = fs::read_to_string(worktree.path.join(".git")) else { return false };
    let Some(record) = link.trim().strip_prefix("gitdir: ").map(Path::new) else { return false };
    let Some(records) = record.parent().filter(|dir| dir.ends_with("worktrees")) else {
        return false;
    };
    !record.exists()
        && records.parent().and_then(|dir| dir.canonicalize().ok()) == common_dir(&worktree.repo)
}

/// Sets the folder aside, has git make a fresh record and `.git` link at the old path without
/// checking anything out, moves the files back and refreshes the index from the branch, so
/// changes that were never committed show as changes again.
fn relink_folder(worktree: &Worktree) -> Result<(), ReviveError> {
    let io = |what: &str, e: std::io::Error| ReviveError::Git(format!("cannot {what}: {e}"));
    let aside = worktree.path.with_file_name(format!(".{}.relink", worktree.name()));
    fs::rename(&worktree.path, &aside).map_err(|e| io("set the folder aside", e))?;
    if let Err(error) = add(worktree, true) {
        let _ = fs::remove_dir_all(&worktree.path);
        let _ = fs::rename(&aside, &worktree.path);
        return Err(error);
    }
    let kept = format!("the files stay in {}", aside.display());
    for entry in fs::read_dir(&aside).map_err(|e| io(&kept, e))? {
        let entry = entry.map_err(|e| io(&kept, e))?;
        if entry.file_name() != ".git" {
            fs::rename(entry.path(), worktree.path.join(entry.file_name()))
                .map_err(|e| io(&kept, e))?;
        }
    }
    let _ = fs::remove_file(aside.join(".git"));
    let _ = fs::remove_dir(&aside);
    let mut index = git(&worktree.path);
    index.args(["reset", "-q"]);
    run(index).map(drop).map_err(|e| ReviveError::Git(e.to_string()))
}
