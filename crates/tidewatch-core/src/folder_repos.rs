//! What a tab's panel shows when the tab runs in a folder that is not a git repo of the window: the
//! repos inside it (one level down, linked worktrees included), which of them the agent touched
//! last, and, with none, the files changed in the last hour. Disk reads only; the caller runs
//! `scan` off the UI thread.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::transcript::{Entry, Role, ToolPhase};

pub const MAX_REPOS: usize = 50;
pub const MAX_RECENT: usize = 200;
pub const RECENT_WINDOW: Duration = Duration::from_hours(1);
const MAX_VISITED: usize = 20_000;
const MAX_DEPTH: usize = 8;
const MAX_EDITS: usize = 64;
const SKIPPED: [&str; 2] = ["node_modules", "target"];
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "write_file"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRepo {
    pub root: PathBuf,
    pub name: String,
    pub branch: Option<String>,
    pub touched_ms: i64,
    pub linked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentFile {
    pub path: String,
    pub modified_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub folder: PathBuf,
    pub inside: Option<PathBuf>,
    pub repos: Vec<FolderRepo>,
    pub recent: Vec<RecentFile>,
}

/// What the panel draws for a folder tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    /// The folder is inside this repo: its panel works as a repo tab's does.
    Inside(&'a Path),
    Repo(&'a FolderRepo),
    /// No repo in the folder: the plain folder view.
    Folder,
}

fn is_hidden_or_build(name: &str) -> bool {
    name.starts_with('.') || SKIPPED.contains(&name)
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    let since = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// The folder a `.git` file points at, relative paths read from the repo's folder.
#[must_use]
pub fn parse_gitdir(text: &str, repo: &Path) -> Option<PathBuf> {
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    Some(repo.join(target))
}

/// The branch a `HEAD` file names; a detached head is its short id.
#[must_use]
pub fn parse_head(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    if let Some(name) = line.strip_prefix("ref:") {
        let name = name.trim();
        return Some(name.strip_prefix("refs/heads/").unwrap_or(name).to_owned());
    }
    (line.len() >= 7 && line.chars().all(|c| c.is_ascii_hexdigit())).then(|| line[..7].to_owned())
}

fn git_dir(repo: &Path) -> Option<(PathBuf, bool)> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Some((dot_git, false));
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    Some((parse_gitdir(&text, repo)?, true))
}

fn top_level_mtimes(root: &Path) -> Vec<i64> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter(|entry| !is_hidden_or_build(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| mtime_ms(&entry.path()))
        .collect()
}

fn read_repo(root: &Path) -> Option<FolderRepo> {
    let (git, linked) = git_dir(root)?;
    let branch = std::fs::read_to_string(git.join("HEAD")).ok().and_then(|text| parse_head(&text));
    let touched_ms = ["index", "HEAD", "logs/HEAD"]
        .iter()
        .filter_map(|name| mtime_ms(&git.join(name)))
        .chain(top_level_mtimes(root))
        .max()
        .unwrap_or_default();
    let name = root.file_name()?.to_string_lossy().into_owned();
    Some(FolderRepo { root: root.to_path_buf(), name, branch, touched_ms, linked })
}

/// The repo root that holds `dir`, found by walking up to a `.git` folder or file.
#[must_use]
pub fn enclosing_repo(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|folder| folder.join(".git").exists()).map(Path::to_path_buf)
}

/// The repos directly inside `folder`: hidden folders, `node_modules` and `target` are skipped,
/// the list is by name and holds at most [`MAX_REPOS`].
#[must_use]
pub fn discover(folder: &Path) -> Vec<FolderRepo> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut repos: Vec<FolderRepo> = entries
        .filter_map(Result::ok)
        .filter(|entry| !is_hidden_or_build(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(".git").exists())
        .filter_map(|path| read_repo(&path))
        .collect();
    repos.sort_by_cached_key(|repo| repo.name.to_lowercase());
    repos.truncate(MAX_REPOS);
    repos
}

/// Files under `folder` changed within [`RECENT_WINDOW`] of `now_ms`, newest first.
#[must_use]
pub fn recent_files(folder: &Path, now_ms: i64) -> Vec<RecentFile> {
    let cutoff = now_ms - i64::try_from(RECENT_WINDOW.as_millis()).unwrap_or(i64::MAX);
    let mut found = Vec::new();
    let mut visited = 0;
    let mut pending = vec![(folder.to_path_buf(), 0)];
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            visited += 1;
            if visited > MAX_VISITED {
                return newest(found);
            }
            if is_hidden_or_build(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
            } else if kind.is_file()
                && let Some(modified_ms) = mtime_ms(&path)
                && modified_ms >= cutoff
                && let Ok(relative) = path.strip_prefix(folder)
            {
                found.push(RecentFile {
                    path: relative.to_string_lossy().into_owned(),
                    modified_ms,
                });
            }
        }
    }
    newest(found)
}

