//! Profiles and agent overrides in `~/.config/tidewatch/profiles.toml`: load, save, first-launch seeding.
//!
//! A bad file never stops the app: it loads as the defaults (no profiles, built-in agents) with a
//! stderr note, and is left on disk for the user to fix.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::{Agent, AgentOverride, AgentTable, ArgList, Level};
use crate::effort_migration;
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

impl Profile {
    /// The profile id a tab saves when its CLI ran on `defaults_for`, which `profiles.toml` has no key for.
    pub const DEFAULTS_ID: &'static str = "default";

    /// The effort a launch passes: the profile's own, else the model's default, else none.
    #[must_use]
    pub fn effective_effort(&self) -> Option<&str> {
        let own = self.effort.as_deref().filter(|effort| !effort.is_empty());
        own.or_else(|| default_effort_for(&self.agent, self.model.as_deref().unwrap_or_default()))
    }

    /// What a CLI runs with when no profile names it: its own model and effort, no setup.
    #[must_use]
    pub fn defaults_for(agent: &str, level: Level) -> Self {
        Self {
            name: "Default".to_owned(),
            agent: agent.to_owned(),
            model: None,
            effort: None,
            level,
            setup: String::new(),
            never_push: true,
            extra_args: ArgList::default(),
            house_style: true,
        }
    }
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
    /// Loading also drops the YOLO profiles earlier versions seeded, when untouched, and rewrites the file.
    #[must_use]
    pub fn load_or_seed(paths: &TidewatchPaths, path_var: Option<&OsStr>) -> Self {
        let file = paths.profiles_file();
        if file.exists() {
            Self::migrate_saved_effort(paths);
            let mut profiles = Self::load(&file);
            if !profiles.drop_seeded_yolo().is_empty()
                && let Err(e) = profiles.save(&file)
            {
                eprintln!("tidewatch: cannot write {} ({e})", file.display());
            }
            return profiles;
        }
        let found = AgentTable::builtin().found_on_path(path_var);
        let seeded = Self::seeded(&found);
        match seeded.save(&file) {
            Ok(()) => effort_migration::mark(paths, effort_migration::PROFILES),
            Err(e) => eprintln!("tidewatch: cannot write {} ({e})", file.display()),
        }
        seeded
    }

    /// Once per machine: the claude Builder an older version seeded at opus and `high` moves to the
    /// opus default. Any other profile, and a Builder the user changed, stays. A file that cannot
    /// be read or parsed is left alone and not marked.
    fn migrate_saved_effort(paths: &TidewatchPaths) {
        if effort_migration::is_done(paths, effort_migration::PROFILES) {
            return;
        }
        let file = paths.profiles_file();
        let Some(mut profiles) =
            std::fs::read_to_string(&file).ok().and_then(|text| Self::parse(&text).ok())
        else {
            return;
        };
        if profiles.migrate_seeded_effort()
            && let Err(e) = profiles.save(&file)
        {
            eprintln!("tidewatch: cannot write {} ({e})", file.display());
            return;
        }
        effort_migration::mark(paths, effort_migration::PROFILES);
    }

    fn migrate_seeded_effort(&mut self) -> bool {
        let mut changed = false;
        for id in ["builder", "claude-builder"] {
            let Some(profile) = self.profile.get_mut(id) else { continue };
            let seeded = profile.agent == "claude"
                && profile.model.as_deref() == Some("opus")
                && profile.effort.as_deref() == Some("high");
            if seeded {
                profile.effort = default_effort("opus").map(str::to_owned);
                changed = true;
            }
        }
        changed
    }

    /// Builder (Auto edit) and Reviewer (Read only) for each agent in `found`. The best default agent
    /// gets the plain names; the rest are prefixed.
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
            profile.extend([
                seed_profile(agent, position == 0, "builder", "Builder", Level::Edits, true),
                seed_profile(agent, position == 0, "reviewer", "Reviewer", Level::Plan, true),
            ]);
        }
        Self { profile, agent: BTreeMap::new() }
    }

    /// Removes the `*-builder-yolo` profiles that earlier versions seeded, but only the ones still
    /// exactly as seeded. An edited one or one the user made stays. Returns the removed ids.
    pub fn drop_seeded_yolo(&mut self) -> Vec<String> {
        let seeds = legacy_yolo_seeds();
        let removed: Vec<String> = self
            .profile
            .iter()
            .filter(|(id, profile)| {
                seeds.iter().any(|(seed_id, seed)| seed_id == *id && seed == *profile)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &removed {
            self.profile.remove(id);
        }
        removed
    }

    /// Built-in agents with this file's `[agent.<id>]` changes applied, plus its custom agents.
    #[must_use]
    pub fn agent_table(&self) -> AgentTable {
        AgentTable::with_overrides(&self.agent)
    }
}

/// The effort a Claude model runs at when nobody picked one: opus medium, sonnet high, haiku xhigh.
/// Matches the model family anywhere in the name, ignoring case; any other model has no default.
#[must_use]
pub fn default_effort(model: &str) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    [("opus", "medium"), ("sonnet", "high"), ("haiku", "xhigh")]
        .into_iter()
        .find_map(|(family, effort)| model.contains(family).then_some(effort))
}

