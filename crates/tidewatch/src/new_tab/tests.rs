use std::collections::HashSet;
use std::ffi::OsString;

use tidewatch_core::agents::{AgentOverride, ArgList, LevelOverrides};
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::{BUILT_IN_IDS, Workflows};

use tidewatch_core::prefs::{NewTabPrefs, StartFrom};

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
    NewTab::open(group, "web".into(), PathBuf::from("/work/web"), Vec::new(), last, 7)
}

fn in_agent_form(group: Group, catalog: &Catalog, last: Option<&str>) -> NewTab {
    let mut state = open(group, last);
    state.settle(catalog);
    state
}

fn in_workflow_form(catalog: &Catalog) -> NewTab {
    let mut state = in_agent_form(Group::Repo, catalog, None);
    assert_eq!(state.choose(Kind::Workflow), None);
    state
}

fn facts(checked_out: Option<&str>, branches: &[&str], taken: &[&str]) -> RepoFacts {
    RepoFacts {
        checked_out: checked_out.map(str::to_owned),
        default_branch: None,
        remote_default: None,
        branches: branches.iter().map(|b| (*b).to_owned()).collect(),
        taken: taken.iter().map(|t| (*t).to_owned()).collect::<HashSet<_>>(),
    }
}

#[test]
fn the_form_opens_on_an_agent_and_a_shell_is_a_request_at_once() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    assert_eq!(state.kind(), Kind::Agent);
    assert_eq!(state.title(), "New agent");
    assert!(state.focus_message);
    assert_eq!(state.choose(Kind::Shell), Some(Request::Shell));
    assert_eq!(state.kind(), Kind::Agent, "a shell never becomes the form's kind");
}

#[test]
fn each_kind_labels_its_tab() {
    let titles = [Kind::Agent, Kind::Workflow, Kind::Shell].map(Kind::title);
    assert_eq!(titles, ["New agent", "New workflow", "New shell"]);
    let catalog = workflow_catalog();
    assert_eq!(in_workflow_form(&catalog).title(), "New workflow");
}

#[test]
fn a_workflow_is_offered_in_a_repo_only() {
    let catalog = workflow_catalog();
    assert!(open(Group::Repo, None).offers(Kind::Workflow));
    let mut other = open(Group::Other, None);
    assert!(!other.offers(Kind::Workflow));
    assert!(other.offers(Kind::Agent) && other.offers(Kind::Shell));
    assert_eq!(other.choose(Kind::Workflow), None);
    assert_eq!(other.kind(), Kind::Agent);
    other.jump("workflow:ship", &catalog);
    assert_eq!(other.kind(), Kind::Agent);
    assert!(!other.makes_a_worktree());
}

#[test]
fn switching_to_a_workflow_takes_the_keyboard_off_the_message_for_good() {
    let catalog = workflow_catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.choose(Kind::Workflow);
    assert!(!state.focus_message);
    state.choose(Kind::Agent);
    assert!(!state.focus_message);
}

#[test]
fn the_profiles_are_grouped_by_cli_in_table_order() {
    let catalog = catalog();
    let state = in_agent_form(Group::Repo, &catalog, None);
    let groups = state.agent_groups(&catalog);
    let names: Vec<&str> = groups.iter().map(|(agent, _)| agent.as_str()).collect();
    assert_eq!(names, ["claude", "copilot"]);
    assert_eq!(groups[0].1, ["builder", "reviewer"]);
    assert_eq!(groups[1].1, ["copilot-builder", "copilot-reviewer"]);
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("builder"));
    assert_eq!(state.level(), Level::Edits);
}

#[test]
fn the_form_starts_on_the_last_used_profile_when_it_still_exists() {
    let catalog = catalog();
    let state = in_agent_form(Group::Repo, &catalog, Some("reviewer"));
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("reviewer"));
    assert_eq!(state.level(), Level::Plan);
    let state = in_agent_form(Group::Repo, &catalog, Some("deleted"));
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("builder"));
}

