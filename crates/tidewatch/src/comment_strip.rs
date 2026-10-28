//! The strip under an agent tab's status line once the comment guard has flagged something (or the
//! tab turned it off): a chip with the flag count and the newest flagged file, cut in the middle to
//! its room, whose popover lists each flag (`path:line`, the rule as a tag, the comment in two
//! lines), never wider than the column, and the tab's own off switch.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, Frame, Key, Modifiers, Painter, Rect, RectAlign, Response, ScrollArea,
    Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::brand;
use crate::comment_model::{self, Line, PathRoots, Tone, chip_text, kind_label, kind_tone};
use crate::middle_cut::{keeps_name, middle_cut};
use crate::reader_view::INSET;
use crate::screen_kit::{Size, TEXT_SECONDARY, TagFill, body, mono, paint_tag, tag_width};

pub(crate) const HEIGHT: f32 = 44.0;
const CARD_H: f32 = 36.0;
const CHIP_H: f32 = Size::Compact.height();
const POPOVER_W: f32 = 460.0;
const EDGE: f32 = 8.0;
const GAP: f32 = 10.0;
const LINES_SHOWN: usize = 12;
const COMMENT_ROWS: usize = 2;
const LIST_RESERVE: f32 = 96.0;
const MIN_LIST_H: f32 = 96.0;

pub(crate) struct Strip<'a> {
    pub(crate) records: &'a [tidewatch_core::comment_flags::Record],
    pub(crate) roots: &'a PathRoots,
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
    let toggle =
        painter.layout_no_wrap(toggle_label.to_owned(), body(TEXT_SECONDARY), brand::TEAL_TEXT);
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
    popover(ui, &chip, strip, card);

    if let Some(last) =
        comment_model::lines(strip.records, 1, strip.roots).first().filter(|_| !strip.off)
    {
        let left = chip.rect.right() + 12.0;
        let room = right - left - 16.0;
        if room > 60.0 {
            let place = cut_place(&painter, &last.place, &mono(TEXT_SECONDARY), room);
            let galley = painter.layout_no_wrap(place, mono(TEXT_SECONDARY), brand::SOFT);
            painter.galley(pos2(left, y - galley.size().y / 2.0), galley, brand::SOFT);
        }
    }
    action
}

fn cut_place(painter: &Painter, place: &str, font: &egui::FontId, room: f32) -> String {
    middle_cut(place, room, &|text| {
        painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
    })
}

fn clipped(
    painter: &Painter,
    text: &str,
    font: egui::FontId,
    ink: Color32,
    width: f32,
    rows: usize,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::simple(text.to_owned(), font, ink, width);
    job.wrap = TextWrapping { max_rows: rows, ..job.wrap };
    painter.layout_job(job)
}

fn popover_width(column_w: f32, frame_w: f32) -> f32 {
    POPOVER_W.min(column_w).min(frame_w - 2.0 * EDGE).max(0.0)
}

fn chip(
    ui: &mut Ui,
    painter: &egui::Painter,
    text: &str,
    left: f32,
    y: f32,
    ink: egui::Color32,
) -> Response {
    let galley = painter.layout_no_wrap(text.to_owned(), body(TEXT_SECONDARY), ink);
    let width = galley.size().x + 28.0;
    let rect = Rect::from_min_size(pos2(left, y - CHIP_H / 2.0), vec2(width, CHIP_H));
    let response = ui.interact(rect, ui.id().with("comment-strip-chip"), Sense::click());
    let fill = ink.gamma_multiply(if response.hovered() { 0.2 } else { 0.12 });
    painter.rect_filled(rect, CornerRadius::same(Size::Compact.radius()), fill);
    painter.circle_filled(pos2(rect.left() + 12.0, y), 3.0, ink);
    painter.galley(pos2(rect.left() + 21.0, y - galley.size().y / 2.0), galley, ink);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("The comments the guard sent back to the agent")
}

fn popover(ui: &Ui, chip: &Response, strip: &Strip<'_>, column: Rect) {
    let lines = comment_model::lines(strip.records, LINES_SHOWN, strip.roots);
    let screen = ui.ctx().content_rect();
    let frame = Frame::popup(ui.style());
    let pad = frame.total_margin().sum();
    let inner_w = popover_width(column.width(), screen.width()) - pad.x;
    let top = chip.rect.bottom();
    let list_h = (screen.bottom() - EDGE - top - pad.y - LIST_RESERVE).max(MIN_LIST_H);
    let total = strip.records.len();
    egui::Popup::menu(chip)
        .anchor(pos2(column.left(), top))
        .align(RectAlign::BOTTOM_START)
        .align_alternatives(&[RectAlign::BOTTOM_START])
        .gap(4.0)
        .frame(frame)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(inner_w);
            ui.spacing_mut().item_spacing.y = 8.0;
            let heading = if total == 0 {
                "No flags yet".to_owned()
            } else {
                format!("Newest {} of {total}", lines.len())
            };
            ui.add(
                egui::Label::new(egui::RichText::new(heading).size(13.0).color(brand::FAINT))
                    .wrap(),
            );
            ScrollArea::vertical().max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                for line in &lines {
                    flag_block(ui, line);
                }
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Settings, General, Guards turns the rule off for every tab",
                    )
                    .size(12.0)
                    .color(brand::FAINT),
                )
                .wrap(),
            );
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                ui.close();
            }
        });
}

fn flag_block(ui: &mut Ui, line: &Line) {
    let ink = match kind_tone(&line.kind) {
        Tone::Loud => brand::AMBER,
        Tone::Quiet => brand::SOFT,
    };
    let width = ui.available_width();
    let painter = ui.painter().clone();
    let label = kind_label(&line.kind);
    let tag_w = tag_width(&painter, &label);
    let font = mono(TEXT_SECONDARY);
    let beside = cut_place(&painter, &line.place, &font, width - tag_w - GAP);
    let tag_below = !keeps_name(&beside, &line.place);
    let place = if tag_below { cut_place(&painter, &line.place, &font, width) } else { beside };
    let place = painter.layout_no_wrap(place, font, brand::TEXT);
    let comment = (!line.text.is_empty())
        .then(|| clipped(&painter, &line.text, mono(12.0), brand::MUTED, width, COMMENT_ROWS));
    let tag_h = Size::Tag.height();
    let head_h = if tag_below { 2.0 * tag_h + 4.0 } else { tag_h };
    let height = head_h + comment.as_ref().map_or(0.0, |galley| 4.0 + galley.size().y);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mid = rect.top() + tag_h / 2.0;
    painter.galley(pos2(rect.left(), mid - place.size().y / 2.0), place, brand::TEXT);
    let tag_at = if tag_below {
        (rect.left(), rect.top() + tag_h + 4.0 + tag_h / 2.0)
    } else {
        (rect.right() - tag_w, mid)
    };
    paint_tag(&painter, tag_at, &label, ink, TagFill::Tint(ink.gamma_multiply(0.14)));
    if let Some(galley) = comment {
        painter.galley(pos2(rect.left(), rect.top() + head_h + 4.0), galley, brand::MUTED);
    }
}

#[cfg(test)]
mod tests;
