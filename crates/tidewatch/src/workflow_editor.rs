//! The settings editor's state for `workflows.toml`: one draft per workflow and per stage, text in
//! and out of the fields, the problems that stop a workflow from running (shown next to the fields)
//! and the workflows to save. No drawing; `workflow_editor_view` draws it.
//!
//! In the editor a stage goes back to an earlier stage by its index from 0; the file counts from 1
//! (`Stage::on_fail`), which `Workflows::save` takes care of.

use std::collections::BTreeMap;

use tidewatch_core::agents::Level;
use tidewatch_core::propose::{self, MAX_LOOPS_LIMIT};
use tidewatch_core::workflows::{
    Action, DEFAULT_MAX_LOOPS, Gate, Stage, StageRun, Workflow, Workflows,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageDraft {
    pub(crate) action: Action,
    /// A profile id, or empty.
    pub(crate) profile: String,
    /// An agent id, or empty. With a profile it replaces the profile's agent.
    pub(crate) agent: String,
    pub(crate) model: String,
    pub(crate) effort: String,
    /// `None` takes the action's default level, or the profile's.
    pub(crate) level: Option<Level>,
    pub(crate) gate: Gate,
    /// The earlier stage (from 0) the work goes back to when the gate fails.
    pub(crate) on_fail: Option<usize>,
    pub(crate) max_loops: String,
    pub(crate) approve: bool,
    /// Empty takes the action's default prompt.
    pub(crate) prompt: String,
}

fn optional(text: &str) -> Option<String> {
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

impl StageDraft {
    fn from_stage(stage: &Stage) -> Self {
        Self {
            action: stage.action,
            profile: stage.run.profile.clone().unwrap_or_default(),
            agent: stage.run.agent.clone().unwrap_or_default(),
            model: stage.run.model.clone().unwrap_or_default(),
            effort: stage.run.effort.clone().unwrap_or_default(),
            level: stage.run.level,
            gate: stage.gate,
            on_fail: stage.on_fail,
            max_loops: stage.max_loops.to_string(),
            approve: stage.approve,
            prompt: stage.prompt.clone().unwrap_or_default(),
        }
    }

    fn new(action: Action, agent: &str) -> Self {
        Self {
            agent: agent.to_owned(),
            ..Self::from_stage(&Stage { run: StageRun::default(), ..Stage::new(action) })
        }
    }

    /// The stage, and what is wrong with the text in its fields.
    fn to_stage(&self, number: usize) -> (Stage, Option<String>) {
        let (max_loops, problem) = match self.max_loops.trim().parse::<u8>() {
            Ok(loops) if (1..=MAX_LOOPS_LIMIT).contains(&loops) => (loops, None),
            _ => (
                DEFAULT_MAX_LOOPS,
                Some(format!(
                    "stage {number} ({}): loops must be a number from 1 to {MAX_LOOPS_LIMIT}",
                    self.action.label()
                )),
            ),
        };
        let stage = Stage {
            action: self.action,
            run: StageRun {
                profile: optional(&self.profile),
                agent: optional(&self.agent),
                model: optional(&self.model),
                effort: optional(&self.effort),
                level: self.level,
            },
            gate: self.gate,
            on_fail: self.on_fail,
            max_loops,
            approve: self.approve,
            prompt: Some(self.prompt.clone()).filter(|prompt| !prompt.trim().is_empty()),
        };
        (stage, problem)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowDraft {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) stages: Vec<StageDraft>,
    /// The tab whose agent proposed this workflow, until it is saved or reverted.
    pub(crate) from: Option<String>,
}

impl WorkflowDraft {
    fn of(id: &str, workflow: &Workflow) -> Self {
        Self {
            id: id.to_owned(),
            name: workflow.name.clone(),
            description: workflow.description.clone(),
            stages: workflow.stages.iter().map(StageDraft::from_stage).collect(),
            from: None,
        }
    }
}

pub(crate) struct Editor {
    pub(crate) drafts: Vec<WorkflowDraft>,
    pub(crate) selected: usize,
    pub(crate) stage: usize,
    baseline: Workflows,
    agents: Vec<(String, String)>,
    profiles: Vec<(String, String)>,
    problems: Vec<Vec<String>>,
    pub(crate) error: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) saving: bool,
}

impl Editor {
    /// `agents` are `(id, display name)` of every agent, `profiles` the same for every profile.
    pub(crate) fn new(
        workflows: &Workflows,
        agents: Vec<(String, String)>,
        profiles: Vec<(String, String)>,
    ) -> Self {
        let mut editor = Self {
            drafts: Self::drafts_of(workflows),
            selected: 0,
            stage: 0,
            baseline: workflows.clone(),
            agents,
            profiles,
            problems: Vec::new(),
            error: None,
            dirty: false,
            saving: false,
        };
        editor.check();
        editor
    }

    fn drafts_of(workflows: &Workflows) -> Vec<WorkflowDraft> {
        workflows.workflow.iter().map(|(id, workflow)| WorkflowDraft::of(id, workflow)).collect()
    }

    pub(crate) fn agents(&self) -> &[(String, String)] {
        &self.agents
    }

    pub(crate) fn profiles(&self) -> &[(String, String)] {
        &self.profiles
    }

    /// The profiles changed (saved in the other section): stages that name one are checked again.
    pub(crate) fn set_profiles(&mut self, profiles: Vec<(String, String)>) {
        self.profiles = profiles;
        self.check();
    }

    /// What is wrong with workflow `index`, in words, as of the last edit.
    pub(crate) fn problems(&self, index: usize) -> &[String] {
        self.problems.get(index).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn problem_count(&self) -> usize {
        self.problems.iter().map(Vec::len).sum()
    }

    pub(crate) fn select(&mut self, index: usize) {
        if index < self.drafts.len() && index != self.selected {
            self.selected = index;
            self.stage = 0;
        }
    }

    pub(crate) fn select_stage(&mut self, index: usize) {
        if self.drafts.get(self.selected).is_some_and(|d| index < d.stages.len()) {
            self.stage = index;
        }
    }

    pub(crate) fn touched(&mut self) {
        self.dirty = true;
        self.error = None;
        self.check();
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

    fn first_agent(&self) -> String {
        self.agents.first().map(|(id, _)| id.clone()).unwrap_or_default()
    }

    /// Adds a workflow with one Work stage on the first agent, selects it and returns its id.
    pub(crate) fn add_workflow(&mut self) -> String {
        let id = self.unused_id("new-workflow");
        let agent = self.first_agent();
        self.drafts.push(WorkflowDraft {
            id: id.clone(),
            name: "New workflow".to_owned(),
            description: String::new(),
            stages: vec![StageDraft::new(Action::Work, &agent)],
            from: None,
        });
        self.selected = self.drafts.len() - 1;
        self.stage = 0;
        self.touched();
        id
    }

    /// Adds an agent's proposal as a new unsaved draft marked as coming from `from`, selects it and
    /// returns the name it got: the proposal's, or with a number when the name is taken.
    pub(crate) fn propose(&mut self, workflow: &Workflow, from: &str) -> String {
        let name = propose::unique_name(
            workflow.name.trim(),
            self.drafts.iter().map(|draft| draft.name.trim()),
        );
        let id = self.unused_id(&propose::id_stem(&name));
        let mut draft = WorkflowDraft::of(&id, workflow);
        draft.name.clone_from(&name);
        draft.from = Some(from.to_owned());
        self.drafts.push(draft);
        self.selected = self.drafts.len() - 1;
        self.stage = 0;
        self.touched();
        name
    }

    /// Throws every unsaved edit away, a proposed draft with them.
    pub(crate) fn revert(&mut self) {
        self.drafts = Self::drafts_of(&self.baseline);
        self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
        self.stage = 0;
        self.dirty = false;
        self.error = None;
        self.check();
    }

    pub(crate) fn remove_workflow(&mut self) {
        if self.selected < self.drafts.len() {
            self.drafts.remove(self.selected);
            self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
            self.stage = 0;
            self.touched();
        }
    }

    /// Adds a stage after the last one, on the agent the last stage uses, and selects it.
    pub(crate) fn add_stage(&mut self) {
        let fallback = self.first_agent();
        let Some(draft) = self.drafts.get_mut(self.selected) else { return };
        let agent = draft
            .stages
            .last()
            .map(|stage| stage.agent.clone())
            .filter(|agent| !agent.is_empty())
            .unwrap_or(fallback);
        draft.stages.push(StageDraft::new(Action::Work, &agent));
        self.stage = draft.stages.len() - 1;
        self.touched();
    }

    /// Removes the selected stage. Stages that went back to it lose that link, later links move up.
    pub(crate) fn remove_stage(&mut self) {
        let Some(draft) = self.drafts.get_mut(self.selected) else { return };
        if self.stage >= draft.stages.len() {
            return;
        }
        let removed = self.stage;
        draft.stages.remove(removed);
        for stage in &mut draft.stages {
            stage.on_fail = match stage.on_fail {
                Some(target) if target == removed => None,
                Some(target) if target > removed => Some(target - 1),
                other => other,
            };
        }
        self.stage = removed.min(draft.stages.len().saturating_sub(1));
        self.touched();
    }

    /// Moves the selected stage one place earlier or later. A link that would then point forward is
    /// dropped: a gate can only send the work back.
    pub(crate) fn move_stage(&mut self, later: bool) {
        let Some(draft) = self.drafts.get_mut(self.selected) else { return };
        let to = if later { self.stage + 1 } else { self.stage.wrapping_sub(1) };
        if to >= draft.stages.len() || self.stage >= draft.stages.len() {
            return;
        }
        draft.stages.swap(self.stage, to);
        for (index, stage) in draft.stages.iter_mut().enumerate() {
            stage.on_fail = stage.on_fail.filter(|target| *target < index);
        }
        self.stage = to;
        self.touched();
    }

    fn convert(draft: &WorkflowDraft) -> (Workflow, Vec<String>) {
        let mut problems = Vec::new();
        let mut stages = Vec::with_capacity(draft.stages.len());
        for (index, stage) in draft.stages.iter().enumerate() {
            let (converted, problem) = stage.to_stage(index + 1);
            problems.extend(problem);
            stages.push(converted);
        }
        let workflow = Workflow {
            id: draft.id.clone(),
            name: draft.name.trim().to_owned(),
            description: draft.description.trim().to_owned(),
            stages,
        };
        if workflow.name.is_empty() {
            problems.push("the workflow needs a name".to_owned());
        }
        (workflow, problems)
    }

    fn problems_of(&self, draft: &WorkflowDraft) -> Vec<String> {
        let (workflow, mut problems) = Self::convert(draft);
        problems.extend(workflow.validate());
        for (index, stage) in workflow.stages.iter().enumerate() {
            let label = format!("stage {} ({})", index + 1, stage.action.label());
            if let Some(profile) = &stage.run.profile
                && !self.profiles.iter().any(|(id, _)| id == profile)
            {
                problems.push(format!("{label}: unknown profile \"{profile}\""));
            }
            if let Some(agent) = &stage.run.agent
                && !self.agents.iter().any(|(id, _)| id == agent)
            {
                problems.push(format!("{label}: unknown agent \"{agent}\""));
            }
        }
        problems
    }

    fn check(&mut self) {
        self.problems = self.drafts.iter().map(|draft| self.problems_of(draft)).collect();
    }

    /// The workflows to write, or why they cannot be written yet. The reasons are shown next to the
    /// fields (`problems`); this says how many.
    pub(crate) fn build(&self) -> Result<Workflows, String> {
        let count = self.problem_count();
        if count > 0 {
            let plural = if count == 1 { "problem" } else { "problems" };
            return Err(format!("Fix {count} {plural} before saving"));
        }
        let workflow: BTreeMap<String, Workflow> =
            self.drafts.iter().map(|draft| (draft.id.clone(), Self::convert(draft).0)).collect();
        Ok(Workflows { workflow })
    }

    /// Saved: the written workflows are the baseline.
    pub(crate) fn saved(&mut self) {
        if let Ok(written) = self.build() {
            self.baseline = written;
        }
        for draft in &mut self.drafts {
            draft.from = None;
        }
        self.saving = false;
        self.dirty = false;
        self.error = None;
    }

    pub(crate) fn save_failed(&mut self, message: String) {
        self.saving = false;
        self.error = Some(message);
    }
}

#[cfg(test)]
mod tests;