#[test]
fn a_form_opened_before_the_profiles_load_settles_when_they_do() {
    let empty = Catalog::new(OsString::from("/usr/bin"));
    let mut state = open(Group::Repo, Some("reviewer"));
    state.settle(&empty);
    assert_eq!(state.selected_id(&empty), None);
    let loaded = catalog();
    state.settle(&loaded);
    assert_eq!(state.selected_id(&loaded).as_deref(), Some("reviewer"));
    state.select_id("builder", &loaded);
    state.settle(&loaded);
    assert_eq!(state.selected_id(&loaded).as_deref(), Some("builder"), "settles once");
}

#[test]
fn the_filter_narrows_by_name_cli_model_or_level_and_resets_the_pick() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.select(1, &catalog);
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
    assert!(state.request(&catalog, None).is_err());
    state.set_filter("", &catalog);
    assert_eq!(state.visible(&catalog).len(), 4);
}

#[test]
fn an_agent_request_carries_the_level_the_message_and_where_the_worktree_goes() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    assert!(state.set_level(Level::Auto, &catalog));
    state.set_text("  fix the login bug  ");
    let repo = facts(Some("main"), &["main"], &[]);
    let Ok(Request::Agent(request)) = state.start_request(&catalog, Some(&repo)) else {
        panic!("agent request")
    };
    assert_eq!(request.profile_id, "builder");
    assert_eq!(request.profile.level, Level::Auto);
    assert_eq!(request.agent.id, "claude");
    assert_eq!(request.task, "fix the login bug", "the message is the agent's first prompt");
    assert_eq!(
        request.placement,
        Placement { name: Some("fix-login-bug".into()), base: Some("main".into()) }
    );
}

#[test]
fn an_empty_message_starts_the_agent_idle() {
    let catalog = catalog();
    let state = in_agent_form(Group::Repo, &catalog, None);
    let Ok(Request::Agent(request)) = state.start_request(&catalog, None) else {
        panic!("agent request")
    };
    assert_eq!(request.task, "");
}

#[test]
fn picking_a_profile_takes_its_level_and_clears_the_error() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.error = Some("old".into());
    state.select_id("reviewer", &catalog);
    assert_eq!((state.level(), state.error.clone()), (Level::Plan, None));
    state.select_id("builder", &catalog);
    assert_eq!(state.level(), Level::Edits);
}

#[test]
fn left_and_right_walk_the_open_levels_and_stop_at_the_ends() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
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
    let mut state = in_agent_form(Group::Other, &catalog, Some("builder"));
    assert_eq!(state.level(), Level::Auto);
    assert!(!state.set_level(Level::Yolo, &catalog));
    assert!(!state.step_level(true, &catalog));
    assert_eq!(state.level(), Level::Auto);
    assert!(state.set_level(Level::Ask, &catalog));
}

#[test]
fn a_yolo_request_in_other_is_refused_even_if_the_level_was_forced() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Other, &catalog, Some("builder"));
    state.level = Level::Yolo;
    let error = state.start_request(&catalog, None).unwrap_err();
    assert!(error.starts_with("YOLO only runs"), "{error}");
}

#[test]
fn other_has_no_worktree_to_name_so_its_request_has_no_placement() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Other, &catalog, None);
    state.type_name("not a valid name");
    assert_eq!(state.name_check(None), NameCheck::Fine);
    let Ok(Request::Agent(request)) = state.start_request(&catalog, None) else {
        panic!("agent request")
    };
    assert_eq!(request.placement, Placement::default());
}

#[test]
fn every_level_has_a_hint() {
    for level in Level::ALL {
        assert!(level_hint(level).ends_with('.'), "{level:?}");
    }
}

#[test]
fn the_workflow_form_lists_workflows_in_order_and_needs_a_task() {
    let catalog = workflow_catalog();
    let mut state = in_workflow_form(&catalog);
    assert_eq!(state.visible(&catalog), BUILT_IN_IDS);
    assert_eq!(state.selected_workflow(&catalog).unwrap().name, "Ship a feature");
    assert!(state.selected_entry(&catalog).is_none(), "no profile ladder");
    assert!(!state.set_level(Level::Auto, &catalog), "a workflow has no level to pick");
    assert_eq!(
        state.start_request(&catalog, None),
        Err("Say what the run should deliver.".to_owned())
    );
    state.set_text("  add saved searches ");
    let Ok(Request::Workflow(request)) = state.start_request(&catalog, None) else {
        panic!("a workflow request")
    };
    assert_eq!(request.task, "add saved searches");
    assert_eq!(request.workflow.id, "ship");
    assert_eq!(request.placement.name.as_deref(), Some("add-saved-searches"));
    assert!(state.request(&catalog, None).is_ok());
}

