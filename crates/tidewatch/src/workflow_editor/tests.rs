use super::*;
use tidewatch_core::workflows::ORIGINAL_IDS;

fn agents() -> Vec<(String, String)> {
    ["claude", "codex", "gemini", "copilot"].map(|id| (id.to_owned(), id.to_owned())).to_vec()
}

fn profiles() -> Vec<(String, String)> {
    vec![("reviewer".into(), "Reviewer".into()), ("builder".into(), "Builder".into())]
}

fn original() -> Workflows {
    let mut workflows = Workflows::defaults();
    workflows.workflow.retain(|id, _| ORIGINAL_IDS.contains(&id.as_str()));
    workflows
}

fn editor() -> Editor {
    Editor::new(&original(), agents(), profiles())
}

fn ship_index(editor: &Editor) -> usize {
    editor.drafts.iter().position(|d| d.id == "ship").unwrap()
}

fn open_ship() -> Editor {
    let mut editor = editor();
    let index = ship_index(&editor);
    editor.select(index);
    editor
}

#[test]
fn drafts_follow_the_file_and_round_trip_to_the_same_workflows() {
    let workflows = original();
    let editor = Editor::new(&workflows, agents(), profiles());
    let ids: Vec<&str> = editor.drafts.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["fix", "pr", "ship"]);
    assert_eq!(editor.build().unwrap(), workflows);
    assert_eq!(editor.problem_count(), 0);
    assert!(!editor.dirty);
}

#[test]
fn the_built_file_text_reads_back_as_the_same_workflows() {
    let mut editor = open_ship();
    editor.drafts[2].stages[1].prompt = "Look hard at {base}..HEAD.\nUse {task}.".into();
    editor.drafts[2].stages[1].max_loops = "3".into();
    editor.drafts[2].stages[3].approve = true;
    editor.touched();
    let built = editor.build().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("workflows.toml");
    built.save(&file).unwrap();
    assert_eq!(Workflows::load(&file), built);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("on_fail = 1"), "{text}");
}

#[test]
fn editing_a_stage_field_reaches_the_built_workflow() {
    let mut editor = open_ship();
    let review = &mut editor.drafts[2].stages[1];
    review.model = " flash ".into();
    review.effort = "high".into();
    review.level = Some(Level::Ask);
    review.gate = Gate::Approve;
    review.profile = "reviewer".into();
    editor.touched();
    let built = editor.build().unwrap();
    let stage = &built.workflow["ship"].stages[1];
    assert_eq!(stage.run.model.as_deref(), Some("flash"));
    assert_eq!(stage.run.effort.as_deref(), Some("high"));
    assert_eq!(stage.run.level, Some(Level::Ask));
    assert_eq!(stage.gate, Gate::Approve);
    assert_eq!(stage.run.profile.as_deref(), Some("reviewer"));
    assert_eq!(stage.run.agent.as_deref(), Some("gemini"));
    assert!(editor.dirty);
}

#[test]
fn an_empty_prompt_takes_the_actions_default_and_a_written_one_is_kept() {
    let mut editor = open_ship();
    assert_eq!(editor.build().unwrap().workflow["ship"].stages[0].prompt, None);
    editor.drafts[2].stages[0].prompt = "   \n".into();
    assert_eq!(editor.build().unwrap().workflow["ship"].stages[0].prompt, None);
    editor.drafts[2].stages[0].prompt = "Do {task} carefully".into();
    assert_eq!(
        editor.build().unwrap().workflow["ship"].stages[0].prompt.as_deref(),
        Some("Do {task} carefully")
    );
}

#[test]
fn problems_are_found_as_the_user_types_and_name_the_stage() {
    type Change = fn(&mut Editor);
    let cases: [(Change, &str); 9] = [
        (|e| e.drafts[2].name = "  ".into(), "the workflow needs a name"),
        (|e| e.drafts[2].stages.clear(), "has no stages"),
        (|e| e.drafts[2].stages[0].agent.clear(), "stage 1 (Work) needs a profile or an agent"),
        (
            |e| e.drafts[2].stages[0].agent = "ghost".into(),
            "stage 1 (Work): unknown agent \"ghost\"",
        ),
        (
            |e| e.drafts[2].stages[0].profile = "nobody".into(),
            "stage 1 (Work): unknown profile \"nobody\"",
        ),
        (
            |e| e.drafts[2].stages[1].max_loops = "many".into(),
            "stage 2 (Review): loops must be a number from 1 to 9",
        ),
        (
            |e| e.drafts[2].stages[1].max_loops = "0".into(),
            "stage 2 (Review): loops must be a number from 1 to 9",
        ),
        (
            |e| e.drafts[2].stages[0].prompt = "Fix {tsak}".into(),
            "stage 1 (Work): unknown placeholder {tsak}",
        ),
        (
            |e| e.drafts[2].stages[0].on_fail = Some(2),
            "stage 1 (Work): on_fail must name an earlier stage, not stage 3",
        ),
    ];
    for (change, want) in cases {
        let mut editor = open_ship();
        change(&mut editor);
        editor.touched();
        assert!(
            editor.problems(2).iter().any(|p| p == want),
            "wanted {want:?}, got {:?}",
            editor.problems(2)
        );
        assert_eq!(editor.problems(0), Vec::<String>::new(), "only the edited workflow");
    }
}

