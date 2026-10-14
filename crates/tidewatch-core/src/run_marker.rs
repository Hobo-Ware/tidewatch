//! Tells a launch whether the last run ended without a clean quit.

use std::path::Path;

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

/// A run leaves `.state/running` behind from the moment it starts until its quit path removes it,
/// so a crash, a kill or a power cut leaves the file for the next launch to find.
#[must_use]
pub fn begin(paths: &TidewatchPaths) -> bool {
    begin_at(&paths.run_marker_file())
}

pub fn end(paths: &TidewatchPaths) {
    end_at(&paths.run_marker_file());
}

fn begin_at(file: &Path) -> bool {
    let left_behind = file.exists();
    if let Err(e) = write_atomic(file, &std::process::id().to_string()) {
        eprintln!("tidewatch: cannot write {} ({e})", file.display());
    }
    left_behind
}

fn end_at(file: &Path) {
    if let Err(e) = std::fs::remove_file(file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("tidewatch: cannot remove {} ({e})", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    #[test]
    fn a_first_launch_is_not_a_crash_and_leaves_its_marker() {
        let (_home, paths) = home();
        assert!(!begin(&paths));
        assert_eq!(
            std::fs::read_to_string(paths.run_marker_file()).unwrap(),
            std::process::id().to_string()
        );
    }

    #[test]
    fn a_clean_quit_leaves_nothing_for_the_next_launch_to_find() {
        let (_home, paths) = home();
        let _ = begin(&paths);
        end(&paths);
        assert!(!paths.run_marker_file().exists());
        assert!(!begin(&paths));
    }

    #[test]
    fn a_run_that_never_quit_is_found_by_the_next_launch_and_only_that_one() {
        let (_home, paths) = home();
        let _ = begin(&paths);
        assert!(begin(&paths), "the first run never removed its marker");
        end(&paths);
        assert!(!begin(&paths), "the second one quit cleanly");
    }

    #[test]
    fn quitting_without_a_marker_is_not_an_error() {
        let (_home, paths) = home();
        end(&paths);
        assert!(!paths.run_marker_file().exists());
    }

    #[test]
    fn an_unwritable_state_folder_reads_as_a_clean_start() {
        let home = tempfile::tempdir().unwrap();
        let blocker = home.path().join(".tidewatch");
        std::fs::write(&blocker, "a file where the folder should be").unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert!(!begin(&paths));
    }
}
