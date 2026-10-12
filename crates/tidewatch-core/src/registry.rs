//! The worktrees Tidewatch created, kept in `.state/worktrees.toml`. `git worktree repair` needs the
//! repo path each worktree was made from, and git forgets it once the repo moves.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::TidewatchPaths;
use crate::worktree::Worktree;

const FILE_NAME: &str = "worktrees.toml";

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    worktree: Vec<Worktree>,
}

fn file(paths: &TidewatchPaths) -> std::path::PathBuf {
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

/// Writes a sibling temp file and renames it over the real one, so a crash never leaves half a file.
pub(crate) fn save(paths: &TidewatchPaths, worktrees: &[Worktree]) -> std::io::Result<()> {
    let target = file(paths);
    std::fs::create_dir_all(paths.state_dir())?;
    let text =
        toml::to_string(&Stored { worktree: worktrees.to_vec() }).map_err(std::io::Error::other)?;
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target)
}

pub(crate) fn record(paths: &TidewatchPaths, worktree: &Worktree) -> std::io::Result<()> {
    let mut all = load(paths);
    all.retain(|w| w.path != worktree.path);
    all.push(worktree.clone());
    save(paths, &all)
}

pub(crate) fn forget(paths: &TidewatchPaths, path: &Path) -> std::io::Result<()> {
    let mut all = load(paths);
    let before = all.len();
    all.retain(|w| w.path != path);
    if all.len() == before { Ok(()) } else { save(paths, &all) }
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
        assert!(load(&TidewatchPaths::new(home.path())).is_empty());
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
    fn corrupt_file_loads_as_empty() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::write(file(&paths), "worktree = [[[").unwrap();
        assert!(load(&paths).is_empty());
    }
}
