//! The completion list of the Reader's reply box, per folder: the built-in commands at once, the
//! skills and custom commands found on disk read once on a job thread the first time a box
//! starts with a slash.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::jobs::Jobs;
use crate::slash::{self, Entry};

pub(crate) struct Host {
    jobs: Jobs<(PathBuf, Arc<Vec<Entry>>)>,
    loaded: HashMap<PathBuf, Arc<Vec<Entry>>>,
    loading: HashSet<PathBuf>,
    builtins: Arc<Vec<Entry>>,
}

impl Host {
    pub(crate) fn new() -> Self {
        Self {
            jobs: Jobs::new(),
            loaded: HashMap::new(),
            loading: HashSet::new(),
            builtins: Arc::new(slash::builtins()),
        }
    }

    pub(crate) fn entries(
        &mut self,
        ctx: &egui::Context,
        claude_dir: Option<&Path>,
        folder: &Path,
    ) -> Arc<Vec<Entry>> {
        for (folder, entries) in self.jobs.drain() {
            self.loading.remove(&folder);
            self.loaded.insert(folder, entries);
        }
        if let Some(entries) = self.loaded.get(folder) {
            return Arc::clone(entries);
        }
        if let Some(claude_dir) = claude_dir
            && self.loading.insert(folder.to_path_buf())
        {
            let (claude_dir, folder) = (claude_dir.to_path_buf(), folder.to_path_buf());
            self.jobs.spawn(ctx, move || {
                let entries = Arc::new(slash::load(&claude_dir, &folder));
                (folder, entries)
            });
        }
        Arc::clone(&self.builtins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn built_ins_show_at_once_and_the_disk_is_read_once_off_the_frame() {
        let claude = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(claude.path().join("skills/deploy")).unwrap();
        std::fs::write(
            claude.path().join("skills/deploy/SKILL.md"),
            "---\ndescription: Ship it.\n---\n",
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut host = Host::new();
        let first = host.entries(&ctx, Some(claude.path()), folder.path());
        assert_eq!(*first, slash::builtins());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut now = first;
        while now.len() == slash::builtins().len() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            now = host.entries(&ctx, Some(claude.path()), folder.path());
        }
        assert!(now.iter().any(|entry| entry.name == "deploy" && entry.description == "Ship it."));
        assert!(Arc::ptr_eq(&now, &host.entries(&ctx, Some(claude.path()), folder.path())));
    }

    #[test]
    fn without_a_claude_folder_only_the_built_ins_are_offered() {
        let mut host = Host::new();
        let entries = host.entries(&egui::Context::default(), None, Path::new("/nowhere"));
        assert_eq!(*entries, slash::builtins());
    }
}
