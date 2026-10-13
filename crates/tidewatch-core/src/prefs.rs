//! App-wide switches in `~/.config/tidewatch/settings.toml`. Today one: whether launch restores
//! the last session (PLAN.md, D11, on by default).
//!
//! A missing or bad file is the defaults, with a stderr note for a bad one: a settings file must
//! never stop the app.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "on")]
    pub restore_session: bool,
}

fn on() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self { restore_session: true }
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
        assert_eq!(Prefs::load(&paths), Prefs { restore_session: true });
    }

    #[test]
    fn an_off_switch_survives_a_save_and_load() {
        let (_home, paths) = paths();
        Prefs { restore_session: false }.save(&paths).unwrap();
        assert_eq!(Prefs::load(&paths), Prefs { restore_session: false });
        Prefs { restore_session: true }.save(&paths).unwrap();
        assert_eq!(Prefs::load(&paths), Prefs { restore_session: true });
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
}