#[test]
fn each_kind_keeps_its_own_text_and_pick() {
    let catalog = workflow_catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.set_text("the message");
    state.select(1, &catalog);
    state.choose(Kind::Workflow);
    assert_eq!(state.text(), "");
    state.set_text("the task");
    state.select(2, &catalog);
    state.choose(Kind::Agent);
    assert_eq!(
        (state.text(), state.selected_id(&catalog).as_deref()),
        ("the message", Some("reviewer"))
    );
    state.choose(Kind::Workflow);
    assert_eq!((state.text(), state.selected_id(&catalog).as_deref()), ("the task", Some("pr")));
}

#[test]
fn a_workflow_that_vanished_since_the_form_opened_says_so() {
    let mut catalog = workflow_catalog();
    let mut state = in_workflow_form(&catalog);
    state.set_text("x");
    let mut workflows = Workflows::defaults();
    workflows.workflow.remove("ship");
    catalog.set_workflows(workflows);
    assert_eq!(state.selected_id(&catalog).as_deref(), Some("fix"));
    assert!(state.start_request(&catalog, None).is_ok());
    let mut none = Workflows::defaults();
    none.workflow.clear();
    catalog.set_workflows(none);
    assert_eq!(
        state.start_request(&catalog, None),
        Err("No workflow to start. Add one in Settings.".to_owned())
    );
}

#[test]
fn with_no_profiles_the_form_is_empty_and_says_where_to_add_one() {
    let catalog = catalog_with(Profiles::default());
    let state = in_agent_form(Group::Repo, &catalog, None);
    assert_eq!(state.visible(&catalog), Vec::<String>::new());
    assert_eq!(
        state.start_request(&catalog, None),
        Err("No profile to start. Add one in Settings.".to_owned())
    );
}

#[test]
fn before_the_profiles_load_start_says_so() {
    let empty = Catalog::new(OsString::from("/usr/bin"));
    let state = open(Group::Repo, None);
    assert_eq!(state.start_request(&empty, None), Err("Profiles are still loading.".to_owned()));
}

#[test]
fn a_jump_picks_a_profile_or_a_workflow_by_id() {
    let catalog = workflow_catalog();
    let mut jumped = open(Group::Repo, None);
    jumped.settle(&catalog);
    jumped.jump("reviewer", &catalog);
    assert_eq!(jumped.kind(), Kind::Agent);
    assert_eq!(jumped.selected_id(&catalog).as_deref(), Some("reviewer"));
    let mut flow = open(Group::Repo, None);
    flow.jump("workflow:fix", &catalog);
    assert_eq!((flow.kind(), flow.selected_id(&catalog).as_deref()), (Kind::Workflow, Some("fix")));
    assert!(!flow.focus_message);
}

#[test]
fn selecting_outside_the_list_changes_nothing() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.select(99, &catalog);
    assert_eq!(state.selected(), 0);
    state.select(3, &catalog);
    assert_eq!(state.selected(), 3);
}

#[test]
fn the_name_follows_the_message_until_the_user_types_one() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    assert!(!state.name_follows_text());
    state.set_text("Add retry to the stage gates");
    assert_eq!(state.name(None), "add-retry-stage-gates");
    assert!(state.name_follows_text());
    state.set_text("Fix the login bug");
    assert_eq!(state.name(None), "fix-login-bug");
    state.type_name("mine");
    state.set_text("something else entirely");
    assert_eq!(state.name(None), "mine");
    assert!(!state.name_follows_text());
}

