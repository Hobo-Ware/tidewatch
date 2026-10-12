//! What the app knows about agents: the loaded profiles, the agent table built from them and which
//! CLIs were found on `PATH`. Filled by `agent_jobs`, read by the new-tab dialog and the settings.

use std::collections::HashMap;
use std::ffi::OsString;

use tidewatch_core::agents::{Agent, AgentTable};
use tidewatch_core::profiles::{Profile, Profiles};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Availability {
    Probing,
    Found(Option<String>),
    Missing,
}

pub(crate) struct Catalog {
    profiles: Option<Profiles>,
    table: AgentTable,
    availability: HashMap<String, Availability>,
    pub(crate) path: OsString,
}

impl Catalog {
    pub(crate) fn new(path: OsString) -> Self {
        Self { profiles: None, table: AgentTable::builtin(), availability: HashMap::new(), path }
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.profiles.is_some()
    }

    pub(crate) fn profiles(&self) -> Option<&Profiles> {
        self.profiles.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn table(&self) -> &AgentTable {
        &self.table
    }

    /// Takes the loaded (or saved) profiles and returns the agents still to probe as `(id, bin)`:
    /// every agent seen for the first time, and any whose binary changed.
    pub(crate) fn set_profiles(&mut self, profiles: Profiles) -> Vec<(String, String)> {
        let previous: HashMap<String, String> =
            self.table.iter().map(|a| (a.id.clone(), a.bin.clone())).collect();
        self.table = profiles.agent_table();
        self.profiles = Some(profiles);
        let mut to_probe = Vec::new();
        for agent in self.table.iter() {
            let unchanged = previous.get(&agent.id) == Some(&agent.bin)
                && self.availability.contains_key(&agent.id);
            if !unchanged {
                self.availability.insert(agent.id.clone(), Availability::Probing);
                to_probe.push((agent.id.clone(), agent.bin.clone()));
            }
        }
        to_probe
    }

    pub(crate) fn set_availability(&mut self, agent: &str, availability: Availability) {
        if self.table.get(agent).is_some() {
            self.availability.insert(agent.to_owned(), availability);
        }
    }

    pub(crate) fn availability(&self, agent: &str) -> Availability {
        self.availability.get(agent).cloned().unwrap_or(Availability::Missing)
    }

    /// Profiles in file order, with the agent each one runs. A profile naming an agent that is not
    /// in the table is left out: it cannot start anything.
    pub(crate) fn entries(&self) -> Vec<(&str, &Profile, &Agent)> {
        let Some(profiles) = &self.profiles else { return Vec::new() };
        profiles
            .profile
            .iter()
            .filter_map(|(id, profile)| {
                self.table.get(&profile.agent).map(|agent| (id.as_str(), profile, agent))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::agents::Level;

    fn catalog() -> Catalog {
        Catalog::new(OsString::from("/usr/bin"))
    }

    fn seeded() -> Profiles {
        Profiles::seeded(&["claude".to_owned(), "gemini".to_owned()])
    }

    #[test]
    fn nothing_is_loaded_until_profiles_arrive() {
        let catalog = catalog();
        assert!(!catalog.is_loaded());
        assert_eq!(catalog.entries().len(), 0);
    }

    #[test]
    fn loading_asks_to_probe_every_agent_once() {
        let mut catalog = catalog();
        let ids: Vec<String> =
            catalog.set_profiles(seeded()).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["claude", "codex", "gemini", "copilot"]);
        assert_eq!(catalog.availability("claude"), Availability::Probing);
        catalog.set_availability("claude", Availability::Found(Some("2.1.296".into())));
        assert_eq!(catalog.set_profiles(seeded()), Vec::<(String, String)>::new());
        assert_eq!(catalog.availability("claude"), Availability::Found(Some("2.1.296".into())));
    }

    #[test]
    fn a_changed_binary_is_probed_again() {
        let mut catalog = catalog();
        catalog.set_profiles(seeded());
        catalog.set_availability("claude", Availability::Missing);
        let mut changed = seeded();
        changed.agent.insert(
            "claude".into(),
            tidewatch_core::agents::AgentOverride {
                bin: Some("claude-next".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            catalog.set_profiles(changed),
            [("claude".to_owned(), "claude-next".to_owned())]
        );
        assert_eq!(catalog.availability("claude"), Availability::Probing);
    }

    #[test]
    fn unknown_agents_are_missing_and_cannot_be_marked() {
        let mut catalog = catalog();
        catalog.set_profiles(seeded());
        catalog.set_availability("ghost", Availability::Found(None));
        assert_eq!(catalog.availability("ghost"), Availability::Missing);
    }

    #[test]
    fn entries_follow_the_file_and_skip_profiles_of_unknown_agents() {
        let mut catalog = catalog();
        let mut profiles = seeded();
        profiles.profile.get_mut("reviewer").unwrap().agent = "nope".into();
        catalog.set_profiles(profiles);
        let ids: Vec<&str> = catalog.entries().iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            ids,
            ["builder", "builder-yolo", "gemini-builder", "gemini-builder-yolo", "gemini-reviewer"]
        );
        let (_, builder, agent) = catalog.entries()[0];
        assert_eq!((builder.level, agent.id.as_str()), (Level::Edits, "claude"));
    }
}
