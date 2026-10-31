//! The House style page: the file's place, what it is for, open it in an editor, reset it.

use egui::{Align, Layout, RichText, Ui};
use tidewatch_core::dead_tools::DeadLine;

use crate::screen_kit::Size;
use crate::screen_kit::TEXT_BODY;
use crate::settings_ui::{
    Control, FAINT, MUTED, TEXT, Tone, button, caption, page_header, row_with, section,
};
use crate::settings_view::Action;

const ARMED: &str = "Click again to reset";
const REMOVE_ARMED: &str = "Click again to remove";

pub(crate) fn page(ui: &mut Ui, file: &str, dead: &[DeadLine]) -> Option<Action> {
    let mut action = None;
    page_header(ui, "House style", "One markdown file, added to every agent session.");
    section(ui, "Your file", |ui| {
        row_with(
            ui,
            Control::button(ui, "Open in editor", Size::Compact),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("house-style.md").size(TEXT_BODY).color(TEXT));
                ui.label(RichText::new(file).monospace().size(12.0).color(FAINT));
            },
            |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Open in editor", Tone::Quiet, Size::Compact).clicked() {
                        action = Some(Action::OpenHouseStyle);
                    }
                });
            },
        );
    });
    let removal = dead_lines(ui, dead);
    ui.add_space(8.0);
    caption(
        ui,
        "Every agent gets these rules on top of the repo's own CLAUDE.md and AGENTS.md, which win on \
         conflicts: how to format answers, the Done, Needs you and Next block that ends a turn, and \
         the Tidewatch tools. Tidewatch writes the file once and never overwrites your edits.",
    );
    section(ui, "Default text", |ui| {
        row_with(
            ui,
            Control::button(ui, ARMED, Size::Compact),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("Tidewatch's own text").size(TEXT_BODY).color(TEXT));
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
    action.or(removal)
}

fn dead_lines(ui: &mut Ui, dead: &[DeadLine]) -> Option<Action> {
    if dead.is_empty() {
        return None;
    }
    let mut action = None;
    section(ui, "Names a tool that no longer exists", |ui| {
        for line in dead {
            row_with(
                ui,
                Control::button(ui, REMOVE_ARMED, Size::Compact),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(
                        RichText::new(format!(
                            "Line {} names {}, removed on {}",
                            line.number, line.tool.name, line.tool.since
                        ))
                        .size(TEXT_BODY)
                        .color(TEXT),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&line.text).monospace().size(12.0).color(FAINT),
                        )
                        .truncate(),
                    );
                },
                |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let key = format!("house_style_remove_{}", line.number);
                        if two_click(ui, &key, "Remove line", REMOVE_ARMED, Tone::Danger) {
                            action = Some(Action::RemoveHouseStyleLine {
                                number: line.number,
                                text: line.text.clone(),
                            });
                        }
                    });
                },
            );
        }
    });
    ui.add_space(8.0);
    caption(
        ui,
        "An agent that reads one of these lines looks for a tool Tidewatch does not offer. Your file \
         is only changed when you click Remove line twice, and only that line goes.",
    );
    action
}

fn reset_button(ui: &mut Ui) -> bool {
    two_click(ui, "house_style_reset", "Reset to default", ARMED, Tone::Danger)
}

fn two_click(ui: &mut Ui, key: &str, idle: &str, armed_label: &str, tone: Tone) -> bool {
    let id = ui.make_persistent_id(key);
    let armed = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let response = button(ui, if armed { armed_label } else { idle }, tone, Size::Compact);
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
