//! The worktrees Tidewatch created, kept in `.state/worktrees.toml`. `git worktree repair` needs the
//! repo path each worktree was made from, and git forgets it once the repo moves.
//!
//! Every change is a read-modify-write under an exclusive lock on `.state/worktrees.lock`, so tabs
//! opening and closing on different threads (or windows) never overwrite each other's entries.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::paths::TidewatchPaths;
use crate::worktree::Worktree;

const FILE_NAME: &str = "worktrees.toml";
const LOCK_NAME: &str = "worktrees.lock";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    worktree: Vec<Worktree>,
}

fn file(paths: &TidewatchPaths) -> PathBuf {
    paths.state_dir().join(FILE_NAME)
}

/// A missing or unreadable file is an empty registry: a bad state file must never stop the app.
pub(crate) fn load(paths: &TidewatchPaths) -> Vec<Worktree> {
    let Ok(text) = std::fs::read_to_string(file(paths)) else {
        return Vec::new();
    };
    match toml::from_str::<Stored>(&text) {
        Ok(stored) => stored.worktree,
        Err(e) => {
            eprintln!("tidewatch: worktrees.toml is unreadable ({e}); starting empty");
            Vec::new()
        }
    }
}

/// Writes a temp file of its own and renames it over the real one, so a crash never leaves half a
/// file and two writers never share a temp file.
fn save(paths: &TidewatchPaths, worktrees: &[Worktree]) -> io::Result<()> {
    let target = file(paths);
    std::fs::create_dir_all(paths.state_dir())?;
    let text =
        toml::to_string(&Stored { worktree: worktrees.to_vec() }).map_err(io::Error::other)?;
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = target.with_extension(format!("toml.{}.{unique}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn locked(paths: &TidewatchPaths) -> io::Result<File> {
    std::fs::create_dir_all(paths.state_dir())?;
    let lock =
        OpenOptions::new().create(true).append(true).open(paths.state_dir().join(LOCK_NAME))?;
    lock.lock()?;
    Ok(lock)
}

/// Runs `change` on a fresh read of the registry while holding its lock and writes the result back
/// when `change` altered it. Whatever `change` returns is handed back.
pub(crate) fn update<R>(
    paths: &TidewatchPaths,
    change: impl FnOnce(&mut Vec<Worktree>) -> R,
) -> io::Result<R> {
    let _lock = locked(paths)?;
    let mut all = load(paths);
    let before = all.clone();
    let out = change(&mut all);
    if all != before {
        save(paths, &all)?;
    }
    Ok(out)
}

pub(crate) fn record(paths: &TidewatchPaths, worktree: &Worktree) -> io::Result<()> {
    update(paths, |all| {
        all.retain(|w| w.path != worktree.path);
        all.push(worktree.clone());
    })
}

pub(crate) fn forget(paths: &TidewatchPaths, path: &Path) -> io::Result<()> {
    update(paths, |all| all.retain(|w| w.path != path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn worktree(repo: &str, name: &str) -> Worktree {
        Worktree {
            repo: PathBuf::from(repo),
            path: PathBuf::from(format!("/root/{name}")),
            branch: format!("agent/{name}"),
        }
    }

    #[test]
    fn missing_file_is_an_empty_registry() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(load(&TidewatchPaths::new(home.path())), []);
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        record(&paths, &worktree("/work/web", "brave-otter")).unwrap();
        record(&paths, &worktree("/work/api", "calm-comet")).unwrap();
        assert_eq!(
            load(&paths),
            [worktree("/work/web", "brave-otter"), worktree("/work/api", "calm-comet")]
        );
    }

    #[test]
    fn recording_the_same_path_twice_keeps_one_entry() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        record(&paths, &worktree("/work/web", "brave-otter")).unwrap();
        record(&paths, &worktree("/moved/web", "brave-otter")).unwrap();
        assert_eq!(load(&paths), [worktree("/moved/web", "brave-otter")]);
    }

    #[test]
    fn forgetting_removes_only_that_path() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        record(&paths, &worktree("/work/web", "brave-otter")).unwrap();
        record(&paths, &worktree("/work/web", "calm-comet")).unwrap();
        forget(&paths, Path::new("/root/brave-otter")).unwrap();
        assert_eq!(load(&paths), [worktree("/work/web", "calm-comet")]);
    }

    #[test]
    fn writers_on_many_threads_never_lose_an_entry() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::thread::scope(|scope| {
            for i in 0..24 {
                let paths = &paths;
                scope.spawn(move || {
                    record(paths, &worktree("/work/web", &format!("burst-{i}"))).unwrap();
                    if i % 3 == 0 {
                        forget(paths, Path::new(&format!("/root/burst-{i}"))).unwrap();
                    }
                });
            }
        });
        assert_eq!(load(&paths).len(), 16);
    }

    #[test]
    fn a_change_that_alters_nothing_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        forget(&paths, Path::new("/root/none")).unwrap();
        assert!(!file(&paths).exists());
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::write(file(&paths), "worktree = [[[").unwrap();
        assert_eq!(load(&paths), []);
    }
}