fn newest(mut files: Vec<RecentFile>) -> Vec<RecentFile> {
    files.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then_with(|| a.path.cmp(&b.path)));
    files.truncate(MAX_RECENT);
    files
}

#[must_use]
pub fn scan(folder: &Path, now_ms: i64) -> Scan {
    let folder = folder.to_path_buf();
    if let Some(root) = enclosing_repo(&folder) {
        return Scan { folder, inside: Some(root), repos: Vec::new(), recent: Vec::new() };
    }
    let repos = discover(&folder);
    let recent = if repos.is_empty() { recent_files(&folder, now_ms) } else { Vec::new() };
    Scan { folder, inside: None, repos, recent }
}

/// The repo holding `file`: the deepest root the path sits under.
#[must_use]
pub fn repo_of<'a>(repos: &'a [FolderRepo], file: &Path) -> Option<&'a FolderRepo> {
    repos
        .iter()
        .filter(|repo| file.starts_with(&repo.root))
        .max_by_key(|repo| repo.root.components().count())
}

/// The repo to show first: the one holding the agent's newest edit (`edits` run oldest to newest),
/// else the one whose index or head changed last, else the first by name.
#[must_use]
pub fn default_pick<'a>(repos: &'a [FolderRepo], edits: &[PathBuf]) -> Option<&'a FolderRepo> {
    edits
        .iter()
        .rev()
        .find_map(|file| repo_of(repos, file))
        .or_else(|| repos.iter().rev().max_by_key(|repo| repo.touched_ms))
}

/// The repo the panel shows: the owner's own pick while it is still in the folder, else the
/// default.
#[must_use]
pub fn pick<'a>(
    repos: &'a [FolderRepo],
    hand: Option<&Path>,
    edits: &[PathBuf],
) -> Option<&'a FolderRepo> {
    hand.and_then(|root| repos.iter().find(|repo| repo.root == root))
        .or_else(|| default_pick(repos, edits))
}

#[must_use]
pub fn target<'a>(scan: &'a Scan, hand: Option<&Path>, edits: &[PathBuf]) -> Target<'a> {
    if let Some(root) = &scan.inside {
        return Target::Inside(root);
    }
    pick(&scan.repos, hand, edits).map_or(Target::Folder, Target::Repo)
}

/// The files an agent wrote or edited in `entries`, in order, as the absolute paths its calls named.
#[must_use]
pub fn edited_paths(entries: &[Entry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| entry.role == Role::Tool)
        .filter_map(|entry| entry.tool.as_ref())
        .filter(|tool| tool.phase == ToolPhase::Call && EDIT_TOOLS.contains(&tool.name.as_str()))
        .map(|tool| PathBuf::from(&tool.summary))
        .filter(|path| path.is_absolute() && !path.to_string_lossy().ends_with("..."))
        .collect()
}

pub fn remember_edits(kept: &mut Vec<PathBuf>, found: Vec<PathBuf>) {
    for path in found {
        kept.retain(|old| *old != path);
        kept.push(path);
    }
    let extra = kept.len().saturating_sub(MAX_EDITS);
    kept.drain(..extra);
}

/// `path` with the home folder written as `~`.
#[must_use]
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests;
