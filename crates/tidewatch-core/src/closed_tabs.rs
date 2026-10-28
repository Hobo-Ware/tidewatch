//! `.state/closed-tabs.jsonl`: one JSON line for every agent tab Tidewatch closed, so the tab can
//! come back with its conversation. The session file's history is gone once the tab closes, so
//! the line keeps what a reopen needs: the worktree, its branch and commits, the profile, level,
//! CLI, session id and title. The newest 100 lines stay; each rewrite is atomic.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agents::Level;
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::removal_log::Entry;
use crate::session::{SavedAgentTab, SavedTab, SavedWindow};
use crate::worktree::Worktree;

pub const KEEP: usize = 100;
pub const WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

/// What the closing tab knows of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub title: String,
    pub agent: String,
    pub profile: String,
    pub level: Level,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    pub at: u64,
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub sha: Option<String>,
    pub head: Option<String>,
    pub profile: String,
    pub level: Level,
    pub agent: String,
    pub session_id: Option<String>,
    pub title: String,
    pub branch_kept: bool,
}

impl Closed {
    /// The line for a removal that was just logged.
    #[must_use]
    pub fn removed(tab: &Tab, entry: &Entry) -> Self {
        Self {
            at: entry.at,
            repo: entry.repo.clone(),
            path: entry.path.clone(),
            branch: entry.branch.clone(),
            sha: entry.sha.clone(),
            head: entry.head.clone(),
            profile: tab.profile.clone(),
            level: tab.level,
            agent: tab.agent.clone(),
            session_id: tab.session_id.clone(),
            title: tab.title.clone(),
            branch_kept: entry.branch_kept,
        }
    }

    /// The line for a tab closed while its worktree stays.
    #[must_use]
    pub fn kept(tab: &Tab, worktree: &Worktree, sha: Option<String>, head: Option<String>) -> Self {
        Self {
            at: Entry::now(),
            repo: worktree.repo.clone(),
            path: worktree.path.clone(),
            branch: worktree.branch.clone(),
            sha,
            head,
            profile: tab.profile.clone(),
            level: tab.level,
            agent: tab.agent.clone(),
            session_id: tab.session_id.clone(),
            title: tab.title.clone(),
            branch_kept: true,
        }
    }

    #[must_use]
    pub fn worktree(&self) -> Worktree {
        Worktree { repo: self.repo.clone(), path: self.path.clone(), branch: self.branch.clone() }
    }

    /// The one-tab saved window the restore path plans to bring the tab back.
    #[must_use]
    pub fn saved_window(&self) -> SavedWindow {
        let tab = SavedTab::Agent(SavedAgentTab {
            dir: self.path.clone(),
            branch: Some(self.branch.clone()),
            agent: self.agent.clone(),
            profile: self.profile.clone(),
            level: self.level,
            session_id: self.session_id.clone(),
            stages: None,
            crashed: false,
            remote: false,
            meeseeks: None,
        });
        SavedWindow {
            dir: self.repo.clone(),
            other: false,
            active: 0,
            focus_mode: false,
            tabs: vec![tab],
            parked: Vec::new(),
        }
    }
}

/// Appends one line and keeps the newest [`KEEP`], through an atomic rewrite.
pub fn append(paths: &TidewatchPaths, closed: &Closed) -> std::io::Result<()> {
    let mut all = load(paths);
    all.push(closed.clone());
    let from = all.len().saturating_sub(KEEP);
    let mut text = String::new();
    for line in &all[from..] {
        text.push_str(&serde_json::to_string(line).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    write_atomic(&paths.closed_tabs_log(), &text)
}

#[must_use]
pub fn load(paths: &TidewatchPaths) -> Vec<Closed> {
    std::fs::read_to_string(paths.closed_tabs_log())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// What the Recently closed list shows: tabs closed within [`WINDOW_SECS`] of `now` that have a
/// session id, are not open in a tab again, newest first, one per session.
#[must_use]
pub fn listing<'a>(all: &'a [Closed], now: u64, is_open: &dyn Fn(&str) -> bool) -> Vec<&'a Closed> {
    let mut newest: Vec<&Closed> = all.iter().collect();
    newest.sort_by_key(|closed| std::cmp::Reverse(closed.at));
    let mut seen = HashSet::new();
    newest
        .into_iter()
        .filter(|closed| closed.at.saturating_add(WINDOW_SECS) >= now)
        .filter_map(|closed| {
            let id = closed.session_id.as_deref().filter(|id| !id.is_empty())?;
            (seen.insert(id) && !is_open(id)).then_some(closed)
        })
        .collect()
}

#[cfg(test)]
mod tests;
