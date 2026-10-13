//! The workflow side of the new-tab dialog, as plain data: the order workflows are listed in, what a
//! picked workflow looks like in the preview, which agents it needs that are missing, and the
//! request Enter turns into. `new_tab` calls these.

use std::collections::BTreeMap;

use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::runner::{RunContext, Runner};
use tidewatch_core::workflows::{Gate, Workflow, Workflows};

use crate::workflow_glue::WorkflowRequest;

/// The built-in workflows come first in the order the concept shows them, then the user's own.
const BUILT_IN_ORDER: [&str; 3] = ["ship", "fix", "pr"];

pub(crate) fn ordered(workflows: &Workflows) -> Vec<&Workflow> {
    let first = BUILT_IN_ORDER.iter().filter_map(|id| workflows.workflow.get(*id));
    let rest = workflows
        .workflow
        .iter()
        .filter_map(|(id, workflow)| (!BUILT_IN_ORDER.contains(&id.as_str())).then_some(workflow));
    first.chain(rest).collect()
}

pub(crate) fn tag(workflow: &Workflow) -> String {
    match workflow.stages.len() {
        1 => "1 stage".to_owned(),
        count => format!("{count} stages"),
    }
}

fn resolved(workflow: &Workflow, profiles: &BTreeMap<String, Profile>) -> Vec<Option<Profile>> {
    workflow
        .stages
        .iter()
        .map(|stage| stage.resolve_profile(&workflow.name, profiles).ok())
        .collect()
}

fn who(profile: &Profile) -> String {
    let model = profile.model.as_deref().map(|model| format!(" {model}")).unwrap_or_default();
    format!("{}{model}, {}", profile.agent, profile.level.label())
}

/// One line per stage: what it does, who does it and what it must pass.
pub(crate) fn preview_lines(workflow: &Workflow, profiles: &Profiles) -> Vec<String> {
    let found = resolved(workflow, &profiles.profile);
    workflow
        .stages
        .iter()
        .zip(found)
        .enumerate()
        .map(|(index, (stage, profile))| {
            let who = profile.as_ref().map_or_else(|| "no agent".to_owned(), who);
            let back = stage.on_fail.map_or_else(String::new, |target| {
                format!(", else back to {} (up to {}x)", target + 1, stage.max_loops)
            });
            let approval = if stage.waits_for_user() && stage.gate != Gate::Approve {
                ", then you approve"
            } else {
                ""
            };
            format!(
                "{}. {} ({who}) - {}{approval}{back}",
                index + 1,
                stage.action.label(),
                stage.gate.label().to_lowercase()
            )
        })
        .collect()
}

/// Warnings for the agents the workflow needs that were not found on `PATH` (`found` says whether one was).
pub(crate) fn missing_agents(
    workflow: &Workflow,
    profiles: &Profiles,
    found: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for profile in resolved(workflow, &profiles.profile).into_iter().flatten() {
        if !found(&profile.agent) && !missing.contains(&profile.agent) {
            missing.push(profile.agent);
        }
    }
    missing
        .into_iter()
        .map(|agent| {
            format!("{agent} was not found on PATH. A stage that uses it will fail to start.")
        })
        .collect()
}

