//! The `PATH` an agent is looked up and started with. An app started from Finder or the dock gets a
//! bare `PATH` without Homebrew or `~/.local/bin`, so the usual install directories are put in front.
//! Pure: no directory is checked, nothing is read from the environment.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The directories agent CLIs are usually installed to, in lookup order.
#[must_use]
pub fn standard_dirs(home: &Path, linux: bool) -> Vec<PathBuf> {
    let local_bin = home.join(".local").join("bin");
    if linux {
        vec![local_bin, PathBuf::from("/usr/local/bin")]
    } else {
        vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), local_bin]
    }
}

/// `current` with every standard directory it lacks put in front, in `standard_dirs` order. A
/// directory already on the path keeps its place, so the user's own ordering wins.
#[must_use]
pub fn augmented_path(current: Option<&OsStr>, home: &Path, linux: bool) -> OsString {
    let existing: Vec<PathBuf> =
        current.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    let missing = standard_dirs(home, linux).into_iter().filter(|dir| !existing.contains(dir));
    let dirs: Vec<PathBuf> = missing.chain(existing.iter().cloned()).collect();
    std::env::join_paths(dirs).unwrap_or_else(|_| current.unwrap_or_default().to_owned())
}

/// `augmented_path` for the platform the app runs on.
#[must_use]
pub fn for_this_platform(current: Option<&OsStr>, home: &Path) -> OsString {
    augmented_path(current, home, cfg!(target_os = "linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &OsStr) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_bare_finder_path_gets_the_homebrew_dirs_in_front() {
        let path = augmented_path(Some(OsStr::new("/usr/bin:/bin")), Path::new("/Users/me"), false);
        assert_eq!(
            text(&path),
            "/opt/homebrew/bin:/usr/local/bin:/Users/me/.local/bin:/usr/bin:/bin"
        );
    }

    #[test]
    fn linux_gets_local_bin_and_usr_local_bin_only() {
        let path = augmented_path(Some(OsStr::new("/usr/bin")), Path::new("/home/me"), true);
        assert_eq!(text(&path), "/home/me/.local/bin:/usr/local/bin:/usr/bin");
    }

    #[test]
    fn a_dir_already_on_the_path_keeps_the_users_order() {
        let current = "/usr/bin:/opt/homebrew/bin:/bin";
        let path = augmented_path(Some(OsStr::new(current)), Path::new("/Users/me"), false);
        assert_eq!(
            text(&path),
            "/usr/local/bin:/Users/me/.local/bin:/usr/bin:/opt/homebrew/bin:/bin"
        );
    }

    #[test]
    fn a_complete_path_is_returned_unchanged() {
        let current = "/opt/homebrew/bin:/usr/local/bin:/Users/me/.local/bin:/usr/bin";
        let path = augmented_path(Some(OsStr::new(current)), Path::new("/Users/me"), false);
        assert_eq!(text(&path), current);
    }

    #[test]
    fn a_missing_path_becomes_just_the_standard_dirs() {
        let path = augmented_path(None, Path::new("/Users/me"), false);
        assert_eq!(text(&path), "/opt/homebrew/bin:/usr/local/bin:/Users/me/.local/bin");
    }

    #[test]
    fn running_it_twice_changes_nothing() {
        let once = augmented_path(Some(OsStr::new("/usr/bin")), Path::new("/Users/me"), false);
        let twice = augmented_path(Some(&once), Path::new("/Users/me"), false);
        assert_eq!(once, twice);
    }
}
