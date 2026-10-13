//! Remembers the folder of the window used last, so a launch with no path (the Finder, or the
//! `open -n` an update restart runs) reopens it and session restore brings its tabs back.
//! A window is "used" when it gains focus and when it closes.

use std::path::{Path, PathBuf};

use tidewatch_core::last_workspace;
use tidewatch_core::paths::TidewatchPaths;

pub(crate) struct LastUsed {
    target: Option<(TidewatchPaths, PathBuf)>,
    focused: bool,
}

impl LastUsed {
    /// Without `paths` or with `enabled` false (a screenshot or background run) nothing is written.
    pub(crate) fn new(paths: Option<&TidewatchPaths>, dir: &Path, enabled: bool) -> Self {
        let target = paths.filter(|_| enabled).map(|paths| (paths.clone(), dir.to_path_buf()));
        Self { target, focused: false }
    }

    /// Once per drawn frame. Returns whether the window just gained focus and was recorded.
    pub(crate) fn tick(&mut self, focused: bool) -> bool {
        let gained = focused && !self.focused;
        self.focused = focused;
        if gained {
            self.record();
        }
        gained
    }

    pub(crate) fn record(&self) {
        let Some((paths, dir)) = &self.target else { return };
        if let Err(e) = last_workspace::save(paths, dir) {
            eprintln!("tidewatch: cannot save last-workspace.toml: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaining_focus_records_the_folder_and_staying_focused_does_not_write_again() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut last = LastUsed::new(Some(&paths), Path::new("/work/web"), true);
        assert!(!last.tick(false));
        assert_eq!(last_workspace::load(&paths), None);

        assert!(last.tick(true));
        assert_eq!(last_workspace::load(&paths), Some(PathBuf::from("/work/web")));

        last_workspace::save(&paths, Path::new("/work/api")).unwrap();
        assert!(!last.tick(true), "still focused");
        assert_eq!(last_workspace::load(&paths), Some(PathBuf::from("/work/api")));

        assert!(!last.tick(false));
        assert!(last.tick(true), "focus came back");
        assert_eq!(last_workspace::load(&paths), Some(PathBuf::from("/work/web")));
    }

    #[test]
    fn closing_records_the_folder_too() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        LastUsed::new(Some(&paths), Path::new("/work/web"), true).record();
        assert_eq!(last_workspace::load(&paths), Some(PathBuf::from("/work/web")));
    }

    #[test]
    fn a_disabled_tracker_or_one_without_a_home_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut off = LastUsed::new(Some(&paths), Path::new("/work/web"), false);
        assert!(off.tick(true));
        off.record();
        let mut homeless = LastUsed::new(None, Path::new("/work/web"), true);
        homeless.tick(true);
        homeless.record();
        assert!(!paths.last_workspace_file().exists());
    }
}
