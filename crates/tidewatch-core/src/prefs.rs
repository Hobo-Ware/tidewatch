//! App-wide switches in `~/.config/tidewatch/settings.toml`: whether launch restores the last
//! session (PLAN.md, D11, on by default) and the `[window]` table (quake mode, its hotkey, the
//! menu-bar item).
//!
//! A missing or bad file is the defaults, with a stderr note for a bad one: a settings file must
//! never stop the app.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

/// `QUAKE_HOTKEY_DEFAULT` (PLAN.md): stdusk keeps `Ctrl+Grave`, so both apps can run at once.
pub const QUAKE_HOTKEY_DEFAULT: &str = "Ctrl+Alt+Grave";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "on")]
    pub restore_session: bool,
    #[serde(default)]
    pub window: WindowPrefs,
}

/// How the window behaves: a quake drop-down summoned by a global hotkey, or a plain window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPrefs {
    /// Drop-down from the top edge, summoned by the hotkey. Off is a plain window.
    #[serde(default = "on")]
    pub quake: bool,
    /// The global summon hotkey, such as `Ctrl+Alt+Grave`. Only read while `quake` is on.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// A menu-bar item with Show/Hide, Overview, Settings and Quit (macOS).
    #[serde(default = "on")]
    pub menu_bar: bool,
}

fn on() -> bool {
    true
}

fn default_hotkey() -> String {
    QUAKE_HOTKEY_DEFAULT.to_owned()
}

impl Default for Prefs {
    fn default() -> Self {
        Self { restore_session: true, window: WindowPrefs::default() }
    }
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self { quake: true, hotkey: default_hotkey(), menu_bar: true }
    }
}

impl Prefs {
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        Self::load_file(&paths.prefs_file())
    }

    fn load_file(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); using defaults", file.display());
                return Self::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: settings.toml is invalid ({e}); using defaults");
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.prefs_file(), &text)
    }

    /// Reads the file afresh, applies `change` and writes it back, so a writer holding an older copy
    /// (the restore switch and the window section are saved by different parts of the app) never
    /// undoes the other's change.
    pub fn update(paths: &TidewatchPaths, change: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let mut prefs = Self::load(paths);
        change(&mut prefs);
        prefs.save(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    #[test]
    fn restoring_is_on_without_a_file() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn an_off_switch_survives_a_save_and_load() {
        let (_home, paths) = paths();
        Prefs { restore_session: false, ..Prefs::default() }.save(&paths).unwrap();
        assert!(!Prefs::load(&paths).restore_session);
        Prefs { restore_session: true, ..Prefs::default() }.save(&paths).unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn an_empty_file_keeps_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "").unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn a_bad_file_is_the_defaults_and_stays_on_disk() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "restore_session = \"maybe\"").unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
        assert_eq!(
            std::fs::read_to_string(paths.prefs_file()).unwrap(),
            "restore_session = \"maybe\""
        );
    }

    #[test]
    fn the_window_defaults_are_quake_on_with_the_tidewatch_hotkey_and_the_menu_bar_item() {
        let window = Prefs::default().window;
        assert!(window.quake && window.menu_bar);
        assert_eq!(window.hotkey, "Ctrl+Alt+Grave");
        assert_ne!(window.hotkey, "Ctrl+Grave", "stdusk keeps that one");
    }

    #[test]
    fn a_file_from_before_the_window_section_gets_its_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "restore_session = false\n").unwrap();
        let prefs = Prefs::load(&paths);
        assert!(!prefs.restore_session);
        assert_eq!(prefs.window, WindowPrefs::default());
    }

    #[test]
    fn a_partial_window_table_keeps_the_rest_of_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\nquake = false\nhotkey = \"F13\"\n").unwrap();
        let window = Prefs::load(&paths).window;
        assert_eq!(window, WindowPrefs { quake: false, hotkey: "F13".to_owned(), menu_bar: true });
    }

    #[test]
    fn the_window_section_survives_a_save_and_load() {
        let (_home, paths) = paths();
        let window = WindowPrefs { quake: false, hotkey: "Cmd+F12".to_owned(), menu_bar: false };
        Prefs { window: window.clone(), ..Prefs::default() }.save(&paths).unwrap();
        assert_eq!(Prefs::load(&paths).window, window);
    }

    #[test]
    fn update_changes_one_part_and_keeps_the_other() {
        let (_home, paths) = paths();
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        Prefs::update(&paths, |p| p.restore_session = false).unwrap();
        let prefs = Prefs::load(&paths);
        assert!(!prefs.restore_session);
        assert!(!prefs.window.quake, "the restore switch did not bring the window section back");
    }
}
