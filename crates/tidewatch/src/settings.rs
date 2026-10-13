//! What the settings view holds: the profile editor and the workflow editor, and which of the two
//! is showing. Pure state; `settings_view` draws it and the app saves what `Save` builds.

use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use crate::profile_editor;
use crate::workflow_editor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Profiles,
    Workflows,
}

pub(crate) struct Settings {
    pub(crate) section: Section,
    pub(crate) profiles: profile_editor::Editor,
    pub(crate) workflows: workflow_editor::Editor,
}

impl Settings {
    /// `agents` are `(id, display name)` of every agent in the table.
    pub(crate) fn new(
        profiles: &Profiles,
        workflows: &Workflows,
        agents: Vec<(String, String)>,
    ) -> Self {
        let profile_names =
            profiles.profile.iter().map(|(id, p)| (id.clone(), p.name.clone())).collect();
        Self {
            section: Section::Profiles,
            workflows: workflow_editor::Editor::new(workflows, agents.clone(), profile_names),
            profiles: profile_editor::Editor::new(profiles, agents),
        }
    }

    /// The tabs of the view, with a mark on the one that has unsaved edits.
    pub(crate) fn sections(&self) -> [(Section, &'static str, bool); 2] {
        [
            (Section::Profiles, "Profiles", self.profiles.dirty),
            (Section::Workflows, "Workflows", self.workflows.dirty),
        ]
    }

    /// What the footer says about the section that shows.
    pub(crate) fn status(&self) -> Status<'_> {
        match self.section {
            Section::Profiles => Status {
                error: self.profiles.error.as_deref(),
                saving: self.profiles.saving,
                dirty: self.profiles.dirty,
            },
            Section::Workflows => Status {
                error: self.workflows.error.as_deref(),
                saving: self.workflows.saving,
                dirty: self.workflows.dirty,
            },
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

pub(crate) struct Status<'a> {
    pub(crate) error: Option<&'a str>,
    pub(crate) saving: bool,
    pub(crate) dirty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let agents = vec![("claude".to_owned(), "Claude Code".to_owned())];
        Settings::new(&profiles, &Workflows::defaults(), agents)
    }

    #[test]
    fn settings_open_on_profiles_with_nothing_unsaved() {
        let settings = settings();
        assert_eq!(settings.section, Section::Profiles);
        assert_eq!(
            settings.sections(),
            [(Section::Profiles, "Profiles", false), (Section::Workflows, "Workflows", false)]
        );
        assert!(!settings.status().dirty);
    }

    #[test]
    fn the_footer_follows_the_section_that_shows() {
        let mut settings = settings();
        settings.workflows.touched();
        assert!(!settings.status().dirty, "the profiles are untouched");
        settings.section = Section::Workflows;
        assert!(settings.status().dirty);
        settings.workflows.save_failed("disk full".into());
        assert_eq!(settings.status().error, Some("disk full"));
        assert!(settings.sections()[1].2, "the tab says it has unsaved edits");
    }

    #[test]
    fn workflows_see_the_profiles_that_were_just_saved() {
        let mut settings = settings();
        let mut profiles = Profiles::seeded(&["claude".to_owned()]);
        profiles.profile.remove("reviewer");
        settings.profiles_saved(&profiles);
        let names: Vec<&str> =
            settings.workflows.profiles().iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["builder", "builder-yolo"]);
    }
}