/// The request Enter turns into, or why it cannot start.
pub(crate) fn build(
    workflow: &Workflow,
    profiles: &Profiles,
    task: &str,
) -> Result<WorkflowRequest, String> {
    let task = task.trim();
    if task.is_empty() {
        return Err("A workflow needs a task to work from.".to_owned());
    }
    let trial = RunContext {
        task: task.to_owned(),
        base: String::new(),
        branch: String::new(),
        test_command: None,
    };
    let runner = Runner::new(workflow, &profiles.profile, trial).map_err(|problems| {
        format!("{} cannot run: {}. Fix it in Settings.", workflow.name, problems.join("; "))
    })?;
    let first_agent = runner.profile(0).map(|profile| profile.agent.clone()).unwrap_or_default();
    Ok(WorkflowRequest {
        workflow: workflow.clone(),
        profiles: profiles.profile.clone(),
        task: task.to_owned(),
        first_agent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::workflows::{Action, Stage};

    fn profiles() -> Profiles {
        Profiles::seeded(&["claude".to_owned(), "gemini".to_owned()])
    }

    fn defaults() -> Workflows {
        Workflows::defaults()
    }

    fn ids(workflows: &Workflows) -> Vec<&str> {
        ordered(workflows).into_iter().map(|w| w.id.as_str()).collect()
    }

    #[test]
    fn the_built_in_workflows_come_in_the_concepts_order_then_the_users_own() {
        let mut workflows = defaults();
        assert_eq!(ids(&workflows), ["ship", "fix", "pr"]);
        let mut own = workflows.workflow["pr"].clone();
        own.id = "audit".into();
        workflows.workflow.insert("audit".into(), own);
        assert_eq!(ids(&workflows), ["ship", "fix", "pr", "audit"]);
        workflows.workflow.remove("fix");
        assert_eq!(ids(&workflows), ["ship", "pr", "audit"]);
    }

    #[test]
    fn the_tag_counts_the_stages() {
        let workflows = defaults();
        assert_eq!(tag(&workflows.workflow["ship"]), "4 stages");
        let one = Workflow { stages: vec![Stage::new(Action::Work)], ..Workflow::default() };
        assert_eq!(tag(&one), "1 stage");
    }

    #[test]
    fn the_preview_shows_every_stage_with_its_agent_gate_and_loop() {
        let workflows = defaults();
        assert_eq!(
            preview_lines(&workflows.workflow["ship"], &profiles()),
            [
                "1. Work (claude opus, Auto edit) - tests pass",
                "2. Review (gemini, Read only) - no blocking findings, else back to 1 (up to 2x)",
                "3. Refactor (claude sonnet, Auto edit) - tests pass, behavior unchanged",
                "4. Hand off (claude haiku, Read only) - you approve",
            ]
        );
    }

    #[test]
    fn a_stage_with_the_approve_flag_says_you_approve_after_its_gate() {
        let mut workflow = defaults().workflow["ship"].clone();
        workflow.stages[0].approve = true;
        assert!(preview_lines(&workflow, &profiles())[0].ends_with("tests pass, then you approve"));
    }

    #[test]
    fn a_stage_that_cannot_find_its_agent_says_so() {
        let mut workflow = defaults().workflow["ship"].clone();
        workflow.stages[1].run.agent = None;
        assert!(preview_lines(&workflow, &profiles())[1].starts_with("2. Review (no agent)"));
    }

    #[test]
    fn each_missing_agent_is_named_once() {
        let workflows = defaults();
        let ship = &workflows.workflow["ship"];
        let found = |agent: &str| agent == "claude";
        assert_eq!(
            missing_agents(ship, &profiles(), found),
            ["gemini was not found on PATH. A stage that uses it will fail to start."]
        );
        assert_eq!(missing_agents(ship, &profiles(), |_| true), Vec::<String>::new());
        assert_eq!(missing_agents(ship, &profiles(), |_| false).len(), 2);
    }

    #[test]
    fn a_workflow_needs_a_task() {
        let workflows = defaults();
        let error = build(&workflows.workflow["ship"], &profiles(), "  \n").unwrap_err();
        assert_eq!(error, "A workflow needs a task to work from.");
    }

    #[test]
    fn the_request_carries_the_trimmed_task_the_profiles_and_the_first_agent() {
        let workflows = defaults();
        let request = build(&workflows.workflow["ship"], &profiles(), "  add a retry\n").unwrap();
        assert_eq!(request.task, "add a retry");
        assert_eq!(request.first_agent, "claude");
        assert_eq!(request.workflow, workflows.workflow["ship"]);
        assert_eq!(request.profiles, profiles().profile);
    }

    #[test]
    fn a_workflow_that_cannot_run_says_what_is_wrong() {
        let mut workflow = defaults().workflow["ship"].clone();
        workflow.stages[0].run.agent = None;
        let error = build(&workflow, &profiles(), "x").unwrap_err();
        assert_eq!(
            error,
            "Ship a feature cannot run: stage 1 (Work) needs a profile or an agent. Fix it in Settings."
        );
    }
}
