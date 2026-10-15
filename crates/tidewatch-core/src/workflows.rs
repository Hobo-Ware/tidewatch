//! Workflows in `~/.config/tidewatch/workflows.toml`: stages, gates, the built-in set, load and save.
//!
//! A workflow is an ordered list of stages that run one after the other in the tab's worktree. A
//! stage says what to do (an [`Action`] and its prompt), who does it (a profile reference and/or
//! agent, model, effort and permission overrides) and the [`Gate`] it must pass. The state machine
//! that walks a workflow lives in `runner.rs`.
//!
//! In the file, `on_fail` is a stage number counted from 1 (as the concept shows it); in memory it
//! is an index counted from 0. A bad file never stops the app: it loads as the built-in workflows
//! with a stderr note. A workflow with problems still loads, so saving never drops the user's text;
//! it just refuses to run (see [`Workflow::validate`]).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use serde::{Deserialize, Serialize, Serializer};

use crate::agents::{AgentTable, ArgList, Level};
use crate::builtin_workflows;
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::profiles::Profile;
use crate::prompt;
use crate::workflow_upgrade;

/// How many times a failed gate may send the work back before the tab stops and asks.
pub const DEFAULT_MAX_LOOPS: u8 = 2;

/// What a stage does. It picks the default prompt, gate and permission level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Work,
    Review,
    Refactor,
    Reproduce,
    Fix,
    Handoff,
}

impl Action {
    pub const ALL: [Action; 6] = [
        Action::Work,
        Action::Review,
        Action::Refactor,
        Action::Reproduce,
        Action::Fix,
        Action::Handoff,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Action::Work => "Work",
            Action::Review => "Review",
            Action::Refactor => "Refactor",
            Action::Reproduce => "Reproduce",
            Action::Fix => "Fix",
            Action::Handoff => "Hand off",
        }
    }

    #[must_use]
    pub fn default_gate(self) -> Gate {
        match self {
            Action::Work => Gate::Tests,
            Action::Review => Gate::NoBlocking,
            Action::Refactor => Gate::SameBehavior,
            Action::Reproduce => Gate::TestFails,
            Action::Fix => Gate::TestPasses,
            Action::Handoff => Gate::Approve,
        }
    }

    /// Reading stages default to Read only, so a reviewer cannot "fix" what it reviews.
    #[must_use]
    pub fn default_level(self) -> Level {
        match self {
            Action::Review | Action::Handoff => Level::Plan,
            _ => Level::Edits,
        }
    }

    #[must_use]
    pub fn default_prompt(self) -> &'static str {
        match self {
            Action::Work => {
                "{task}\n\nWork in small steps. Commit after each step that passes the tests. \
                 Do not push."
            }
            Action::Review => {
                "Review {base}..HEAD for bugs, edge cases and missing tests.\nReport each \
                 finding with file, line, blocking or nit, and a one-line fix.\nDo not edit \
                 files."
            }
            Action::Refactor => {
                "Simplify the code changed in {base}..HEAD: remove duplication, improve \
                 names.\nDo not change behavior. Keep the tests green. Commit when done."
            }
            Action::Reproduce => {
                "{task}\n\nWrite one test that fails because of this bug. Do not fix the bug yet."
            }
            Action::Fix => {
                "Make the failing test pass with the smallest change.\nDo not edit the test. \
                 Open findings:\n{findings}\n\nOutput of the failing test run:\n{test_output}"
            }
            Action::Handoff => {
                "Write a PR title and description for {base}..HEAD.\nList what changed and how \
                 it was tested."
            }
        }
    }
}

/// What Tidewatch checks before the next stage starts. The agent is never trusted for this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// The repo's test command exits 0.
    Tests,
    /// The test command exits non-zero: the new test proves the bug.
    TestFails,
    /// The test command exits 0: the new test passes and the rest still do.
    TestPasses,
    /// The stage reported no blocking finding.
    NoBlocking,
    /// The test command exits 0: the tests that pin the behavior still pass.
    SameBehavior,
    /// The user approves.
    Approve,
}

