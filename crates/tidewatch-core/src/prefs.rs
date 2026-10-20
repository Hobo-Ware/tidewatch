//! App-wide switches in `~/.config/tidewatch/settings.toml`: whether launch restores the last
//! session (PLAN.md, D11, on by default) and how it brings agents back (`resume_agents`), the
//! `[window]` table (quake mode, its hotkey, the
//! menu-bar item, the panel layout), the `[updates]` table (check for releases, install them) and
//! the `[github]` table (what a new tab does when its pull request changes) and the `[new_tab]`
//! table (what the new-tab form opens with) and the `[tabs]` table (when a quiet tab is parked).
//!
//! A missing or bad file is the defaults, with a stderr note for a bad one: a settings file must
//! never stop the app.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};

use crate::agents::Level;
use crate::fsutil::write_atomic;
use crate::meeseeks::Criteria;
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
    pub github: GithubPrefs,
    #[serde(default)]
    pub new_tab: NewTabPrefs,
    #[serde(default)]
    pub tabs: TabsPrefs,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParkAfter {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "1h")]
    Hour,
    #[default]
    #[serde(rename = "4h")]
    FourHours,
    #[serde(rename = "8h")]
    EightHours,
    #[serde(rename = "24h")]
    Day,
}

impl ParkAfter {
    pub const ALL: [Self; 5] =
        [Self::Off, Self::Hour, Self::FourHours, Self::EightHours, Self::Day];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Hour => "1 h",
            Self::FourHours => "4 h",
            Self::EightHours => "8 h",
            Self::Day => "24 h",
        }
    }

    #[must_use]
    pub fn duration(self) -> Option<Duration> {
        match self {
            Self::Off => None,
            Self::Hour => Some(Duration::from_hours(1)),
            Self::FourHours => Some(Duration::from_hours(4)),
            Self::EightHours => Some(Duration::from_hours(8)),
            Self::Day => Some(Duration::from_hours(24)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabsPrefs {
    #[serde(default)]
    pub park_after: ParkAfter,
}

/// Where a new tab's worktree is cut from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartFrom {
    /// The branch the repo has checked out.
    Current,
    /// The remote's default branch, fetched fresh: `origin/HEAD` names it, else `main` or `master`.
    #[default]
    DefaultBranch,
}

/// Which side of an agent tab's right card shows first once the agent keeps a ledger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RightPanel {
    #[default]
    Progress,
    History,
}

/// What the new-tab form opens with. The form still changes every one of them per tab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTabPrefs {
    /// A `profiles.toml` key. Without one the form opens on the profile used last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Without one the form opens at the profile's own level. YOLO cannot be a default: a file
    /// that names it is read as `edits`.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "default_level")]
    pub permission: Option<Level>,
    #[serde(default)]
    pub remote: bool,
    /// Only what the person picked in Settings is stored. Earlier versions wrote `start_from =
    /// "current"` for everyone whenever any setting was saved, so that key is not read: it cannot
    /// say who chose it. Without a choice the default applies.
    #[serde(default, rename = "start_base", skip_serializing_if = "Option::is_none")]
    pub start: Option<StartFrom>,
    #[serde(default)]
    pub right_panel: RightPanel,
    /// The ticks of the Meeseeks form's "Done means", kept from the last summon.
    #[serde(default, skip_serializing_if = "Criteria::is_default")]
    pub meeseeks: Criteria,
}

impl NewTabPrefs {
    #[must_use]
    pub fn start_from(&self) -> StartFrom {
        self.start.unwrap_or_default()
    }
}

fn default_level<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Level>, D::Error> {
    let level = Option::<Level>::deserialize(deserializer)?;
    Ok(level.map(|level| {
        if level == Level::Yolo {
            eprintln!("tidewatch: settings.toml [new_tab] permission cannot be yolo; using edits");
            Level::Edits
        } else {
            level
        }
    }))
}

