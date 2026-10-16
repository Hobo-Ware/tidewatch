//! The Workflows page of Settings: the list, the selected workflow's stages and the stage card.

use egui::{Align, Align2, CornerRadius, FontId, Layout, Margin, Rect, ScrollArea, Ui, UiBuilder};
use tidewatch_core::agents::Level;
use tidewatch_core::workflows::{Action, Gate};

use crate::settings_ui::editor::{
    Bar, BarAction, Item, card_header, columns, list_item, page_area, page_top, save_bar,
    split_card,
};
use crate::settings_ui::{
    CARD, MUTED, RED, TEAL_TEXT, Tone, button, caption, fade_edges, field, fits_inline, note, pick,
    switch_line, text_area,
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
                0 if draft.from.is_some() => "Draft".to_owned(),
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
        can_revert: true,
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
    let form_scroll =
        ScrollArea::vertical().id_salt("workflow_form").auto_shrink(false).show(&mut body, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(20, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(draft) = editor.drafts.get_mut(selected_index) {
                    let form = Form {
                        stage_index,
                        problems: &problems,
                        agents: &agents,
                        profiles: &profiles,
                    };
                    (changed, stage_action) = form.show(ui, draft);
                }
            });
        });
    fade_edges(&body, &form_scroll, CARD);
    let mut action = None;
    if bar.shows() {
        let mut bar_ui = ui.new_child(UiBuilder::new().max_rect(parts.bar));
        match save_bar(&mut bar_ui, &bar) {
            Some(BarAction::Save) => action = Some(PageAction::Save),
            Some(BarAction::Revert) => editor.revert(),
            None => {}
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
        if let Some(from) = &draft.from {
            note(
                ui,
                &format!(
                    "Proposed by {from}. Nothing is written until you press Save; Revert discards it."
                ),
                TEAL_TEXT,
            );
            ui.add_space(10.0);
        }
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
        let stage_labels: Vec<String> = draft
            .stages
            .iter()
            .enumerate()
            .map(|(index, stage)| format!("{}. {}", index + 1, stage.action.label()))
            .collect();
        let labels: Vec<&str> = stage_labels.iter().map(String::as_str).collect();
        let beside = fits_inline(ui, &labels);
        ui.horizontal(|ui| {
            if let Some(index) = pick(ui, "stages", &labels, Some(self.stage_index)) {
                action = Some(StageAction::Select(index));
            }
            if beside && button(ui, "Add stage", Tone::Quiet).clicked() {
                action = Some(StageAction::Add);
            }
        });
        if !beside {
            ui.add_space(2.0);
            if button(ui, "Add stage", Tone::Quiet).clicked() {
                action = Some(StageAction::Add);
            }
        }
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
                if button(ui, "Remove stage", Tone::Danger).clicked() {
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
        let labels = Action::ALL.map(Action::label);
        let active = Action::ALL.iter().position(|action| *action == stage.action);
        if let Some(index) = pick(ui, "action", &labels, active) {
            stage.action = Action::ALL[index];
            stage.gate = stage.action.default_gate();
            changed = true;
        }
        ui.add_space(12.0);
        caption(ui, "Agent");
        let names: Vec<&str> = self.agents.iter().map(|(_, name)| name.as_str()).collect();
        let active = self.agents.iter().position(|(id, _)| *id == stage.agent);
        if let Some(index) = pick(ui, "stage_agent", &names, active) {
            stage.agent.clone_from(&self.agents[index].0);
            changed = true;
        }
        ui.add_space(12.0);
        caption(ui, "Profile (its model, level and setup apply; the fields below override them)");
        let names: Vec<&str> = std::iter::once("None")
            .chain(self.profiles.iter().map(|(_, name)| name.as_str()))
            .collect();
        let active = if stage.profile.is_empty() {
            Some(0)
        } else {
            self.profiles.iter().position(|(id, _)| *id == stage.profile).map(|at| at + 1)
        };
        if let Some(index) = pick(ui, "profile", &names, active) {
            match index.checked_sub(1) {
                None => stage.profile.clear(),
                Some(at) => stage.profile.clone_from(&self.profiles[at].0),
            }
            changed = true;
        }
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
        let labels: Vec<&str> =
            std::iter::once("Default").chain(Level::ALL.map(Level::label)).collect();
        let active = stage
            .level
            .map_or(Some(0), |level| Level::ALL.iter().position(|l| *l == level).map(|at| at + 1));
        if let Some(index) = pick(ui, "permissions", &labels, active) {
            stage.level = index.checked_sub(1).map(|at| Level::ALL[at]);
            changed = true;
        }
        if stage.level == Some(Level::Yolo) {
            note(
                ui,
                "YOLO runs unattended in the worktree. Nothing is pushed, but it can change anything in it.",
                RED,
            );
        }
        ui.add_space(12.0);
        caption(ui, "Gate: what Tidewatch checks before the next stage");
        let labels = Gate::ALL.map(Gate::label);
        let active = Gate::ALL.iter().position(|gate| *gate == stage.gate);
        if let Some(index) = pick(ui, "gate", &labels, active) {
            stage.gate = Gate::ALL[index];
            changed = true;
        }
        ui.add_space(12.0);
        caption(ui, "If the gate fails");
        let labels: Vec<&str> =
            std::iter::once("Stop and ask").chain(earlier.iter().map(String::as_str)).collect();
        let active = stage.on_fail.map_or(Some(0), |at| Some(at + 1));
        if let Some(index) = pick(ui, "on_fail", &labels, active) {
            stage.on_fail = index.checked_sub(1);
            changed = true;
        }
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
