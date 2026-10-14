//! What the settings view holds: the profile, workflow and window editors and which page shows.
//! Pure state; `settings_view` draws it and the app saves what `Save` builds.

use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use tidewatch_core::prefs::WindowPrefs;

use crate::login_item;
use crate::profile_editor;
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
}

pub(crate) struct Settings {
    pub(crate) section: Section,
    pub(crate) profiles: profile_editor::Editor,
    pub(crate) workflows: workflow_editor::Editor,
    pub(crate) window: window_settings::Editor,
    pub(crate) login: login_item::Row,
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
            login: login_item::Row::default(),
        }
    }

    pub(crate) fn sections(&self) -> [(Section, &'static str, bool); 6] {
        [
            (Section::General, "General", false),
            (Section::Profiles, "Agents and profiles", self.profiles.dirty),
            (Section::Workflows, "Workflows", self.workflows.dirty),
            (Section::Trust, "Folder trust", false),
            (Section::Shortcuts, "Shortcuts", false),
            (Section::HouseStyle, "House style", false),
        ]
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
            ]
        );
        assert!(!settings.profiles.dirty && !settings.workflows.dirty);
    }

    #[test]
    fn an_item_is_marked_only_while_its_own_page_has_unsaved_edits() {
        let mut settings = settings();
        settings.workflows.touched();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false, false, true, false, false, false]);
        settings.workflows.saved();
        settings.profiles.touched();
        let marks: Vec<bool> = settings.sections().iter().map(|item| item.2).collect();
        assert_eq!(marks, [false, true, false, false, false, false]);
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
