use std::ffi::OsString;

use tidewatch_core::agents::{AgentOverride, ArgList, LevelOverrides};
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use super::*;

fn catalog_with(profiles: Profiles) -> Catalog {
    let mut catalog = Catalog::new(OsString::from("/usr/bin"));
    catalog.set_profiles(profiles);
    catalog
}

fn catalog() -> Catalog {
    catalog_with(Profiles::seeded(&["claude".to_owned(), "copilot".to_owned()]))
}

fn workflow_catalog() -> Catalog {
    let mut catalog = catalog();
    catalog.set_workflows(Workflows::defaults());
    catalog
}

fn open(group: Group, last: Option<&str>) -> NewTab {
    NewTab::open(group, "web".into(), PathBuf::from("/work/web"), Vec::new(), last)
}

fn in_agent_step(group: Group, catalog: &Catalog, last: Option<&str>) -> NewTab {
    let mut state = open(group, last);
    assert_eq!(state.choose(Kind::Agent, catalog), None);
    state
}

fn in_workflow_step(catalog: &Catalog) -> NewTab {
    let mut state = open(Group::Repo, None);
    assert_eq!(state.choose(Kind::Workflow, catalog), None);
    state
}

#[test]
fn the_first_screen_lists_nothing_and_a_plain_shell_is_a_request_at_once() {
    let catalog = catalog();
    let mut state = open(Group::Repo, None);
    assert_eq!(state.step(), Step::Choose);
    assert_eq!(state.visible(&catalog), Vec::<String>::new());
    assert!(state.request(&catalog).is_err());
    assert_eq!(state.choose(Kind::Shell, &catalog), Some(Request::Shell));
}

#[test]
fn a_workflow_is_offered_in_a_repo_only() {
    let catalog = workflow_catalog();
    assert!(open(Group::Repo, None).offers(Kind::Workflow));
    let mut other = open(Group::Other, None);
    assert!(!other.offers(Kind::Workflow));
    assert!(other.offers(Kind::Agent) && other.offers(Kind::Shell));
    assert_eq!(other.choose(Kind::Workflow, &catalog), None);
    assert_eq!(other.step(), Step::Choose);
    other.jump("workflow:ship", &catalog);
    assert_eq!(other.step(), Step::Choose);
}

#[test]
fn the_agent_step_groups_profiles_by_cli_in_table_order() {
    let catalog = catalog();
    let state = in_agent_step(Group::Repo, &catalog, None);
    let groups = state.agent_groups(&catalog);
    let names: Vec<&str> = groups.iter().map(|(agent, _)| agent.as_str()).collect();
    assert_eq!(names, ["claude", "copilot"]);
    assert_eq!(groups[0].1, ["builder", "reviewer"]);
    assert_eq!(groups[1].1, ["copilot-builder", "copilot-reviewer"]);
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("builder"));
    assert_eq!(state.level(), Level::Edits);
}

#[test]
fn the_agent_step_starts_on_the_last_used_profile_when_it_still_exists() {
    let catalog = catalog();
    let state = in_agent_step(Group::Repo, &catalog, Some("reviewer"));
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("reviewer"));
    assert_eq!(state.level(), Level::Plan);
    let state = in_agent_step(Group::Repo, &catalog, Some("deleted"));
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("builder"));
}

#[test]
fn the_filter_narrows_by_name_cli_model_or_level_and_resets_the_pick() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    state.step_selection(true, &catalog);
    state.set_filter("copilot", &catalog);
    assert_eq!(state.visible(&catalog).len(), 2);
    assert_eq!(state.selected(), 0);
    state.set_filter("read only", &catalog);
    assert_eq!(state.visible(&catalog), ["reviewer", "copilot-reviewer"]);
    state.set_filter("copilot read", &catalog);
    assert_eq!(
        state.visible(&catalog),
        Vec::<String>::new(),
        "each word is not matched on its own"
    );
    state.set_filter("opus", &catalog);
    assert_eq!(state.visible(&catalog), ["builder"]);
    state.set_filter("nothing like this", &catalog);
    assert_eq!(state.visible(&catalog), Vec::<String>::new());
    assert!(state.request(&catalog).is_err());
    state.set_filter("", &catalog);
    assert_eq!(state.visible(&catalog).len(), 4);
}

#[test]
fn an_agent_request_has_no_task_and_carries_the_picked_level() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    assert!(state.set_level(Level::Auto, &catalog));
    let Ok(Request::Agent(request)) = state.request(&catalog) else { panic!("agent request") };
    assert_eq!(request.profile_id, "builder");
    assert_eq!(request.profile.level, Level::Auto);
    assert_eq!(request.agent.id, "claude");
    assert_eq!(request.task, "");
}

#[test]
fn picking_a_profile_takes_its_level_and_clears_the_error() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    state.error = Some("old".into());
    state.select_id("reviewer", &catalog);
    assert_eq!((state.level(), state.error.clone()), (Level::Plan, None));
    state.select_id("builder", &catalog);
    assert_eq!(state.level(), Level::Edits);
}

