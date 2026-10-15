use egui::{RawInput, pos2, vec2};
use tidewatch_core::workflows::Workflows;

use super::*;

fn agents() -> Vec<(String, String)> {
    ["claude", "codex", "gemini", "copilot"].map(|id| (id.to_owned(), id.to_owned())).to_vec()
}

fn editor() -> Editor {
    let profiles = vec![("builder".to_owned(), "Builder".to_owned())];
    Editor::new(&Workflows::defaults(), agents(), profiles)
}

fn render(editor: &mut Editor, frames: usize) {
    let ctx = crate::fonts::test_context();
    for _ in 0..frames {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 800.0))),
            focused: true,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            page(ui, area, editor, "workflows.toml");
        });
    }
}

#[test]
fn the_page_renders_every_workflow_and_stage_without_touching_the_edits() {
    let mut editor = editor();
    for index in 0..editor.drafts.len() {
        editor.select(index);
        for stage in 0..editor.drafts[index].stages.len() {
            editor.select_stage(stage);
            render(&mut editor, 2);
        }
    }
    assert!(!editor.dirty);
}

#[test]
fn a_workflow_with_problems_and_an_empty_list_still_render() {
    let mut editor = editor();
    editor.select(2);
    editor.drafts[2].stages[0].agent = "ghost".into();
    editor.drafts[2].stages[0].prompt = "Fix {tsak}".into();
    editor.drafts[2].stages[1].on_fail = Some(0);
    editor.touched();
    assert!(editor.problem_count() >= 2);
    render(&mut editor, 2);
    while !editor.drafts.is_empty() {
        editor.remove_workflow();
    }
    render(&mut editor, 2);
}

#[test]
fn a_proposed_draft_renders_with_its_note_and_a_revert_that_discards_it() {
    let mut editor = editor();
    let text = "name = \"Mine\"\n[[stage]]\naction = \"work\"\nagent = \"claude\"\n";
    let workflow = tidewatch_core::propose::parse_toml(text).unwrap();
    editor.propose(&workflow, "calm-comet");
    render(&mut editor, 2);
    assert!(editor.dirty);
    editor.revert();
    render(&mut editor, 2);
    assert!(editor.drafts.iter().all(|d| d.from.is_none()));
}