/// `default_effort` for what an agent runs: only Claude's own models have one.
#[must_use]
pub fn default_effort_for(agent: &str, model: &str) -> Option<&'static str> {
    (agent == "claude").then(|| default_effort(model)).flatten()
}

fn seed_profile(
    agent: &Agent,
    plain: bool,
    suffix: &str,
    label: &str,
    level: Level,
    never_push: bool,
) -> (String, Profile) {
    let (id_prefix, name_prefix) = if plain {
        (String::new(), String::new())
    } else {
        (format!("{}-", agent.id), format!("{} ", agent.name))
    };
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
        let model = if level == Level::Plan { "sonnet" } else { "opus" };
        seeded.effort = default_effort(model).map(str::to_owned);
        seeded.model = Some(model.into());
    }
    (format!("{id_prefix}{suffix}"), seeded)
}

fn legacy_yolo_seeds() -> Vec<(String, Profile)> {
    let table = AgentTable::builtin();
    SEED_ORDER
        .iter()
        .filter_map(|id| table.get(id))
        .filter(|agent| agent.verified)
        .flat_map(|agent| {
            [true, false].map(|plain| {
                let (id, mut seed) =
                    seed_profile(agent, plain, "builder-yolo", "Builder YOLO", Level::Yolo, false);
                if agent.id == "claude" {
                    seed.effort = Some("high".to_owned());
                }
                (id, seed)
            })
        })
        .collect()
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
    fn seeding_one_agent_gives_builder_and_reviewer_only() {
        let seeded = Profiles::seeded(&found(&["claude"]));
        assert_eq!(
            summary(&seeded),
            [
                ("builder".to_owned(), "claude".to_owned(), Level::Edits, true),
                ("reviewer".to_owned(), "claude".to_owned(), Level::Plan, true),
            ]
        );
        let names: Vec<_> = seeded.profile.values().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Builder", "Reviewer"]);
    }

    #[test]
    fn seeded_claude_profiles_carry_models_and_other_agents_do_not() {
        let seeded = Profiles::seeded(&found(&["claude", "gemini"]));
        let claude_builder = &seeded.profile["builder"];
        assert_eq!(
            (claude_builder.model.as_deref(), claude_builder.effort.as_deref()),
            (Some("opus"), Some("medium"))
        );
        let claude_reviewer = &seeded.profile["reviewer"];
        assert_eq!(
            (claude_reviewer.model.as_deref(), claude_reviewer.effort.as_deref()),
            (Some("sonnet"), Some("high"))
        );
        assert_eq!(seeded.profile["gemini-builder"].model, None);
    }

    #[test]
    fn the_default_effort_follows_the_model_family() {
        let cases = [
            ("opus", Some("medium")),
            ("claude-opus-5-5", Some("medium")),
            ("OPUS", Some("medium")),
            ("sonnet", Some("high")),
            ("sonnet[1m]", Some("high")),
            ("claude-sonnet-5-5", Some("high")),
            ("haiku", Some("xhigh")),
            ("Claude-Haiku-4-5", Some("xhigh")),
            ("gpt-5", None),
            ("gemini-2.5-pro", None),
            ("", None),
        ];
        for (model, want) in cases {
            assert_eq!(default_effort(model), want, "model {model:?}");
        }
    }

    #[test]
    fn only_claude_gets_a_default_effort() {
        assert_eq!(default_effort_for("claude", "haiku"), Some("xhigh"));
        assert_eq!(default_effort_for("copilot", "claude-haiku-4.5"), None);
    }

    #[test]
    fn extra_agents_get_prefixed_ids_and_names() {
        let seeded = Profiles::seeded(&found(&["gemini", "copilot", "claude"]));
        let ids: Vec<_> = seeded.profile.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "builder",
                "copilot-builder",
                "copilot-reviewer",
                "gemini-builder",
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
        assert_eq!(codex, ["codex-builder", "codex-reviewer"]);
        let only_codex = Profiles::seeded(&found(&["codex"]));
        assert_eq!(only_codex.profile.keys().collect::<Vec<_>>(), ["builder", "reviewer"]);
    }

    #[test]
    fn no_agents_found_seeds_nothing() {
        assert_eq!(Profiles::seeded(&[]), Profiles::default());
    }

    #[test]
    fn nothing_seeded_is_at_yolo() {
        let seeded = Profiles::seeded(&found(&["claude", "codex", "gemini", "copilot"]));
        assert!(seeded.profile.values().all(|p| p.level != Level::Yolo && p.never_push));
    }

    fn with_legacy_yolo(ids: &[&str]) -> Profiles {
        let mut profiles = Profiles::seeded(&found(&["claude", "gemini", "codex"]));
        let seeds = legacy_yolo_seeds();
        for id in ids {
            let (_, seed) = seeds.iter().find(|(seed_id, _)| seed_id == id).unwrap();
            profiles.profile.insert((*id).to_owned(), seed.clone());
        }
        profiles
    }

    #[test]
    fn an_untouched_seeded_yolo_profile_is_removed_and_the_rest_stays() {
        let mut profiles =
            with_legacy_yolo(&["builder-yolo", "gemini-builder-yolo", "codex-builder-yolo"]);
        let expected = Profiles::seeded(&found(&["claude", "gemini", "codex"]));
        let mut removed = profiles.drop_seeded_yolo();
        removed.sort();
        assert_eq!(removed, ["builder-yolo", "codex-builder-yolo", "gemini-builder-yolo"]);
        assert_eq!(profiles, expected);
    }

    #[test]
    fn the_legacy_seed_matches_what_older_versions_wrote() {
        let text = "[profile.builder-yolo]\nname = \"Builder YOLO\"\nagent = \"claude\"\nmodel = \"opus\"\n\
                    effort = \"high\"\npermissions = \"yolo\"\nsetup = \"\"\nnever_push = false\n\
                    house_style = true\n\
                    [profile.copilot-builder-yolo]\nname = \"Copilot CLI Builder YOLO\"\nagent = \"copilot\"\n\
                    permissions = \"yolo\"\nnever_push = false\n";
        let mut parsed = Profiles::parse(text).unwrap();
        assert_eq!(parsed.drop_seeded_yolo(), ["builder-yolo", "copilot-builder-yolo"]);
        assert_eq!(parsed, Profiles::default());
    }

    #[test]
    fn an_edited_seeded_yolo_profile_stays() {
        let edits: [fn(&mut Profile); 8] = [
            |p| p.name = "Builder YOLO (mine)".into(),
            |p| p.agent = "gemini".into(),
            |p| p.model = Some("sonnet".into()),
            |p| p.effort = None,
            |p| p.setup = "npm ci".into(),
            |p| p.never_push = true,
            |p| p.extra_args = ArgList(vec!["--verbose".into()]),
            |p| p.house_style = false,
        ];
        for edit in edits {
            let mut profiles = with_legacy_yolo(&["builder-yolo"]);
            edit(profiles.profile.get_mut("builder-yolo").unwrap());
            let before = profiles.clone();
            assert_eq!(profiles.drop_seeded_yolo(), Vec::<String>::new());
            assert_eq!(profiles, before);
        }
        let mut leveled = with_legacy_yolo(&["builder-yolo"]);
        leveled.profile.get_mut("builder-yolo").unwrap().level = Level::Auto;
        assert_eq!(leveled.drop_seeded_yolo(), Vec::<String>::new());
    }

    #[test]
    fn a_users_own_yolo_profile_stays() {
        let mut profiles = with_legacy_yolo(&[]);
        let mut mine = with_legacy_yolo(&["builder-yolo"]).profile["builder-yolo"].clone();
        mine.name = "Night shift".into();
        profiles.profile.insert("night-shift".into(), mine.clone());
        mine.name = "Builder YOLO".into();
        profiles.profile.insert("my-builder-yolo".into(), mine);
        profiles.profile.insert(
            "aider-builder-yolo".into(),
            Profile {
                name: "Aider Builder YOLO".into(),
                agent: "aider".into(),
                model: None,
                effort: None,
                level: Level::Yolo,
                setup: String::new(),
                never_push: false,
                extra_args: ArgList::default(),
                house_style: true,
            },
        );
        let before = profiles.clone();
        assert_eq!(profiles.drop_seeded_yolo(), Vec::<String>::new());
        assert_eq!(profiles, before);
    }

    #[test]
    fn loading_cleans_the_file_once_and_keeps_every_other_entry() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut old = with_legacy_yolo(&["builder-yolo", "gemini-builder-yolo"]);
        old.profile.get_mut("reviewer").unwrap().setup = "make".into();
        old.agent = toml::from_str::<Profiles>("[agent.aider]\nbin = \"aider\"\n").unwrap().agent;
        old.save(&paths.profiles_file()).unwrap();
        let (_bin_dir, path) = path_with(&["claude"]);

        let loaded = Profiles::load_or_seed(&paths, Some(&path));
        let mut expected = old.clone();
        expected.profile.remove("builder-yolo");
        expected.profile.remove("gemini-builder-yolo");
        assert_eq!(loaded, expected);
        assert_eq!(Profiles::load(&paths.profiles_file()), expected);
        let cleaned = std::fs::read_to_string(paths.profiles_file()).unwrap();
        assert!(!cleaned.contains("yolo"), "{cleaned}");

        assert_eq!(Profiles::load_or_seed(&paths, Some(&path)), expected);
        assert_eq!(std::fs::read_to_string(paths.profiles_file()).unwrap(), cleaned);
    }

    #[test]
    fn a_file_with_nothing_to_remove_is_not_rewritten() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        let text = "# mine\n[profile.builder]\nagent = \"claude\"\npermissions = \"edits\"\n";
        std::fs::write(paths.profiles_file(), text).unwrap();
        let (_bin_dir, path) = path_with(&["claude"]);
        let loaded = Profiles::load_or_seed(&paths, Some(&path));
        assert_eq!(loaded.profile.len(), 1);
        assert_eq!(std::fs::read_to_string(paths.profiles_file()).unwrap(), text);
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
        assert_eq!(first.profile.keys().filter(|id| id.starts_with("copilot-")).count(), 2);
        assert!(paths.profiles_file().exists());

        let mut edited = first.clone();
        edited.profile.remove("reviewer");
        edited.save(&paths.profiles_file()).unwrap();
        let (_other_dir, other_path) = path_with(&["claude", "gemini", "codex"]);
        assert_eq!(Profiles::load_or_seed(&paths, Some(&other_path)), edited);
    }

    const OLD_SEED: &str = "[profile.builder]\nname = \"Builder\"\nagent = \"claude\"\nmodel = \"opus\"\n\
                            effort = \"high\"\npermissions = \"edits\"\n\
                            [profile.reviewer]\nname = \"Reviewer\"\nagent = \"claude\"\n\
                            model = \"sonnet\"\npermissions = \"plan\"\n";

    fn saved(text: &str) -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.profiles_file(), text).unwrap();
        (home, paths)
    }

    #[test]
    fn a_builder_seeded_at_opus_high_moves_to_medium_once() {
        let (_home, paths) = saved(OLD_SEED);
        let first = Profiles::load_or_seed(&paths, Some(std::ffi::OsStr::new("")));
        assert_eq!(first.profile["builder"].effort.as_deref(), Some("medium"));
        assert_eq!(first.profile["reviewer"].effort, None);
        assert_eq!(Profiles::load(&paths.profiles_file()), first);

        let mut by_hand = first.clone();
        by_hand.profile.get_mut("builder").unwrap().effort = Some("high".into());
        by_hand.save(&paths.profiles_file()).unwrap();
        let second = Profiles::load_or_seed(&paths, Some(std::ffi::OsStr::new("")));
        assert_eq!(second, by_hand);
    }

    #[test]
    fn a_builder_the_user_changed_keeps_its_effort() {
        let cases = [
            OLD_SEED.replace("opus", "sonnet"),
            OLD_SEED.replace("effort = \"high\"", "effort = \"max\""),
            OLD_SEED.replace(
                "agent = \"claude\"\nmodel = \"opus\"",
                "agent = \"gemini\"\nmodel = \"opus\"",
            ),
            OLD_SEED.replace("[profile.builder]", "[profile.mine]"),
        ];
        for text in cases {
            let (_home, paths) = saved(&text);
            let before = Profiles::parse(&text).unwrap();
            assert_eq!(Profiles::load_or_seed(&paths, Some(std::ffi::OsStr::new(""))), before);
            assert_eq!(std::fs::read_to_string(paths.profiles_file()).unwrap(), text);
        }
    }

    #[test]
    fn a_bad_file_is_not_marked_so_fixing_it_still_migrates() {
        let (_home, paths) = saved("[[[");
        let _ = Profiles::load_or_seed(&paths, Some(std::ffi::OsStr::new("")));
        assert!(!effort_migration::is_done(&paths, effort_migration::PROFILES));
        std::fs::write(paths.profiles_file(), OLD_SEED).unwrap();
        let fixed = Profiles::load_or_seed(&paths, Some(std::ffi::OsStr::new("")));
        assert_eq!(fixed.profile["builder"].effort.as_deref(), Some("medium"));
    }

    #[test]
    fn a_fresh_seed_is_marked_so_a_later_high_stays() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let (_bin_dir, path) = path_with(&["claude"]);
        let mut seeded = Profiles::load_or_seed(&paths, Some(&path));
        seeded.profile.get_mut("builder").unwrap().effort = Some("high".into());
        seeded.save(&paths.profiles_file()).unwrap();
        assert_eq!(Profiles::load_or_seed(&paths, Some(&path)), seeded);
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
