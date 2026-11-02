//! The `add_worktree` tool: an agent that needs a checkout of another repository gets a worktree
//! in its tab's state folder, never next to that repository, and closing the tab removes it when
//! nothing in it would be lost. The state folder is hidden, so these never show as leftovers.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};

use crate::branch_name;
use crate::limited;
use crate::paths::home_dir;

pub const TOOL: &str = "add_worktree";
const DIR: &str = "worktrees";
pub const HOUSE_BLOCK: &str = "## Other repositories\n\nWhen the task needs a checkout of another repository, call `add_worktree` and work in the folder it returns. Never run `git worktree add` yourself: it leaves folders next to the user's repositories. Tidewatch removes these when the tab closes, unless they hold work that is on no remote.\n";

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": TOOL,
        "description": "Make a git worktree of another repository for this task, instead of running `git worktree add` yourself. Tidewatch puts it in this tab's own folder and removes it when the tab closes, unless it holds uncommitted files or commits that are on no remote. Your own worktree is already set up; use this only for a second repository.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "The repository's folder, absolute or starting with ~/." },
                "branch": { "type": "string", "description": "The branch to check out: made from base when it does not exist yet." },
                "base": { "type": "string", "description": "Where a new branch starts, such as origin/main. Default: the repository's current HEAD." },
            },
            "required": ["repo", "branch"],
            "additionalProperties": false,
        },
    })
}

/// The folder that holds the tab's extra worktrees, beside its notes.
#[must_use]
pub fn dir_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(DIR)
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

fn run(mut command: Command) -> Result<String, String> {
    let out = limited::output(&mut command, limited::WORKTREE)
        .map_err(|e| limited::failure("git", &e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The main checkout of the repository that holds `dir`.
fn main_checkout(dir: &Path) -> Result<PathBuf, String> {
    let mut command = git(dir);
    command.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let common = PathBuf::from(run(command)?);
    common.parent().map(Path::to_path_buf).ok_or_else(|| "no main checkout".to_owned())
}

fn folder_name(repo: &Path, branch: &str) -> String {
    let repo = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let branch: String =
        branch.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    format!("{repo}-{}", branch.trim_matches('-'))
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    let mut command = git(repo);
    command.args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]);
    run(command).is_ok()
}

fn text_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// Makes the worktree and says where it is, or why not.
#[must_use]
pub fn add(args: &Map<String, Value>, notes_file: &Path) -> (String, bool) {
    let (Some(repo), Some(branch)) = (text_arg(args, "repo"), text_arg(args, "branch")) else {
        return ("add_worktree needs repo and branch".to_owned(), true);
    };
    if !branch_name::is_usable_ref(branch) {
        return (format!("branch: {branch} is not a name git accepts"), true);
    }
    let repo = expand(repo);
    if !repo.is_absolute() || !repo.is_dir() {
        return (format!("{} is not a folder; give an absolute path", repo.display()), true);
    }
    let main = match main_checkout(&repo) {
        Ok(main) => main,
        Err(e) => return (format!("{} is not a git repository: {e}", repo.display()), true),
    };
    let path = dir_beside(notes_file).join(folder_name(&main, branch));
    if path.join(".git").is_file() {
        return (
            format!("Already there: {} on {branch}. Work in that folder.", path.display()),
            false,
        );
    }
    let mut command = git(&main);
    command.args(["worktree", "add"]);
    if branch_exists(&main, branch) {
        command.arg(&path).arg(branch);
    } else {
        command.args(["-b", branch]).arg(&path);
        if let Some(base) = text_arg(args, "base") {
            command.arg(base);
        }
    }
    match run(command) {
        Ok(_) => (
            format!(
                "Made {} on {branch} (a worktree of {}). Work in that folder. Tidewatch removes it when this tab closes, unless it holds uncommitted files or commits that are on no remote.",
                path.display(),
                main.display()
            ),
            false,
        ),
        Err(e) => (format!("git worktree add failed: {e}"), true),
    }
}

/// The tab's extra worktrees on disk.
#[must_use]
pub fn listed(notes_file: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir_beside(notes_file)) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(".git").is_file())
        .collect();
    found.sort();
    found
}

/// Whether a worktree can go without losing work: no uncommitted files and no commit that is on
/// no remote branch. Unknown counts as no.
fn safe_to_remove(path: &Path) -> bool {
    let mut status = git(path);
    status.args(["status", "--porcelain"]);
    let mut local = git(path);
    local.args(["log", "--oneline", "HEAD", "--not", "--remotes"]);
    matches!((run(status), run(local)), (Ok(changed), Ok(commits)) if changed.is_empty() && commits.is_empty())
}

#[must_use]
pub fn remove_all_but_kept(notes_file: &Path) -> Vec<PathBuf> {
    listed(notes_file)
        .into_iter()
        .filter(|path| {
            let removed = safe_to_remove(path)
                && main_checkout(path).is_ok_and(|main| {
                    let mut command = git(&main);
                    command.args(["worktree", "remove"]).arg(path);
                    run(command).is_ok()
                });
            !removed
        })
        .collect()
}

#[cfg(test)]
mod tests;