impl Gate {
    pub const ALL: [Gate; 6] = [
        Gate::Tests,
        Gate::TestFails,
        Gate::TestPasses,
        Gate::NoBlocking,
        Gate::SameBehavior,
        Gate::Approve,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Gate::Tests => "Tests pass",
            Gate::TestFails => "A new test fails",
            Gate::TestPasses => "The new test passes, the rest still pass",
            Gate::NoBlocking => "No blocking findings",
            Gate::SameBehavior => "Tests pass, behavior unchanged",
            Gate::Approve => "You approve",
        }
    }

    #[must_use]
    pub fn runs_tests(self) -> bool {
        matches!(self, Gate::Tests | Gate::TestFails | Gate::TestPasses | Gate::SameBehavior)
    }
}

/// Who runs a stage: an optional profile reference plus overrides. Without a profile the stage
/// needs an agent; the rest falls back to the action's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(
        default,
        rename = "permissions",
        alias = "level",
        skip_serializing_if = "Option::is_none"
    )]
    pub level: Option<Level>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StageRaw")]
pub struct Stage {
    pub action: Action,
    #[serde(flatten)]
    pub run: StageRun,
    pub gate: Gate,
    /// Index of the earlier stage to go back to when the gate fails. `None` stops and asks.
    #[serde(serialize_with = "serialize_stage_number", skip_serializing_if = "Option::is_none")]
    pub on_fail: Option<usize>,
    #[serde(skip_serializing_if = "is_default_loops")]
    pub max_loops: u8,
    /// Wait for the user's approval once the gate has passed, before the next stage starts.
    #[serde(skip_serializing_if = "is_false")]
    pub approve: bool,
    /// Replaces the action's default prompt when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Deserialize)]
struct StageRaw {
    action: Action,
    #[serde(flatten)]
    run: StageRun,
    #[serde(default)]
    gate: Option<Gate>,
    #[serde(default)]
    on_fail: Option<usize>,
    #[serde(default = "default_max_loops")]
    max_loops: u8,
    #[serde(default)]
    approve: bool,
    #[serde(default)]
    prompt: Option<String>,
}

impl TryFrom<StageRaw> for Stage {
    type Error = String;

