//! The Agents and profiles page in its first form: a list, a form and a footer with Save.

use egui::{Align, Layout, Rect, RichText, ScrollArea, UiBuilder, Vec2};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, chip, text_field, toggle_switch};
use tidewatch_core::agents::Level;

use crate::list_row::{Row, pick_row};
use crate::profile_editor::Editor;
use crate::settings_view::Action;

const LIST_WIDTH: f32 = 260.0;
const FIELD_WIDTH: f32 = 460.0;

pub(crate) fn page(
    ui: &mut egui::Ui,
    area: Rect,
    editor: &mut Editor,
    _file: &str,
) -> Option<Action> {
    ui.scope_builder(UiBuilder::new().max_rect(area.shrink(16.0)), |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let body_height = (ui.available_height() - 44.0).max(120.0);
        profiles_page(ui, editor, body_height);
        ui.add_space(4.0);
        footer(ui, editor.error.as_deref(), editor.saving, editor.dirty)
    })
    .inner
}

fn profiles_page(ui: &mut egui::Ui, editor: &mut Editor, body_height: f32) {
    let top = ui.cursor().min;
    let gap = 20.0;
    let left = Rect::from_min_size(top, Vec2::new(LIST_WIDTH, body_height));
    let right = Rect::from_min_max(
        egui::pos2(left.right() + gap, top.y),
        egui::pos2(ui.max_rect().right(), top.y + body_height),
    );
    let column = |rect: Rect| UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min));
    ui.scope_builder(column(left), |ui| list(ui, editor, body_height));
    ui.scope_builder(column(right), |ui| form(ui, editor, body_height));
    ui.advance_cursor_after_rect(Rect::from_min_size(top, Vec2::new(1.0, body_height)));
}

fn list(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    ui.set_width(LIST_WIDTH);
    let names: Vec<Row> = editor
        .drafts
        .iter()
        .map(|draft| {
            let agent = editor
                .agents()
                .iter()
                .find(|(id, _)| *id == draft.agent)
                .map_or_else(|| format!("{} (unknown)", draft.agent), |(_, name)| name.clone());
            Row {
                name: draft.name.clone(),
                detail: agent,
                detail_color: colors::dim(),
                tag: String::new(),
                dimmed: false,
            }
        })
        .collect();
    let mut clicked = None;
    ScrollArea::vertical().id_salt("profile_list").max_height(height - 44.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (index, row) in names.iter().enumerate() {
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
        if action_button(ui, "New profile", false).clicked() {
            editor.add();
        }
        if !editor.drafts.is_empty() && action_button(ui, "Delete", false).clicked() {
            editor.remove_selected();
        }
    });
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(colors::dim()));
}

fn text_row(ui: &mut egui::Ui, caption: &str, value: &mut String, hint: &str) -> bool {
    label(ui, caption);
    ui.scope(|ui| text_field(ui, value, hint, FIELD_WIDTH, colors::fg()).changed()).inner
}

fn form(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    let agents = editor.agents().to_vec();
    let mut changed = false;
    ScrollArea::vertical().id_salt("profile_form").max_height(height).show(ui, |ui| {
        let Some(draft) = editor.selected_draft() else {
            ui.label(RichText::new("No profiles. Add one with New profile.").color(colors::dim()));
            return;
        };
        ui.spacing_mut().item_spacing.y = 6.0;
        changed |= text_row(ui, "Name", &mut draft.name, "Builder");
        label(ui, "Agent");
        ui.horizontal_wrapped(|ui| {
            for (id, name) in &agents {
                if chip(ui, name, draft.agent == *id).clicked() {
                    draft.agent.clone_from(id);
                    changed = true;
                }
            }
        });
        changed |= text_row(ui, "Model", &mut draft.model, "empty: the agent's default");
        changed |= text_row(ui, "Effort", &mut draft.effort, "empty: the agent's default");
        label(ui, "Permissions");
        ui.horizontal_wrapped(|ui| {
            for level in Level::ALL {
                if chip(ui, level.label(), draft.level == level).clicked() {
                    draft.level = level;
                    changed = true;
                }
            }
        });
        if draft.level == Level::Yolo {
            ui.label(
                RichText::new("YOLO only runs in agent worktree tabs; Other refuses it.")
                    .size(12.0)
                    .color(colors::red()),
            );
        }
        changed |= text_row(
            ui,
            "Setup command",
            &mut draft.setup,
            "runs in the worktree before the agent, for example npm ci",
        );
        changed |= text_row(
            ui,
            "Extra arguments",
            &mut draft.extra_args,
            "added to the agent's command line, quote words with spaces",
        );
        ui.add_space(4.0);
        changed |= switch_row(ui, "Block git push in this agent's tab", &mut draft.never_push);
        changed |= switch_row(ui, "Add the Tidewatch house style", &mut draft.house_style);
    });
    if changed {
        editor.touched();
    }
}

fn switch_row(ui: &mut egui::Ui, text: &str, on: &mut bool) -> bool {
    ui.horizontal(|ui| {
        let changed = toggle_switch(ui, on).changed();
        ui.label(RichText::new(text).color(colors::fg()));
        changed
    })
    .inner
}

pub(crate) fn footer(
    ui: &mut egui::Ui,
    error: Option<&str>,
    saving: bool,
    dirty: bool,
) -> Option<Action> {
    let mut action = None;
    if let Some(error) = error {
        ui.label(RichText::new(error).color(colors::red()));
    } else {
        let text = if saving {
            "Saving..."
        } else if dirty {
            "Unsaved changes. Saving rewrites the file; comments in it are not kept."
        } else {
            "Saved"
        };
        ui.label(RichText::new(text).size(12.0).color(colors::dim()));
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let can_save = dirty && !saving;
        if ui.add_enabled_ui(can_save, |ui| action_button(ui, "Save", true)).inner.clicked() {
            action = Some(Action::Save);
        }
    });
    action
}
