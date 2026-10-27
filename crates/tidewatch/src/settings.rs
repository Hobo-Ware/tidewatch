//! What the settings view holds: the profile and window editors and which page shows.
//! Pure state; `settings_view` draws it and the app saves what `Save` builds.

use tidewatch_core::profiles::Profiles;

use tidewatch_core::prefs::WindowPrefs;

use crate::font_view;
use crate::login_item;
use crate::profile_editor;
use crate::setup_editor;
use crate::sync_view;
use crate::window_settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    General,
    Profiles,
    Trust,
    Shortcuts,
    HouseStyle,
    WorktreeSetup,
}

pub(crate) struct Settings {
    pub(crate) section: Section,
    pub(crate) profiles: profile_editor::Editor,
    pub(crate) window: window_settings::Editor,
    pub(crate) font: font_view::Picker,
    pub(crate) login: login_item::Row,
    pub(crate) setup: setup_editor::Editor,
    pub(crate) sync: sync_view::Editor,
}

impl Settings {
    /// `agents` are `(id, display name)` of every agent in the table.
    pub(crate) fn new(
        profiles: &Profiles,
        agents: Vec<(String, String)>,
        window: &WindowPrefs,
    ) -> Self {
        Self {
            section: Section::Profiles,
            profiles: profile_editor::Editor::new(profiles, agents),
            window: window_settings::Editor::new(window),
            font: font_view::Picker::default(),
            login: login_item::Row::default(),
            setup: setup_editor::Editor::default(),
            sync: sync_view::Editor::default(),
        }
    }

    pub(crate) fn sections(&self) -> [(Section, &'static str, bool); 6] {
        [
            (Section::General, "General", false),
            (Section::Profiles, "Agents and profiles", self.profiles.dirty),
            (Section::Trust, "Folder trust", false),
            (Section::Shortcuts, "Shortcuts", false),
            (Section::HouseStyle, "House style", false),
            (Section::WorktreeSetup, "Worktree setup", false),
        ]
    }

    /// Profiles were replaced on disk by a sync. A page with unsaved edits keeps its drafts; a page
    /// without any shows what arrived.
    pub(crate) fn pulled(&mut self, profiles: &Profiles, agents: Vec<(String, String)>) {
        if !self.profiles.dirty {
            let selected = self.profiles.selected;
            self.profiles = profile_editor::Editor::new(profiles, agents);
            self.profiles.selected = selected.min(self.profiles.drafts.len().saturating_sub(1));
        }
    }

    pub(crate) fn profiles_saved(&mut self, profiles: &Profiles) {
        self.profiles.saved(profiles.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let agents = vec![("claude".to_owned(), "Claude Code".to_owned())];
        Settings::new(&profiles, agents, &WindowPrefs::default())
    }

    fn pulled_profiles() -> Profiles {
        let mut profiles = Profiles::seeded(&["claude".to_owned()]);
        profiles.profile.remove("reviewer");
        profiles
    }

    fn agents() -> Vec<(String, String)> {
        vec![("claude".to_owned(), "Claude Code".to_owned())]
    }

    #[test]
    fn a_pull_replaces_the_page_nobody_edited() {
        let mut settings = settings();
        let profiles = pulled_profiles();
        settings.pulled(&profiles, agents());
        assert_eq!(settings.profiles.drafts.len(), profiles.profile.len());
        assert!(!settings.profiles.dirty);
    }

    #[test]
    fn a_pull_never_throws_away_a_page_with_unsaved_edits() {
        let mut settings = settings();
        let before = settings.profiles.drafts.len();
        settings.profiles.touched();
        settings.pulled(&pulled_profiles(), agents());
        assert_eq!(settings.profiles.drafts.len(), before);
        assert!(settings.profiles.dirty);
    }

    #[test]
    fn the_selection_stays_in_range_when_the_pull_has_fewer_entries() {
        let mut settings = settings();
        settings.profiles.selected = 1;
        settings.pulled(&pulled_profiles(), agents());
        assert_eq!(settings.profiles.selected, 0);
    }

    #[test]
    fn settings_open_on_profiles_with_nothing_unsaved() {
        let settings = settings();
        assert_eq!(settings.section, Section::Profiles);
        assert_eq!(
            settings.sections(),
            [
                (Section::General, "General", false),
                (Section::Profiles, "Agents and profiles", false),
                (Section::Trust, "Folder trust", false),
                (Section::Shortcuts, "Shortcuts", false),
                (Section::HouseStyle, "House style", false),
                (Section::WorktreeSetup, "Worktree setup", false),
            ]
        );
        assert!(!settings.profiles.dirty);
    }

    #[test]
    fn an_item_is_marked_only_while_its_own_page_has_unsaved_edits() {
        let mut settings = settings();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false; 6]);
        settings.profiles.touched();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false, true, false, false, false, false]);
    }
}
