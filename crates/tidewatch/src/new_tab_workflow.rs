//! The workflow side of the new-tab dialog as plain data: order, missing agents, the request.

use std::collections::BTreeMap;

use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::runner::{RunContext, Runner};
use tidewatch_core::workflows::{Workflow, Workflows};

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

pub(crate) fn stage_count(count: usize) -> String {
    if count == 1 { "1 stage".to_owned() } else { format!("{count} stages") }
}

pub(crate) fn tag(workflow: &Workflow) -> String {
    stage_count(workflow.stages.len())
}

fn resolved(workflow: &Workflow, profiles: &BTreeMap<String, Profile>) -> Vec<Option<Profile>> {
    workflow
        .stages
        .iter()
        .map(|stage| stage.resolve_profile(&workflow.name, profiles).ok())
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

/// The request Enter turns into, or why the workflow cannot run. It carries no task: the tab asks
/// for it.
pub(crate) fn build(workflow: &Workflow, profiles: &Profiles) -> Result<WorkflowRequest, String> {
    let trial = RunContext {
        task: String::new(),
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
        task: String::new(),
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
    fn the_request_carries_the_profiles_and_the_first_agent_but_no_task() {
        let workflows = defaults();
        let request = build(&workflows.workflow["ship"], &profiles()).unwrap();
        assert_eq!(request.task, "");
        assert_eq!(request.first_agent, "claude");
        assert_eq!(request.workflow, workflows.workflow["ship"]);
        assert_eq!(request.profiles, profiles().profile);
    }

    #[test]
    fn a_workflow_that_cannot_run_says_what_is_wrong() {
        let mut workflow = defaults().workflow["ship"].clone();
        workflow.stages[0].run.agent = None;
        let error = build(&workflow, &profiles()).unwrap_err();
        assert_eq!(
            error,
            "Ship a feature cannot run: stage 1 (Work) needs a profile or an agent. Fix it in Settings."
        );
    }
}
