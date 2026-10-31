//! Create, list, repair and remove the git worktrees Tidewatch tabs run in.
//!
//! Every function shells out to `git` and blocks until it finishes: call them from a background
//! thread, never the UI thread. Behavior was checked against git 2.55.0 (see `PLAN.md`, Phase 0
//! findings).

use std::collections::HashSet;
use std::fmt;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::closed_tabs::{self, Closed};
use crate::limited;
use crate::names;
use crate::paths::TidewatchPaths;
use crate::registry;
use crate::removal_log;
use crate::worktree_safety::{Facts, Owner, Safety, Watch, assess, with_machine};

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
    /// Removal was not proven safe, and the caller did not pass an explicit discard.
    Unsafe(Safety),
    /// The target folder already holds something.
    PathTaken(PathBuf),
    /// git could not be started, or the disk refused.
    Io(String),
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(message) | Self::Io(message) => f.write_str(message),
            Self::Unsafe(safety) => write!(f, "not removed: {}", safety.reasons().join(", ")),
            Self::PathTaken(path) => write!(f, "{} already exists", path.display()),
        }
    }
}

impl std::error::Error for WorktreeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchChoice {
    Keep,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveMode {
    /// Remove only what is proven safe to lose nothing.
    Safe,
    /// The caller asked explicitly, with the loss in front of them: whatever the check found is
    /// thrown away.
    Discard,
}

/// Why a worktree is being removed, for the removal log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    ClosedTab,
    MergedTab,
    Poofed,
    Leftover,
    Rollback,
    Other,
}

impl Why {
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::ClosedTab => "closed tab",
            Self::MergedTab => "merged pull request",
            Self::Poofed => "meeseeks poofed",
            Self::Leftover => "leftover worktree",
            Self::Rollback => "failed move rolled back",
            Self::Other => "removed",
        }
    }
}

/// How a removal is asked for. The defaults are the strictest: nobody owns the worktree and no
/// commits are known to be delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms {
    pub branch: BranchChoice,
    pub mode: RemoveMode,
    pub why: Why,
    pub owner: Owner,
    /// A commit, such as a merged pull request's head, that holds what the branch had: commits it
    /// reaches are delivered even when no branch or remote has them.
    pub delivered: Option<String>,
    /// The agent tab being closed: once the removal succeeds it is noted in `closed-tabs.jsonl`.
    pub closed: Option<closed_tabs::Tab>,
    /// With [`BranchChoice::Delete`]: keep the branch when it holds commits found nowhere else.
    pub spare_unique: bool,
}

impl Terms {
    #[must_use]
    pub fn new(branch: BranchChoice, mode: RemoveMode) -> Self {
        Self {
            branch,
            mode,
            why: Why::Other,
            owner: Owner::Nobody,
            delivered: None,
            closed: None,
            spare_unique: false,
        }
    }

    #[must_use]
    pub fn closed(self, closed: Option<closed_tabs::Tab>) -> Self {
        Self { closed, ..self }
    }

    #[must_use]
    pub fn why(self, why: Why) -> Self {
        Self { why, ..self }
    }

    #[must_use]
    pub fn owner(self, owner: Owner) -> Self {
        Self { owner, ..self }
    }

    #[must_use]
    pub fn spare_unique(self, spare_unique: bool) -> Self {
        Self { spare_unique, ..self }
    }

