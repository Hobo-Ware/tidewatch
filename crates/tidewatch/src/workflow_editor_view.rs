//! The workflow editor page of the settings view: the workflow list on the left, the selected
//! workflow's stages and the selected stage's fields on the right, and the problems that stop it
//! from running right under the fields. All state is in `workflow_editor`.

use egui::{Align, Layout, Rect, RichText, ScrollArea, UiBuilder, Vec2};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, chip, text_field, toggle_switch};
use tidewatch_core::agents::Level;
use tidewatch_core::workflows::{Action, Gate};

use crate::list_row::{Row, pick_row};
use crate::workflow_editor::{Editor, StageDraft};

const LIST_WIDTH: f32 = 260.0;
const FIELD_WIDTH: f32 = 560.0;
const PROMPT_ROWS: usize = 7;
const COLUMN_GAP: f32 = 20.0;

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(colors::dim()));
}

fn text_row(ui: &mut egui::Ui, title: &str, value: &mut String, hint: &str) -> bool {
    caption(ui, title);
    ui.scope(|ui| text_field(ui, value, hint, FIELD_WIDTH, colors::fg()).changed()).inner
}

/// The prompt box: `text_field`'s look (field background, 8x6 margin) on several lines.
fn multiline_field(ui: &mut egui::Ui, text: &mut String, hint: &str) -> bool {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.5));
        ui.visuals_mut().extreme_bg_color = colors::bg();
        ui.add(
            egui::TextEdit::multiline(text)
                .desired_width(FIELD_WIDTH)
                .desired_rows(PROMPT_ROWS)
                .margin(egui::Margin::symmetric(8, 6))
                .hint_text(hint),
        )
        .changed()
    })
    .inner
}

pub(crate) fn page(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    let top = ui.cursor().min;
    let left = Rect::from_min_size(top, Vec2::new(LIST_WIDTH, height));
    let right = Rect::from_min_max(
        egui::pos2(left.right() + COLUMN_GAP, top.y),
        egui::pos2(ui.max_rect().right(), top.y + height),
    );
    let column = |rect: Rect| UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min));
    ui.scope_builder(column(left), |ui| list(ui, editor, height));
    ui.scope_builder(column(right), |ui| form(ui, editor, height));
    ui.advance_cursor_after_rect(Rect::from_min_size(top, Vec2::new(1.0, height)));
}

fn list(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    ui.set_width(LIST_WIDTH);
    let rows: Vec<Row> = editor
        .drafts
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let problems = editor.problems(index).len();
            Row {
                name: draft.name.clone(),
                detail: String::new(),
                detail_color: colors::dim(),
                tag: match problems {
                    0 => crate::new_tab_workflow::stage_count(draft.stages.len()),
                    1 => "1 problem".to_owned(),
                    n => format!("{n} problems"),
                },
                dimmed: false,
            }
        })
        .collect();
    let mut clicked = None;
    ScrollArea::vertical().id_salt("workflow_list").max_height(height - 44.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (index, row) in rows.iter().enumerate() {
            if pick_row(ui, row, index == editor.selected).clicked() {
                clicked = Some(index);
            }
        }
    });
    if let Some(index) = clicked {
        editor.select(index);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if action_button(ui, "New workflow", false).clicked() {
            editor.add_workflow();
        }
        if !editor.drafts.is_empty() && action_button(ui, "Delete", false).clicked() {
            editor.remove_workflow();
        }
    });
}

fn form(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    let mut changed = false;
    let selected = editor.selected;
    let stage_index = editor.stage;
    let problems: Vec<String> = editor.problems(selected).to_vec();
    let agents = editor.agents().to_vec();
    let profiles = editor.profiles().to_vec();
    let mut action = None;
    ScrollArea::vertical().id_salt("workflow_form").max_height(height).show(ui, |ui| {
        ui.set_width(FIELD_WIDTH + 20.0);
        let Some(draft) = editor.drafts.get_mut(selected) else {
            ui.label(
                RichText::new("No workflows. Add one with New workflow.").color(colors::dim()),
            );
            return;
        };
        ui.spacing_mut().item_spacing.y = 6.0;
        changed |= text_row(ui, "Name", &mut draft.name, "Ship a feature");
        changed |=
            text_row(ui, "What it is for", &mut draft.description, "shown in the new-tab dialog");
        ui.add_space(4.0);
        caption(ui, "Stages, run one after the other");
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for (index, stage) in draft.stages.iter().enumerate() {
                let label = format!("{}. {}", index + 1, stage.action.label());
                if chip(ui, &label, index == stage_index).clicked() {
                    picked = Some(index);
                }
            }
            if action_button(ui, "Add stage", false).clicked() {
                action = Some(StageAction::Add);
            }
        });
        if let Some(index) = picked {
            action = Some(StageAction::Select(index));
        }
        let count = draft.stages.len();
        if stage_index < count {
            let earlier: Vec<String> = draft.stages[..stage_index]
                .iter()
                .enumerate()
                .map(|(index, earlier)| {
                    format!("Back to {}. {}", index + 1, earlier.action.label())
                })
                .collect();
            changed |= stage_form(ui, &mut draft.stages[stage_index], &agents, &profiles, &earlier);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if stage_index > 0 && action_button(ui, "Move earlier", false).clicked() {
                    action = Some(StageAction::Move(false));
                }
                if stage_index + 1 < count && action_button(ui, "Move later", false).clicked() {
                    action = Some(StageAction::Move(true));
                }
                if action_button(ui, "Remove stage", false).clicked() {
                    action = Some(StageAction::Remove);
                }
            });
        }
        if !problems.is_empty() {
            ui.add_space(6.0);
            for problem in &problems {
                ui.label(RichText::new(problem).size(12.5).color(colors::red()));
            }
        }
    });
    match action {
        Some(StageAction::Select(index)) => editor.select_stage(index),
        Some(StageAction::Add) => editor.add_stage(),
        Some(StageAction::Remove) => editor.remove_stage(),
        Some(StageAction::Move(later)) => editor.move_stage(later),
        None => {}
    }
    if changed {
        editor.touched();
    }
}

