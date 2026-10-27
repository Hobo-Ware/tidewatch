//! Whether a worktree can go without losing work, as one typed answer. `assess` is pure; `Watch`
//! is the seam for what only the machine knows (a process with the folder as its working
//! directory, an agent session that wrote there lately). Anything that cannot be read is
//! `Unknown`, never counted as zero.

use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::elsewhere::{ProcessTable, parse_ps};
use crate::elsewhere_scan::{RealSystem, ScanEnv, System, newest_session_write};
use crate::paths::TidewatchPaths;

/// An agent session that wrote in the folder more recently than this keeps the worktree.
pub const RECENT_SESSION: Duration = Duration::from_hours(24);
const COMMAND_CHARS: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Risk {
    Uncommitted {
        files: usize,
    },
    /// Commits found nowhere else: on no remote branch, no tag, no other branch.
    Unpushed {
        commits: usize,
    },
    Running {
        pid: u32,
        command: String,
    },
    RecentSession {
        minutes: u64,
    },
}

impl Risk {
    /// What would be lost, in words.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Uncommitted { files } => {
                format!("{files} {} with uncommitted changes", noun(*files, "file", "files"))
            }
            Self::Unpushed { commits } => format!(
                "{commits} {} found nowhere else (on no remote branch, not merged)",
                noun(*commits, "commit", "commits")
            ),
            Self::Running { pid, command } => format!("{command} (pid {pid}) is running in it"),
            Self::RecentSession { minutes } => {
                format!("an agent session wrote in it {} ago", ago(*minutes))
            }
        }
    }

    /// Whether removing the worktree folder (not the branch) is what loses it.
    #[must_use]
    pub fn is_in_folder(&self) -> bool {
        !matches!(self, Self::Unpushed { .. })
    }
}

fn noun<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

fn ago(minutes: u64) -> String {
    match minutes {
        0..=59 => format!("{minutes} min"),
        _ => format!("{} h", minutes / 60),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Safety {
    Safe,
    Unsafe(Vec<Risk>),
    /// Something could not be read, so nothing is proven.
    Unknown(String),
}

impl Safety {
    #[must_use]
    pub fn is_safe(&self) -> bool {
        *self == Self::Safe
    }

    #[must_use]
    pub fn risks(&self) -> &[Risk] {
        match self {
            Self::Unsafe(risks) => risks,
            Self::Safe | Self::Unknown(_) => &[],
        }
    }

    /// Removing the folder loses nothing, whatever happens to the branch.
    #[must_use]
    pub fn folder_is_safe(&self) -> bool {
        match self {
            Self::Safe => true,
            Self::Unsafe(risks) => risks.iter().all(|risk| !risk.is_in_folder()),
            Self::Unknown(_) => false,
        }
    }

    #[must_use]
    pub fn reasons(&self) -> Vec<String> {
        match self {
            Self::Safe => Vec::new(),
            Self::Unsafe(risks) => risks.iter().map(Risk::text).collect(),
            Self::Unknown(error) => vec![format!("could not check it: {error}")],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub command: String,
}

/// What was read about one worktree. A read that failed is carried as its error.
#[derive(Clone, Debug)]
pub struct Facts {
    pub changed: Result<usize, String>,
    pub unmerged: Result<usize, String>,
    pub holders: Result<Vec<Holder>, String>,
    /// How long ago an agent session wrote in the folder; `None` when none did.
    pub session_age: Option<Duration>,
}

#[must_use]
pub fn assess(facts: Facts) -> Safety {
    let errors: Vec<&str> =
        [facts.changed.as_ref().err(), facts.unmerged.as_ref().err(), facts.holders.as_ref().err()]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
    if !errors.is_empty() {
        return Safety::Unknown(errors.join("; "));
    }
    let mut risks = Vec::new();
    if let Ok(files @ 1..) = facts.changed {
        risks.push(Risk::Uncommitted { files });
    }
    if let Ok(commits @ 1..) = facts.unmerged {
        risks.push(Risk::Unpushed { commits });
    }
    for holder in facts.holders.into_iter().flatten() {
        risks.push(Risk::Running { pid: holder.pid, command: holder.command });
    }
    if let Some(age) = facts.session_age.filter(|age| *age < RECENT_SESSION) {
        risks.push(Risk::RecentSession { minutes: age.as_secs() / 60 });
    }
    if risks.is_empty() { Safety::Safe } else { Safety::Unsafe(risks) }
}

pub trait Watch {
    fn holders(&self, dir: &Path) -> Result<Vec<Holder>, String>;
    fn session_age(&self, dir: &Path) -> Option<Duration>;
}

/// Whose worktree is being looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// No open tab: a leftover, or a folder made by an earlier run. Every process and every recent
    /// session counts, Tidewatch's own too.
    Nobody,
    /// A tab of this window, being closed by its user. Tidewatch stops the tab's own processes
    /// first, and the agent's session of the tab is the one being closed, so neither counts.
    ThisWindow,
}

pub struct SystemWatch<'a> {
    system: &'a dyn System,
    env: ScanEnv,
    owner: Owner,
}

impl<'a> SystemWatch<'a> {
    #[must_use]
    pub fn new(system: &'a dyn System, env: ScanEnv, owner: Owner) -> Self {
        Self { system, env, owner }
    }
}

impl Watch for SystemWatch<'_> {
    fn holders(&self, dir: &Path) -> Result<Vec<Holder>, String> {
        let pids = self
            .system
            .cwd_holders(dir)
            .ok_or_else(|| "the processes in its folder could not be listed".to_owned())?;
        if pids.is_empty() {
            return Ok(Vec::new());
        }
        let table = self.system.ps().map(|text| ProcessTable::new(parse_ps(&text)));
        let mut holders = Vec::new();
        for pid in pids {
            let row = table.as_ref().and_then(|table| table.get(pid));
            if self.owner == Owner::ThisWindow
                && (pid == self.env.own_pid
                    || table.as_ref().is_some_and(|t| t.descends_from(pid, self.env.own_pid)))
            {
                continue;
            }
            let command = row.map_or_else(|| "a process".to_owned(), |row| short(&row.command()));
            holders.push(Holder { pid, command });
        }
        Ok(holders)
    }

    fn session_age(&self, dir: &Path) -> Option<Duration> {
        if self.owner == Owner::ThisWindow {
            return None;
        }
        let wrote = newest_session_write(&self.env, dir)?;
        Some(SystemTime::now().duration_since(wrote).unwrap_or_default())
    }
}

fn short(command: &str) -> String {
    let mut chars = command.chars();
    let head: String = chars.by_ref().take(COMMAND_CHARS).collect();
    if chars.next().is_some() { format!("{head}...") } else { head }
}

pub fn with_machine<T>(
    paths: &TidewatchPaths,
    owner: Owner,
    use_watch: impl FnOnce(&dyn Watch) -> T,
) -> T {
    let system = RealSystem::new();
    use_watch(&SystemWatch::new(&system, ScanEnv::for_paths(paths), owner))
}

#[cfg(test)]
mod tests;