#[test]
fn with_no_words_the_name_is_an_adjective_noun_nobody_has() {
    let catalog = catalog();
    let state = in_agent_form(Group::Repo, &catalog, None);
    let first = state.name(None);
    assert_eq!(first, names::pick(7, |_| false));
    assert_eq!(state.name(None), first, "stable from frame to frame");
    let taken = facts(None, &[], &[first.as_str()]);
    let next = state.name(Some(&taken));
    assert_ne!(next, first);
    assert_eq!(state.name_check(Some(&taken)), NameCheck::Fine);
}

#[test]
fn the_worktree_name_is_checked_by_git_s_rules_and_against_what_is_taken() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    let repo = facts(Some("main"), &[], &["retry", "retry-2"]);
    let cases = [
        ("fine-name", NameCheck::Fine),
        ("", NameCheck::Invalid(Problem::Empty)),
        ("two words", NameCheck::Invalid(Problem::Space)),
        ("a..b", NameCheck::Invalid(Problem::DoubleDot)),
        ("-lead", NameCheck::Invalid(Problem::LeadingDash)),
        ("x.lock", NameCheck::Invalid(Problem::EndsWithLock)),
        ("tab\tchar", NameCheck::Invalid(Problem::Control)),
        ("a/b", NameCheck::Invalid(Problem::Forbidden('/'))),
        ("retry", NameCheck::Taken { suggestion: "retry-3".into() }),
    ];
    for (name, want) in cases {
        state.type_name(name);
        assert_eq!(state.name_check(Some(&repo)), want, "{name:?}");
    }
}

#[test]
fn a_bad_or_taken_name_blocks_start_with_its_reason() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    let repo = facts(Some("main"), &[], &["taken"]);
    state.type_name("taken");
    assert_eq!(
        state.start_request(&catalog, Some(&repo)),
        Err("taken is taken. Try taken-2.".to_owned())
    );
    state.type_name("has space");
    assert_eq!(
        state.start_request(&catalog, Some(&repo)),
        Err("A branch name cannot have spaces.".to_owned())
    );
    state.type_name("free");
    assert!(state.start_request(&catalog, Some(&repo)).is_ok());
}

#[test]
fn a_slug_that_is_taken_is_a_problem_until_the_suggestion_is_taken_up() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.set_text("fix the login bug");
    let repo = facts(None, &[], &["fix-login-bug"]);
    assert_eq!(
        state.name_check(Some(&repo)),
        NameCheck::Taken { suggestion: "fix-login-bug-2".into() }
    );
    state.type_name("fix-login-bug-2");
    assert_eq!(state.name_check(Some(&repo)), NameCheck::Fine);
}

#[test]
fn the_base_is_the_checked_out_branch_unless_one_is_picked() {
    let catalog = catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    assert_eq!(state.base(None), None);
    let repo = facts(Some("feat/x"), &["main", "feat/x", "release"], &[]);
    assert_eq!(state.base(Some(&repo)).as_deref(), Some("feat/x"));
    state.pick_base("release");
    assert_eq!(state.base(Some(&repo)).as_deref(), Some("release"));
    state.pick_base("HEAD");
    assert_eq!(state.base(Some(&repo)).as_deref(), Some("feat/x"));
    let detached = facts(None, &["main"], &[]);
    assert_eq!(state.base(Some(&detached)), None, "HEAD");
}

#[test]
fn the_branch_list_starts_with_the_remote_default_then_the_checked_out_branch_or_head() {
    let mut repo = facts(Some("feat/x"), &["main", "feat/x", "release"], &[]);
    assert_eq!(NewTab::base_options(Some(&repo)), ["feat/x", "main", "release"]);
    repo.remote_default = Some("main".into());
    assert_eq!(NewTab::base_options(Some(&repo)), ["origin/main", "feat/x", "main", "release"]);
    let detached = facts(None, &["main"], &[]);
    assert_eq!(NewTab::base_options(Some(&detached)), ["HEAD", "main"]);
    assert_eq!(NewTab::base_options(None), Vec::<String>::new());
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
    let mut state = in_agent_form(Group::Repo, &catalog, Some("aider"));
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
    let state = in_agent_form(Group::Repo, &catalog, Some("aider"));
    assert_eq!(state.level(), Level::Ask);
}