#[test]
fn left_and_right_walk_the_open_levels_and_stop_at_the_ends() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    assert_eq!(state.level(), Level::Edits);
    assert!(state.step_level(true, &catalog));
    assert_eq!(state.level(), Level::Auto);
    assert!(state.step_level(true, &catalog));
    assert_eq!(state.level(), Level::Yolo);
    assert!(!state.step_level(true, &catalog));
    for _ in 0..5 {
        state.step_level(false, &catalog);
    }
    assert_eq!(state.level(), Level::Plan);
    assert!(!state.step_level(false, &catalog));
}

#[test]
fn yolo_is_open_in_a_repo_worktree_and_refused_in_other_with_a_reason() {
    let catalog = catalog();
    let agent = catalog.table().get("claude").unwrap();
    assert_eq!(level_refusal(Group::Repo, agent, Level::Yolo), None);
    let reason = level_refusal(Group::Other, agent, Level::Yolo).unwrap();
    assert!(reason.contains("not in Other"), "{reason}");
    for level in [Level::Plan, Level::Ask, Level::Edits, Level::Auto] {
        assert_eq!(level_refusal(Group::Other, agent, level), None, "{level:?}");
    }
}

#[test]
fn in_other_a_yolo_profile_opens_on_auto_and_the_arrows_never_reach_yolo() {
    let mut profiles = Profiles::seeded(&["claude".to_owned()]);
    profiles.profile.get_mut("builder").unwrap().level = Level::Yolo;
    let catalog = catalog_with(profiles);
    let mut state = in_agent_step(Group::Other, &catalog, Some("builder"));
    assert_eq!(state.level(), Level::Auto);
    assert!(!state.set_level(Level::Yolo, &catalog));
    assert!(!state.step_level(true, &catalog));
    assert_eq!(state.level(), Level::Auto);
    assert!(state.set_level(Level::Ask, &catalog));
}

#[test]
fn a_yolo_request_in_other_is_refused_even_if_the_level_was_forced() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Other, &catalog, Some("builder"));
    state.level = Level::Yolo;
    let error = state.request(&catalog).unwrap_err();
    assert!(error.starts_with("YOLO only runs"), "{error}");
}

#[test]
fn every_level_has_a_hint() {
    for level in Level::ALL {
        assert!(level_hint(level).ends_with('.'), "{level:?}");
    }
}

#[test]
fn the_workflow_step_lists_workflows_in_order_and_builds_a_request_with_no_task() {
    let catalog = workflow_catalog();
    let mut state = in_workflow_step(&catalog);
    assert_eq!(state.visible(&catalog), ["ship", "fix", "pr"]);
    assert_eq!(state.selected_workflow(&catalog).unwrap().name, "Ship a feature");
    assert!(state.selected_entry(&catalog).is_none(), "no profile ladder");
    assert!(!state.set_level(Level::Auto, &catalog), "a workflow has no level to pick");
    let Ok(Request::Workflow(request)) = state.request(&catalog) else {
        panic!("a workflow request")
    };
    assert_eq!((request.task.as_str(), request.workflow.id.as_str()), ("", "ship"));
    state.set_filter("bug", &catalog);
    assert_eq!(state.visible(&catalog), ["fix"]);
}

#[test]
fn a_workflow_that_vanished_since_the_dialog_opened_says_so() {
    let mut catalog = workflow_catalog();
    let state = in_workflow_step(&catalog);
    let mut workflows = Workflows::defaults();
    workflows.workflow.remove("ship");
    catalog.set_workflows(workflows);
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("fix"));
    assert!(state.request(&catalog).is_ok());
    let mut none = Workflows::defaults();
    none.workflow.clear();
    catalog.set_workflows(none);
    assert_eq!(
        state.request(&catalog),
        Err("No workflow to start. Add one in Settings.".to_owned())
    );
}

#[test]
fn with_no_profiles_the_agent_step_is_empty_and_says_where_to_add_one() {
    let catalog = catalog_with(Profiles::default());
    let state = in_agent_step(Group::Repo, &catalog, None);
    assert_eq!(state.visible(&catalog), Vec::<String>::new());
    assert_eq!(
        state.request(&catalog),
        Err("No profile to start. Add one in Settings.".to_owned())
    );
}

#[test]
fn back_returns_to_the_first_screen_only_when_the_dialog_came_from_it() {
    let catalog = workflow_catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    state.set_filter("copilot", &catalog);
    assert!(state.back());
    assert_eq!((state.step(), state.filter()), (Step::Choose, ""));
    assert!(!state.back());

    let mut jumped = open(Group::Repo, None);
    jumped.jump("reviewer", &catalog);
    assert_eq!(jumped.step(), Step::Agent);
    assert_eq!(jumped.selected_id(&catalog).as_deref(), Some("reviewer"));
    assert!(!jumped.back(), "nothing to go back to from the palette");

    let mut flow = open(Group::Repo, None);
    flow.jump("workflow:fix", &catalog);
    assert_eq!((flow.step(), flow.selected_id(&catalog).as_deref()), (Step::Workflow, Some("fix")));
}