#[test]
fn a_workflow_with_problems_blocks_the_save_and_says_how_many() {
    let mut editor = open_ship();
    editor.drafts[2].name.clear();
    editor.drafts[2].stages[0].agent = "ghost".into();
    editor.touched();
    assert_eq!(editor.build().unwrap_err(), "Fix 2 problems before saving");
    editor.drafts[2].stages[0].agent = "claude".into();
    editor.touched();
    assert_eq!(editor.build().unwrap_err(), "Fix 1 problem before saving");
    editor.drafts[2].name = "Ship it".into();
    editor.touched();
    assert_eq!(editor.build().unwrap().workflow["ship"].name, "Ship it");
}

#[test]
fn adding_a_workflow_makes_a_unique_selected_one_that_can_run() {
    let mut editor = editor();
    assert_eq!(editor.add_workflow(), "new-workflow");
    assert_eq!(editor.add_workflow(), "new-workflow-2");
    assert_eq!(editor.selected, editor.drafts.len() - 1);
    assert_eq!(editor.problem_count(), 0);
    let built = editor.build().unwrap();
    let added = &built.workflow["new-workflow-2"];
    assert_eq!(added.name, "New workflow");
    assert_eq!(added.stages.len(), 1);
    assert_eq!(added.stages[0].run.agent.as_deref(), Some("claude"));
    assert!(editor.dirty);
}

#[test]
fn removing_a_workflow_keeps_the_selection_in_range() {
    let mut editor = editor();
    editor.select(2);
    editor.remove_workflow();
    assert_eq!((editor.selected, editor.drafts.len()), (1, 2));
    editor.remove_workflow();
    editor.remove_workflow();
    editor.remove_workflow();
    assert_eq!((editor.drafts.len(), editor.selected), (0, 0));
    assert_eq!(editor.build().unwrap(), Workflows::default());
}

#[test]
fn adding_a_stage_uses_the_agent_of_the_last_one_and_selects_it() {
    let mut editor = open_ship();
    editor.add_stage();
    let draft = &editor.drafts[2];
    assert_eq!(draft.stages.len(), 5);
    assert_eq!((editor.stage, draft.stages[4].agent.as_str()), (4, "claude"));
    assert_eq!(draft.stages[4].gate, Action::Work.default_gate());
    assert_eq!(editor.problem_count(), 0);
}

#[test]
fn removing_a_stage_drops_links_to_it_and_moves_later_links_up() {
    let mut editor = open_ship();
    editor.drafts[2].stages[3].on_fail = Some(2);
    editor.drafts[2].stages[2].on_fail = Some(0);
    editor.select_stage(0);
    editor.remove_stage();
    let stages = &editor.drafts[2].stages;
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[0].on_fail, None, "Review went back to the removed Work");
    assert_eq!(stages[1].on_fail, None, "Refactor went back to the removed Work");
    assert_eq!(stages[2].on_fail, Some(1), "Hand off went back to Refactor, now stage 2");
    assert_eq!(editor.stage, 0);
    assert_eq!(editor.problem_count(), 0);
}

#[test]
fn removing_the_last_stage_leaves_the_selection_on_the_new_last_one() {
    let mut editor = open_ship();
    editor.select_stage(3);
    editor.remove_stage();
    assert_eq!(editor.stage, 2);
    assert_eq!(editor.drafts[2].stages.len(), 3);
}

#[test]
fn moving_a_stage_keeps_every_link_pointing_backward() {
    let mut editor = open_ship();
    editor.select_stage(1);
    editor.move_stage(false);
    let stages = &editor.drafts[2].stages;
    let labels: Vec<_> = stages.iter().map(|s| s.action.label()).collect();
    assert_eq!(labels, ["Review", "Work", "Refactor", "Hand off"]);
    assert_eq!(stages[0].on_fail, None, "Review now comes first and cannot go back");
    assert_eq!(editor.stage, 0);
    assert_eq!(editor.problem_count(), 0);
    editor.move_stage(false);
    assert_eq!(editor.stage, 0, "the first stage stays first");
    editor.select_stage(3);
    editor.move_stage(true);
    assert_eq!(editor.stage, 3, "the last stage stays last");
}

