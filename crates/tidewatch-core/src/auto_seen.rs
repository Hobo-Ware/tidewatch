//! Which kinds of Auto message a repo has shown its owner already (pure apart from the file).
//! Auto's first conflict, failed check or review fix in a repo waits for a click on Send, so the
//! owner reads the wording once; after that, that kind goes on its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::{load_toml_or_default, save_toml_pretty};
use crate::paths::TidewatchPaths;
use crate::pr_prompt::Trigger;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    #[serde(default)]
    pub checks: bool,
    #[serde(default)]
    pub reviews: bool,
    #[serde(default)]
    pub conflicts: bool,
}

impl Shown {
    #[must_use]
    pub fn has(self, kind: Trigger) -> bool {
        match kind {
            Trigger::FailedChecks => self.checks,
            Trigger::Reviews => self.reviews,
            Trigger::Conflicts => self.conflicts,
        }
    }

    #[must_use]
    pub fn with(self, kind: Trigger) -> Self {
        match kind {
            Trigger::FailedChecks => Self { checks: true, ..self },
            Trigger::Reviews => Self { reviews: true, ..self },
            Trigger::Conflicts => Self { conflicts: true, ..self },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    repo: BTreeMap<String, Shown>,
}

fn key_of(repo: &Path) -> String {
    repo.components().collect::<PathBuf>().to_string_lossy().into_owned()
}

impl Store {
    /// A missing or broken file is an empty store; a broken one says so on stderr.
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        load_toml_or_default(&paths.auto_seen_file(), "Auto asks again")
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        save_toml_pretty(&paths.auto_seen_file(), self)
    }

    #[must_use]
    pub fn get(&self, repo: &Path) -> Shown {
        self.repo.get(&key_of(repo)).copied().unwrap_or_default()
    }

    pub fn mark(&mut self, repo: &Path, kind: Trigger) {
        let shown = self.get(repo).with(kind);
        self.set(repo, shown);
    }

    pub fn set(&mut self, repo: &Path, shown: Shown) {
        self.repo.insert(key_of(repo), shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_seen_for_its_repo_only() {
        let mut store = Store::default();
        store.mark(Path::new("/a"), Trigger::Conflicts);
        assert!(store.get(Path::new("/a")).has(Trigger::Conflicts));
        assert!(!store.get(Path::new("/a")).has(Trigger::FailedChecks));
        assert!(!store.get(Path::new("/a")).has(Trigger::Reviews));
        assert_eq!(store.get(Path::new("/b")), Shown::default());
    }

    #[test]
    fn marking_keeps_the_kinds_already_seen() {
        let shown = Shown::default().with(Trigger::Reviews).with(Trigger::FailedChecks);
        assert_eq!(shown, Shown { checks: true, reviews: true, conflicts: false });
    }

    #[test]
    fn the_file_survives_a_save_and_a_broken_one_reads_as_empty() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut store = Store::default();
        store.mark(Path::new("/a"), Trigger::Reviews);
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        store.save(&paths).unwrap();
        assert_eq!(Store::load(&paths), store);
        std::fs::write(paths.auto_seen_file(), "not = [toml").unwrap();
        assert_eq!(Store::load(&paths), Store::default());
    }
}
