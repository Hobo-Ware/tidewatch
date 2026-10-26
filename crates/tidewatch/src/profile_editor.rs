//! The settings editor's state for `profiles.toml`: one draft per profile, text in and out of the
//! fields, validation and the profiles to save. No drawing; `settings_view` draws it.
//!
//! Saving rewrites the whole file, so comments in it are not kept. The `[agent.<id>]` tables are
//! carried over untouched.

use std::collections::BTreeMap;

use tidewatch_core::agents::{ArgList, Level};
use tidewatch_core::profiles::{Profile, Profiles, default_effort};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Draft {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) agent: String,
    pub(crate) model: String,
    pub(crate) effort: String,
    pub(crate) level: Level,
    pub(crate) setup: String,
    pub(crate) extra_args: String,
    pub(crate) never_push: bool,
    pub(crate) house_style: bool,
}

impl Draft {
    fn from_profile(id: &str, profile: &Profile) -> Self {
        Self {
            id: id.to_owned(),
            name: profile.name.clone(),
            agent: profile.agent.clone(),
            model: profile.model.clone().unwrap_or_default(),
            effort: profile.effort.clone().unwrap_or_default(),
            level: profile.level,
            setup: profile.setup.clone(),
            extra_args: shell_words::join(profile.extra_args.iter()),
            never_push: profile.never_push,
            house_style: profile.house_style,
        }
    }

    pub(crate) fn follow_model(&mut self, previous_model: &str) {
        let followed = default_effort(previous_model).unwrap_or_default();
        if self.effort.trim() == followed {
            self.effort = default_effort(&self.model).unwrap_or_default().to_owned();
        }
    }

    fn to_profile(&self) -> Result<Profile, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(format!("profile {} needs a name", self.id));
        }
        let extra = shell_words::split(&self.extra_args)
            .map_err(|e| format!("{name}: extra arguments: {e}"))?;
        let optional = |text: &str| Some(text.trim().to_owned()).filter(|t| !t.is_empty());
        Ok(Profile {
            name: name.to_owned(),
            agent: self.agent.clone(),
            model: optional(&self.model),
            effort: optional(&self.effort),
            level: self.level,
            setup: self.setup.trim().to_owned(),
            never_push: self.never_push,
            extra_args: ArgList(extra),
            house_style: self.house_style,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) agent: String,
    pub(crate) name: String,
    pub(crate) known: bool,
    pub(crate) drafts: Vec<usize>,
}

pub(crate) struct Editor {
    base: Profiles,
    pub(crate) drafts: Vec<Draft>,
    pub(crate) selected: usize,
    agents: Vec<(String, String)>,
    pub(crate) error: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) saving: bool,
}

impl Editor {
    /// `agents` are `(id, display name)` for every agent in the table.
    pub(crate) fn new(profiles: &Profiles, agents: Vec<(String, String)>) -> Self {
        let drafts = profiles.profile.iter().map(|(id, p)| Draft::from_profile(id, p)).collect();
        Self {
            base: profiles.clone(),
            drafts,
            selected: 0,
            agents,
            error: None,
            dirty: false,
            saving: false,
        }
    }

    pub(crate) fn agents(&self) -> &[(String, String)] {
        &self.agents
    }

