//! Where Claude Code files a project's auto memory, so a tab's `--settings` can point it at the
//! repo's own folder. Pure.
//!
//! Checked against Claude Code 2.1.310: the folder name is the project path with every character
//! outside `[a-zA-Z0-9]` turned into `-`, and a name over 200 characters gets a hash suffix that is
//! not reproduced here (`memory_dir` gives none for such a path).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::transcript_locate::claude_project_dir_name;

const MAX_NAME_UTF16: usize = 200;

/// Claude Code's config folder: `CLAUDE_CONFIG_DIR` when set, else `~/.claude`.
#[must_use]
pub fn config_dir(home: &Path, var: Option<OsString>) -> PathBuf {
    var.filter(|v| !v.is_empty()).map_or_else(|| home.join(".claude"), PathBuf::from)
}

/// The folder name Claude Code gives a project path, or `None` for one too long to name here.
/// Claude Code counts UTF-16 units, so one character outside the BMP is two dashes.
#[must_use]
pub fn project_dir_name(path: &Path) -> Option<String> {
    let name = claude_project_dir_name(path);
    (name.len() <= MAX_NAME_UTF16).then_some(name)
}

/// `<config>/projects/<encoded repo>/memory`: the folder Claude Code uses when it starts in `repo`.
#[must_use]
pub fn memory_dir(claude_dir: &Path, repo: &Path) -> Option<PathBuf> {
    Some(claude_dir.join("projects").join(project_dir_name(repo)?).join("memory"))
}

#[cfg(test)]
mod tests;
