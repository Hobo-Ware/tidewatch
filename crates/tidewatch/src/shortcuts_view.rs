//! The Shortcuts page of Settings: the command table for this platform, grouped, each chord as key caps.

use egui::{Align, CornerRadius, FontId, Layout, RichText, ScrollArea, Sense, Stroke, vec2};
use stdusk_egui::colors;

use crate::commands::{self, Group, Platform};

const KEYS_WIDTH: f32 = 230.0;
const ROW_HEIGHT: f32 = 24.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Cap<'a> {
    Key(&'a str),
    To,
}

fn caps(keys: &str) -> Vec<Cap<'_>> {
    let mut caps = Vec::new();
    for (index, span) in keys.split(" to ").enumerate() {
        if index > 0 {
            caps.push(Cap::To);
        }
        caps.extend(span.split('+').map(Cap::Key));
    }
    caps
}

pub(crate) fn page(ui: &mut egui::Ui, height: f32) {
    let rows = commands::rows(Platform::CURRENT);
    ScrollArea::vertical().id_salt("shortcuts_page").max_height(height).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(
            RichText::new(format!("Keys on {}", Platform::CURRENT.name()))
                .size(12.0)
                .color(colors::dim()),
        );
        ui.add_space(4.0);
        for group in Group::ALL {
            ui.label(RichText::new(group.title().to_uppercase()).size(11.0).color(colors::dim()));
            for row in rows.iter().filter(|row| row.group == group) {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(KEYS_WIDTH, ROW_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for cap in caps(&row.keys) {
                                match cap {
                                    Cap::Key(text) => key_cap(ui, text),
                                    Cap::To => {
                                        ui.label(
                                            RichText::new("to").size(11.5).color(colors::dim()),
                                        );
                                    }
                                }
                            }
                        },
                    );
                    ui.label(RichText::new(&row.title).color(colors::fg()));
                });
            }
            ui.add_space(8.0);
        }
        ui.label(
            RichText::new("Enter, Esc and the arrow keys inside dialogs are fixed.")
                .size(12.0)
                .color(colors::dim()),
        );
    });
}

fn key_cap(ui: &mut egui::Ui, text: &str) {
    let galley =
        ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(11.5), colors::fg());
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(12.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), colors::bg());
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0, colors::border()),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, colors::fg());
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, Rect, pos2};

    #[test]
    fn a_chord_is_one_cap_per_key() {
        assert_eq!(caps("Cmd+Shift+P"), [Cap::Key("Cmd"), Cap::Key("Shift"), Cap::Key("P")]);
        assert_eq!(caps("Ctrl+,"), [Cap::Key("Ctrl"), Cap::Key(",")]);
    }

    #[test]
    fn a_range_keeps_its_to() {
        assert_eq!(caps("Cmd+1 to 9"), [Cap::Key("Cmd"), Cap::Key("1"), Cap::To, Cap::Key("9")]);
    }

    #[test]
    fn the_page_draws_every_row_inside_its_height() {
        let ctx = egui::Context::default();
        let raw = || RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        };
        let mut used = Rect::NOTHING;
        for _ in 0..2 {
            let _ = ctx.run_ui(raw(), |ui| {
                used = ui
                    .scope(|ui| {
                        page(ui, 600.0);
                    })
                    .response
                    .rect;
            });
        }
        assert!(used.height() > 200.0 && used.height() <= 600.0, "{used:?}");
    }
}
