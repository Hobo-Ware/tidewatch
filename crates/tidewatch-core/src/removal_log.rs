//! `.state/removals.log`: one JSON line for every worktree Tidewatch removes, written before the
//! removal, so a mistake is undone with `git branch <name> <sha>`. Append-only; a line that cannot
//! be written stops the removal.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::paths::TidewatchPaths;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub at: u64,
    pub why: String,
    pub path: PathBuf,
    pub repo: PathBuf,
    pub branch: String,
    /// The branch's commit, when the branch exists.
    pub sha: Option<String>,
    /// The worktree's own HEAD, which differs from the branch when it was detached.
    pub head: Option<String>,
    pub mode: String,
    pub branch_kept: bool,
    pub lost: Vec<String>,
}

impl Entry {
    #[must_use]
    pub fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
    }

    /// The one command that brings the branch back.
    #[must_use]
    pub fn restore_command(&self) -> Option<String> {
        Some(format!("git branch {} {}", self.branch, self.sha.as_deref()?))
    }
}

pub fn append(paths: &TidewatchPaths, entry: &Entry) -> std::io::Result<()> {
    let file = paths.removals_log();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut open = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    open.write_all(line.as_bytes())?;
    open.sync_all()
}

#[must_use]
pub fn load(paths: &TidewatchPaths) -> Vec<Entry> {
    std::fs::read_to_string(paths.removals_log())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(branch: &str, sha: Option<&str>) -> Entry {
        Entry {
            at: 1,
            why: "closed tab".into(),
            path: "/w/a".into(),
            repo: "/r".into(),
            branch: branch.into(),
            sha: sha.map(str::to_owned),
            head: None,
            mode: "safe".into(),
            branch_kept: false,
            lost: Vec::new(),
        }
    }

    #[test]
    fn lines_are_appended_in_order_and_read_back() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        append(&paths, &entry("agent/a", Some("abc123"))).unwrap();
        append(&paths, &entry("agent/b", None)).unwrap();
        let text = std::fs::read_to_string(paths.removals_log()).unwrap();
        assert_eq!(text.lines().count(), 2);
        let all = load(&paths);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].branch, "agent/a");
        assert_eq!(all[1].sha, None);
    }

    #[test]
    fn the_restore_command_names_the_branch_and_its_commit() {
        assert_eq!(
            entry("agent/a", Some("abc123")).restore_command().as_deref(),
            Some("git branch agent/a abc123")
        );
        assert_eq!(entry("agent/a", None).restore_command(), None);
    }

    #[test]
    fn a_log_that_cannot_be_written_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.removals_log()).unwrap();
        assert!(append(&paths, &entry("agent/a", None)).is_err());
    }
}