/// What a tab does when GitHub reports something about its pull request: `ask` shows a card with
/// one button, `auto` sends the same message itself when the agent is idle, `off` stops looking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrMode {
    #[default]
    Ask,
    Auto,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPrefs {
    #[serde(default)]
    pub mode: PrMode,
    #[serde(default = "on")]
    pub failed_checks: bool,
    #[serde(default = "on")]
    pub reviews: bool,
    #[serde(default = "on")]
    pub conflicts: bool,
}

impl Default for GithubPrefs {
    fn default() -> Self {
        Self { mode: PrMode::Ask, failed_checks: true, reviews: true, conflicts: true }
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
            github: GithubPrefs::default(),
            new_tab: NewTabPrefs::default(),
            tabs: TabsPrefs::default(),
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
    fn github_asks_first_with_every_trigger_on_by_default() {
        let github = Prefs::default().github;
        assert_eq!(
            github,
            GithubPrefs { mode: PrMode::Ask, failed_checks: true, reviews: true, conflicts: true }
        );
    }

    #[test]
    fn a_file_from_before_the_github_table_gets_its_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[window]\nquake = false\n").unwrap();
        assert_eq!(Prefs::load(&paths).github, GithubPrefs::default());
    }

    #[test]
    fn every_github_mode_reads_from_the_file_and_a_partial_table_keeps_the_rest() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        for (text, want) in [("ask", PrMode::Ask), ("auto", PrMode::Auto), ("off", PrMode::Off)] {
            std::fs::write(
                paths.prefs_file(),
                format!("[github]\nmode = \"{text}\"\nreviews = false\n"),
            )
            .unwrap();
            assert_eq!(
                Prefs::load(&paths).github,
                GithubPrefs { mode: want, failed_checks: true, reviews: false, conflicts: true },
                "{text}"
            );
        }
    }

    #[test]
    fn an_unknown_github_mode_makes_the_whole_file_fall_back_to_the_defaults() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), "[github]\nmode = \"sometimes\"\n").unwrap();
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }

    #[test]
    fn the_github_table_survives_an_update_of_another_part() {
        let (_home, paths) = paths();
        let github =
            GithubPrefs { mode: PrMode::Auto, failed_checks: false, ..GithubPrefs::default() };
        Prefs::update(&paths, |p| p.github = github).unwrap();
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        let prefs = Prefs::load(&paths);
        assert_eq!(prefs.github, github);
        assert!(!prefs.window.quake);
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

    fn write_prefs(paths: &TidewatchPaths, text: &str) {
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.prefs_file(), text).unwrap();
    }

    #[test]
    fn new_tabs_open_on_the_last_profile_at_its_own_level_with_remote_off_without_a_file() {
        let (_home, paths) = paths();
        assert_eq!(
            Prefs::load(&paths).new_tab,
            NewTabPrefs {
                agent: None,
                permission: None,
                remote: false,
                start: None,
                right_panel: RightPanel::Progress,
                meeseeks: Criteria { tests: true, committed: true, pr: true },
            }
        );
        assert_eq!(Prefs::load(&paths).new_tab.start_from(), StartFrom::DefaultBranch);
    }

    #[test]
    fn every_new_tab_key_is_read_from_the_file() {
        let (_home, paths) = paths();
        let cases = [
            (
                "agent = \"reviewer\"",
                NewTabPrefs { agent: Some("reviewer".into()), ..NewTabPrefs::default() },
            ),
            (
                "permission = \"plan\"",
                NewTabPrefs { permission: Some(Level::Plan), ..NewTabPrefs::default() },
            ),
            (
                "permission = \"ask\"",
                NewTabPrefs { permission: Some(Level::Ask), ..NewTabPrefs::default() },
            ),
            (
                "permission = \"edits\"",
                NewTabPrefs { permission: Some(Level::Edits), ..NewTabPrefs::default() },
            ),
            (
                "permission = \"auto\"",
                NewTabPrefs { permission: Some(Level::Auto), ..NewTabPrefs::default() },
            ),
            ("remote = true", NewTabPrefs { remote: true, ..NewTabPrefs::default() }),
            (
                "start_base = \"current\"",
                NewTabPrefs { start: Some(StartFrom::Current), ..NewTabPrefs::default() },
            ),
            (
                "start_base = \"default_branch\"",
                NewTabPrefs { start: Some(StartFrom::DefaultBranch), ..NewTabPrefs::default() },
            ),
            ("right_panel = \"progress\"", NewTabPrefs::default()),
            (
                "right_panel = \"history\"",
                NewTabPrefs { right_panel: RightPanel::History, ..NewTabPrefs::default() },
            ),
        ];
        for (line, want) in cases {
            write_prefs(&paths, &format!("[new_tab]\n{line}\n"));
            assert_eq!(Prefs::load(&paths).new_tab, want, "{line}");
        }
    }

    #[test]
    fn the_start_branch_is_the_remote_default_until_one_is_picked_and_old_files_do_not_pick() {
        let (_home, paths) = paths();
        let cases = [
            ("", StartFrom::DefaultBranch, None),
            ("start_from = \"current\"\n", StartFrom::DefaultBranch, None),
            ("start_from = \"default_branch\"\n", StartFrom::DefaultBranch, None),
            (
                "start_from = \"current\"\nstart_base = \"current\"\n",
                StartFrom::Current,
                Some(StartFrom::Current),
            ),
            (
                "start_base = \"default_branch\"\n",
                StartFrom::DefaultBranch,
                Some(StartFrom::DefaultBranch),
            ),
        ];
        for (lines, want, stored) in cases {
            write_prefs(&paths, &format!("[new_tab]\nremote = true\n{lines}"));
            let new_tab = Prefs::load(&paths).new_tab;
            assert_eq!((new_tab.start_from(), new_tab.start), (want, stored), "{lines:?}");
        }
    }

    #[test]
    fn saving_with_no_pick_writes_no_start_key_and_a_pick_is_kept() {
        let (_home, paths) = paths();
        write_prefs(&paths, "[new_tab]\nstart_from = \"current\"\n");
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        let text = std::fs::read_to_string(paths.prefs_file()).unwrap();
        assert!(!text.contains("start_"), "{text}");
        Prefs::update(&paths, |p| p.new_tab.start = Some(StartFrom::Current)).unwrap();
        assert!(
            std::fs::read_to_string(paths.prefs_file())
                .unwrap()
                .contains("start_base = \"current\"")
        );
        assert_eq!(Prefs::load(&paths).new_tab.start_from(), StartFrom::Current);
    }

    #[test]
    fn a_yolo_default_is_read_as_auto_edit() {
        let (_home, paths) = paths();
        write_prefs(&paths, "[new_tab]\npermission = \"yolo\"\nremote = true\n");
        let new_tab = Prefs::load(&paths).new_tab;
        assert_eq!(new_tab.permission, Some(Level::Edits));
        assert!(new_tab.remote, "the rest of the table is kept");
    }

    #[test]
    fn a_bad_new_tab_value_makes_the_whole_file_fall_back_to_the_defaults() {
        let (_home, paths) = paths();
        for bad in [
            "permission = \"sudo\"",
            "start_base = \"origin\"",
            "right_panel = \"left\"",
            "remote = \"yes\"",
            "agent = 3",
        ] {
            write_prefs(&paths, &format!("restore_session = false\n[new_tab]\n{bad}\n"));
            assert_eq!(Prefs::load(&paths), Prefs::default(), "{bad}");
        }
    }

    #[test]
    fn the_new_tab_table_survives_a_save_and_an_update_of_another_part() {
        let (_home, paths) = paths();
        let wanted = NewTabPrefs {
            agent: Some("builder".into()),
            permission: Some(Level::Auto),
            remote: true,
            start: Some(StartFrom::Current),
            right_panel: RightPanel::History,
            meeseeks: Criteria { tests: false, committed: true, pr: false },
        };
        Prefs::update(&paths, |p| p.new_tab = wanted.clone()).unwrap();
        Prefs::update(&paths, |p| p.window.quake = false).unwrap();
        assert_eq!(Prefs::load(&paths).new_tab, wanted);
        Prefs::update(&paths, |p| p.new_tab = NewTabPrefs::default()).unwrap();
        let text = std::fs::read_to_string(paths.prefs_file()).unwrap();
        assert!(!text.contains("agent =") && !text.contains("permission ="), "{text}");
    }

    #[test]
    fn the_meeseeks_ticks_start_on_are_kept_when_changed_and_leave_no_trace_when_back_on() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::load(&paths).new_tab.meeseeks, Criteria::default());
        let off = Criteria { tests: true, committed: false, pr: false };
        Prefs::update(&paths, |p| p.new_tab.meeseeks = off).unwrap();
        let text = std::fs::read_to_string(paths.prefs_file()).unwrap();
        assert!(text.contains("[new_tab.meeseeks]") && text.contains("pr = false"), "{text}");
        assert_eq!(Prefs::load(&paths).new_tab.meeseeks, off);
        write_prefs(&paths, "[new_tab.meeseeks]\npr = false\n");
        let partial = Prefs::load(&paths).new_tab.meeseeks;
        assert_eq!(partial, Criteria { tests: true, committed: true, pr: false });
        Prefs::update(&paths, |p| p.new_tab.meeseeks = Criteria::default()).unwrap();
        let text = std::fs::read_to_string(paths.prefs_file()).unwrap();
        assert!(!text.contains("meeseeks"), "{text}");
    }

    #[test]
    fn a_file_from_before_the_new_tab_table_gets_its_defaults() {
        let (_home, paths) = paths();
        write_prefs(&paths, "restore_session = false\n[github]\nmode = \"off\"\n");
        assert_eq!(Prefs::load(&paths).new_tab, NewTabPrefs::default());
    }

    #[test]
    fn quiet_tabs_park_after_four_hours_until_a_choice_and_old_files_get_that() {
        let (_home, paths) = paths();
        assert_eq!(Prefs::load(&paths).tabs.park_after, ParkAfter::FourHours);
        write_prefs(&paths, "restore_session = false\n");
        assert_eq!(Prefs::load(&paths).tabs, TabsPrefs::default());
    }

    #[test]
    fn every_park_after_choice_reads_from_the_file_and_survives_a_save() {
        let (_home, paths) = paths();
        let cases = [
            ("off", ParkAfter::Off, None),
            ("1h", ParkAfter::Hour, Some(Duration::from_hours(1))),
            ("4h", ParkAfter::FourHours, Some(Duration::from_hours(4))),
            ("8h", ParkAfter::EightHours, Some(Duration::from_hours(8))),
            ("24h", ParkAfter::Day, Some(Duration::from_hours(24))),
        ];
        for (word, want, after) in cases {
            write_prefs(&paths, &format!("[tabs]\npark_after = \"{word}\"\n"));
            let loaded = Prefs::load(&paths).tabs.park_after;
            assert_eq!(loaded, want, "{word}");
            assert_eq!(loaded.duration(), after, "{word}");
            Prefs::update(&paths, |p| p.window.quake = false).unwrap();
            assert_eq!(Prefs::load(&paths).tabs.park_after, want, "{word} after a save");
        }
        assert_eq!(ParkAfter::ALL.map(ParkAfter::label), ["Off", "1 h", "4 h", "8 h", "24 h"]);
    }

    #[test]
    fn an_unknown_park_after_makes_the_whole_file_fall_back_to_the_defaults() {
        let (_home, paths) = paths();
        write_prefs(&paths, "restore_session = false\n[tabs]\npark_after = \"2h\"\n");
        assert_eq!(Prefs::load(&paths), Prefs::default());
    }
}
