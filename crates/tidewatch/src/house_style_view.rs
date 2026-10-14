//! The House style page: the file's place, what it is for, open it in an editor, reset it.

use egui::{Align, Layout, RichText, Ui};

use crate::settings_ui::{
    Control, FAINT, MUTED, TEXT, Tone, button, page_header, row, row_with, section,
};
use crate::settings_view::Action;

const ARMED: &str = "Click again to reset";

pub(crate) fn page(ui: &mut Ui, file: &str) -> Option<Action> {
    let mut action = None;
    page_header(ui, "House style", "One markdown file, added to every agent session.");
    section(ui, "Your file", |ui| {
        row_with(
            ui,
            Control::button(ui, "Open in editor"),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("house-style.md").size(15.0).color(TEXT));
                ui.label(RichText::new(file).monospace().size(12.0).color(FAINT));
            },
            |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Open in editor", Tone::Quiet).clicked() {
                        action = Some(Action::OpenHouseStyle);
                    }
                });
            },
        );
        row(
            ui,
            "What it does",
            "Every agent gets these rules on top of the repo's own CLAUDE.md and AGENTS.md, which \
             win on conflicts: how to format answers, the Done, Needs you and Next block that ends \
             a turn, and the Tidewatch tools. Tidewatch writes the file once and never overwrites \
             your edits.",
            Control::NONE,
            |_| {},
        );
    });
    section(ui, "Default text", |ui| {
        row_with(
            ui,
            Control::button(ui, ARMED),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("Reset to default").size(15.0).color(TEXT));
                ui.label(
                    RichText::new(
                        "Replaces the file with the text Tidewatch ships. Your edits are lost.",
                    )
                    .size(13.0)
                    .color(MUTED),
                );
            },
            |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if reset_button(ui) {
                        action = Some(Action::ResetHouseStyle);
                    }
                });
            },
        );
    });
    action
}

fn reset_button(ui: &mut Ui) -> bool {
    let id = ui.make_persistent_id("house_style_reset");
    let armed = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let response = button(ui, if armed { ARMED } else { "Reset to default" }, Tone::Quiet);
    let clicked = response.clicked();
    let disarm = armed && (clicked || response.clicked_elsewhere());
    if clicked && !armed {
        ui.data_mut(|data| data.insert_temp(id, true));
    } else if disarm {
        ui.data_mut(|data| data.insert_temp(id, false));
    }
    clicked && armed
}

#[cfg(test)]
mod tests;
