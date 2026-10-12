//! Create, list, repair and remove the git worktrees Tidewatch tabs run in.
//!
//! Every function shells out to `git` and blocks until it finishes: call them from a background
//! thread, never the UI thread. Behavior was checked against git 2.55.0 (see `PLAN.md`, Phase 0
//! findings).

use std::fmt;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::names;
use crate::paths::TidewatchPaths;
use crate::registry;

pub const BRANCH_PREFIX: &str = "agent/";
const LOCK_REASON: &str = "tidewatch";

/// A worktree Tidewatch made: the repo it belongs to, its folder and its `agent/<name>` branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
}

impl Worktree {
    /// The tab title: the branch without its `agent/` prefix.
    #[must_use]
    pub fn name(&self) -> &str {
        self.branch.strip_prefix(BRANCH_PREFIX).unwrap_or(&self.branch)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WorktreeError {
    /// git refused; carries its message.
    Git(String),
    /// Uncommitted changes, and the caller did not pass an explicit discard.
    Dirty { changed: usize },
    /// The target folder already holds something.
    PathTaken(PathBuf),
    /// git could not be started, or the disk refused.
    Io(String),
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(message) | Self::Io(message) => f.write_str(message),
            Self::Dirty { changed } => {
                write!(f, "{changed} file{} with uncommitted changes", plural(*changed))
            }
            Self::PathTaken(path) => write!(f, "{} already exists", path.display()),
        }
    }
}

impl std::error::Error for WorktreeError {}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchChoice {
    Keep,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveMode {
    /// Refuse a worktree with uncommitted changes.
    Safe,
    /// The caller asked explicitly: uncommitted changes are thrown away.
    Discard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchOutcome {
    Kept,
    Deleted,
    /// The worktree is gone but git would not delete the branch.
    DeleteFailed(String),
}

/// What closing a worktree would throw away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inspection {
    pub changed: usize,
    /// Commits on the branch that no other branch or remote has.
    pub unmerged: usize,
}

/// One entry of `git worktree list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub locked: Option<String>,
    pub prunable: bool,
}

/// A worktree folder under the Tidewatch root that no open tab owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    pub path: PathBuf,
    pub branch: Option<String>,
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

