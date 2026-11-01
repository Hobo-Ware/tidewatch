//! Tells a launch whether the last run ended without a clean quit, and whether crashes come in a loop.

use std::path::Path;

use crate::fsutil::{remove_file_if_present, write_atomic};
use crate::paths::TidewatchPaths;

/// A run that crashed this soon after it started, found by a launch this soon after that start, is
/// taken as a crash loop. The window cannot tell when it crashed, but a launch within this long of
/// the crashed run's own start proves the run lived less than this.
pub const CRASH_LOOP_SECS: u64 = 5 * 60;

/// A run leaves `.state/running` behind from the moment it starts until its quit path removes it,
/// so a crash, a kill or a power cut leaves the file for the next launch to find. The file holds the
/// run's pid and its start in unix seconds. Returns whether the run before this one was a crash
/// that followed a launch by less than [`CRASH_LOOP_SECS`].
#[must_use]
pub fn begin(paths: &TidewatchPaths) -> bool {
    let now = crate::clock::now_secs();
    begin_at(&paths.run_marker_file(), now)
}

pub fn end(paths: &TidewatchPaths) {
    end_at(&paths.run_marker_file());
}

fn begin_at(file: &Path, now: u64) -> bool {
    let crash_loop = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| started_at(&text))
        .is_some_and(|started| now.saturating_sub(started) < CRASH_LOOP_SECS);
    if let Err(e) = write_atomic(file, &format!("{} {now}", std::process::id())) {
        eprintln!("tidewatch: cannot write {} ({e})", file.display());
    }
    crash_loop
}

fn started_at(marker: &str) -> Option<u64> {
    marker.split_whitespace().nth(1)?.parse().ok()
}

fn end_at(file: &Path) {
    if let Err(e) = remove_file_if_present(file) {
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
    fn a_first_launch_is_not_a_crash_and_leaves_its_marker_with_its_pid_and_start() {
        let (_home, paths) = home();
        assert!(!begin_at(&paths.run_marker_file(), 1_000));
        assert_eq!(
            std::fs::read_to_string(paths.run_marker_file()).unwrap(),
            format!("{} 1000", std::process::id())
        );
    }

    #[test]
    fn a_clean_quit_leaves_nothing_for_the_next_launch_to_find() {
        let (_home, paths) = home();
        let file = paths.run_marker_file();
        let _ = begin_at(&file, 1_000);
        end(&paths);
        assert!(!file.exists());
        assert!(!begin_at(&file, 1_010));
    }

    #[test]
    fn a_crash_is_a_loop_only_when_the_next_launch_follows_its_own_start_closely() {
        let cases = [
            (0, true),
            (60, true),
            (CRASH_LOOP_SECS - 1, true),
            (CRASH_LOOP_SECS, false),
            (3 * 3600, false),
        ];
        for (after, want) in cases {
            let (_home, paths) = home();
            let file = paths.run_marker_file();
            let _ = begin_at(&file, 1_000);
            assert_eq!(begin_at(&file, 1_000 + after), want, "relaunch {after}s after the start");
        }
    }

    #[test]
    fn a_marker_without_a_start_is_not_a_loop_and_a_clock_that_went_back_is() {
        let (_home, paths) = home();
        let file = paths.run_marker_file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "4242").unwrap();
        assert!(!begin_at(&file, 1_000), "a marker from before the start was recorded");
        std::fs::write(&file, "4242 garbage").unwrap();
        assert!(!begin_at(&file, 1_000));
        std::fs::write(&file, "4242 5000").unwrap();
        assert!(begin_at(&file, 1_000), "the clock moved back: treat it as soon");
    }

    #[test]
    fn a_run_that_never_quit_is_found_by_the_next_launch_and_only_that_one() {
        let (_home, paths) = home();
        let file = paths.run_marker_file();
        let _ = begin_at(&file, 1_000);
        assert!(begin_at(&file, 1_030), "the first run never removed its marker");
        end(&paths);
        assert!(!begin_at(&file, 1_060), "the second one quit cleanly");
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
