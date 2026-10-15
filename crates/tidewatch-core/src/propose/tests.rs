use super::*;

fn args(value: Value) -> Map<String, Value> {
    let Value::Object(object) = value else { panic!("not an object: {value}") };
    object
}

fn good_args() -> Map<String, Value> {
    args(json!({
        "name": "Change with a check",
        "description": "Do it, then prove it.",
        "stages": [
            { "action": "work", "prompt": "{task}" },
            { "action": "review", "prompt": "Review {base}..HEAD.", "on_fail": 1, "max_loops": 3 },
            { "action": "handoff", "prompt": "Summarize.", "approve": true, "permissions": "plan" },
        ],
    }))
}

fn proposal() -> Workflow {
    let mut workflow = parse_args(&good_args()).unwrap();
    with_agent(&mut workflow, "claude");
    workflow
}

#[test]
fn the_prompt_asks_for_a_reusable_workflow_for_a_new_task() {
    let text = prompt();
    assert!(text.starts_with("Review this whole session: the goal you were given"), "{text}");
    for want in [
        "where you got stuck",
        "which checks proved the result",
        "for a NEW task, not for this one",
        "No names, paths, URLs, ticket numbers, branch names or secrets from this session",
        "Call the Tidewatch MCP tool `propose_workflow` (your CLI may list it as `mcp__tidewatch__propose_workflow`)",
        "reply with exactly one fenced ```toml block",
        "nothing is saved until they press Save",
    ] {
        assert!(text.contains(want), "missing {want:?}");
    }
    assert!(!text.contains("<<"), "an unfilled marker");
    assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "a dash");
}

#[test]
fn the_schema_part_of_the_prompt_lists_every_action_gate_and_placeholder_of_the_code() {
    let text = prompt();
    for action in Action::ALL {
        assert!(text.contains(&format!("\n  - {}: ", key_of(action))), "action {action:?}");
        assert!(!action_help(action).is_empty(), "{action:?}");
    }
    for gate in Gate::ALL {
        assert!(text.contains(&format!("\n  - {}: ", key_of(gate))), "gate {gate:?}");
        assert!(!gate_help(gate).is_empty(), "{gate:?}");
    }
    for name in prompt::PLACEHOLDERS {
        assert!(text.contains(&format!("- {{{name}}}: ")), "placeholder {name}");
        assert!(!placeholder_help(name).is_empty(), "{name}");
    }
    for level in proposable_levels() {
        assert!(text.contains(&key_of(level)), "level {level:?}");
        assert!(!level_help(level).is_empty(), "{level:?}");
    }
    assert!(text.contains("one of claude, codex, gemini, copilot"), "the agent ids");
    assert!(!text.contains("yolo"), "a proposal never offers YOLO");
}

#[test]
fn the_prompt_states_the_loop_limits_the_editor_enforces() {
    let text = prompt();
    assert!(text.contains(&format!("1 to {MAX_LOOPS_LIMIT}")));
    assert!(text.contains(&format!("The default is {DEFAULT_MAX_LOOPS}")));
    assert!(text.contains("on_fail: when the gate fails, go back to this earlier stage"));
    assert!(text.contains("Stages count from 1"));
}

#[test]
fn the_example_in_the_prompt_is_a_valid_workflow() {
    let toml = last_toml_block(&prompt()).expect("the prompt holds a toml block");
    let mut workflow = parse_toml(&toml).unwrap();
    with_agent(&mut workflow, "claude");
    assert_eq!(problems(&workflow), Vec::<String>::new());
    assert_eq!(workflow.name, "Change with a review");
    let shape: Vec<_> = workflow.stages.iter().map(|s| (s.action, s.on_fail)).collect();
    assert_eq!(shape, [(Action::Work, None), (Action::Review, Some(0)), (Action::Handoff, None)]);
}