#[test]
fn warnings_cover_a_missing_cli_an_unverified_one_copilot_and_a_shared_folder() {
    let mut catalog = custom_catalog();
    catalog.set_availability("aider", Availability::Missing);
    let state = in_agent_form(Group::Repo, &catalog, Some("aider"));
    let warnings = state.warnings(&catalog);
    assert!(warnings[0].starts_with("aider was not found on PATH"), "{warnings:?}");
    assert_eq!(warnings[1], "aider support is experimental.");

    let catalog = catalog_with(Profiles::seeded(&["copilot".to_owned()]));
    let repo = in_agent_form(Group::Repo, &catalog, None);
    assert!(repo.warnings(&catalog).iter().any(|w| w.starts_with("Copilot asks to trust")));
    let mut other = NewTab::open(
        Group::Other,
        "Other: notes".into(),
        PathBuf::from("/Users/dev/notes"),
        vec!["Builder".into()],
        None,
        7,
    );
    other.settle(&catalog);
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
    let state = in_workflow_form(&catalog);
    assert_eq!(
        state.warnings(&catalog),
        ["gemini was not found on PATH. A stage that uses it will fail to start."]
    );
}

fn all_clis() -> Catalog {
    catalog_with(Profiles::seeded(&[
        "claude".to_owned(),
        "codex".to_owned(),
        "gemini".to_owned(),
        "copilot".to_owned(),
    ]))
}

fn agent_request_of(state: &NewTab, catalog: &Catalog) -> AgentRequest {
    match state.request(catalog, None).unwrap() {
        Request::Agent(request) => *request,
        other => panic!("not an agent request: {other:?}"),
    }
}

#[test]
fn the_remote_row_offers_each_cli_what_it_has() {
    let catalog = all_clis();
    let cases = [
        ("builder", true, "Drive it from the Claude app"),
        ("copilot-builder", true, "Drive it from GitHub mobile"),
        ("codex-builder", false, "Experimental in Codex, not yet"),
        ("gemini-builder", false, "Not available for Gemini CLI"),
    ];
    for (id, available, line) in cases {
        let mut state = in_agent_form(Group::Repo, &catalog, None);
        state.select_id(id, &catalog);
        let (_, agent) = state.selected_entry(&catalog).unwrap();
        let offer = RemoteOffer::of(agent);
        assert_eq!((offer.is_available(), offer.line()), (available, line), "{id}");
    }
}

#[test]
fn remote_control_starts_off_and_only_reaches_the_request_for_a_cli_that_has_it() {
    let catalog = all_clis();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    assert!(!state.remote());
    assert!(!agent_request_of(&state, &catalog).remote);
    state.set_remote(true);
    for (id, want) in [
        ("builder", true),
        ("copilot-builder", true),
        ("codex-builder", false),
        ("gemini-builder", false),
        ("reviewer", true),
    ] {
        state.select_id(id, &catalog);
        assert!(state.remote(), "{id}: the form keeps what was switched");
        assert_eq!(agent_request_of(&state, &catalog).remote, want, "{id}");
    }
}

#[test]
fn a_custom_agent_has_no_remote_row_to_switch() {
    let catalog = custom_catalog();
    let mut state = in_agent_form(Group::Repo, &catalog, None);
    state.select_id("aider", &catalog);
    state.set_remote(true);
    let (_, agent) = state.selected_entry(&catalog).unwrap();
    assert!(!RemoteOffer::of(agent).is_available());
    assert!(!state.remote_for(agent));
}

fn defaults(change: impl FnOnce(&mut NewTabPrefs)) -> NewTabPrefs {
    let mut prefs = NewTabPrefs::default();
    change(&mut prefs);
    prefs
}

fn opened_with(prefs: NewTabPrefs, last: Option<&str>, catalog: &Catalog) -> NewTab {
    let mut state = open(Group::Repo, last).with_defaults(prefs);
    state.settle(catalog);
    state
}

