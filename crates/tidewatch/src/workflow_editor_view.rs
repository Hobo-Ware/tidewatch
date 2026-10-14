//! The Workflows page of Settings: the list, the selected workflow's stages and the stage card.

use egui::{Align, Align2, CornerRadius, FontId, Layout, Margin, Rect, ScrollArea, Ui, UiBuilder};
use tidewatch_core::agents::Level;
use tidewatch_core::workflows::{Action, Gate};

use crate::settings_ui::editor::{
    Bar, BarAction, Item, card_header, columns, list_item, page_area, page_top, save_bar,
    split_card,
};
use crate::settings_ui::{
    CARD, MUTED, RED, TEAL_TEXT, Tone, button, caption, choice, field, note, switch_line, text_area,
};
use crate::settings_view::Action as PageAction;
use crate::workflow_editor::{Editor, StageDraft};

const PROMPT_ROWS: usize = 7;

pub(crate) fn page(ui: &mut Ui, area: Rect, editor: &mut Editor, file: &str) -> Option<PageAction> {
    let inner = page_area(area);
    let (bottom, new) = page_top(
        ui,
        inner,
        "Workflows",
        "A workflow runs stages one after the other. Tidewatch checks a gate before each next one.",
        file,
        "New workflow",
    );
    if new {
        editor.add_workflow();
    }
    let (list_rect, card) = columns(inner, bottom);
    list(ui, list_rect, editor);
    editor_card(ui, card, editor, file)
}

fn list(ui: &mut Ui, rect: Rect, editor: &mut Editor) {
    let items: Vec<(String, String, usize)> = editor
        .drafts
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let problems = editor.problems(index).len();
            let tag = match problems {
                0 => crate::new_tab_workflow::stage_count(draft.stages.len()),
                1 => "1 problem".to_owned(),
                n => format!("{n} problems"),
            };
            (draft.name.clone(), tag, problems)
        })
        .collect();
    let mut clicked = None;
    let mut column =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    ScrollArea::vertical().id_salt("workflow_list").auto_shrink(false).show(&mut column, |ui| {
        ui.set_width(rect.width());
        ui.spacing_mut().item_spacing.y = 6.0;
        for (index, (name, tag, problems)) in items.iter().enumerate() {
            let selected = index == editor.selected;
            let item = Item {
                badge: None,
                name,
                tag,
                tag_color: match (*problems, selected) {
                    (0, true) => TEAL_TEXT,
                    (0, false) => MUTED,
                    _ => RED,
                },
                selected,
                dimmed: false,
            };
            if list_item(ui, &item).clicked() {
                clicked = Some(index);
            }
        }
    });
    if let Some(index) = clicked {
        editor.select(index);
    }
}

fn editor_card(ui: &mut Ui, rect: Rect, editor: &mut Editor, file: &str) -> Option<PageAction> {
    let Some(selected) = editor.drafts.get(editor.selected) else {
        ui.painter().rect_filled(rect, CornerRadius::same(14), CARD);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No workflows. Add one with New workflow.",
            FontId::proportional(14.0),
            MUTED,
        );
        return None;
    };
    let name = selected.name.clone();
    let line = if selected.description.trim().is_empty() {
        crate::new_tab_workflow::stage_count(selected.stages.len())
    } else {
        selected.description.clone()
    };
    let error = editor.error.clone();
    let bar = Bar {
        dirty: editor.dirty,
        saving: editor.saving,
        error: error.as_deref(),
        can_revert: false,
        file,
    };
    let parts = split_card(ui, rect, bar.shows());
    let delete = card_header(ui, parts.header, None, &name, &line);
    let mut body =
        ui.new_child(UiBuilder::new().max_rect(parts.body).layout(Layout::top_down(Align::Min)));
    let mut changed = false;
    let mut stage_action = None;
    let selected_index = editor.selected;
    let stage_index = editor.stage;
    let problems: Vec<String> = editor.problems(selected_index).to_vec();
    let agents = editor.agents().to_vec();
    let profiles = editor.profiles().to_vec();
    ScrollArea::vertical().id_salt("workflow_form").auto_shrink(false).show(&mut body, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(draft) = editor.drafts.get_mut(selected_index) {
                let form =
                    Form { stage_index, problems: &problems, agents: &agents, profiles: &profiles };
                (changed, stage_action) = form.show(ui, draft);
            }
        });
    });
    let mut action = None;
    if bar.shows() {
        let mut bar_ui = ui.new_child(UiBuilder::new().max_rect(parts.bar));
        if save_bar(&mut bar_ui, &bar) == Some(BarAction::Save) {
            action = Some(PageAction::Save);
        }
    }
    match stage_action {
        Some(StageAction::Select(index)) => editor.select_stage(index),
        Some(StageAction::Add) => editor.add_stage(),
        Some(StageAction::Remove) => editor.remove_stage(),
        Some(StageAction::Move(later)) => editor.move_stage(later),
        None => {}
    }
    if changed {
        editor.touched();
    }
    if delete {
        editor.remove_workflow();
    }
    action
}

enum StageAction {
    Select(usize),
    Add,
    Remove,
    Move(bool),
}

struct Form<'a> {
    stage_index: usize,
    problems: &'a [String],
    agents: &'a [(String, String)],
    profiles: &'a [(String, String)],
}