#[test]
fn the_tool_schema_takes_the_workflow_fields_and_its_enums_come_from_the_code() {
    let tool = tool_definition();
    assert_eq!(tool["name"], "propose_workflow");
    let schema = &tool["inputSchema"];
    assert_eq!(schema["required"], json!(["name", "stages"]));
    assert_eq!(schema["additionalProperties"], false);
    let stage = &schema["properties"]["stages"]["items"];
    assert_eq!(stage["required"], json!(["action", "prompt"]));
    let keys: Vec<&str> =
        stage["properties"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut expected = STAGE_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    let actions: Vec<String> = Action::ALL.into_iter().map(key_of).collect();
    assert_eq!(stage["properties"]["action"]["enum"], json!(actions));
    let gates: Vec<String> = Gate::ALL.into_iter().map(key_of).collect();
    assert_eq!(stage["properties"]["gate"]["enum"], json!(gates));
    assert_eq!(stage["properties"]["permissions"]["enum"], json!(["plan", "ask", "edits", "auto"]));
    assert_eq!(stage["properties"]["max_loops"]["maximum"], MAX_LOOPS_LIMIT);
}

#[test]
fn arguments_become_a_workflow_with_the_files_conventions() {
    let workflow = proposal();
    assert_eq!(workflow.name, "Change with a check");
    assert_eq!(workflow.description, "Do it, then prove it.");
    assert_eq!(workflow.stages.len(), 3);
    let review = &workflow.stages[1];
    assert_eq!(review.action, Action::Review);
    assert_eq!(review.gate, Gate::NoBlocking, "the action's default gate");
    assert_eq!(review.on_fail, Some(0), "stages count from 1 in the arguments, from 0 in memory");
    assert_eq!(review.max_loops, 3);
    let handoff = &workflow.stages[2];
    assert!(handoff.approve);
    assert_eq!(handoff.run.level, Some(Level::Plan));
    assert_eq!(workflow.stages[0].prompt.as_deref(), Some("{task}"));
    assert!(workflow.stages.iter().all(|s| s.run.agent.as_deref() == Some("claude")));
    assert_eq!(problems(&workflow), Vec::<String>::new());
}

#[test]
fn a_stage_that_names_an_agent_keeps_it() {
    let mut given = good_args();
    given["stages"][0]["agent"] = json!("gemini");
    let mut workflow = parse_args(&given).unwrap();
    with_agent(&mut workflow, "claude");
    assert_eq!(workflow.stages[0].run.agent.as_deref(), Some("gemini"));
    assert_eq!(workflow.stages[1].run.agent.as_deref(), Some("claude"));
}

#[test]
fn bad_arguments_list_every_problem_in_words() {
    let bad = args(json!({
        "title": "x",
        "stages": [
            { "action": "ponder", "prompt": "a" },
            { "prompt": "a" },
            { "action": "work" },
            { "action": "work", "prompt": "a", "gate": "vibes", "on_fail": 0, "max_loops": 999, "approve": "yes", "color": 1 },
            "work",
        ],
    }));
    let found = parse_args(&bad).unwrap_err();
    for want in [
        "unknown field \"title\"",
        "name must be a non-empty string",
        "stage 1: action must be one of: work, review, refactor, reproduce, fix, handoff",
        "stage 2: action is required, one of: work, review",
        "stage 3: prompt must be a non-empty string",
        "stage 4: gate must be one of: tests, test_fails, test_passes, no_blocking, same_behavior, approve",
        "stage 4: on_fail must be a stage number from 1",
        "stage 4: max_loops must be a number",
        "stage 4: approve must be true or false",
        "stage 4: unknown field \"color\"",
        "stage 5: must be an object",
    ] {
        assert!(found.iter().any(|p| p.starts_with(want)), "missing {want:?} in {found:#?}");
    }
}

#[test]
fn a_workflow_without_a_stage_list_is_refused() {
    let found = parse_args(&args(json!({ "name": "x" }))).unwrap_err();
    assert_eq!(found, ["stages must be a list of stages"]);
    let empty = parse_args(&args(json!({ "name": "x", "stages": [] }))).unwrap();
    assert_eq!(problems(&empty), ["has no stages"]);
}

#[test]
fn the_editors_rules_apply_to_a_proposal() {
    let mut workflow = proposal();
    workflow.name = "  ".into();
    workflow.stages[0].max_loops = 0;
    workflow.stages[1].on_fail = Some(2);
    workflow.stages[1].prompt = Some("{nope}".into());
    workflow.stages[2].run.agent = None;
    assert_eq!(
        problems(&workflow),
        [
            "the workflow needs a name",
            "stage 2 (Review): on_fail must name an earlier stage, not stage 3",
            "stage 2 (Review): unknown placeholder {nope}",
            "stage 3 (Hand off) needs a profile or an agent",
            "stage 1 (Work): loops must be a number from 1 to 9",
        ]
    );
}

#[test]
fn a_proposal_cannot_use_yolo_a_profile_or_an_unknown_agent() {
    let mut workflow = proposal();
    workflow.stages[0].run.level = Some(Level::Yolo);
    workflow.stages[1].run.profile = Some("builder".into());
    workflow.stages[2].run.agent = Some("gpt".into());
    assert_eq!(
        problems(&workflow),
        [
            "stage 1 (Work): a proposal cannot use YOLO permissions",
            "stage 2 (Review): leave profile out, profiles are the user's own",
            "stage 3 (Hand off): unknown agent \"gpt\", use claude, codex, gemini, copilot",
        ]
    );
}

#[test]
fn the_last_complete_toml_block_is_taken() {
    let text = "Here it is:\n```toml\nname = \"old\"\n```\nbetter:\n\n  ```TOML\n  name = \"new\"\n  \n  ```\nthat's all\n```toml\nname = \"cut off\"";
    assert_eq!(last_toml_block(text).as_deref(), Some("  name = \"new\"\n  "));
    assert_eq!(last_toml_block("```rust\nfn main() {}\n```"), None);
    assert_eq!(last_toml_block("no blocks"), None);
}

#[test]
fn a_toml_reply_may_be_workflows_toml_or_one_bare_workflow() {
    let table = "[workflow.mine]\nname = \"Mine\"\n[[workflow.mine.stage]]\naction = \"work\"\nagent = \"claude\"\n";
    let from_table = parse_toml(table).unwrap();
    assert_eq!((from_table.name.as_str(), from_table.stages.len()), ("Mine", 1));
    let bare = "name = \"Bare\"\ndescription = \"d\"\n[[stage]]\naction = \"review\"\non_fail = 1\n[[stage]]\naction = \"work\"\n";
    let from_bare = parse_toml(bare).unwrap();
    assert_eq!((from_bare.name.as_str(), from_bare.stages.len()), ("Bare", 2));
    assert_eq!(from_bare.stages[0].gate, Gate::NoBlocking);
    let unnamed =
        parse_toml("[workflow.quick-fix]\n[[workflow.quick-fix.stage]]\naction = \"fix\"\n");
    assert_eq!(unnamed.unwrap().name, "quick-fix");
}

#[test]
fn a_toml_reply_that_is_not_a_workflow_says_why() {
    let broken = parse_toml("name = \n").unwrap_err();
    assert_eq!(broken.len(), 1);
    assert!(broken[0].starts_with("the toml block is not valid: "), "{broken:?}");
    let wrong =
        parse_toml("[workflow.x]\n[[workflow.x.stage]]\naction = \"juggle\"\n").unwrap_err();
    assert!(wrong[0].starts_with("the toml block is not valid: "), "{wrong:?}");
    assert_eq!(parse_toml("title = \"x\"\n").unwrap_err(), ["the toml block holds no workflow"]);
}

#[test]
fn a_clashing_name_gets_the_next_free_number() {
    let taken = ["Ship a feature", "Fix a bug", "fix a bug (2)"];
    assert_eq!(unique_name("Review a PR", taken), "Review a PR");
    assert_eq!(unique_name("Ship a feature", taken), "Ship a feature (2)");
    assert_eq!(unique_name("fix a BUG", taken), "fix a BUG (3)");
}

#[test]
fn a_name_makes_a_file_key() {
    assert_eq!(id_stem("Change with a check"), "change-with-a-check");
    assert_eq!(id_stem("  Fix: the (bug)!  "), "fix-the-bug");
    assert_eq!(id_stem("???"), "workflow");
}