    pub(crate) fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = self
            .agents
            .iter()
            .map(|(id, name)| Group {
                agent: id.clone(),
                name: name.clone(),
                known: true,
                drafts: Vec::new(),
            })
            .collect();
        for (index, draft) in self.drafts.iter().enumerate() {
            match groups.iter_mut().find(|group| group.agent == draft.agent) {
                Some(group) => group.drafts.push(index),
                None => groups.push(Group {
                    agent: draft.agent.clone(),
                    name: format!("{} (unknown)", draft.agent),
                    known: false,
                    drafts: vec![index],
                }),
            }
        }
        groups.retain(|group| !group.drafts.is_empty());
        groups
    }

    pub(crate) fn selected_draft(&mut self) -> Option<&mut Draft> {
        self.drafts.get_mut(self.selected)
    }

    pub(crate) fn select(&mut self, index: usize) {
        if index < self.drafts.len() {
            self.selected = index;
        }
    }

    pub(crate) fn touched(&mut self) {
        self.dirty = true;
        self.error = None;
    }

    /// Adds a profile on the first agent, selects it and returns its id.
    pub(crate) fn add(&mut self) -> String {
        let id = self.unused_id("new-profile");
        let agent = self.agents.first().map(|(id, _)| id.clone()).unwrap_or_default();
        self.drafts.push(Draft {
            id: id.clone(),
            name: "New profile".to_owned(),
            agent,
            model: String::new(),
            effort: String::new(),
            level: Level::Edits,
            setup: String::new(),
            extra_args: String::new(),
            never_push: true,
            house_style: true,
        });
        self.selected = self.drafts.len() - 1;
        self.touched();
        id
    }

    pub(crate) fn remove_selected(&mut self) {
        if self.selected < self.drafts.len() {
            self.drafts.remove(self.selected);
            self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
            self.touched();
        }
    }

    fn unused_id(&self, stem: &str) -> String {
        let taken = |id: &str| self.drafts.iter().any(|d| d.id == id);
        if !taken(stem) {
            return stem.to_owned();
        }
        (2..=self.drafts.len() + 2)
            .map(|n| format!("{stem}-{n}"))
            .find(|id| !taken(id))
            .unwrap_or_default()
    }

    /// The profiles to write: every draft validated, the agent tables as loaded.
    pub(crate) fn build(&self) -> Result<Profiles, String> {
        let mut profile = BTreeMap::new();
        for draft in &self.drafts {
            if !self.agents.iter().any(|(id, _)| *id == draft.agent) {
                return Err(format!("{}: unknown agent {}", draft.name.trim(), draft.agent));
            }
            profile.insert(draft.id.clone(), draft.to_profile()?);
        }
        Ok(Profiles { profile, agent: self.base.agent.clone() })
    }

    /// Saved: the written profiles become the baseline.
    pub(crate) fn saved(&mut self, profiles: Profiles) {
        self.base = profiles;
        self.saving = false;
        self.dirty = false;
        self.error = None;
    }

    pub(crate) fn revert(&mut self) {
        self.drafts = self.base.profile.iter().map(|(id, p)| Draft::from_profile(id, p)).collect();
        self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
        self.dirty = false;
        self.error = None;
    }

    pub(crate) fn save_failed(&mut self, message: String) {
        self.saving = false;
        self.error = Some(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::agents::AgentOverride;

    fn agents() -> Vec<(String, String)> {
        vec![("claude".into(), "Claude Code".into()), ("gemini".into(), "Gemini CLI".into())]
    }

    fn editor() -> Editor {
        Editor::new(&Profiles::seeded(&["claude".to_owned()]), agents())
    }

    #[test]
    fn drafts_follow_the_file_and_round_trip_to_the_same_profiles() {
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let editor = Editor::new(&profiles, agents());
        let ids: Vec<&str> = editor.drafts.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["builder", "reviewer"]);
        assert_eq!(editor.build().unwrap(), profiles);
    }

    #[test]
    fn an_effort_nobody_chose_follows_the_model_and_a_chosen_one_stays() {
        let mut draft = editor().drafts[1].clone();
        assert_eq!((draft.model.as_str(), draft.effort.as_str()), ("sonnet", "high"));
        let steps = [("opus", "medium"), ("haiku", "xhigh"), ("gpt-5", ""), ("opus", "medium")];
        for (model, want) in steps {
            let previous = std::mem::replace(&mut draft.model, model.to_owned());
            draft.follow_model(&previous);
            assert_eq!(draft.effort, want, "model {model}");
        }
        draft.effort = "max".into();
        let previous = std::mem::replace(&mut draft.model, "sonnet".to_owned());
        draft.follow_model(&previous);
        assert_eq!(draft.effort, "max");
    }

    #[test]
    fn extra_arguments_keep_words_with_spaces_through_edit_and_save() {
        let mut editor = editor();
        editor.drafts[0].extra_args = "--add-dir '../shared lib' --verbose".into();
        let built = editor.build().unwrap();
        assert_eq!(
            built.profile["builder"].extra_args.0,
            ["--add-dir", "../shared lib", "--verbose"]
        );
        let reopened = Editor::new(&built, agents());
        assert_eq!(reopened.drafts[0].extra_args, "--add-dir '../shared lib' --verbose");
    }

    #[test]
    fn empty_model_and_effort_are_left_out_and_setup_is_trimmed() {
        let mut editor = editor();
        editor.drafts[0].model = "  ".into();
        editor.drafts[0].effort = String::new();
        editor.drafts[0].setup = "  npm ci \n".into();
        let profile = &editor.build().unwrap().profile["builder"];
        assert_eq!((profile.model.clone(), profile.effort.clone()), (None, None));
        assert_eq!(profile.setup, "npm ci");
    }

    #[test]
    fn a_bad_profile_blocks_the_save_with_a_reason() {
        type Change = fn(&mut Draft);
        let cases: [(Change, &str); 3] = [
            (|d| d.name = "  ".into(), "needs a name"),
            (|d| d.extra_args = "--flag 'unclosed".into(), "extra arguments"),
            (|d| d.agent = "ghost".into(), "unknown agent ghost"),
        ];
        for (change, want) in cases {
            let mut editor = editor();
            change(&mut editor.drafts[0]);
            let error = editor.build().unwrap_err();
            assert!(error.contains(want), "{error}");
        }
    }

    #[test]
    fn adding_makes_a_unique_selected_profile_on_the_first_agent() {
        let mut editor = editor();
        assert_eq!(editor.add(), "new-profile");
        assert_eq!(editor.add(), "new-profile-2");
        assert_eq!(editor.selected, editor.drafts.len() - 1);
        assert!(editor.dirty);
        let added = editor.build().unwrap().profile["new-profile-2"].clone();
        assert_eq!(
            (added.agent.as_str(), added.level, added.never_push),
            ("claude", Level::Edits, true)
        );
    }

    #[test]
    fn removing_keeps_the_selection_in_range() {
        let mut editor = editor();
        editor.select(1);
        editor.remove_selected();
        assert_eq!(editor.selected, 0);
        assert_eq!(editor.drafts.len(), 1);
        editor.remove_selected();
        editor.remove_selected();
        assert_eq!((editor.drafts.len(), editor.selected), (0, 0));
        assert_eq!(editor.build().unwrap(), Profiles::default());
    }

    #[test]
    fn agent_tables_are_carried_over_untouched() {
        let mut profiles = Profiles::seeded(&["claude".to_owned()]);
        profiles.agent.insert(
            "aider".into(),
            AgentOverride { bin: Some("aider".into()), ..Default::default() },
        );
        let built = Editor::new(&profiles, agents()).build().unwrap();
        assert_eq!(built.agent, profiles.agent);
    }

    #[test]
    fn saving_clears_the_dirty_flag_and_a_failure_keeps_the_edits() {
        let mut editor = editor();
        editor.touched();
        editor.saving = true;
        editor.save_failed("disk full".into());
        assert_eq!(
            (editor.dirty, editor.saving, editor.error.as_deref()),
            (true, false, Some("disk full"))
        );
        editor.saved(Profiles::default());
        assert_eq!((editor.dirty, editor.error.clone()), (false, None));
    }

    #[test]
    fn revert_puts_back_the_saved_profiles_and_keeps_the_selection_in_range() {
        let mut editor = editor();
        editor.select(1);
        editor.drafts[1].name = "Changed".into();
        editor.remove_selected();
        editor.add();
        assert!(editor.dirty);
        editor.revert();
        let ids: Vec<&str> = editor.drafts.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["builder", "reviewer"]);
        assert_eq!(editor.drafts[1].name, "Reviewer");
        assert_eq!((editor.dirty, editor.error.clone(), editor.selected), (false, None, 1));
        editor.error = Some("x".into());
        editor.revert();
        assert_eq!(editor.error, None);
    }

    #[test]
    fn groups_follow_the_agent_table_and_leave_out_agents_without_profiles() {
        let mut editor = editor();
        let groups = editor.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].agent.as_str(), groups[0].name.as_str()), ("claude", "Claude Code"));
        assert_eq!(groups[0].drafts, [0, 1]);
        editor.add();
        editor.drafts[2].agent = "gemini".into();
        editor.add();
        editor.drafts[3].agent = "ghost".into();
        let groups = editor.groups();
        let names: Vec<(&str, bool, &[usize])> =
            groups.iter().map(|g| (g.agent.as_str(), g.known, g.drafts.as_slice())).collect();
        assert_eq!(
            names,
            [("claude", true, &[0, 1][..]), ("gemini", true, &[2][..]), ("ghost", false, &[3][..])]
        );
        assert_eq!(groups[2].name, "ghost (unknown)");
    }
}