#[test]
fn selecting_a_workflow_goes_back_to_its_first_stage() {
    let mut editor = open_ship();
    editor.select_stage(2);
    editor.select(0);
    assert_eq!((editor.selected, editor.stage), (0, 0));
    editor.select(9);
    editor.select_stage(9);
    assert_eq!((editor.selected, editor.stage), (0, 0));
}

#[test]
fn saving_clears_the_dirty_flag_and_a_failure_keeps_the_edits() {
    let mut editor = editor();
    editor.touched();
    editor.saving = true;
    editor.save_failed("disk full".into());
    assert_eq!(
        (editor.dirty, editor.saving, editor.error.as_deref()),
        (true, false, Some("disk full"))
    );
    editor.saved();
    assert_eq!((editor.dirty, editor.error.clone()), (false, None));
}

#[test]
fn a_workflow_file_that_cannot_run_still_opens_in_the_editor_with_its_problems() {
    let text = "[workflow.mine]\nname = \"Mine\"\n[[workflow.mine.stage]]\naction = \"work\"\n";
    let workflows = Workflows::parse(text).unwrap();
    let editor = Editor::new(&workflows, agents(), profiles());
    assert_eq!(editor.problems(0), ["stage 1 (Work) needs a profile or an agent"]);
    assert_eq!(editor.drafts[0].stages[0].max_loops, "2");
}

fn proposal(name: &str) -> Workflow {
    let text = format!(
        "name = \"{name}\"\ndescription = \"d\"\n[[stage]]\naction = \"work\"\nprompt = \"{{task}}\"\nagent = \"claude\"\n[[stage]]\naction = \"review\"\non_fail = 1\nagent = \"claude\"\n"
    );
    tidewatch_core::propose::parse_toml(&text).unwrap()
}

#[test]
fn a_proposal_is_a_new_selected_unsaved_draft_marked_with_its_tab() {
    let mut editor = editor();
    let before = editor.drafts.len();
    let name = editor.propose(&proposal("Change with a check"), "calm-comet");
    assert_eq!(name, "Change with a check");
    assert_eq!(editor.drafts.len(), before + 1);
    assert_eq!(editor.selected, before);
    assert!(editor.dirty);
    let draft = &editor.drafts[before];
    assert_eq!(draft.id, "change-with-a-check");
    assert_eq!(draft.from.as_deref(), Some("calm-comet"));
    assert_eq!(draft.stages[1].on_fail, Some(0));
    assert_eq!(editor.problems(before), Vec::<String>::new());
    assert!(editor.drafts[..before].iter().all(|d| d.from.is_none()));
    let built = editor.build().unwrap();
    assert_eq!(built.workflow["change-with-a-check"].stages.len(), 2);
}

#[test]
fn a_proposal_whose_name_is_taken_gets_the_next_number_and_its_own_key() {
    let mut editor = editor();
    assert_eq!(editor.propose(&proposal("Fix a bug"), "a"), "Fix a bug (2)");
    assert_eq!(editor.propose(&proposal("Fix a bug"), "b"), "Fix a bug (3)");
    let ids: Vec<_> = editor.drafts.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains(&"fix-a-bug-2") && ids.contains(&"fix-a-bug-3"), "{ids:?}");
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    assert_eq!(editor.drafts.last().unwrap().name, "Fix a bug (3)");
}

#[test]
fn revert_discards_the_proposal_and_every_other_unsaved_edit() {
    let mut editor = editor();
    let before = editor.drafts.len();
    let saved = editor.build().unwrap();
    editor.drafts[0].name = "Renamed".into();
    editor.propose(&proposal("Mine"), "calm-comet");
    editor.revert();
    assert_eq!(editor.drafts.len(), before);
    assert_ne!(editor.drafts[0].name, "Renamed");
    assert_eq!((editor.dirty, editor.error.clone(), editor.stage), (false, None, 0));
    assert!(editor.selected < editor.drafts.len());
    assert_eq!(editor.build().unwrap(), saved);
}

#[test]
fn saving_makes_the_proposal_a_workflow_like_any_other() {
    let mut editor = editor();
    editor.propose(&proposal("Mine"), "calm-comet");
    editor.saving = true;
    editor.saved();
    assert!(editor.drafts.iter().all(|d| d.from.is_none()));
    editor.add_workflow();
    editor.revert();
    assert!(editor.drafts.iter().any(|d| d.name == "Mine"), "saved, so a revert keeps it");
    assert!(editor.drafts.iter().all(|d| d.name != "New workflow"));
}
