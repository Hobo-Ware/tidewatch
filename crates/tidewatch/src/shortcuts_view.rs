//! The Shortcuts page of Settings: the command table for this platform, grouped, each chord as key caps.

use egui::{RichText, Ui};

use crate::commands::{self, Group, Platform};
use crate::screen_kit::TEXT_TAG;
use crate::settings_ui::{Control, FAINT, TEXT, key_cap, note, padded_row, page_header, section};

const KEYS_WIDTH: f32 = 230.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Cap<'a> {
    Key(&'a str),
    To,
}

fn caps(keys: &str) -> Vec<Cap<'_>> {
    let mut caps = Vec::new();
    if keys.is_empty() {
        return caps;
    }
    for (index, span) in keys.split(" to ").enumerate() {
        if index > 0 {
            caps.push(Cap::To);
        }
        caps.extend(span.split('+').map(Cap::Key));
    }
    caps
}

pub(crate) fn page(ui: &mut Ui) {
    let rows = commands::rows(Platform::CURRENT);
    page_header(ui, "Shortcuts", &format!("Keys on {}", Platform::CURRENT.name()));
    for group in Group::ALL {
        section(ui, group.title(), |ui| {
            for row in rows.iter().filter(|row| row.group == group) {
                shortcut_row(ui, &row.title, &row.keys);
            }
        });
    }
    section(ui, "In the terminal", |ui| {
        for key in commands::terminal_keys() {
            shortcut_row(ui, key.title, &key.keys_on(Platform::CURRENT));
        }
    });
    ui.add_space(22.0);
    note(
        ui,
        "Every command except the dialog keys is also in the command palette. Enter, Esc and the \
         arrow keys inside dialogs and the palette are fixed.",
        FAINT,
    );
}

fn shortcut_row(ui: &mut Ui, title: &str, keys: &str) {
    padded_row(
        ui,
        10,
        Control::new(KEYS_WIDTH, 26.0),
        |ui| {
            ui.label(RichText::new(title).size(14.0).color(TEXT));
        },
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if keys.is_empty() {
                ui.label(RichText::new("palette only").size(12.0).color(FAINT));
            }
            for cap in caps(keys).into_iter().rev() {
                match cap {
                    Cap::Key(text) => {
                        key_cap(ui, text, 12.0);
                    }
                    Cap::To => {
                        ui.label(RichText::new("to").size(TEXT_TAG).color(FAINT));
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, Rect, pos2, vec2};

    #[test]
    fn a_chord_is_one_cap_per_key() {
        assert_eq!(caps("Cmd+Shift+P"), [Cap::Key("Cmd"), Cap::Key("Shift"), Cap::Key("P")]);
        assert_eq!(caps("Ctrl+,"), [Cap::Key("Ctrl"), Cap::Key(",")]);
    }

    #[test]
    fn a_command_with_no_chord_has_no_caps() {
        assert_eq!(caps(""), Vec::<Cap>::new());
    }

    #[test]
    fn a_range_keeps_its_to() {
        assert_eq!(caps("Cmd+1 to 9"), [Cap::Key("Cmd"), Cap::Key("1"), Cap::To, Cap::Key("9")]);
    }

    #[test]
    fn the_page_draws_every_row_inside_its_height() {
        let ctx = crate::fonts::test_context();
        let raw = || RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..RawInput::default()
        };
        let mut used = Rect::NOTHING;
        for _ in 0..2 {
            let _ = ctx.run_ui(raw(), |ui| {
                used = ui.scope(page).response.rect;
            });
        }
        assert!(used.height() > 600.0, "{used:?}");
        assert!(used.width() <= 900.0, "{used:?}");
    }
}