#[test]
fn the_form_opens_on_the_default_profile_else_the_last_used_one_else_the_first() {
    let catalog = all_clis();
    let cases = [
        (None, None, "builder"),
        (None, Some("codex-builder"), "codex-builder"),
        (Some("gemini-builder"), Some("codex-builder"), "gemini-builder"),
        (Some("gemini-builder"), None, "gemini-builder"),
        (Some("deleted"), Some("codex-builder"), "codex-builder"),
        (Some("deleted"), None, "builder"),
    ];
    for (agent, last, want) in cases {
        let state = opened_with(defaults(|p| p.agent = agent.map(str::to_owned)), last, &catalog);
        assert_eq!(state.selected_id(&catalog).as_deref(), Some(want), "{agent:?} {last:?}");
    }
}

#[test]
fn the_default_permission_replaces_the_profiles_level_when_the_agent_allows_it() {
    let catalog = all_clis();
    let cases = [
        (None, "builder", Level::Edits),
        (None, "reviewer", Level::Plan),
        (Some(Level::Auto), "builder", Level::Auto),
        (Some(Level::Auto), "reviewer", Level::Auto),
        (Some(Level::Plan), "builder", Level::Plan),
        (Some(Level::Ask), "gemini-builder", Level::Ask),
    ];
    for (permission, agent, want) in cases {
        let prefs = defaults(|p| {
            p.agent = Some(agent.to_owned());
            p.permission = permission;
        });
        let state = opened_with(prefs, None, &catalog);
        assert_eq!(state.level(), want, "{permission:?} {agent}");
    }
}

#[test]
fn a_default_permission_the_agent_lacks_leaves_the_profiles_level() {
    let catalog = custom_catalog();
    let prefs = defaults(|p| {
        p.agent = Some("aider".to_owned());
        p.permission = Some(Level::Auto);
    });
    let state = opened_with(prefs, None, &catalog);
    assert_eq!(state.level(), Level::Ask, "Aider has flags for Ask alone");
}

#[test]
fn the_default_remote_setting_opens_the_switch() {
    let catalog = all_clis();
    for on in [false, true] {
        let state = opened_with(defaults(|p| p.remote = on), None, &catalog);
        assert_eq!(state.remote(), on);
        assert_eq!(agent_request_of(&state, &catalog).remote, on, "builder is a Claude profile");
    }
}

#[test]
fn start_from_follows_the_default_until_a_branch_is_picked() {
    let catalog = catalog();
    let mut repo = facts(Some("feat/x"), &["main", "feat/x", "release"], &[]);
    repo.default_branch = Some("main".into());
    repo.remote_default = Some("main".into());
    let mut renamed = facts(Some("feat/x"), &["trunk", "feat/x"], &[]);
    renamed.remote_default = Some("trunk".into());
    let mut local_only = facts(Some("feat/x"), &["main", "feat/x"], &[]);
    local_only.default_branch = Some("main".into());
    let no_default = facts(Some("feat/x"), &["feat/x"], &[]);
    let detached = facts(None, &["main"], &[]);
    let cases = [
        (StartFrom::Current, &repo, Some("feat/x")),
        (StartFrom::DefaultBranch, &repo, Some("origin/main")),
        (StartFrom::DefaultBranch, &renamed, Some("origin/trunk")),
        (StartFrom::DefaultBranch, &local_only, Some("main")),
        (StartFrom::DefaultBranch, &no_default, Some("feat/x")),
        (StartFrom::DefaultBranch, &detached, None),
    ];
    for (start_from, facts, want) in cases {
        let mut state = opened_with(defaults(|p| p.start = Some(start_from)), None, &catalog);
        assert_eq!(state.base(Some(facts)).as_deref(), want, "{start_from:?}");
        state.pick_base("release");
        assert_eq!(state.base(Some(facts)).as_deref(), Some("release"));
        state.pick_base("HEAD");
        assert_eq!(state.base(Some(facts)).as_deref(), want, "{start_from:?} back to the default");
    }
    let state = opened_with(defaults(|p| p.start = Some(StartFrom::DefaultBranch)), None, &catalog);
    assert_eq!(state.base(None), None);
}
