//! Saves one part of the settings file off the UI thread.

use std::thread::{self, JoinHandle};

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;

pub(crate) fn update_in_background(
    paths: TidewatchPaths,
    change: impl FnOnce(&mut Prefs) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if let Err(e) = Prefs::update(&paths, change) {
            eprintln!("tidewatch: cannot save settings.toml: {e}");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_update_changes_one_part_and_keeps_the_other() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        update_in_background(paths.clone(), |p| p.window.quake = false).join().unwrap();
        update_in_background(paths.clone(), |p| p.restore_session = false).join().unwrap();
        let prefs = Prefs::load(&paths);
        assert!(!prefs.restore_session);
        assert!(!prefs.window.quake, "the second save did not bring the first part back");
    }
}
