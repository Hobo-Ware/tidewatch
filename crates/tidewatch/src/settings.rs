//! What the settings view holds: the profile, workflow and window editors and which page shows.
//! Pure state; `settings_view` draws it and the app saves what `Save` builds.

use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use tidewatch_core::prefs::WindowPrefs;

use crate::font_view;
use crate::login_item;
use crate::profile_editor;
use crate::setup_editor;
use crate::sync_view;
use crate::window_settings;
use crate::workflow_editor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    General,
    Profiles,
    Workflows,
    Trust,
    Shortcuts,
    HouseStyle,
    WorktreeSetup,
}

pub(crate) struct Settings {
    pub(crate) section: Section,
    pub(crate) profiles: profile_editor::Editor,
    pub(crate) workflows: workflow_editor::Editor,
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
        workflows: &Workflows,
        agents: Vec<(String, String)>,
        window: &WindowPrefs,
    ) -> Self {
        let profile_names =
            profiles.profile.iter().map(|(id, p)| (id.clone(), p.name.clone())).collect();
        Self {
            section: Section::Profiles,
            workflows: workflow_editor::Editor::new(workflows, agents.clone(), profile_names),
            profiles: profile_editor::Editor::new(profiles, agents),
            window: window_settings::Editor::new(window),
            font: font_view::Picker::default(),
            login: login_item::Row::default(),
            setup: setup_editor::Editor::default(),
            sync: sync_view::Editor::default(),
        }
    }

    pub(crate) fn sections(&self) -> [(Section, &'static str, bool); 7] {
        [
            (Section::General, "General", false),
            (Section::Profiles, "Agents and profiles", self.profiles.dirty),
            (Section::Workflows, "Workflows", self.workflows.dirty),
            (Section::Trust, "Folder trust", false),
            (Section::Shortcuts, "Shortcuts", false),
            (Section::HouseStyle, "House style", false),
            (Section::WorktreeSetup, "Worktree setup", false),
        ]
    }

    /// Profiles and workflows were replaced on disk by a sync. A page with unsaved edits keeps its
    /// drafts; a page without any shows what arrived.
    pub(crate) fn pulled(
        &mut self,
        profiles: &Profiles,
        workflows: &Workflows,
        agents: Vec<(String, String)>,
    ) {
        let names: Vec<(String, String)> =
            profiles.profile.iter().map(|(id, p)| (id.clone(), p.name.clone())).collect();
        if !self.profiles.dirty {
            let selected = self.profiles.selected;
            self.profiles = profile_editor::Editor::new(profiles, agents.clone());
            self.profiles.selected = selected.min(self.profiles.drafts.len().saturating_sub(1));
        }
        if self.workflows.dirty {
            self.workflows.set_profiles(names);
        } else {
            let (selected, stage) = (self.workflows.selected, self.workflows.stage);
            self.workflows = workflow_editor::Editor::new(workflows, agents, names);
            self.workflows.selected = selected.min(self.workflows.drafts.len().saturating_sub(1));
            self.workflows.stage = stage;
        }
    }

    /// The names of the profiles, for a workflow stage that runs one. Called after profiles were saved.
    pub(crate) fn profiles_saved(&mut self, profiles: &Profiles) {
        self.profiles.saved(profiles.clone());
        self.workflows.set_profiles(
            profiles.profile.iter().map(|(id, p)| (id.clone(), p.name.clone())).collect(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let agents = vec![("claude".to_owned(), "Claude Code".to_owned())];
        Settings::new(&profiles, &Workflows::defaults(), agents, &WindowPrefs::default())
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
    fn a_pull_replaces_the_pages_nobody_edited() {
        let mut settings = settings();
        let profiles = pulled_profiles();
        let mut workflows = Workflows::defaults();
        workflows.workflow.remove(Workflows::defaults().workflow.keys().next().unwrap());
        settings.pulled(&profiles, &workflows, agents());
        assert_eq!(settings.profiles.drafts.len(), profiles.profile.len());
        assert_eq!(settings.workflows.drafts.len(), workflows.workflow.len());
        assert!(!settings.profiles.dirty && !settings.workflows.dirty);
        let names: Vec<&str> =
            settings.workflows.profiles().iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["builder"]);
    }

    #[test]
    fn a_pull_never_throws_away_a_page_with_unsaved_edits() {
        let mut settings = settings();
        let before = settings.profiles.drafts.len();
        settings.profiles.touched();
        settings.workflows.touched();
        let workflows_before = settings.workflows.drafts.len();
        let mut workflows = Workflows::defaults();
        workflows.workflow.clear();
        settings.pulled(&pulled_profiles(), &workflows, agents());
        assert_eq!(settings.profiles.drafts.len(), before);
        assert_eq!(settings.workflows.drafts.len(), workflows_before);
        assert!(settings.profiles.dirty && settings.workflows.dirty);
        let names: Vec<&str> =
            settings.workflows.profiles().iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["builder"], "stage pickers follow the profiles that arrived");
    }

    #[test]
    fn the_selection_stays_in_range_when_the_pull_has_fewer_entries() {
        let mut settings = settings();
        settings.profiles.selected = 1;
        settings.pulled(&pulled_profiles(), &Workflows::defaults(), agents());
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
                (Section::Workflows, "Workflows", false),
                (Section::Trust, "Folder trust", false),
                (Section::Shortcuts, "Shortcuts", false),
                (Section::HouseStyle, "House style", false),
                (Section::WorktreeSetup, "Worktree setup", false),
            ]
        );
        assert!(!settings.profiles.dirty && !settings.workflows.dirty);
    }

    #[test]
    fn an_item_is_marked_only_while_its_own_page_has_unsaved_edits() {
        let mut settings = settings();
        settings.workflows.touched();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false, false, true, false, false, false, false]);
        settings.workflows.saved();
        settings.profiles.touched();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false, true, false, false, false, false, false]);
    }

    #[test]
    fn workflows_see_the_profiles_that_were_just_saved() {
        let mut settings = settings();
        let mut profiles = Profiles::seeded(&["claude".to_owned()]);
        profiles.profile.remove("reviewer");
        settings.profiles_saved(&profiles);
        let names: Vec<&str> =
            settings.workflows.profiles().iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["builder"]);
    }
}
