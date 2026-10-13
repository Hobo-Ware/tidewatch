//! Profiles and agent overrides in `~/.config/tidewatch/profiles.toml`: load, save, first-launch seeding.
//!
//! A bad file never stops the app: it loads as the defaults (no profiles, built-in agents) with a
//! stderr note, and is left on disk for the user to fix.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::{Agent, AgentOverride, AgentTable, ArgList, Level};
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

fn yes() -> bool {
    true
}

fn no_args(args: &ArgList) -> bool {
    args.is_empty()
}

/// One launch recipe: which agent, with what model and freedom, and the per-tab behavior around it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Shown in the UI. Empty in the file means the table key.
    #[serde(default)]
    pub name: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(rename = "permissions", alias = "level")]
    pub level: Level,
    /// Shell command run in the worktree before the agent starts. Empty means none.
    #[serde(default)]
    pub setup: String,
    #[serde(default = "yes")]
    pub never_push: bool,
    #[serde(default, skip_serializing_if = "no_args")]
    pub extra_args: ArgList,
    #[serde(default = "yes")]
    pub house_style: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profiles {
    #[serde(default)]
    pub profile: BTreeMap<String, Profile>,
    #[serde(default)]
    pub agent: BTreeMap<String, AgentOverride>,
}

/// Agents ordered by how good a default they make: verified ones first, Codex last.
const SEED_ORDER: [&str; 4] = ["claude", "gemini", "copilot", "codex"];