    #[must_use]
    pub fn delivered(self, delivered: Option<String>) -> Self {
        Self { delivered, ..self }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchOutcome {
    Kept,
    /// The branch is gone; `restore` is the command that brings it back.
    Deleted {
        restore: Option<String>,
    },
    /// The worktree is gone but git would not delete the branch.
    DeleteFailed(String),
    /// The branch stays because it holds commits that exist nowhere else.
    KeptCommits {
        commits: usize,
    },
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
    /// The branch it has checked out. `None` for a detached HEAD, or when git cannot read it.
    pub branch: Option<String>,
    /// The repository it belongs to. `None` only when git cannot read the folder.
    pub repo: Option<PathBuf>,
    /// Whether the registry, or the `agent/<folder name>` rule, says Tidewatch made it.
    pub recorded: bool,
}

impl Leftover {
    /// The worktree to hand to [`inspect`] or [`remove`]. `None` when git cannot name the
    /// repository, because git needs it to remove anything. A detached HEAD has an empty branch.
    #[must_use]
    pub fn worktree(&self) -> Option<Worktree> {
        Some(Worktree {
            repo: self.repo.clone()?,
            path: self.path.clone(),
            branch: self.branch.clone().unwrap_or_default(),
        })
    }
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// `git worktree add` and `remove` write a whole checkout and get [`limited::WORKTREE`]; every
/// other call gets [`limited::DEFAULT`].
fn limit_of(command: &Command) -> Duration {
    let args: Vec<&std::ffi::OsStr> = command.get_args().collect();
    let heavy = args
        .windows(2)
        .any(|pair| pair[0] == "worktree" && (pair[1] == "add" || pair[1] == "remove"));
    if heavy { limited::WORKTREE } else { limited::DEFAULT }
}

fn output_of(command: &mut Command) -> std::io::Result<std::process::Output> {
    limited::output(command, limit_of(command))
}

fn run(mut command: Command) -> Result<String, WorktreeError> {
    let out =
        output_of(&mut command).map_err(|e| WorktreeError::Io(limited::failure("git", &e)))?;
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
    output_of(&mut command).is_ok_and(|out| out.status.success())
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

#[must_use]
pub fn local_branches(repo: &Path) -> Vec<String> {
    let mut command = git(repo);
    command.args(["for-each-ref", "--format=%(refname:short)", "refs/heads"]);
    run(command).map(|out| out.lines().map(str::to_owned).collect()).unwrap_or_default()
}

#[must_use]
pub fn taken_names(paths: &TidewatchPaths, repo: &Path) -> HashSet<String> {
    let branches = local_branches(repo);
    let on_branches =
        branches.iter().filter_map(|branch| branch.strip_prefix(BRANCH_PREFIX)).map(str::to_owned);
    let folders = std::fs::read_dir(paths.repo_dir(repo))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok());
    on_branches.chain(folders).collect()
}

/// Adds a worktree on a new `agent/<name>` branch off the repo's current commit, locked so no
/// other tool prunes or removes it. Picks a name that collides with no branch and no folder.
/// Nothing half-made is left behind when git fails.
pub fn add(paths: &TidewatchPaths, repo: &Path) -> Result<Worktree, WorktreeError> {
    let _one_at_a_time = serial();
    let taken = |name: &str| is_taken(paths, repo, name);
    create(paths, repo, &names::pick(random_seed(), taken), None)
}

/// [`add_named`] with `wanted`, or `wanted-2`, `wanted-3`... when a branch or folder took it since
/// the name was checked.
pub fn add_free(
    paths: &TidewatchPaths,
    repo: &Path,
    wanted: &str,
    base: Option<&str>,
) -> Result<Worktree, WorktreeError> {
    let _one_at_a_time = serial();
    let taken = |name: &str| is_taken(paths, repo, name);
    let name = if taken(wanted) {
        crate::branch_name::next_free(wanted, taken)
    } else {
        wanted.to_owned()
    };
    create(paths, repo, &name, base)
}

/// Two `git worktree add` runs at once can fail with "failed to read .git/worktrees/<other>/commondir"
/// while git scans the other's half-made entry, so this process makes one worktree at a time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static ADDING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ADDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn is_taken(paths: &TidewatchPaths, repo: &Path, name: &str) -> bool {
    paths.worktree_dir(repo, name).symlink_metadata().is_ok()
        || branch_exists(repo, &format!("{BRANCH_PREFIX}{name}"))
}

/// [`add`] with the name chosen by the caller; `base` is the branch or commit the new branch starts
/// from, the current one when `None`.
pub fn add_named(
    paths: &TidewatchPaths,
    repo: &Path,
    name: &str,
    base: Option<&str>,
) -> Result<Worktree, WorktreeError> {
    let _one_at_a_time = serial();
    create(paths, repo, name, base)
}

fn create(
    paths: &TidewatchPaths,
    repo: &Path,
    name: &str,
    base: Option<&str>,
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
        .args([
            "worktree",
            "add",
            "--lock",
            "--no-track",
            "--reason",
            LOCK_REASON,
            "-b",
            &worktree.branch,
        ])
        .arg(&worktree.path);
    command.args(base);
    if let Err(error) = run(command) {
        roll_back(&worktree, branch_existed, path_existed);
        return Err(error);
    }
    if let Err(e) = registry::record(paths, &worktree) {
        roll_back(&worktree, branch_existed, path_existed);
        return Err(WorktreeError::Io(format!(
            "cannot record the new worktree in the registry: {e}"
        )));
    }
    Ok(worktree)
}

/// git can fail late: a non-empty target leaves its new branch behind, and a failing
/// `post-checkout` hook leaves the whole worktree. Undo whatever this attempt created.
fn roll_back(worktree: &Worktree, branch_existed: bool, path_existed: bool) {
    unlock(worktree);
    let mut remove = git(&worktree.repo);
    remove.args(["worktree", "remove", "--force", "--force"]).arg(&worktree.path);
    let _ = output_of(&mut remove);
    if !path_existed && worktree.path.exists() {
        let _ = std::fs::remove_dir_all(&worktree.path);
    }
    if !branch_existed {
        let mut delete = git(&worktree.repo);
        delete.args(["branch", "-D", &worktree.branch]);
        let _ = output_of(&mut delete);
    }
}

fn unlock(worktree: &Worktree) {
    let mut command = git(&worktree.repo);
    command.args(["worktree", "unlock"]).arg(&worktree.path);
    let _ = output_of(&mut command);
}

fn relock(worktree: &Worktree) {
    let mut command = git(&worktree.repo);
    command.args(["worktree", "lock", "--reason", LOCK_REASON]).arg(&worktree.path);
    let _ = output_of(&mut command);
}

/// Whether the worktree can go without losing work. Anything git or the machine cannot answer
/// is `Unknown`, never zero.
#[must_use]
pub fn inspect(paths: &TidewatchPaths, worktree: &Worktree, owner: Owner) -> Safety {
    with_machine(paths, owner, |watch| inspect_watched(worktree, None, watch))
}

#[must_use]
pub fn inspect_watched(worktree: &Worktree, delivered: Option<&str>, watch: &dyn Watch) -> Safety {
    let path = &worktree.path;
    let there = path.exists();
    assess(Facts {
        changed: changed_files(worktree),
        unmerged: unmerged_commits(worktree, delivered),
        holders: if there { watch.holders(path) } else { Ok(Vec::new()) },
        session_age: if there { watch.session_age(path) } else { None },
    })
}

fn confirm_worktree(worktree: &Worktree) -> Result<(), String> {
    if !worktree.path.exists() {
        return Ok(());
    }
    if !worktree.path.join(".git").is_file() {
        return Err("the folder is not a git worktree".to_owned());
    }
    if belongs_to(&worktree.repo, &worktree.path) {
        Ok(())
    } else {
        Err("the folder no longer belongs to its repository".to_owned())
    }
}

fn changed_files(worktree: &Worktree) -> Result<usize, String> {
    confirm_worktree(worktree)?;
    if !worktree.path.exists() {
        return Ok(0);
    }
    let mut command = git(&worktree.path);
    command.args(["status", "--porcelain"]);
    let out = run(command).map_err(|e| format!("git status failed: {e}"))?;
    Ok(out.lines().filter(|l| !l.trim().is_empty()).count())
}

fn count_of(command: Command, what: &str) -> Result<usize, String> {
    let out = run(command).map_err(|e| format!("could not count {what}: {e}"))?;
    out.trim().parse().map_err(|_| format!("could not count {what}: unexpected output"))
}

fn object_exists(repo: &Path, id: &str) -> bool {
    let mut command = git(repo);
    command.args(["cat-file", "-e"]).arg(format!("{id}^{{commit}}"));
    succeeds(command)
}

fn delivered_in(worktree: &Worktree, delivered: Option<&str>) -> Option<String> {
    delivered.filter(|id| object_exists(&worktree.repo, id)).map(str::to_owned)
}

fn branch_only_commits(worktree: &Worktree, delivered: Option<&str>) -> Result<usize, String> {
    if worktree.branch.is_empty() {
        return Ok(0);
    }
    let mut on_branch = git(&worktree.repo);
    on_branch
        .args(["rev-list", "--count", &worktree.branch, "--not"])
        .arg(format!("--exclude={}", worktree.branch))
        .args(["--branches", "--remotes", "--tags"])
        .args(delivered_in(worktree, delivered));
    count_of(on_branch, "the commits on its branch")
}

/// Commits that live on the branch, or on the worktree's own HEAD, and nowhere else.
fn unmerged_commits(worktree: &Worktree, delivered: Option<&str>) -> Result<usize, String> {
    let mut total = branch_only_commits(worktree, delivered)?;
    if worktree.path.exists() && confirm_worktree(worktree).is_ok() {
        let mut on_head = git(&worktree.path);
        on_head
            .args(["rev-list", "--count", "HEAD", "--not", "--branches", "--remotes", "--tags"])
            .args(delivered_in(worktree, delivered));
        total += count_of(on_head, "the commits on its HEAD")?;
    }
    Ok(total)
}

fn rev_of(dir: &Path, rev: &str) -> Option<String> {
    let mut command = git(dir);
    command.args(["rev-parse", "--verify", "--quiet", rev]);
    run(command).ok().map(|out| out.trim().to_owned()).filter(|id| !id.is_empty())
}

fn allowed(safety: &Safety, worktree: &Worktree, branch: BranchChoice) -> bool {
    if worktree.branch.is_empty() {
        return safety.is_safe();
    }
    match branch {
        BranchChoice::Keep => safety.folder_is_safe(),
        BranchChoice::Delete => safety.is_safe(),
    }
}

/// What the branch choice comes to once the branch has been looked at: a detached HEAD has no
/// branch to delete, and a branch with commits found nowhere else is kept. The count is carried
/// when the branch is kept for that reason.
fn settle_branch(worktree: &Worktree, terms: &Terms) -> (BranchChoice, Option<usize>) {
    if worktree.branch.is_empty() {
        return (BranchChoice::Keep, None);
    }
    if terms.branch == BranchChoice::Delete && terms.spare_unique {
        return match branch_only_commits(worktree, terms.delivered.as_deref()) {
            Ok(0) => (BranchChoice::Delete, None),
            Ok(commits) => (BranchChoice::Keep, Some(commits)),
            Err(_) => (BranchChoice::Keep, None),
        };
    }
    (terms.branch, None)
}

/// [`remove_with`] on the strictest terms: nobody owns the worktree.
pub fn remove(
    paths: &TidewatchPaths,
    worktree: &Worktree,
    branch: BranchChoice,
    mode: RemoveMode,
) -> Result<BranchOutcome, WorktreeError> {
    remove_with(paths, worktree, &Terms::new(branch, mode))
}

pub fn remove_with(
    paths: &TidewatchPaths,
    worktree: &Worktree,
    terms: &Terms,
) -> Result<BranchOutcome, WorktreeError> {
    with_machine(paths, terms.owner, |watch| remove_watched(paths, worktree, terms, watch))
}

/// Removes the worktree folder and its git registration, then keeps or deletes the branch.
/// `Safe` goes ahead only when the check proves nothing is lost: a clean tree, every commit also
/// somewhere else, no process in the folder, no recent agent session. A branch that is deleted goes
/// with `git branch -d`, except after a discard or for commits `terms.delivered` holds. The branch
/// and its commit are written to the removal log first; a failed removal puts the lock back.
pub fn remove_watched(
    paths: &TidewatchPaths,
    worktree: &Worktree,
    terms: &Terms,
    watch: &dyn Watch,
) -> Result<BranchOutcome, WorktreeError> {
    let safety = inspect_watched(worktree, terms.delivered.as_deref(), watch);
    if terms.mode == RemoveMode::Safe && !allowed(&safety, worktree, terms.branch) {
        return Err(WorktreeError::Unsafe(safety));
    }
    let (branch, only_here) = settle_branch(worktree, terms);
    let terms = &Terms { branch, ..terms.clone() };
    let entry = removal_entry(worktree, terms, &safety);
    removal_log::append(paths, &entry)
        .map_err(|e| WorktreeError::Io(format!("cannot write the removal log: {e}")))?;
    unlock(worktree);
    let mut command = git(&worktree.repo);
    command.args(["worktree", "remove"]);
    if terms.mode == RemoveMode::Discard || !worktree.path.exists() {
        command.arg("--force");
    }
    command.arg(&worktree.path);
    if let Err(error) = run(command) {
        relock(worktree);
        return Err(error);
    }
    let outcome = match terms.branch {
        BranchChoice::Keep => match only_here {
            Some(commits) => BranchOutcome::KeptCommits { commits },
            None => BranchOutcome::Kept,
        },
        BranchChoice::Delete => {
            let force = terms.mode == RemoveMode::Discard || terms.delivered.is_some();
            delete_branch(worktree, force, entry.restore_command())
        }
    };
    if let Err(e) = registry::forget(paths, &worktree.path) {
        eprintln!("tidewatch: cannot update the worktree registry: {e}");
    }
    if let Some(tab) = &terms.closed
        && let Err(e) = closed_tabs::append(paths, &Closed::removed(tab, &entry))
    {
        eprintln!("tidewatch: cannot note the closed tab: {e}");
    }
    if let Some(parent) = worktree.path.parent()
        && parent != paths.root()
    {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(outcome)
}

/// Whether `path` is a folder git cannot read: a `.git` link that points nowhere, a repository
/// that is gone. Nothing in it can be inspected, so only [`discard_unreadable`] can remove it.
#[must_use]
pub fn is_unreadable(path: &Path) -> bool {
    path.is_dir() && git_dirs(path).is_none()
}

/// Logged, then trashed, never deleted; refused once git can read the folder, since [`inspect`]
/// would then list the loss.
pub fn discard_unreadable(
    paths: &TidewatchPaths,
    path: &Path,
    known: Option<&Worktree>,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), WorktreeError> {
    if !path.is_dir() {
        return Err(WorktreeError::Io(format!("{} is not a folder", path.display())));
    }
    if git_dirs(path).is_some() {
        return Err(WorktreeError::Git("git can read this folder now".to_owned()));
    }
    let lost = vec![
        "a folder git could not read, so what it held is unknown; moved to the Trash".to_owned(),
    ];
    let entry = match known {
        Some(worktree) => {
            let terms = Terms::new(BranchChoice::Keep, RemoveMode::Discard).why(Why::Leftover);
            removal_log::Entry { lost, ..removal_entry(worktree, &terms, &Safety::Safe) }
        }
        None => removal_log::Entry {
            at: removal_log::Entry::now(),
            why: Why::Leftover.text().to_owned(),
            path: path.to_path_buf(),
            repo: PathBuf::new(),
            branch: String::new(),
            sha: None,
            head: None,
            mode: "discard".to_owned(),
            branch_kept: true,
            lost,
        },
    };
    removal_log::append(paths, &entry)
        .map_err(|e| WorktreeError::Io(format!("cannot write the removal log: {e}")))?;
    trash(path).map_err(WorktreeError::Io)?;
    if let Some(worktree) = known.filter(|worktree| worktree.repo.is_dir()) {
        unlock(worktree);
        let mut prune = git(&worktree.repo);
        prune.args(["worktree", "prune"]);
        let _ = output_of(&mut prune);
    }
    if let Err(e) = registry::forget(paths, path) {
        eprintln!("tidewatch: cannot update the worktree registry: {e}");
    }
    Ok(())
}

/// Notes a tab closed while its worktree and branch stay, for the Recently closed list.
pub fn note_kept(
    paths: &TidewatchPaths,
    worktree: &Worktree,
    tab: &closed_tabs::Tab,
) -> std::io::Result<()> {
    let sha = rev_of(&worktree.repo, &format!("refs/heads/{}", worktree.branch));
    let head = worktree.path.exists().then(|| rev_of(&worktree.path, "HEAD")).flatten();
    closed_tabs::append(paths, &Closed::kept(tab, worktree, sha, head))
}

fn removal_entry(worktree: &Worktree, terms: &Terms, safety: &Safety) -> removal_log::Entry {
    let loses_branch = terms.branch == BranchChoice::Delete;
    let mut lost: Vec<String> = match safety {
        Safety::Unsafe(risks) => risks
            .iter()
            .filter(|risk| risk.is_in_folder() || loses_branch)
            .map(crate::worktree_safety::Risk::text)
            .collect(),
        other => other.reasons(),
    };
    if terms.mode == RemoveMode::Safe {
        lost.clear();
    }
    removal_log::Entry {
        at: removal_log::Entry::now(),
        why: terms.why.text().to_owned(),
        path: worktree.path.clone(),
        repo: worktree.repo.clone(),
        branch: worktree.branch.clone(),
        sha: rev_of(&worktree.repo, &format!("refs/heads/{}", worktree.branch)),
        head: worktree.path.exists().then(|| rev_of(&worktree.path, "HEAD")).flatten(),
        mode: match terms.mode {
            RemoveMode::Safe => "safe",
            RemoveMode::Discard => "discard",
        }
        .to_owned(),
        branch_kept: terms.branch == BranchChoice::Keep,
        lost,
    }
}

fn delete_branch(worktree: &Worktree, force: bool, restore: Option<String>) -> BranchOutcome {
    let mut command = git(&worktree.repo);
    command.args(["branch", if force { "-D" } else { "-d" }, &worktree.branch]);
    match run(command) {
        Ok(_) => BranchOutcome::Deleted { restore },
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
    let mut repaired = Vec::new();
    if let Err(e) = registry::update(paths, |stored| repaired = repair_stored(stored, repo)) {
        eprintln!("tidewatch: cannot update the worktree registry: {e}");
    }
    repaired
}

fn repair_stored(stored: &mut Vec<Worktree>, repo: &Path) -> Vec<Worktree> {
    stored.retain(|w| w.path.exists());
    let candidates: Vec<usize> =
        (0..stored.len()).filter(|&i| moved_from(&stored[i].repo, repo)).collect();
    let mut repaired = Vec::new();
    if !candidates.is_empty() {
        let mut command = git(repo);
        command.args(["worktree", "repair"]);
        command.args(candidates.iter().map(|&i| &stored[i].path));
        let _ = output_of(&mut command);
        for i in candidates {
            if belongs_to(repo, &stored[i].path) {
                stored[i].repo = repo.to_path_buf();
                repaired.push(stored[i].clone());
            }
        }
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
/// never closed, or left by "keep worktree", or made some other way. A folder that is the main
/// checkout of a repository is never listed. Sorted by path.
#[must_use]
pub fn leftovers(paths: &TidewatchPaths, open: &[PathBuf]) -> Vec<Leftover> {
    let stored = registry::load(paths);
    let mut found: Vec<Leftover> = subdirs(paths.root())
        .filter(|dir| !is_hidden(dir))
        .flat_map(|repo_dir| subdirs(&repo_dir).collect::<Vec<_>>())
        .filter(|dir| dir.join(".git").is_file() && !open.contains(dir))
        .filter(|dir| !is_main_checkout(dir))
        .map(|path| {
            let known = stored
                .iter()
                .find(|w| w.path == path)
                .map(|w| (w.repo.clone(), w.branch.clone()))
                .or_else(|| claim(paths, &path));
            match known {
                Some((repo, branch)) => {
                    Leftover { path, repo: Some(repo), branch: Some(branch), recorded: true }
                }
                None => Leftover {
                    repo: main_checkout(&path),
                    branch: checked_out_branch(&path),
                    path,
                    recorded: false,
                },
            }
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn git_dirs(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut command = git(dir);
    command.args(["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"]);
    let out = run(command).ok()?;
    let mut lines = out.lines();
    let own = PathBuf::from(lines.next()?).canonicalize().ok()?;
    let common = PathBuf::from(lines.next()?).canonicalize().ok()?;
    Some((own, common))
}

/// Whether git says the folder is a repository's own checkout rather than a linked worktree.
fn is_main_checkout(dir: &Path) -> bool {
    matches!(git_dirs(dir), Some((own, common)) if own == common)
}

/// Whether Tidewatch made this worktree, from where it sits and what it has checked out: directly
/// in the repo's folder under the Tidewatch root, on the `agent/<folder name>` branch `add` gives
/// it. The registry is a convenience for `repair`; this is the ownership rule.
#[must_use]
pub fn is_made_by_tidewatch(
    paths: &TidewatchPaths,
    repo: &Path,
    path: &Path,
    branch: Option<&str>,
) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.parent() == Some(paths.repo_dir(repo).as_path())
        && branch.and_then(|branch| branch.strip_prefix(BRANCH_PREFIX)) == Some(name)
}

/// The repository and branch of a worktree the registry does not list but that is Tidewatch's by
/// [`is_made_by_tidewatch`]. Anything else, or anything git cannot read, is `None`.
fn claim(paths: &TidewatchPaths, path: &Path) -> Option<(PathBuf, String)> {
    let branch = checked_out_branch(path)?;
    let repo = main_checkout(path)?;
    is_made_by_tidewatch(paths, &repo, path, Some(&branch)).then_some((repo, branch))
}

fn checked_out_branch(dir: &Path) -> Option<String> {
    let mut command = git(dir);
    command.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
    run(command).ok().map(|out| out.trim().to_owned()).filter(|branch| !branch.is_empty())
}

/// The repository a linked worktree belongs to: its main checkout, or the bare repository.
fn main_checkout(dir: &Path) -> Option<PathBuf> {
    let mut command = git(dir);
    command.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let common = PathBuf::from(run(command).ok()?.trim());
    if common.file_name()? == ".git" {
        common.parent().map(Path::to_path_buf)
    } else {
        Some(common)
    }
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

mod revive;
pub use revive::{ReviveError, Revived, revive};

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
    fn an_unsafe_error_lists_every_reason() {
        let error = WorktreeError::Unsafe(Safety::Unsafe(vec![
            crate::worktree_safety::Risk::Uncommitted { files: 1 },
            crate::worktree_safety::Risk::Unpushed { commits: 3 },
        ]));
        assert_eq!(
            error.to_string(),
            "not removed: 1 file with uncommitted changes, 3 commits found nowhere else (on no remote branch, not merged)"
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
        assert_eq!(repair(&paths, Path::new("/work/web")), []);
        assert_eq!(registry::load(&paths), []);
    }

    #[test]
    fn ownership_needs_the_repo_folder_and_the_agent_branch_of_the_same_name() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let repo = Path::new("/work/web");
        let ours = paths.worktree_dir(repo, "upbeat-lantern");
        let cases: [(&str, PathBuf, Option<&str>, bool); 9] = [
            (
                "inside the root on its agent branch",
                ours.clone(),
                Some("agent/upbeat-lantern"),
                true,
            ),
            (
                "another repo's folder",
                paths.worktree_dir(Path::new("/work/api"), "upbeat-lantern"),
                Some("agent/upbeat-lantern"),
                false,
            ),
            (
                "outside the root",
                PathBuf::from("/work/upbeat-lantern"),
                Some("agent/upbeat-lantern"),
                false,
            ),
            ("below the repo folder", ours.join("nested"), Some("agent/nested"), false),
            ("another branch", ours.clone(), Some("main"), false),
            ("a branch with no agent prefix", ours.clone(), Some("upbeat-lantern"), false),
            ("a name that is not the folder's", ours.clone(), Some("agent/calm-comet"), false),
            ("a prefix and nothing after it", ours.clone(), Some("agent/"), false),
            ("no branch, a detached head", ours, None, false),
        ];
        for (what, path, branch, want) in cases {
            assert_eq!(is_made_by_tidewatch(&paths, repo, &path, branch), want, "{what}");
        }
    }

    #[test]
    fn parses_empty_list_output() {
        assert_eq!(parse_list(""), []);
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

    #[test]
    fn only_worktree_add_and_remove_get_the_long_limit() {
        let cases: [(&[&str], Duration); 8] = [
            (&["worktree", "add", "-b", "agent/x", "/w"], limited::WORKTREE),
            (&["worktree", "remove", "--force", "/w"], limited::WORKTREE),
            (&["-C", "/repo", "worktree", "add", "/w"], limited::WORKTREE),
            (&["worktree", "list", "--porcelain"], limited::DEFAULT),
            (&["worktree", "prune"], limited::DEFAULT),
            (&["worktree", "unlock", "/w"], limited::DEFAULT),
            (&["branch", "-D", "add"], limited::DEFAULT),
            (&["status", "--porcelain"], limited::DEFAULT),
        ];
        for (args, want) in cases {
            let mut command = Command::new("git");
            command.args(args);
            assert_eq!(limit_of(&command), want, "{args:?}");
        }
    }
}
