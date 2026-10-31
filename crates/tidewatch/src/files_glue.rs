//! The files an `@` offers, per folder: listed once on a job thread when a box first has an `@`,
//! and again when the list is more than `STALE` old, so a new file shows up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tidewatch_core::limited;

use crate::jobs::Jobs;

const STALE: Duration = Duration::from_secs(20);
const LIST_LIMIT: Duration = Duration::from_secs(10);
const MAX_FILES: usize = 50_000;
const WALK_FILES: usize = 5_000;
const WALK_DEPTH: usize = 5;
const SKIPPED: [&str; 2] = ["node_modules", "target"];

type Listed = (PathBuf, Arc<Vec<String>>);

pub(crate) struct Host {
    jobs: Jobs<Listed>,
    loaded: HashMap<PathBuf, (Instant, Arc<Vec<String>>)>,
    loading: HashSet<PathBuf>,
}

fn from_git(folder: &Path) -> Option<Vec<String>> {
    let mut command = Command::new("git");
    command
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(folder);
    let output = limited::output(&mut command, LIST_LIMIT).ok().filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = text
        .split('\0')
        .filter(|path| !path.is_empty())
        .take(MAX_FILES)
        .map(String::from)
        .collect();
    Some(files)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || SKIPPED.contains(&name.as_str()) || out.len() >= WALK_FILES {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if depth < WALK_DEPTH {
                walk(root, &path, depth + 1, out);
            }
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.to_string_lossy().into_owned());
        }
    }
}

/// The files of `folder` as paths relative to it: what git tracks and does not ignore, else a
/// shallow walk for a folder that is no repo.
pub(crate) fn list(folder: &Path) -> Vec<String> {
    from_git(folder).filter(|files| !files.is_empty()).unwrap_or_else(|| {
        let mut out = Vec::new();
        walk(folder, folder, 0, &mut out);
        out
    })
}

impl Host {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), loaded: HashMap::new(), loading: HashSet::new() }
    }

    pub(crate) fn files(&mut self, ctx: &egui::Context, folder: &Path) -> Arc<Vec<String>> {
        for (folder, files) in self.jobs.drain() {
            self.loading.remove(&folder);
            self.loaded.insert(folder, (Instant::now(), files));
        }
        let known = self.loaded.get(folder);
        let stale = known.is_none_or(|(at, _)| at.elapsed() > STALE);
        if stale && self.loading.insert(folder.to_path_buf()) {
            let folder = folder.to_path_buf();
            self.jobs.spawn(ctx, move || {
                let files = Arc::new(list(&folder));
                (folder, files)
            });
        }
        known.map_or_else(Arc::default, |(_, files)| Arc::clone(files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_folder_lists_its_files_without_hidden_or_build_folders() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["a.txt", "src/b.rs", ".git/x", "node_modules/y.js", "target/z"] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        assert_eq!(list(dir.path()), ["a.txt", "src/b.rs"]);
    }

    #[test]
    fn a_repo_lists_what_git_knows_and_leaves_out_what_it_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git").args(args).current_dir(dir.path()).status().unwrap();
            assert!(status.success(), "{args:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.path().join(".gitignore"), "out/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("out")).unwrap();
        std::fs::write(dir.path().join("out/o.txt"), "").unwrap();
        std::fs::write(dir.path().join("kept.rs"), "").unwrap();
        assert_eq!(list(dir.path()), [".gitignore", "kept.rs"]);
    }
}