impl Profiles {
    /// Parses the file text. A profile without a `name` is named after its key.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut parsed: Self = toml::from_str(text)?;
        for (id, profile) in &mut parsed.profile {
            if profile.name.is_empty() {
                profile.name.clone_from(id);
            }
        }
        Ok(parsed)
    }

    /// A missing file is the defaults. An unreadable or invalid one is the defaults plus a stderr note.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); using defaults", file.display());
                return Self::default();
            }
        };
        Self::parse(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: profiles.toml is invalid ({e}); using defaults");
            Self::default()
        })
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }

    /// First launch: when `profiles.toml` does not exist, seed profiles for the agents found on
    /// `path_var` and write the file. Afterwards it only loads, so the user's edits are never reseeded.
    #[must_use]
    pub fn load_or_seed(paths: &TidewatchPaths, path_var: Option<&OsStr>) -> Self {
        let file = paths.profiles_file();
        if file.exists() {
            return Self::load(&file);
        }
        let found = AgentTable::builtin().found_on_path(path_var);
        let seeded = Self::seeded(&found);
        if let Err(e) = seeded.save(&file) {
            eprintln!("tidewatch: cannot write {} ({e})", file.display());
        }
        seeded
    }

    /// Builder (Auto edit), Reviewer (Read only) and, for verified agents, a YOLO builder, for each
    /// agent in `found`. The best default agent gets the plain names; the rest are prefixed.
    #[must_use]
    pub fn seeded(found: &[String]) -> Self {
        let table = AgentTable::builtin();
        let ordered: Vec<&Agent> = SEED_ORDER
            .iter()
            .filter(|id| found.iter().any(|f| f == *id))
            .filter_map(|id| table.get(id))
            .collect();

        let mut profile = BTreeMap::new();
        for (position, agent) in ordered.into_iter().enumerate() {
            let (id_prefix, name_prefix) = if position == 0 {
                (String::new(), String::new())
            } else {
                (format!("{}-", agent.id), format!("{} ", agent.name))
            };
            let seed = |suffix: &str, label: &str, level, never_push| {
                let mut seeded = Profile {
                    name: format!("{name_prefix}{label}"),
                    agent: agent.id.clone(),
                    model: None,
                    effort: None,
                    level,
                    setup: String::new(),
                    never_push,
                    extra_args: ArgList::default(),
                    house_style: true,
                };
                if agent.id == "claude" {
                    seeded.model =
                        Some(if level == Level::Plan { "sonnet" } else { "opus" }.into());
                    seeded.effort = (level != Level::Plan).then(|| "high".to_owned());
                }
                (format!("{id_prefix}{suffix}"), seeded)
            };
            profile.extend([
                seed("builder", "Builder", Level::Edits, true),
                seed("reviewer", "Reviewer", Level::Plan, true),
            ]);
            if agent.verified {
                profile.extend([seed("builder-yolo", "Builder YOLO", Level::Yolo, false)]);
            }
        }
        Self { profile, agent: BTreeMap::new() }
    }

    /// Built-in agents with this file's `[agent.<id>]` changes applied, plus its custom agents.
    #[must_use]
    pub fn agent_table(&self) -> AgentTable {
        AgentTable::with_overrides(&self.agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn summary(profiles: &Profiles) -> Vec<(String, String, Level, bool)> {
        profiles
            .profile
            .iter()
            .map(|(id, p)| (id.clone(), p.agent.clone(), p.level, p.never_push))
            .collect()
    }

    #[test]
    fn seeding_one_verified_agent_gives_builder_reviewer_and_an_unguarded_yolo_builder() {
        let seeded = Profiles::seeded(&found(&["claude"]));
        assert_eq!(
            summary(&seeded),
            [
                ("builder".to_owned(), "claude".to_owned(), Level::Edits, true),
                ("builder-yolo".to_owned(), "claude".to_owned(), Level::Yolo, false),
                ("reviewer".to_owned(), "claude".to_owned(), Level::Plan, true),
            ]
        );
        let names: Vec<_> = seeded.profile.values().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Builder", "Builder YOLO", "Reviewer"]);
    }

    #[test]
    fn seeded_claude_profiles_carry_models_and_other_agents_do_not() {
        let seeded = Profiles::seeded(&found(&["claude", "gemini"]));
        let claude_builder = &seeded.profile["builder"];
        assert_eq!(
            (claude_builder.model.as_deref(), claude_builder.effort.as_deref()),
            (Some("opus"), Some("high"))
        );
        let claude_reviewer = &seeded.profile["reviewer"];
        assert_eq!(
            (claude_reviewer.model.as_deref(), claude_reviewer.effort.as_deref()),
            (Some("sonnet"), None)
        );
        assert_eq!(seeded.profile["gemini-builder"].model, None);
    }

    #[test]
    fn extra_agents_get_prefixed_ids_and_names() {
        let seeded = Profiles::seeded(&found(&["gemini", "copilot", "claude"]));
        let ids: Vec<_> = seeded.profile.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "builder",
                "builder-yolo",
                "copilot-builder",
                "copilot-builder-yolo",
                "copilot-reviewer",
                "gemini-builder",
                "gemini-builder-yolo",
                "gemini-reviewer",
                "reviewer",
            ]
        );
        assert_eq!(seeded.profile["gemini-reviewer"].name, "Gemini CLI Reviewer");
        assert_eq!(seeded.profile["builder"].agent, "claude");
    }

    #[test]
    fn codex_is_seeded_last_like_any_verified_agent() {
        let seeded = Profiles::seeded(&found(&["codex", "gemini"]));
        assert_eq!(seeded.profile["builder"].agent, "gemini");
        let codex: Vec<_> = seeded.profile.keys().filter(|id| id.starts_with("codex-")).collect();
        assert_eq!(codex, ["codex-builder", "codex-builder-yolo", "codex-reviewer"]);
        let only_codex = Profiles::seeded(&found(&["codex"]));
        assert_eq!(
            only_codex.profile.keys().collect::<Vec<_>>(),
            ["builder", "builder-yolo", "reviewer"]
        );
    }

    #[test]
    fn no_agents_found_seeds_nothing() {
        assert_eq!(Profiles::seeded(&[]), Profiles::default());
    }

    #[test]
    fn every_seeded_yolo_profile_is_the_only_unguarded_one() {
        let seeded = Profiles::seeded(&found(&["claude", "codex", "gemini", "copilot"]));
        for (id, profile) in &seeded.profile {
            assert_eq!(profile.never_push, profile.level != Level::Yolo, "{id}");
        }
    }

    #[test]
    fn round_trip_keeps_profiles_and_agent_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("profiles.toml");
        let mut profiles = Profiles::seeded(&found(&["claude", "copilot"]));
        profiles.profile.get_mut("builder").unwrap().extra_args =
            ArgList(vec!["--verbose".into(), "two words".into()]);
        profiles.profile.get_mut("builder").unwrap().setup = "npm ci".into();
        profiles.profile.get_mut("reviewer").unwrap().house_style = false;
        profiles.agent = toml::from_str::<Profiles>(
            "[agent.claude.permissions]\nedits = \"--permission-mode acceptEdits --verbose\"\n\
             [agent.aider]\nbin = \"aider\"\n[agent.aider.permissions]\nplan = \"--chat-mode ask\"\n",
        )
        .unwrap()
        .agent;
        profiles.save(&file).unwrap();
        assert_eq!(Profiles::load(&file), profiles);
        assert_eq!(profiles.agent_table().get("aider").unwrap().bin, "aider");
    }

    #[test]
    fn concept_style_toml_parses_with_defaults_and_aliases() {
        let parsed = Profiles::parse(
            "[profile.builder]\nname = \"Builder\"\nagent = \"claude\"\nmodel = \"opus\"\neffort = \"high\"\n\
             permissions = \"edits\"\nsetup = \"npm ci\"\nnever_push = true\nextra_args = \"--verbose --foo\"\n\
             [profile.old]\nagent = \"gemini\"\nlevel = \"plan\"\n",
        )
        .unwrap();
        let builder = &parsed.profile["builder"];
        assert_eq!(builder.level, Level::Edits);
        assert_eq!(builder.extra_args.0, ["--verbose", "--foo"]);
        let old = &parsed.profile["old"];
        assert_eq!((old.name.as_str(), old.level), ("old", Level::Plan));
        assert!(old.never_push);
        assert!(old.house_style);
        assert_eq!(old.setup, "");
    }

    #[test]
    fn bad_file_loads_as_defaults_and_stays_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("profiles.toml");
        for bad in [
            "[[[",
            "[profile.x]\nagent = \"claude\"\npermissions = \"godmode\"\n",
            "[profile.x]\npermissions = \"plan\"\n",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(Profiles::load(&file), Profiles::default(), "text {bad:?}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), bad);
        }
    }

    #[test]
    fn missing_file_loads_as_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Profiles::load(&dir.path().join("none.toml")), Profiles::default());
    }

    fn path_with(bins: &[&str]) -> (tempfile::TempDir, std::ffi::OsString) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for bin in bins {
            let file = dir.path().join(bin);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([dir.path()]).unwrap();
        (dir, path)
    }

    #[test]
    fn first_launch_seeds_from_path_writes_the_file_then_never_reseeds() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let (_bin_dir, path) = path_with(&["claude", "copilot"]);

        let first = Profiles::load_or_seed(&paths, Some(&path));
        assert_eq!(first.profile.keys().filter(|id| id.starts_with("copilot-")).count(), 3);
        assert!(paths.profiles_file().exists());

        let mut edited = first.clone();
        edited.profile.remove("builder-yolo");
        edited.save(&paths.profiles_file()).unwrap();
        let (_other_dir, other_path) = path_with(&["claude", "gemini", "codex"]);
        assert_eq!(Profiles::load_or_seed(&paths, Some(&other_path)), edited);
    }

    #[test]
    fn first_launch_with_a_bad_file_does_not_overwrite_it() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.profiles_file(), "[[[").unwrap();
        let (_bin_dir, path) = path_with(&["claude"]);
        assert_eq!(Profiles::load_or_seed(&paths, Some(&path)), Profiles::default());
        assert_eq!(std::fs::read_to_string(paths.profiles_file()).unwrap(), "[[[");
    }
}
