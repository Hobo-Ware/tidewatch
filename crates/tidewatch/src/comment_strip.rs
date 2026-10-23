//! The strip under an agent tab's status line once the comment guard has flagged something (or the
//! tab turned it off): a chip with the flag count, whose popover lists the lines, and the tab's own
//! off switch.

use egui::{
    Align2, CornerRadius, Key, Modifiers, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::brand;
use crate::comment_model::{self, Line, Tone, chip_text, kind_tone};
use crate::reader_view::INSET;
use crate::screen_kit::{body, mono};

pub(crate) const HEIGHT: f32 = 44.0;
const CARD_H: f32 = 36.0;
const CHIP_H: f32 = 24.0;
const POPOVER_W: f32 = 460.0;
const LINES_SHOWN: usize = 12;

pub(crate) struct Strip<'a> {
    pub(crate) records: &'a [tidewatch_core::comment_flags::Record],
    pub(crate) off: bool,
    pub(crate) dev_open_popover: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    SetOff(bool),
}

pub(crate) fn show(ui: &mut Ui, band: Rect, strip: &Strip<'_>) -> Option<Action> {
    let card = Rect::from_center_size(band.center(), vec2(band.width() - 2.0 * INSET, CARD_H));
    let painter = ui.painter().clone();
    painter.rect(
        card,
        CornerRadius::same(10),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
    let y = card.center().y;
    let mut action = None;

    let toggle_label = if strip.off { "Turn on for this tab" } else { "Turn off for this tab" };
    let toggle = painter.layout_no_wrap(toggle_label.to_owned(), body(12.5), brand::TEAL_TEXT);
    let right = card.right() - 14.0 - toggle.size().x;
    let toggle_rect = Rect::from_min_size(pos2(right, y - 11.0), vec2(toggle.size().x, 22.0));
    let toggle_response =
        ui.interact(toggle_rect, ui.id().with("comment-strip-off"), Sense::click());
    let ink = if toggle_response.hovered() { brand::TEXT } else { brand::TEAL_TEXT };
    painter.galley(pos2(right, y - toggle.size().y / 2.0), toggle, ink);
    if toggle_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        action = Some(Action::SetOff(!strip.off));
    }

    let tone = if strip.off { brand::MUTED } else { brand::AMBER };
    let chip =
        chip(ui, &painter, &chip_text(strip.records.len(), strip.off), card.left() + 14.0, y, tone);
    if strip.dev_open_popover && !strip.records.is_empty() {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&chip));
    }
    popover(&chip, strip.records);

    if let Some(last) = comment_model::lines(strip.records, 1).first().filter(|_| !strip.off) {
        let left = chip.rect.right() + 12.0;
        let room = right - left - 16.0;
        if room > 60.0 {
            let text = format!("{}  {}", last.place, last.kind);
            let galley =
                crate::reader_view::ellipsized(&painter, &text, mono(12.5), brand::SOFT, room);
            painter.galley(pos2(left, y - galley.size().y / 2.0), galley, brand::SOFT);
        }
    }
    action
}

fn chip(
    ui: &mut Ui,
    painter: &egui::Painter,
    text: &str,
    left: f32,
    y: f32,
    ink: egui::Color32,
) -> Response {
    let galley = painter.layout_no_wrap(text.to_owned(), body(12.5), ink);
    let width = galley.size().x + 28.0;
    let rect = Rect::from_min_size(pos2(left, y - CHIP_H / 2.0), vec2(width, CHIP_H));
    let response = ui.interact(rect, ui.id().with("comment-strip-chip"), Sense::click());
    let fill = ink.gamma_multiply(if response.hovered() { 0.2 } else { 0.12 });
    painter.rect_filled(rect, CornerRadius::same(255), fill);
    painter.circle_filled(pos2(rect.left() + 12.0, y), 3.0, ink);
    painter.galley(pos2(rect.left() + 21.0, y - galley.size().y / 2.0), galley, ink);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("The comments the guard sent back to the agent")
}

fn popover(chip: &Response, records: &[tidewatch_core::comment_flags::Record]) {
    let lines = comment_model::lines(records, LINES_SHOWN);
    egui::Popup::menu(chip).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(
        |ui| {
            ui.set_width(POPOVER_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            let total = records.len();
            let heading = if total == 0 {
                "No flags yet".to_owned()
            } else {
                format!("Newest {} of {total}", lines.len())
            };
            ui.label(egui::RichText::new(heading).size(13.0).color(brand::FAINT));
            for line in &lines {
                line_row(ui, line);
            }
            ui.label(
                egui::RichText::new("Settings, General, Guards turns the rule off for every tab")
                    .size(12.0)
                    .color(brand::FAINT),
            );
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                ui.close();
            }
        },
    );
}

fn line_row(ui: &mut Ui, line: &Line) {
    let ink = match kind_tone(&line.kind) {
        Tone::Loud => brand::AMBER,
        Tone::Quiet => brand::SOFT,
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    let place = painter.text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &line.place,
        mono(12.5),
        brand::TEXT,
    );
    let kind = painter.text(
        pos2(place.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        &line.kind,
        body(12.0),
        ink,
    );
    let room = rect.right() - kind.right() - 10.0;
    if room > 30.0 && !line.text.is_empty() {
        let galley =
            crate::reader_view::ellipsized(painter, &line.text, mono(12.0), brand::MUTED, room);
        painter.galley(
            pos2(kind.right() + 10.0, rect.center().y - galley.size().y / 2.0),
            galley,
            brand::MUTED,
        );
    }
}

#[cfg(test)]
mod tests;