enum StageAction {
    Select(usize),
    Add,
    Remove,
    Move(bool),
}

/// The fields of one stage. `earlier` names the stages the work can go back to.
fn stage_form(
    ui: &mut egui::Ui,
    stage: &mut StageDraft,
    agents: &[(String, String)],
    profiles: &[(String, String)],
    earlier: &[String],
) -> bool {
    let mut changed = false;
    ui.add_space(6.0);
    caption(ui, "Action");
    ui.horizontal_wrapped(|ui| {
        for action in Action::ALL {
            if chip(ui, action.label(), stage.action == action).clicked() {
                stage.action = action;
                stage.gate = action.default_gate();
                changed = true;
            }
        }
    });
    caption(ui, "Agent");
    ui.horizontal_wrapped(|ui| {
        for (id, name) in agents {
            if chip(ui, name, stage.agent == *id).clicked() {
                stage.agent.clone_from(id);
                changed = true;
            }
        }
    });
    caption(ui, "Profile (its model, level and setup apply; the fields below override them)");
    ui.horizontal_wrapped(|ui| {
        if chip(ui, "None", stage.profile.is_empty()).clicked() {
            stage.profile.clear();
            changed = true;
        }
        for (id, name) in profiles {
            if chip(ui, name, stage.profile == *id).clicked() {
                stage.profile.clone_from(id);
                changed = true;
            }
        }
    });
    changed |= text_row(ui, "Model", &mut stage.model, "empty: the agent's or profile's default");
    changed |= text_row(ui, "Effort", &mut stage.effort, "empty: the agent's or profile's default");
    caption(ui, "Permissions");
    ui.horizontal_wrapped(|ui| {
        if chip(ui, "Default", stage.level.is_none()).clicked() {
            stage.level = None;
            changed = true;
        }
        for level in Level::ALL {
            if chip(ui, level.label(), stage.level == Some(level)).clicked() {
                stage.level = Some(level);
                changed = true;
            }
        }
    });
    if stage.level == Some(Level::Yolo) {
        ui.label(
            RichText::new("YOLO runs unattended in the worktree. Nothing is pushed, but it can change anything in it.")
                .size(12.0)
                .color(colors::red()),
        );
    }
    caption(ui, "Gate: what Tidewatch checks before the next stage");
    ui.horizontal_wrapped(|ui| {
        for gate in Gate::ALL {
            if chip(ui, gate.label(), stage.gate == gate).clicked() {
                stage.gate = gate;
                changed = true;
            }
        }
    });
    caption(ui, "If the gate fails");
    ui.horizontal_wrapped(|ui| {
        if chip(ui, "Stop and ask", stage.on_fail.is_none()).clicked() {
            stage.on_fail = None;
            changed = true;
        }
        for (index, label) in earlier.iter().enumerate() {
            if chip(ui, label, stage.on_fail == Some(index)).clicked() {
                stage.on_fail = Some(index);
                changed = true;
            }
        }
    });
    if stage.on_fail.is_some() {
        changed |= text_row(ui, "Loop limit (1 to 9, then it asks you)", &mut stage.max_loops, "2");
    }
    ui.horizontal(|ui| {
        changed |= toggle_switch(ui, &mut stage.approve).changed();
        ui.label(RichText::new("Wait for my approval before the next stage").color(colors::fg()));
    });
    caption(ui, "Prompt: {task} {base} {branch} {prev} {findings} {test_output}");
    let default = stage.action.default_prompt();
    changed |= multiline_field(ui, &mut stage.prompt, default);
    changed
}

#[cfg(test)]
mod tests;