#[test]
fn arrow_steps_stay_inside_the_list() {
    let catalog = catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, None);
    state.step_selection(false, &catalog);
    assert_eq!(state.selected(), 0);
    for _ in 0..20 {
        state.step_selection(true, &catalog);
    }
    assert_eq!(state.selected(), state.visible(&catalog).len() - 1);
}

fn custom_catalog() -> Catalog {
    let mut profiles = Profiles::default();
    profiles.agent.insert(
        "aider".into(),
        AgentOverride {
            bin: Some("aider".into()),
            permissions: LevelOverrides {
                ask: Some(ArgList(vec!["--no-auto-commits".into()])),
                ..LevelOverrides::default()
            },
            ..AgentOverride::default()
        },
    );
    profiles.profile.insert(
        "aider".into(),
        Profile {
            name: "Aider".into(),
            agent: "aider".into(),
            model: None,
            effort: None,
            level: Level::Ask,
            setup: String::new(),
            never_push: true,
            extra_args: ArgList::default(),
            house_style: false,
        },
    );
    catalog_with(profiles)
}

#[test]
fn a_custom_agent_can_only_pick_levels_it_has_flags_for() {
    let catalog = custom_catalog();
    let mut state = in_agent_step(Group::Repo, &catalog, Some("aider"));
    assert_eq!(state.level(), Level::Ask);
    assert!(!state.set_level(Level::Yolo, &catalog));
    assert!(!state.step_level(true, &catalog), "no flags for the levels above Ask");
    let agent = catalog.table().get("aider").unwrap();
    assert_eq!(
        level_refusal(Group::Repo, agent, Level::Yolo),
        Some("aider has no flags for YOLO in profiles.toml.".to_owned())
    );
}

#[test]
fn a_profile_whose_level_has_no_flags_opens_on_a_level_that_does() {
    let mut catalog = custom_catalog();
    let mut profiles = catalog.profiles().unwrap().clone();
    profiles.profile.get_mut("aider").unwrap().level = Level::Yolo;
    catalog.set_profiles(profiles);
    let state = in_agent_step(Group::Repo, &catalog, Some("aider"));
    assert_eq!(state.level(), Level::Ask);
}

#[test]
fn warnings_cover_a_missing_cli_an_unverified_one_copilot_and_a_shared_folder() {
    let mut catalog = custom_catalog();
    catalog.set_availability("aider", Availability::Missing);
    let state = in_agent_step(Group::Repo, &catalog, Some("aider"));
    let warnings = state.warnings(&catalog);
    assert!(warnings[0].starts_with("aider was not found on PATH"), "{warnings:?}");
    assert_eq!(warnings[1], "aider support is experimental.");

    let catalog = catalog_with(Profiles::seeded(&["copilot".to_owned()]));
    let repo = in_agent_step(Group::Repo, &catalog, None);
    assert!(repo.warnings(&catalog).iter().any(|w| w.starts_with("Copilot asks to trust")));
    let mut other = NewTab::open(
        Group::Other,
        "Other: notes".into(),
        PathBuf::from("/Users/dev/notes"),
        vec!["Builder".into()],
        None,
    );
    other.choose(Kind::Agent, &catalog);
    assert_eq!(
        other.warnings(&catalog),
        [
            "The Builder tab already has an agent in this folder. They can overwrite each other's files."
        ]
    );
}

#[test]
fn the_same_folder_warning_names_every_agent_tab() {
    let two = same_folder_warning(&["a".into(), "b".into()]);
    assert!(two.starts_with("The a, b tabs already have an agent in this folder."), "{two}");
}

#[test]
fn a_workflow_warns_about_each_agent_it_needs_that_is_missing() {
    let mut catalog = workflow_catalog();
    catalog.set_availability("gemini", Availability::Missing);
    let state = in_workflow_step(&catalog);
    assert_eq!(
        state.warnings(&catalog),
        ["gemini was not found on PATH. A stage that uses it will fail to start."]
    );
}

#[test]
fn the_arrows_move_the_highlight_over_the_cards_that_are_on_offer() {
    let mut state = open(Group::Repo, None);
    assert_eq!(state.highlight(), Kind::Agent);
    state.move_highlight(false);
    assert_eq!(state.highlight(), Kind::Agent);
    state.move_highlight(true);
    state.move_highlight(true);
    state.move_highlight(true);
    assert_eq!(state.highlight(), Kind::Shell);
    let mut other = open(Group::Other, None);
    other.move_highlight(true);
    assert_eq!(other.highlight(), Kind::Shell, "Workflow is skipped in Other");
    other.move_highlight(false);
    assert_eq!(other.highlight(), Kind::Agent);
}
