//! App-wide switches in `~/.config/tidewatch/settings.toml`: whether launch restores the last
//! session (PLAN.md, D11, on by default) and how it brings agents back (`resume_agents`), the
//! `[window]` table (quake mode, its hotkey, the
//! menu-bar item, the panel layout), the `[updates]` table (check for releases, install them) and
//! the `[terminal]` table (the terminal's font).
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
    pub resume_agents: ResumeAgents,
    #[serde(default)]
    pub window: WindowPrefs,
    #[serde(default)]
    pub updates: UpdatePrefs,
    #[serde(default)]
    pub terminal: TerminalPrefs,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPrefs {
    #[serde(default)]
    pub font: FontChoice,
}

/// The terminal's font: an empty string is the bundled face, anything else names an installed
/// family (stdusk's `appearance.font` works the same way).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum FontChoice {
    #[default]
    BuiltIn,
    Family(String),
}

impl From<String> for FontChoice {
    fn from(text: String) -> Self {
        match text.trim() {
            "" => Self::BuiltIn,
            family => Self::Family(family.to_owned()),
        }
    }
}

impl From<FontChoice> for String {
    fn from(choice: FontChoice) -> Self {
        match choice {
            FontChoice::BuiltIn => Self::new(),
            FontChoice::Family(family) => family,
        }
    }
}

/// What a restored agent tab does: `auto` runs its resume command, `prefill` types it and leaves
/// Enter to the user, `off` reopens a plain shell. Only used while `restore_session` is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeAgents {
    #[default]
    Auto,
    Prefill,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePrefs {
    #[serde(default = "on")]
    pub check: bool,
    #[serde(default = "on")]
    pub install: bool,
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
    /// Where History and the diff sit around the terminal.
    #[serde(default)]
    pub panels: Panels,
}

/// The tab layout: History beside the terminal, or the History and Diff panes under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panels {
    #[default]
    Side,
    Below,
}

fn on() -> bool {
    true
}

fn default_hotkey() -> String {
    QUAKE_HOTKEY_DEFAULT.to_owned()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            restore_session: true,
            resume_agents: ResumeAgents::default(),
            window: WindowPrefs::default(),
            updates: UpdatePrefs::default(),
            terminal: TerminalPrefs::default(),
        }
    }
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self { check: true, install: true }
    }
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self { quake: true, hotkey: default_hotkey(), menu_bar: true, panels: Panels::Side }
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
    fn the_terminal_font_is_built_in_when_empty_and_a_family_otherwise() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::default().terminal.font, FontChoice::BuiltIn);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        for (text, want) in [
            ("", FontChoice::BuiltIn),
            ("  ", FontChoice::BuiltIn),
            ("  MesloLGS NF ", FontChoice::Family("MesloLGS NF".to_owned())),
        ] {
            std::fs::write(paths.prefs_file(), format!("[terminal]\nfont = \"{text}\"\n")).unwrap();
            assert_eq!(Prefs::load(&paths).terminal.font, want, "{text:?}");
        }
        let family = FontChoice::Family("JetBrains Mono".to_owned());
        Prefs::update(&paths, |p| p.terminal.font = family.clone()).unwrap();
        let text = std::fs::read_to_string(paths.prefs_file()).unwrap();
        assert!(text.contains("[terminal]\nfont = \"JetBrains Mono\""), "{text}");
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        assert_eq!(Prefs::load(&paths).terminal.font, family);
    }

    #[test]
    fn a_file_from_before_the_terminal_table_keeps_the_built_in_font() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\nquake = false\n").unwrap();
        assert_eq!(Prefs::load(&paths).terminal.font, FontChoice::BuiltIn);
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
    fn resume_agents_defaults_to_auto_and_reads_every_mode_from_the_file() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::default().resume_agents, ResumeAgents::Auto);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        for (text, want) in [
            ("auto", ResumeAgents::Auto),
            ("prefill", ResumeAgents::Prefill),
            ("off", ResumeAgents::Off),
        ] {
            std::fs::write(paths.prefs_file(), format!("resume_agents = \"{text}\"\n")).unwrap();
            let prefs = Prefs::load(&paths);
            assert_eq!(prefs.resume_agents, want, "{text}");
            assert!(prefs.restore_session, "{text}");
        }
    }

    #[test]
    fn a_file_from_before_resume_agents_resumes_automatically() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "restore_session = false\n").unwrap();
        assert_eq!(Prefs::load(&paths).resume_agents, ResumeAgents::Auto);
    }

    #[test]
    fn an_unknown_resume_mode_makes_the_whole_file_fall_back_to_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "resume_agents = \"sometimes\"\n").unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn the_resume_mode_survives_a_save_and_an_update_of_another_part() {
        let (_home, paths) = paths();
        Prefs::update(&paths, |p| p.resume_agents = ResumeAgents::Prefill).unwrap();
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        let prefs = Prefs::load(&paths);
        assert_eq!(prefs.resume_agents, ResumeAgents::Prefill);
        assert!(!prefs.window.quake);
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
        assert_eq!(
            window,
            WindowPrefs {
                quake: false,
                hotkey: "F13".to_owned(),
                menu_bar: true,
                panels: Panels::Side
            }
        );
    }

    #[test]
    fn panels_default_to_side_by_side_and_read_below_from_the_file() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::default().window.panels, Panels::Side);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\npanels = \"below\"\n").unwrap();
        let window = Prefs::load(&paths).window;
        assert_eq!(window.panels, Panels::Below);
        assert!(window.quake);
    }

    #[test]
    fn an_unknown_panels_value_makes_the_whole_file_fall_back_to_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\npanels = \"left\"\n").unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn the_window_section_survives_a_save_and_load() {
        let (_home, paths) = paths();
        let window = WindowPrefs {
            quake: false,
            hotkey: "Cmd+F12".to_owned(),
            menu_bar: false,
            panels: Panels::Below,
        };
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

    #[test]
    fn updates_check_and_install_by_default() {
        assert_eq!(Prefs::default().updates, UpdatePrefs { check: true, install: true });
    }

    #[test]
    fn a_file_from_before_the_updates_table_gets_its_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\nquake = false\n").unwrap();
        assert_eq!(Prefs::load(&paths).updates, UpdatePrefs::default());
    }

    #[test]
    fn a_partial_updates_table_keeps_the_rest_of_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[updates]\ninstall = false\n").unwrap();
        assert_eq!(Prefs::load(&paths).updates, UpdatePrefs { check: true, install: false });
    }

    #[test]
    fn the_updates_table_survives_a_save_and_load_and_leaves_the_rest_alone() {
        let (_home, paths) = paths();
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        Prefs::update(&paths, |p| p.updates.check = false).unwrap();
        let prefs = Prefs::load(&paths);
        assert_eq!(prefs.updates, UpdatePrefs { check: false, install: true });
        assert!(!prefs.window.quake);
        assert!(prefs.restore_session);
    }
}