    fn try_from(raw: StageRaw) -> Result<Self, String> {
        let on_fail = match raw.on_fail {
            Some(0) => return Err("on_fail counts stages from 1".to_owned()),
            other => other.map(|number| number - 1),
        };
        Ok(Self {
            action: raw.action,
            run: raw.run,
            gate: raw.gate.unwrap_or_else(|| raw.action.default_gate()),
            on_fail,
            max_loops: raw.max_loops,
            approve: raw.approve,
            prompt: raw.prompt,
        })
    }
}

fn default_max_loops() -> u8 {
    DEFAULT_MAX_LOOPS
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_loops(loops: &u8) -> bool {
    *loops == DEFAULT_MAX_LOOPS
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(flag: &bool) -> bool {
    !*flag
}

#[allow(clippy::ref_option)]
fn serialize_stage_number<S: Serializer>(
    index: &Option<usize>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match index {
        Some(index) => serializer.serialize_u64(*index as u64 + 1),
        None => serializer.serialize_none(),
    }
}

impl Stage {
    /// A stage with the action's defaults and no agent yet.
    #[must_use]
    pub fn new(action: Action) -> Self {
        Self {
            action,
            run: StageRun::default(),
            gate: action.default_gate(),
            on_fail: None,
            max_loops: DEFAULT_MAX_LOOPS,
            approve: false,
            prompt: None,
        }
    }

    #[must_use]
    pub fn prompt_template(&self) -> &str {
        self.prompt.as_deref().unwrap_or_else(|| self.action.default_prompt())
    }

    /// True when the stage ends with the user's say, by its gate or by `approve`.
    #[must_use]
    pub fn waits_for_user(&self) -> bool {
        self.gate == Gate::Approve || self.approve
    }

    /// The launch profile: the referenced profile (or a fresh one for the agent) with this stage's
    /// overrides on top. A different agent than the profile's drops the profile's model and effort,
    /// which name the other agent's models.
    pub fn resolve_profile(
        &self,
        workflow_name: &str,
        profiles: &BTreeMap<String, Profile>,
    ) -> Result<Profile, String> {
        let run = &self.run;
        let mut profile = match (&run.profile, &run.agent) {
            (Some(id), _) => {
                profiles.get(id).cloned().ok_or_else(|| format!("unknown profile \"{id}\""))?
            }
            (None, Some(agent)) => Profile {
                name: format!("{workflow_name} - {}", self.action.label()),
                agent: agent.clone(),
                model: None,
                effort: None,
                level: self.action.default_level(),
                setup: String::new(),
                never_push: true,
                extra_args: ArgList::default(),
                house_style: true,
            },
            (None, None) => return Err("needs a profile or an agent".to_owned()),
        };
        if let Some(agent) = &run.agent
            && *agent != profile.agent
        {
            profile.agent.clone_from(agent);
            profile.model = None;
            profile.effort = None;
        }
        if run.model.is_some() {
            profile.model.clone_from(&run.model);
        }
        if run.effort.is_some() {
            profile.effort.clone_from(&run.effort);
        }
        if let Some(level) = run.level {
            profile.level = level;
        }
        Ok(profile)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// The table key in the file. Not stored inside the table.
    #[serde(skip)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, rename = "stage")]
    pub stages: Vec<Stage>,
}

impl Workflow {
    /// What stops this workflow from running, in words for the settings editor. Empty means it can run.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.stages.is_empty() {
            problems.push("has no stages".to_owned());
        }
        for (index, stage) in self.stages.iter().enumerate() {
            let number = index + 1;
            let label = stage.action.label();
            if stage.run.profile.is_none() && stage.run.agent.is_none() {
                problems.push(format!("stage {number} ({label}) needs a profile or an agent"));
            }
            if let Some(target) = stage.on_fail
                && target >= index
            {
                problems.push(format!(
                    "stage {number} ({label}): on_fail must name an earlier stage, not stage {}",
                    target + 1
                ));
            }
            for name in prompt::unknown_placeholders(stage.prompt_template()) {
                problems.push(format!("stage {number} ({label}): unknown placeholder {{{name}}}"));
            }
        }
        problems
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflows {
    #[serde(default)]
    pub workflow: BTreeMap<String, Workflow>,
}

/// The ids of every built-in workflow, in the order the new-tab dialog lists them.
pub const BUILT_IN_IDS: [&str; 9] = [
    "ship",
    "fix",
    "pr",
    builtin_workflows::ROOT_CAUSE,
    builtin_workflows::FINISH,
    builtin_workflows::UI_PROOF,
    builtin_workflows::PORT,
    builtin_workflows::RELEASE,
    builtin_workflows::REFACTOR,
];

/// The built-ins that every `workflows.toml` on disk already had when later ones were added.
pub const ORIGINAL_IDS: [&str; 3] = ["ship", "fix", "pr"];

/// Agents ordered by how good a stand-in they make when a default workflow names one that is not
/// installed.
const FALLBACK_ORDER: [&str; 4] = ["claude", "gemini", "copilot", "codex"];

pub(crate) fn default_stage(
    action: Action,
    agent: &str,
    model: Option<&str>,
    effort: Option<&str>,
    level: Level,
) -> Stage {
    Stage {
        run: StageRun {
            profile: None,
            agent: Some(agent.to_owned()),
            model: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            level: Some(level),
        },
        ..Stage::new(action)
    }
}

impl Workflows {
    /// Parses the file text. A workflow without a `name` is named after its key.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut parsed: Self = toml::from_str(text)?;
        for (id, workflow) in &mut parsed.workflow {
            id.clone_into(&mut workflow.id);
            if workflow.name.is_empty() {
                workflow.name.clone_from(id);
            }
        }
        Ok(parsed)
    }

    /// Ship a feature, Fix a bug and Review a PR, as the concept defines them, then the six habit
    /// workflows of `builtin_workflows`.
    #[must_use]
    pub fn defaults() -> Self {
        let ship = Workflow {
            id: "ship".into(),
            name: "Ship a feature".into(),
            description: "Build, get a second opinion, tidy up, hand off.".into(),
            stages: vec![
                default_stage(Action::Work, "claude", Some("opus"), Some("high"), Level::Edits),
                Stage {
                    on_fail: Some(0),
                    ..default_stage(Action::Review, "gemini", None, None, Level::Plan)
                },
                default_stage(Action::Refactor, "claude", Some("sonnet"), None, Level::Edits),
                Stage {
                    approve: true,
                    ..default_stage(Action::Handoff, "claude", Some("haiku"), None, Level::Plan)
                },
            ],
        };
        let fix = Workflow {
            id: "fix".into(),
            name: "Fix a bug".into(),
            description: "Prove the bug with a test, fix it, check the fix.".into(),
            stages: vec![
                default_stage(Action::Reproduce, "codex", None, None, Level::Auto),
                default_stage(Action::Fix, "codex", None, None, Level::Auto),
                Stage {
                    on_fail: Some(1),
                    ..default_stage(Action::Review, "claude", Some("sonnet"), None, Level::Plan)
                },
            ],
        };
        let pr = Workflow {
            id: "pr".into(),
            name: "Review a PR".into(),
            description: "Two models review, one writes the summary.".into(),
            stages: vec![
                default_stage(Action::Review, "claude", Some("sonnet"), None, Level::Plan),
                default_stage(Action::Review, "gemini", None, None, Level::Plan),
                default_stage(Action::Handoff, "claude", Some("haiku"), None, Level::Plan),
            ],
        };
        let workflow = [ship, fix, pr]
            .into_iter()
            .chain(builtin_workflows::all())
            .map(|w| (w.id.clone(), w))
            .collect();
        Self { workflow }
    }

    /// The defaults for a machine that has only some of the agents: a stage whose agent is missing
    /// moves to the best agent found, without its model and effort. With no agent found the
    /// defaults stay as they are.
    #[must_use]
    pub fn seeded(found: &[String]) -> Self {
        let mut workflows = Self::defaults();
        let Some(stand_in) = FALLBACK_ORDER.iter().find(|id| found.iter().any(|f| f == *id)) else {
            return workflows;
        };
        for stage in workflows.workflow.values_mut().flat_map(|w| w.stages.iter_mut()) {
            let missing =
                stage.run.agent.as_ref().is_some_and(|agent| !found.iter().any(|f| f == agent));
            if missing {
                stage.run.agent = Some((*stand_in).to_owned());
                stage.run.model = None;
                stage.run.effort = None;
            }
        }
        workflows
    }

    /// A missing file is the built-in workflows. An unreadable or invalid one is the built-in
    /// workflows plus a stderr note, and stays on disk for the user to fix.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::defaults(),
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); using defaults", file.display());
                return Self::defaults();
            }
        };
        let Ok(parsed) = Self::parse(&text).inspect_err(|e| {
            eprintln!("tidewatch: workflows.toml is invalid ({e}); using defaults");
        }) else {
            return Self::defaults();
        };
        for workflow in parsed.workflow.values() {
            for problem in workflow.validate() {
                eprintln!(
                    "tidewatch: workflow \"{}\" cannot run until fixed: {problem}",
                    workflow.id
                );
            }
        }
        parsed
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }

    /// First launch: when `workflows.toml` does not exist, seed it from the agents found on
    /// `path_var` and write the file. Afterwards it only loads, so the user's edits are never
    /// reseeded; the one exception is a built-in the file has never been offered, which is
    /// appended once (see `workflow_upgrade`).
    #[must_use]
    pub fn load_or_seed(paths: &TidewatchPaths, path_var: Option<&OsStr>) -> Self {
        let file = paths.workflows_file();
        if file.exists() {
            workflow_upgrade::add_new_built_ins(paths, path_var);
            return Self::load(&file);
        }
        let seeded = Self::seeded(&AgentTable::builtin().found_on_path(path_var));
        match seeded.save(&file) {
            Ok(()) => workflow_upgrade::record_all_offered(paths),
            Err(e) => eprintln!("tidewatch: cannot write {} ({e})", file.display()),
        }
        seeded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn found(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn shape(workflow: &Workflow) -> Vec<(Action, Gate, Option<usize>, Option<&str>)> {
        workflow
            .stages
            .iter()
            .map(|s| (s.action, s.gate, s.on_fail, s.run.agent.as_deref()))
            .collect()
    }

    fn named(profile: &str) -> StageRun {
        StageRun { profile: Some(profile.into()), ..StageRun::default() }
    }

    #[test]
    fn the_concepts_three_keep_their_shape() {
        let defaults = Workflows::defaults();
        let names: Vec<_> =
            ["ship", "fix", "pr"].iter().map(|id| defaults.workflow[*id].name.as_str()).collect();
        assert_eq!(names, ["Ship a feature", "Fix a bug", "Review a PR"]);
        assert_eq!(
            shape(&defaults.workflow["ship"]),
            [
                (Action::Work, Gate::Tests, None, Some("claude")),
                (Action::Review, Gate::NoBlocking, Some(0), Some("gemini")),
                (Action::Refactor, Gate::SameBehavior, None, Some("claude")),
                (Action::Handoff, Gate::Approve, None, Some("claude")),
            ]
        );
        assert_eq!(
            shape(&defaults.workflow["fix"]),
            [
                (Action::Reproduce, Gate::TestFails, None, Some("codex")),
                (Action::Fix, Gate::TestPasses, None, Some("codex")),
                (Action::Review, Gate::NoBlocking, Some(1), Some("claude")),
            ]
        );
        assert_eq!(
            shape(&defaults.workflow["pr"]),
            [
                (Action::Review, Gate::NoBlocking, None, Some("claude")),
                (Action::Review, Gate::NoBlocking, None, Some("gemini")),
                (Action::Handoff, Gate::Approve, None, Some("claude")),
            ]
        );
    }

    #[test]
    fn every_default_workflow_validates() {
        for (id, workflow) in &Workflows::defaults().workflow {
            assert_eq!(workflow.validate(), Vec::<String>::new(), "{id}");
            assert_eq!(&workflow.id, id);
        }
    }

    #[test]
    fn default_review_stages_are_read_only_and_loops_default_to_two() {
        for workflow in Workflows::defaults().workflow.values() {
            for stage in &workflow.stages {
                if stage.action == Action::Review {
                    assert_eq!(stage.run.level, Some(Level::Plan), "{}", workflow.name);
                }
                assert_eq!(stage.max_loops, DEFAULT_MAX_LOOPS);
            }
        }
    }

    #[test]
    fn every_action_has_a_default_gate_prompt_and_label() {
        for action in Action::ALL {
            assert_ne!(action.default_prompt(), "", "{action:?}");
            assert_ne!(action.label(), "", "{action:?}");
            assert_eq!(Stage::new(action).gate, action.default_gate());
        }
        for gate in Gate::ALL {
            assert_ne!(gate.label(), "", "{gate:?}");
        }
    }

    #[test]
    fn the_defaults_survive_a_toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config").join("workflows.toml");
        let defaults = Workflows::defaults();
        defaults.save(&file).unwrap();
        assert_eq!(Workflows::load(&file), defaults);
    }

    #[test]
    fn the_file_counts_on_fail_from_one_and_hides_defaults() {
        let mut original = Workflows::defaults();
        original.workflow.retain(|id, _| ORIGINAL_IDS.contains(&id.as_str()));
        let text = toml::to_string(&original).unwrap();
        for expected in [
            "[workflow.ship]",
            "[[workflow.ship.stage]]",
            "on_fail = 1\n",
            "on_fail = 2\n",
            "permissions = \"plan\"",
            "approve = true",
        ] {
            assert!(text.contains(expected), "{expected} in {text}");
        }
        for hidden in ["max_loops", "prompt"] {
            assert!(!text.contains(hidden), "{hidden} in {text}");
        }
    }

    #[test]
    fn a_custom_prompt_loop_limit_and_profile_round_trip() {
        let stage = Stage {
            run: named("reviewer"),
            on_fail: Some(0),
            max_loops: 5,
            prompt: Some("Check {base}..HEAD\n\"quoted\" {task}".into()),
            ..Stage::new(Action::Review)
        };
        let first = default_stage(Action::Work, "claude", None, None, Level::Auto);
        let mine = Workflow {
            id: "mine".into(),
            name: "Mine".into(),
            description: "Two steps".into(),
            stages: vec![first, stage],
        };
        let workflows = Workflows { workflow: BTreeMap::from([("mine".to_owned(), mine)]) };
        let parsed = Workflows::parse(&toml::to_string(&workflows).unwrap()).unwrap();
        assert_eq!(parsed, workflows);
    }

    #[test]
    fn a_hand_written_file_gets_defaults_from_the_action_and_its_key_as_name() {
        let text = r#"
[[workflow.quick.stage]]
action = "work"
agent = "claude"
level = "auto"

[[workflow.quick.stage]]
action = "review"
agent = "gemini"
on_fail = 1
max_loops = 3
gate = "tests"
"#;
        let parsed = Workflows::parse(text).unwrap();
        let quick = &parsed.workflow["quick"];
        assert_eq!((quick.id.as_str(), quick.name.as_str()), ("quick", "quick"));
        let (work, review) = (&quick.stages[0], &quick.stages[1]);
        assert_eq!(
            (work.gate, work.run.level, work.max_loops),
            (Gate::Tests, Some(Level::Auto), DEFAULT_MAX_LOOPS)
        );
        assert_eq!((review.gate, review.on_fail, review.max_loops), (Gate::Tests, Some(0), 3));
        assert_eq!(quick.validate(), Vec::<String>::new());
    }

    #[test]
    fn on_fail_zero_and_unknown_names_are_parse_errors() {
        let cases = [
            "[[workflow.a.stage]]\naction = \"review\"\nagent = \"x\"\non_fail = 0\n",
            "[[workflow.a.stage]]\naction = \"deploy\"\nagent = \"x\"\n",
            "[[workflow.a.stage]]\naction = \"work\"\nagent = \"x\"\ngate = \"vibes\"\n",
            "[[workflow.a.stage]]\nagent = \"x\"\n",
        ];
        for text in cases {
            assert!(Workflows::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn validate_names_every_problem() {
        let workflow = Workflow {
            id: "bad".into(),
            name: "Bad".into(),
            description: String::new(),
            stages: vec![
                Stage::new(Action::Work),
                Stage {
                    run: StageRun { agent: Some("claude".into()), ..StageRun::default() },
                    on_fail: Some(1),
                    prompt: Some("{taks}".into()),
                    ..Stage::new(Action::Review)
                },
            ],
        };
        assert_eq!(
            workflow.validate(),
            [
                "stage 1 (Work) needs a profile or an agent",
                "stage 2 (Review): on_fail must name an earlier stage, not stage 2",
                "stage 2 (Review): unknown placeholder {taks}",
            ]
        );
        let empty = Workflow { stages: Vec::new(), ..workflow };
        assert_eq!(empty.validate(), ["has no stages"]);
    }

    #[test]
    fn a_forward_on_fail_is_invalid() {
        let mut ship = Workflows::defaults().workflow.remove("ship").unwrap();
        ship.stages[0].on_fail = Some(2);
        assert_eq!(ship.validate().len(), 1);
    }

    #[test]
    fn missing_file_loads_the_defaults_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("workflows.toml");
        assert_eq!(Workflows::load(&file), Workflows::defaults());
        assert!(!file.exists());
    }

    #[test]
    fn bad_file_loads_the_defaults_and_stays_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("workflows.toml");
        let garbage = "[workflow.x\nthis is = not toml {{";
        std::fs::write(&file, garbage).unwrap();
        assert_eq!(Workflows::load(&file), Workflows::defaults());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), garbage);
    }

    #[test]
    fn a_workflow_with_problems_still_loads_so_saving_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("workflows.toml");
        std::fs::write(&file, "[[workflow.odd.stage]]\naction = \"work\"\n").unwrap();
        let loaded = Workflows::load(&file);
        assert_eq!(loaded.workflow.keys().collect::<Vec<_>>(), ["odd"]);
        assert_ne!(loaded.workflow["odd"].validate(), Vec::<String>::new());
    }

    #[test]
    fn first_launch_seeds_once_and_never_reseeds() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let bin = tempfile::tempdir().unwrap();
        for name in ["claude", "gemini"] {
            let exe = bin.path().join(name);
            std::fs::write(&exe, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([bin.path()]).unwrap();

        let seeded = Workflows::load_or_seed(&paths, Some(&path));
        assert_eq!(Workflows::load(&paths.workflows_file()), seeded);
        assert_eq!(seeded.workflow["fix"].stages[0].run.agent.as_deref(), Some("claude"));
        assert_eq!(seeded.workflow["ship"].stages[1].run.agent.as_deref(), Some("gemini"));

        let edited = "[[workflow.mine.stage]]\naction = \"work\"\nagent = \"claude\"\n";
        std::fs::write(paths.workflows_file(), edited).unwrap();
        let after_edit = Workflows::load_or_seed(&paths, Some(&path));
        assert_eq!(after_edit.workflow.keys().collect::<Vec<_>>(), ["mine"]);
    }

    #[test]
    fn missing_agents_are_replaced_by_the_best_one_found() {
        let seeded = Workflows::seeded(&found(&["claude"]));
        let ship = &seeded.workflow["ship"];
        assert_eq!(ship.stages[1].run.agent.as_deref(), Some("claude"));
        assert_eq!(ship.stages[1].run.model, None);
        assert_eq!(ship.stages[1].run.level, Some(Level::Plan));
        assert_eq!(ship.stages[0].run.model.as_deref(), Some("opus"));
        assert_eq!(seeded.workflow["fix"].stages[0].run.agent.as_deref(), Some("claude"));
        for workflow in seeded.workflow.values() {
            assert_eq!(workflow.validate(), Vec::<String>::new());
        }
    }

    #[test]
    fn nothing_found_or_everything_found_leaves_the_defaults() {
        assert_eq!(Workflows::seeded(&[]), Workflows::defaults());
        assert_eq!(
            Workflows::seeded(&found(&["claude", "gemini", "codex", "copilot"])),
            Workflows::defaults()
        );
    }

    #[test]
    fn a_profile_reference_takes_overrides_and_drops_the_other_agents_model() {
        let profile = Profile {
            name: "Builder".into(),
            agent: "claude".into(),
            model: Some("opus".into()),
            effort: Some("high".into()),
            level: Level::Edits,
            setup: "npm ci".into(),
            never_push: true,
            extra_args: ArgList::default(),
            house_style: true,
        };
        let profiles = BTreeMap::from([("builder".to_owned(), profile.clone())]);
        let resolve = |run: StageRun| {
            Stage { run, ..Stage::new(Action::Work) }.resolve_profile("W", &profiles)
        };

        assert_eq!(resolve(named("builder")), Ok(profile));

        let level_and_model =
            StageRun { level: Some(Level::Plan), model: Some("sonnet".into()), ..named("builder") };
        let overridden = resolve(level_and_model).unwrap();
        assert_eq!(
            (overridden.model.as_deref(), overridden.effort.as_deref(), overridden.level),
            (Some("sonnet"), Some("high"), Level::Plan)
        );
        assert_eq!(overridden.setup, "npm ci");

        let other_agent = StageRun { agent: Some("gemini".into()), ..named("builder") };
        let switched = resolve(other_agent).unwrap();
        assert_eq!(
            (switched.agent.as_str(), switched.model, switched.effort),
            ("gemini", None, None)
        );

        assert_eq!(resolve(named("ghost")), Err("unknown profile \"ghost\"".to_owned()));
        assert_eq!(resolve(StageRun::default()), Err("needs a profile or an agent".to_owned()));
    }

    #[test]
    fn an_agent_only_stage_gets_the_actions_default_level_and_never_pushes() {
        let agent = StageRun { agent: Some("claude".into()), ..StageRun::default() };
        let cases = [
            (Action::Review, Level::Plan),
            (Action::Work, Level::Edits),
            (Action::Handoff, Level::Plan),
        ];
        for (action, level) in cases {
            let stage = Stage { run: agent.clone(), ..Stage::new(action) };
            let profile = stage.resolve_profile("Ship", &BTreeMap::new()).unwrap();
            assert_eq!((profile.level, profile.never_push), (level, true), "{action:?}");
            assert_eq!(profile.name, format!("Ship - {}", action.label()));
        }
    }

    #[test]
    fn a_stage_waits_for_the_user_by_gate_or_flag() {
        let cases = [
            (Stage::new(Action::Work), false),
            (Stage { approve: true, ..Stage::new(Action::Work) }, true),
            (Stage::new(Action::Handoff), true),
        ];
        for (stage, waits) in cases {
            assert_eq!(stage.waits_for_user(), waits, "{:?}", stage.action);
        }
    }
}
