//! The per-repo workflow profile in `~/.config/tidewatch/workflow.toml`: the only source of method
//! words in what Tidewatch says to an agent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatchUp {
    #[default]
    Rebase,
    Merge,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fixes {
    #[default]
    Fixup,
    NewCommits,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullRequests {
    #[default]
    Draft,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    The,
    Each,
    It,
}

/// The checks Tidewatch runs itself on an agent that says it is done, one switch each. The first
/// three are on by default; the other two cost tokens or send a message, so a repo turns them on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // five independent switches, not a mode
pub struct Checks {
    #[serde(default = "on")]
    pub tests: bool,
    #[serde(default = "on")]
    pub verified: bool,
    #[serde(default = "on")]
    pub scope: bool,
    #[serde(default)]
    pub fix_test: bool,
    #[serde(default)]
    pub reviewer: bool,
}

fn on() -> bool {
    true
}

impl Checks {
    pub const OFF: Self =
        Self { tests: false, verified: false, scope: false, fix_test: false, reviewer: false };
}

impl Default for Checks {
    fn default() -> Self {
        Self { tests: true, verified: true, scope: true, fix_test: false, reviewer: false }
    }
}

/// The defaults are what the owner's own rules say most often.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default)]
    pub catch_up: CatchUp,
    #[serde(default)]
    pub fixes: Fixes,
    #[serde(default)]
    pub pull_requests: PullRequests,
    #[serde(default)]
    pub checks: Checks,
}

const LEASE: &str = "push with --force-with-lease";

impl Workflow {
    #[must_use]
    pub fn catch_up_rewrites(self) -> bool {
        self.catch_up == CatchUp::Rebase
    }

    #[must_use]
    pub fn fix_rewrites(self) -> bool {
        self.fixes == Fixes::Fixup
    }

    #[must_use]
    pub fn catch_up_clause(self, base: &str) -> String {
        match self.catch_up {
            CatchUp::Rebase => format!("Rebase onto origin/{base}"),
            CatchUp::Merge => format!("Merge origin/{base} into this branch"),
        }
    }

    #[must_use]
    pub fn push_after_catch_up(self) -> &'static str {
        if self.catch_up_rewrites() { LEASE } else { "push" }
    }

    /// Where a fix goes, without the push.
    #[must_use]
    pub fn fold(self, fix: Fix) -> &'static str {
        match (self.fixes, fix) {
            (Fixes::Fixup, Fix::The) => "fold the fix into the commit it belongs to",
            (Fixes::Fixup, Fix::Each) => "fold each fix into its commit",
            (Fixes::Fixup, Fix::It) => "fold it into its commit",
            (Fixes::NewCommits, Fix::The) => "commit the fix",
            (Fixes::NewCommits, Fix::Each) => "commit each fix",
            (Fixes::NewCommits, Fix::It) => "commit it",
        }
    }

    #[must_use]
    pub fn fix_and_push(self, fix: Fix) -> String {
        let fold = self.fold(fix);
        match self.fixes {
            Fixes::Fixup => format!("{fold} and {LEASE}"),
            Fixes::NewCommits => format!("{fold} and push"),
        }
    }

    /// What happens to each fix a review asked for, before any push.
    #[must_use]
    pub fn each_fix(self) -> &'static str {
        match self.fixes {
            Fixes::Fixup => "fold each fix into its commit",
            Fixes::NewCommits => "commit each fix",
        }
    }

    #[must_use]
    pub fn pushes_unpushed_with_lease(self) -> bool {
        self.catch_up_rewrites() || self.fix_rewrites()
    }

    #[must_use]
    pub fn new_pull_request(self) -> &'static str {
        match self.pull_requests {
            PullRequests::Draft => "a draft pull request",
            PullRequests::Ready => "a pull request (not a draft)",
        }
    }

    /// One sentence per setting for the agent's launch text.
    #[must_use]
    pub fn launch_lines(self) -> [&'static str; 4] {
        [
            match self.catch_up {
                CatchUp::Rebase => "Branches catch up by rebasing onto the default branch.",
                CatchUp::Merge => "Branches catch up by merging the default branch into them.",
            },
            match self.fixes {
                Fixes::Fixup => {
                    "Fixes fold into the commit they belong to: `git commit --fixup`, then autosquash."
                }
                Fixes::NewCommits => "Fixes go in new commits.",
            },
            if self.pushes_unpushed_with_lease() {
                "Push a rewritten branch with `--force-with-lease`."
            } else {
                "Push without force."
            },
            match self.pull_requests {
                PullRequests::Draft => "New pull requests open as drafts (`gh pr create --draft`).",
                PullRequests::Ready => {
                    "New pull requests open ready for review (`gh pr create`, no `--draft`)."
                }
            },
        ]
    }

    #[must_use]
    pub fn draft_flag(self) -> &'static str {
        match self.pull_requests {
            PullRequests::Draft => " --draft",
            PullRequests::Ready => "",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    repo: BTreeMap<String, Workflow>,
}

fn key_of(repo: &Path) -> String {
    repo.components().collect::<PathBuf>().to_string_lossy().into_owned()
}

impl Store {
    /// A missing or broken file is an empty store; a broken one says so on stderr.
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        let file = paths.workflow_file();
        let Ok(text) = std::fs::read_to_string(&file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); no workflow profiles", file.display());
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.workflow_file(), &text)
    }

    pub fn update(paths: &TidewatchPaths, change: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let mut store = Self::load(paths);
        change(&mut store);
        store.save(paths)
    }

    #[must_use]
    pub fn get(&self, repo: &Path) -> Option<Workflow> {
        self.repo.get(&key_of(repo)).copied()
    }

    pub fn set(&mut self, repo: &Path, workflow: Workflow) {
        self.repo.insert(key_of(repo), workflow);
    }
}

#[cfg(test)]
mod tests;