fn run(mut command: Command) -> Result<String, WorktreeError> {
    let out = command.output().map_err(|e| WorktreeError::Io(format!("cannot run git: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(WorktreeError::Git(clean_message(&String::from_utf8_lossy(&out.stderr))))
    }
}

fn clean_message(stderr: &str) -> String {
    let text = stderr.trim();
    text.strip_prefix("fatal: ").or_else(|| text.strip_prefix("error: ")).unwrap_or(text).to_owned()
}

fn succeeds(mut command: Command) -> bool {
    command.output().is_ok_and(|out| out.status.success())
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    let mut command = git(repo);
    command.args(["show-ref", "--verify", "--quiet"]).arg(format!("refs/heads/{branch}"));
    succeeds(command)
}

fn has_commits(repo: &Path) -> bool {
    let mut command = git(repo);
    command.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
    succeeds(command)
}

fn holds_something(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => path.symlink_metadata().is_ok(),
    }
}

fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// Adds a worktree on a new `agent/<name>` branch off the repo's current commit, locked so no
/// other tool prunes or removes it. Picks a name that collides with no branch and no folder.
/// Nothing half-made is left behind when git fails.
pub fn add(paths: &TidewatchPaths, repo: &Path) -> Result<Worktree, WorktreeError> {
    let taken = |name: &str| {
        paths.worktree_dir(repo, name).symlink_metadata().is_ok()
            || branch_exists(repo, &format!("{BRANCH_PREFIX}{name}"))
    };
    add_named(paths, repo, &names::pick(random_seed(), taken))
}

/// [`add`] with the name chosen by the caller.
pub fn add_named(
    paths: &TidewatchPaths,
    repo: &Path,
    name: &str,
) -> Result<Worktree, WorktreeError> {
    let worktree = Worktree {
        repo: repo.to_path_buf(),
        path: paths.worktree_dir(repo, name),
        branch: format!("{BRANCH_PREFIX}{name}"),
    };
    if holds_something(&worktree.path) {
        return Err(WorktreeError::PathTaken(worktree.path));
    }
    if !has_commits(repo) {
        return Err(WorktreeError::Git("the repository has no commits yet".into()));
    }
    let branch_existed = branch_exists(repo, &worktree.branch);
    let path_existed = worktree.path.exists();
    let mut command = git(repo);
    command
        .args(["worktree", "add", "--lock", "--reason", LOCK_REASON, "-b", &worktree.branch])
        .arg(&worktree.path);
    if let Err(error) = run(command) {
        roll_back(&worktree, branch_existed, path_existed);
        return Err(error);
    }
    if let Err(e) = registry::record(paths, &worktree) {
        eprintln!("tidewatch: cannot record worktree {}: {e}", worktree.path.display());
    }
    Ok(worktree)
}

/// git can fail late: a non-empty target leaves its new branch behind, and a failing
/// `post-checkout` hook leaves the whole worktree. Undo whatever this attempt created.
fn roll_back(worktree: &Worktree, branch_existed: bool, path_existed: bool) {
    unlock(worktree);
    let mut remove = git(&worktree.repo);
    remove.args(["worktree", "remove", "--force", "--force"]).arg(&worktree.path);
    let _ = remove.output();
    if !path_existed && worktree.path.exists() {
        let _ = std::fs::remove_dir_all(&worktree.path);
    }
    if !branch_existed {
        let mut delete = git(&worktree.repo);
        delete.args(["branch", "-D", &worktree.branch]);
        let _ = delete.output();
    }
}

fn unlock(worktree: &Worktree) {
    let mut command = git(&worktree.repo);
    command.args(["worktree", "unlock"]).arg(&worktree.path);
    let _ = command.output();
}

fn relock(worktree: &Worktree) {
    let mut command = git(&worktree.repo);
    command.args(["worktree", "lock", "--reason", LOCK_REASON]).arg(&worktree.path);
    let _ = command.output();
}

/// Counts what closing the worktree would lose, for the close dialog.
#[must_use]
pub fn inspect(worktree: &Worktree) -> Inspection {
    Inspection { changed: changed_files(&worktree.path), unmerged: unmerged_commits(worktree) }
}

fn changed_files(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    let mut command = git(path);
    command.args(["status", "--porcelain"]);
    run(command).map_or(0, |out| out.lines().filter(|l| !l.trim().is_empty()).count())
}

fn unmerged_commits(worktree: &Worktree) -> usize {
    let mut command = git(&worktree.repo);
    command
        .args(["rev-list", "--count", &worktree.branch, "--not"])
        .arg(format!("--exclude={}", worktree.branch))
        .args(["--branches", "--remotes", "--tags"]);
    run(command).ok().and_then(|out| out.trim().parse().ok()).unwrap_or(0)
}

/// Removes the worktree folder and its git registration, then keeps or deletes the branch.
/// `Safe` refuses uncommitted changes; `Discard` is the caller's explicit choice to lose them.
/// A failed removal puts the lock back so the worktree stays protected.
pub fn remove(
    paths: &TidewatchPaths,
    worktree: &Worktree,
    branch: BranchChoice,
    mode: RemoveMode,
) -> Result<BranchOutcome, WorktreeError> {
    if mode == RemoveMode::Safe {
        let changed = changed_files(&worktree.path);
        if changed > 0 {
            return Err(WorktreeError::Dirty { changed });
        }
    }
    unlock(worktree);
    let mut command = git(&worktree.repo);
    command.args(["worktree", "remove"]);
    if mode == RemoveMode::Discard || !worktree.path.exists() {
        command.arg("--force");
    }
    command.arg(&worktree.path);
    if let Err(error) = run(command) {
        relock(worktree);
        return Err(error);
    }
    let outcome = match branch {
        BranchChoice::Keep => BranchOutcome::Kept,
        BranchChoice::Delete => delete_branch(worktree),
    };
    if let Err(e) = registry::forget(paths, &worktree.path) {
        eprintln!("tidewatch: cannot update the worktree registry: {e}");
    }
    if let Some(parent) = worktree.path.parent()
        && parent != paths.root()
    {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(outcome)
}

fn delete_branch(worktree: &Worktree) -> BranchOutcome {
    let mut command = git(&worktree.repo);
    command.args(["branch", "-D", &worktree.branch]);
    match run(command) {
        Ok(_) => BranchOutcome::Deleted,
        Err(error) => BranchOutcome::DeleteFailed(error.to_string()),
    }
}

/// Every worktree git knows for the repo, the main checkout first.
pub fn list(repo: &Path) -> Result<Vec<Listed>, WorktreeError> {
    let mut command = git(repo);
    command.args(["worktree", "list", "--porcelain", "-z"]);
    run(command).map(|out| parse_list(&out))
}

fn parse_list(output: &str) -> Vec<Listed> {
    let mut listed = Vec::new();
    let mut current: Option<Listed> = None;
    for field in output.split('\0') {
        if field.is_empty() {
            listed.extend(current.take());
        } else if let Some(path) = field.strip_prefix("worktree ") {
            listed.extend(current.take());
            current = Some(Listed {
                path: PathBuf::from(path),
                branch: None,
                locked: None,
                prunable: false,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(branch) = field.strip_prefix("branch ") {
                entry.branch =
                    Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).to_owned());
            } else if field == "locked" {
                entry.locked = Some(String::new());
            } else if let Some(reason) = field.strip_prefix("locked ") {
                entry.locked = Some(reason.to_owned());
            } else if field == "prunable" || field.starts_with("prunable ") {
                entry.prunable = true;
            }
        }
    }
    listed.extend(current);
    listed
}

/// Startup pass for a repo that moved: finds stored worktrees whose recorded repo no longer
/// exists but whose folder name matches `repo`, runs `git worktree repair` for them from `repo`,
/// and keeps those git now agrees belong to `repo`. Also forgets stored worktrees whose folder
/// is gone. Returns the worktrees that were repaired.
#[must_use]
pub fn repair(paths: &TidewatchPaths, repo: &Path) -> Vec<Worktree> {
    let mut stored = registry::load(paths);
    let before = stored.len();
    stored.retain(|w| w.path.exists());
    let mut changed = stored.len() != before;
    let candidates: Vec<usize> =
        (0..stored.len()).filter(|&i| moved_from(&stored[i].repo, repo)).collect();
    let mut repaired = Vec::new();
    if !candidates.is_empty() {
        let mut command = git(repo);
        command.args(["worktree", "repair"]);
        command.args(candidates.iter().map(|&i| &stored[i].path));
        let _ = command.output();
        for i in candidates {
            if belongs_to(repo, &stored[i].path) {
                stored[i].repo = repo.to_path_buf();
                repaired.push(stored[i].clone());
                changed = true;
            }
        }
    }
    if changed && let Err(e) = registry::save(paths, &stored) {
        eprintln!("tidewatch: cannot update the worktree registry: {e}");
    }
    repaired
}

fn moved_from(recorded: &Path, current: &Path) -> bool {
    recorded != current && !recorded.exists() && recorded.file_name() == current.file_name()
}

fn common_dir(dir: &Path) -> Option<PathBuf> {
    let mut command = git(dir);
    command.args(["rev-parse", "--git-common-dir"]);
    let out = run(command).ok()?;
    dir.join(out.trim()).canonicalize().ok()
}

fn belongs_to(repo: &Path, worktree: &Path) -> bool {
    matches!((common_dir(repo), common_dir(worktree)), (Some(a), Some(b)) if a == b)
}

/// Worktree folders under the Tidewatch root that are not in `open`: made by an earlier run and
/// never closed, or left by "keep worktree". Sorted by path.
#[must_use]
pub fn leftovers(paths: &TidewatchPaths, open: &[PathBuf]) -> Vec<Leftover> {
    let stored = registry::load(paths);
    let mut found: Vec<Leftover> = subdirs(paths.root())
        .filter(|dir| !is_hidden(dir))
        .flat_map(|repo_dir| subdirs(&repo_dir).collect::<Vec<_>>())
        .filter(|dir| dir.join(".git").is_file() && !open.contains(dir))
        .map(|path| Leftover {
            branch: stored.iter().find(|w| w.path == path).map(|w| w.branch.clone()),
            path,
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn subdirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_drops_the_branch_prefix() {
        let worktree =
            Worktree { repo: "/r".into(), path: "/w".into(), branch: "agent/brave-otter".into() };
        assert_eq!(worktree.name(), "brave-otter");
    }

    #[test]
    fn git_messages_lose_their_severity_prefix() {
        let cases = [
            ("fatal: a branch named 'x' already exists\n", "a branch named 'x' already exists"),
            ("error: the branch is not fully merged", "the branch is not fully merged"),
            ("  plain text  ", "plain text"),
        ];
        for (raw, want) in cases {
            assert_eq!(clean_message(raw), want, "input {raw:?}");
        }
    }

    #[test]
    fn dirty_error_counts_files() {
        assert_eq!(
            WorktreeError::Dirty { changed: 1 }.to_string(),
            "1 file with uncommitted changes"
        );
        assert_eq!(
            WorktreeError::Dirty { changed: 3 }.to_string(),
            "3 files with uncommitted changes"
        );
    }

    #[test]
    fn parses_porcelain_list_output() {
        let out = "worktree /repo\0HEAD abc\0branch refs/heads/main\0\0\
                   worktree /wt/a\0HEAD def\0branch refs/heads/agent/a\0locked tidewatch\0\0\
                   worktree /wt/b\0HEAD 123\0detached\0locked\0prunable gitdir file points to non-existent location\0\0";
        let listed = parse_list(out);
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].path, PathBuf::from("/repo"));
        assert_eq!(listed[0].branch.as_deref(), Some("main"));
        assert_eq!(listed[0].locked, None);
        assert_eq!(listed[1].branch.as_deref(), Some("agent/a"));
        assert_eq!(listed[1].locked.as_deref(), Some("tidewatch"));
        assert_eq!(listed[2].branch, None);
        assert_eq!(listed[2].locked.as_deref(), Some(""));
        assert!(listed[2].prunable);
    }

    #[test]
    fn repair_forgets_stored_worktrees_whose_folder_is_gone() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let gone = Worktree {
            repo: "/work/web".into(),
            path: home.path().join("missing"),
            branch: "agent/gone".into(),
        };
        registry::record(&paths, &gone).unwrap();
        assert!(repair(&paths, Path::new("/work/web")).is_empty());
        assert!(registry::load(&paths).is_empty());
    }

    #[test]
    fn parses_empty_list_output() {
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn moved_repo_needs_a_missing_old_path_and_the_same_folder_name() {
        let here = std::env::temp_dir();
        let cases = [
            ("/gone/web", "/new/web", true),
            ("/gone/web", "/new/api", false),
            ("/gone/web", "/gone/web", false),
        ];
        for (recorded, current, want) in cases {
            assert_eq!(moved_from(Path::new(recorded), Path::new(current)), want, "{recorded}");
        }
        assert!(!moved_from(&here, Path::new("/new/tmp")), "an existing old repo did not move");
    }
}
