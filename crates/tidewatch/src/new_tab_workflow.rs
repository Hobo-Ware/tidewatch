//! The workflow side of the new-tab form as plain data: order, stage chips and cards, missing agents, the request.

use std::collections::BTreeMap;

use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::runner::{RunContext, Runner};
use tidewatch_core::workflows::{BUILT_IN_IDS, Workflow, Workflows};

use crate::new_tab::Placement;
use crate::screen_kit::cli_short_name;
use crate::workflow_glue::WorkflowRequest;

/// The built-in workflows first, in the order of `BUILT_IN_IDS`, then the user's own.
pub(crate) fn ordered(workflows: &Workflows) -> Vec<&Workflow> {
    let first = BUILT_IN_IDS.iter().filter_map(|id| workflows.workflow.get(*id));
    let rest = workflows
        .workflow
        .iter()
        .filter_map(|(id, workflow)| (!BUILT_IN_IDS.contains(&id.as_str())).then_some(workflow));
    first.chain(rest).collect()
}

pub(crate) fn stage_count(count: usize) -> String {
    if count == 1 { "1 stage".to_owned() } else { format!("{count} stages") }
}

/// The stage names of a workflow, in order: the chips of its card.
pub(crate) fn chips(workflow: &Workflow) -> Vec<&'static str> {
    workflow.stages.iter().map(|stage| stage.action.label()).collect()
}

/// One stage of the picked workflow: its place, name and who runs it (the gate when nobody can).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageCard {
    pub(crate) number: usize,
    pub(crate) title: &'static str,
    pub(crate) detail: String,
    pub(crate) gate: &'static str,
}

pub(crate) fn stage_cards(workflow: &Workflow, profiles: &Profiles) -> Vec<StageCard> {
    workflow
        .stages
        .iter()
        .zip(resolved(workflow, &profiles.profile))
        .enumerate()
        .map(|(index, (stage, profile))| {
            let detail = profile.map_or_else(
                || stage.gate.label().to_owned(),
                |profile| {
                    let who = match stage.run.profile {
                        Some(_) => profile.name,
                        None => cli_short_name(&profile.agent),
                    };
                    format!("{who} \u{b7} {}", profile.level.label())
                },
            );
            StageCard {
                number: index + 1,
                title: stage.action.label(),
                detail,
                gate: stage.gate.label(),
            }
        })
        .collect()
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

/// The request for `workflow`, or why it cannot run. It carries no task and the default placement:
/// the form fills them in.
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
        placement: Placement::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(ids(&workflows), BUILT_IN_IDS);
        let mut own = workflows.workflow["pr"].clone();
        own.id = "audit".into();
        workflows.workflow.insert("audit".into(), own);
        let mut with_own = BUILT_IN_IDS.to_vec();
        with_own.push("audit");
        assert_eq!(ids(&workflows), with_own);
        workflows.workflow.remove("fix");
        with_own.remove(1);
        assert_eq!(ids(&workflows), with_own);
    }

    #[test]
    fn the_stage_count_is_singular_for_one() {
        assert_eq!(stage_count(1), "1 stage");
        assert_eq!(stage_count(4), "4 stages");
    }

    #[test]
    fn the_chips_are_the_stage_names_in_order() {
        let workflows = defaults();
        assert_eq!(chips(&workflows.workflow["ship"]), ["Work", "Review", "Refactor", "Hand off"]);
        assert_eq!(chips(&workflows.workflow["fix"]), ["Reproduce", "Fix", "Review"]);
        assert_eq!(chips(&Workflow::default()), Vec::<&str>::new());
    }

    #[test]
    fn a_stage_card_names_who_runs_it_and_falls_back_to_the_gate() {
        let workflows = defaults();
        let cards = stage_cards(&workflows.workflow["ship"], &profiles());
        let line = |card: &StageCard| format!("{} {} - {}", card.number, card.title, card.detail);
        assert_eq!(
            cards.iter().map(line).collect::<Vec<_>>(),
            [
                "1 Work - Claude \u{b7} Auto edit",
                "2 Review - Gemini \u{b7} Read only",
                "3 Refactor - Claude \u{b7} Auto edit",
                "4 Hand off - Claude \u{b7} Read only",
            ]
        );
        assert_eq!(cards[0].gate, "Tests pass");
        let mut own = workflows.workflow["ship"].clone();
        own.stages[0].run.profile = Some("builder".into());
        own.stages[1].run.agent = None;
        let cards = stage_cards(&own, &profiles());
        assert_eq!(cards[0].detail, "Builder \u{b7} Auto edit");
        assert_eq!(cards[1].detail, cards[1].gate);
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
    fn the_request_carries_the_profiles_and_the_first_agent_but_no_task_and_a_plain_placement() {
        let workflows = defaults();
        let request = build(&workflows.workflow["ship"], &profiles()).unwrap();
        assert_eq!(request.task, "");
        assert_eq!(request.placement, Placement::default());
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
