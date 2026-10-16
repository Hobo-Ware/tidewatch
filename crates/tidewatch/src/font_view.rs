//! The Terminal section of General: the Font row, and its picker listed under it (Built-in, then a
//! search over the installed families). A pick applies at once.

use egui::{Align2, FontId, Margin, RichText, Sense, Ui, vec2};
use stdusk_egui::ui::icons;
use tidewatch_core::prefs::FontChoice;

use crate::screen_kit::{Size, selection_row};
use crate::settings_ui::{self, Control, FAINT, MUTED, ORANGE, TEXT, Tone, row_with, text_input};
use crate::terminal_font::{self, Status};

const ROW_H: f32 = 28.0;
const LIST_MAX_H: f32 = 240.0;

/// What the section shows beside the picker's own state.
pub(crate) struct View {
    pub(crate) choice: FontChoice,
    pub(crate) status: Option<Status>,
    pub(crate) families: &'static [String],
}

#[derive(Default)]
pub(crate) struct Picker {
    open: bool,
    query: String,
    focus: bool,
}

impl Picker {
    fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.focus = self.open;
    }
}

/// The rows the list shows: Built-in while nothing is typed, then the matches.
pub(crate) fn rows(families: &[String], query: &str) -> Vec<FontChoice> {
    let fixed = query.trim().is_empty().then_some(FontChoice::BuiltIn);
    fixed
        .into_iter()
        .chain(
            terminal_font::filter(families, query)
                .into_iter()
                .map(|index| FontChoice::Family(families[index].clone())),
        )
        .collect()
}

pub(crate) fn section(ui: &mut Ui, picker: &mut Picker, view: &View) -> Option<FontChoice> {
    settings_ui::section(ui, "Terminal", |ui| {
        let label = format!("{}  {}", terminal_font::label(&view.choice), caret(picker.open));
        let mut clicked = false;
        row_with(
            ui,
            Control::button(ui, &label, Size::Compact),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("Font").size(15.0).color(TEXT));
                settings_ui::note(
                    ui,
                    "The terminal only: the rest of the window keeps its own fonts.",
                    MUTED,
                );
                match &view.status {
                    Some(Status::Fine(text)) => settings_ui::note(ui, text, FAINT),
                    Some(Status::Warn(text)) => settings_ui::note(ui, text, ORANGE),
                    None => {}
                }
            },
            |ui| clicked = settings_ui::button(ui, &label, Tone::Quiet, Size::Compact).clicked(),
        );
        if clicked {
            picker.toggle();
        }
        if !picker.open {
            return None;
        }
        let picked = list(ui, picker, view);
        if picked.is_some() {
            picker.toggle();
        }
        picked.filter(|choice| *choice != view.choice)
    })
}

fn caret(open: bool) -> &'static str {
    if open { icons::CARET_UP } else { icons::CARET_DOWN }
}

fn list(ui: &mut Ui, picker: &mut Picker, view: &View) -> Option<FontChoice> {
    egui::Frame::new()
        .inner_margin(Margin { left: 18, right: 18, top: 0, bottom: 14 })
        .show(ui, |ui| {
            let hint = format!("Search {} installed fonts", view.families.len());
            let field = text_input(ui, &mut picker.query, &hint);
            if std::mem::take(&mut picker.focus) {
                field.request_focus();
            }
            ui.add_space(6.0);
            let rows = rows(view.families, &picker.query);
            if rows.is_empty() {
                settings_ui::note(ui, "No installed font matches.", FAINT);
                return None;
            }
            let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let mut picked = enter.then(|| rows[0].clone());
            egui::ScrollArea::vertical()
                .max_height(LIST_MAX_H)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for choice in &rows[range] {
                        if font_row(ui, choice, *choice == view.choice) {
                            picked = Some(choice.clone());
                        }
                    }
                });
            picked
        })
        .inner
}

fn font_row(ui: &mut Ui, choice: &FontChoice, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let painter = ui.painter();
    selection_row(painter, rect, selected, response.hovered());
    painter.text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        terminal_font::label(choice),
        FontId::proportional(13.5),
        TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_offers_built_in_until_a_search_is_typed() {
        let families = vec!["Hack Nerd Font Mono".to_owned(), "Noto Sans".to_owned()];
        assert_eq!(
            rows(&families, ""),
            [
                FontChoice::BuiltIn,
                FontChoice::Family("Hack Nerd Font Mono".to_owned()),
                FontChoice::Family("Noto Sans".to_owned()),
            ]
        );
        assert_eq!(rows(&families, "hack"), [FontChoice::Family("Hack Nerd Font Mono".to_owned())]);
        assert_eq!(rows(&families, "zzz"), Vec::<FontChoice>::new());
    }

    #[test]
    fn the_section_draws_closed_and_open_and_closing_clears_the_search() {
        let ctx = crate::fonts::test_context();
        let families: &'static [String] = Box::leak(
            vec!["Hack Nerd Font Mono".to_owned(), "Noto Sans".to_owned()].into_boxed_slice(),
        );
        let view = View { choice: FontChoice::BuiltIn, status: None, families };
        let mut picker = Picker::default();
        let frame = |ctx: &egui::Context, picker: &mut Picker, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let mut out = None;
            let _ = ctx.run_ui(raw, |ui| out = section(ui, picker, &view));
            out
        };
        assert_eq!(frame(&ctx, &mut picker, Vec::new()), None);
        assert!(!picker.open);
        picker.toggle();
        assert_eq!(frame(&ctx, &mut picker, Vec::new()), None);
        assert!(picker.open && !picker.focus);
        picker.toggle();
        assert!(!picker.open && picker.query.is_empty());
    }
}