impl Form<'_> {
    fn show(
        &self,
        ui: &mut Ui,
        draft: &mut crate::workflow_editor::WorkflowDraft,
    ) -> (bool, Option<StageAction>) {
        let mut changed = false;
        let mut action = None;
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.columns(2, |columns| {
            changed |= field(&mut columns[0], "Name", &mut draft.name, "Ship a feature");
            changed |= field(
                &mut columns[1],
                "What it is for",
                &mut draft.description,
                "shown in the new-tab dialog",
            );
        });
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        ui.add_space(12.0);
        caption(ui, "Stages, run one after the other");
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            for (index, stage) in draft.stages.iter().enumerate() {
                let label = format!("{}. {}", index + 1, stage.action.label());
                if choice(ui, &label, index == self.stage_index).clicked() {
                    action = Some(StageAction::Select(index));
                }
            }
            if button(ui, "Add stage", Tone::Quiet).clicked() {
                action = Some(StageAction::Add);
            }
        });
        let count = draft.stages.len();
        if self.stage_index < count {
            let earlier: Vec<String> = draft.stages[..self.stage_index]
                .iter()
                .enumerate()
                .map(|(index, earlier)| {
                    format!("Back to {}. {}", index + 1, earlier.action.label())
                })
                .collect();
            changed |= self.stage_form(ui, &mut draft.stages[self.stage_index], &earlier);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if self.stage_index > 0 && button(ui, "Move earlier", Tone::Quiet).clicked() {
                    action = Some(StageAction::Move(false));
                }
                if self.stage_index + 1 < count && button(ui, "Move later", Tone::Quiet).clicked() {
                    action = Some(StageAction::Move(true));
                }
                if button(ui, "Remove stage", Tone::Quiet).clicked() {
                    action = Some(StageAction::Remove);
                }
            });
        }
        if !self.problems.is_empty() {
            ui.add_space(12.0);
            for problem in self.problems {
                note(ui, problem, RED);
            }
        }
        (changed, action)
    }

    /// The fields of one stage. `earlier` names the stages the work can go back to.
    fn stage_form(&self, ui: &mut Ui, stage: &mut StageDraft, earlier: &[String]) -> bool {
        let mut changed = false;
        ui.add_space(12.0);
        caption(ui, "Action");
        ui.horizontal_wrapped(|ui| {
            for action in Action::ALL {
                if choice(ui, action.label(), stage.action == action).clicked() {
                    stage.action = action;
                    stage.gate = action.default_gate();
                    changed = true;
                }
            }
        });
        ui.add_space(12.0);
        caption(ui, "Agent");
        ui.horizontal_wrapped(|ui| {
            for (id, name) in self.agents {
                if choice(ui, name, stage.agent == *id).clicked() {
                    stage.agent.clone_from(id);
                    changed = true;
                }
            }
        });
        ui.add_space(12.0);
        caption(ui, "Profile (its model, level and setup apply; the fields below override them)");
        ui.horizontal_wrapped(|ui| {
            if choice(ui, "None", stage.profile.is_empty()).clicked() {
                stage.profile.clear();
                changed = true;
            }
            for (id, name) in self.profiles {
                if choice(ui, name, stage.profile == *id).clicked() {
                    stage.profile.clone_from(id);
                    changed = true;
                }
            }
        });
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.columns(2, |columns| {
            changed |= field(
                &mut columns[0],
                "Model",
                &mut stage.model,
                "empty: the agent's or profile's default",
            );
            changed |= field(
                &mut columns[1],
                "Effort",
                &mut stage.effort,
                "empty: the agent's or profile's default",
            );
        });
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        ui.add_space(12.0);
        caption(ui, "Permissions");
        ui.horizontal_wrapped(|ui| {
            if choice(ui, "Default", stage.level.is_none()).clicked() {
                stage.level = None;
                changed = true;
            }
            for level in Level::ALL {
                if choice(ui, level.label(), stage.level == Some(level)).clicked() {
                    stage.level = Some(level);
                    changed = true;
                }
            }
        });
        if stage.level == Some(Level::Yolo) {
            note(
                ui,
                "YOLO runs unattended in the worktree. Nothing is pushed, but it can change anything in it.",
                RED,
            );
        }
        ui.add_space(12.0);
        caption(ui, "Gate: what Tidewatch checks before the next stage");
        ui.horizontal_wrapped(|ui| {
            for gate in Gate::ALL {
                if choice(ui, gate.label(), stage.gate == gate).clicked() {
                    stage.gate = gate;
                    changed = true;
                }
            }
        });
        ui.add_space(12.0);
        caption(ui, "If the gate fails");
        ui.horizontal_wrapped(|ui| {
            if choice(ui, "Stop and ask", stage.on_fail.is_none()).clicked() {
                stage.on_fail = None;
                changed = true;
            }
            for (index, label) in earlier.iter().enumerate() {
                if choice(ui, label, stage.on_fail == Some(index)).clicked() {
                    stage.on_fail = Some(index);
                    changed = true;
                }
            }
        });
        if stage.on_fail.is_some() {
            ui.add_space(12.0);
            changed |=
                field(ui, "Loop limit (1 to 9, then it asks you)", &mut stage.max_loops, "2");
        }
        ui.add_space(12.0);
        changed |=
            switch_line(ui, "Wait for my approval before the next stage", &mut stage.approve);
        ui.add_space(12.0);
        caption(ui, "Prompt: {task} {base} {branch} {prev} {findings} {test_output}");
        let default = stage.action.default_prompt();
        changed |= text_area(ui, &mut stage.prompt, default, PROMPT_ROWS);
        changed
    }
}

#[cfg(test)]
mod tests;
