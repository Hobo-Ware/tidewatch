//! Remembers the repos of the window used last, so a launch with no path (the Finder, or the
//! `open -n` an update restart runs) reopens them and session restore brings their tabs back.
//! A window is "used" when it gains focus and when it closes.

use tidewatch_core::last_workspace::{self, Remembered};
use tidewatch_core::paths::TidewatchPaths;

pub(crate) struct LastUsed {
    target: Option<TidewatchPaths>,
    focused: bool,
}

impl LastUsed {
    /// Without `paths` or with `enabled` false (a screenshot or background run) nothing is written.
    pub(crate) fn new(paths: Option<&TidewatchPaths>, enabled: bool) -> Self {
        Self { target: paths.filter(|_| enabled).cloned(), focused: false }
    }

    /// Once per drawn frame. Returns whether the window just gained focus, which is when to record.
    pub(crate) fn gained_focus(&mut self, focused: bool) -> bool {
        let gained = focused && !self.focused;
        self.focused = focused;
        gained
    }

    pub(crate) fn record(&self, remembered: &Remembered) {
        let Some(paths) = &self.target else { return };
        if let Err(e) = last_workspace::save(paths, remembered) {
            eprintln!("tidewatch: cannot save last-workspace.toml: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn web() -> Remembered {
        Remembered { dir: PathBuf::from("/work/web"), also: vec![PathBuf::from("/work/api")] }
    }

    fn api() -> Remembered {
        Remembered { dir: PathBuf::from("/work/api"), also: Vec::new() }
    }

    #[test]
    fn focus_is_gained_once_per_time_it_comes_back() {
        let mut last = LastUsed::new(None, true);
        assert!(!last.gained_focus(false));
        assert!(last.gained_focus(true));
        assert!(!last.gained_focus(true), "still focused");
        assert!(!last.gained_focus(false));
        assert!(last.gained_focus(true), "focus came back");
    }

    #[test]
    fn recording_writes_the_repos_and_the_next_one_replaces_them() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let last = LastUsed::new(Some(&paths), true);
        assert_eq!(last_workspace::load(&paths), None);
        last.record(&web());
        assert_eq!(last_workspace::load(&paths), Some(web()));
        last.record(&api());
        assert_eq!(last_workspace::load(&paths), Some(api()));
    }

    #[test]
    fn closing_records_the_repos_too() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        LastUsed::new(Some(&paths), true).record(&web());
        assert_eq!(last_workspace::load(&paths), Some(web()));
    }

    #[test]
    fn a_disabled_tracker_or_one_without_a_home_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        LastUsed::new(Some(&paths), false).record(&web());
        LastUsed::new(None, true).record(&web());
        assert!(!paths.last_workspace_file().exists());
    }
}
