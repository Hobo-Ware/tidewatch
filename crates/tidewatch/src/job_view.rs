//! Draws the job cards on top of the Progress panel: a card with a name, a percent, a thin bar and
//! one detail line for a job with a total or a time, segments for a small total, one quiet line for
//! a job with nothing to measure, a compact orange row for a job that stopped moving (with Ask
//! agent), and one line for each job that finished.

use egui::{Color32, CornerRadius, Rect, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::job_model::{
    BarCard, Card, FinishedLine, QuietLine, Segment, SegmentsCard, StalledCard,
};
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_TITLE;
use crate::screen_kit::{Size, body, button_width, heading, mono, quiet_button};

pub(crate) const GAP: f32 = 12.0;
const PAD_X: f32 = 16.0;
const PAD_Y: f32 = 14.0;
const NAME_H: f32 = 20.0;
const BAR_H: f32 = 6.0;
const DETAIL_H: f32 = 16.0;
const INNER_GAP: f32 = 10.0;
const QUIET_H: f32 = 26.0;
const FINISHED_H: f32 = 26.0;
const STALLED_PAD_Y: f32 = 8.0;
const SEGMENT_GAP: f32 = 4.0;

/// Draws the cards and returns the job and the message of an Ask agent click.
pub(crate) fn show(ui: &mut Ui, width: f32, cards: &[Card]) -> Option<(u64, String)> {
    let mut action = None;
    for (index, card) in cards.iter().enumerate() {
        if index > 0 {
            let small = matches!(card, Card::Finished(_) | Card::Quiet(_));
            ui.add_space(if small { 4.0 } else { GAP });
        }
        match card {
            Card::Bar(card) => bar_card(ui, width, card),
            Card::Segments(card) => segments_card(ui, width, card),
            Card::Quiet(line) => quiet_line(ui, width, line),
            Card::Stalled(card) => {
                if stalled_row(ui, width, card) {
                    action = Some((card.id, card.ask.clone()));
                }
            }
            Card::Finished(line) => finished_line(ui, width, line),
        }
    }
    action
}

fn card_frame(painter: &egui::Painter, rect: Rect, fill: Color32, line: Color32) {
    painter.rect(
        rect,
        CornerRadius::same(12),
        fill,
        Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
}

fn name_row(painter: &egui::Painter, row: Rect, name: &str, right: &str) {
    let right_galley = painter.layout_no_wrap(right.to_owned(), mono(13.0), brand::MUTED);
    let room = (row.width() - right_galley.size().x - 10.0).max(1.0);
    let name = ellipsized(painter, name, heading(TEXT_TITLE), brand::TEXT, room);
    painter.galley(pos2(row.left(), row.center().y - name.size().y / 2.0), name, brand::TEXT);
    painter.galley(
        pos2(row.right() - right_galley.size().x, row.center().y - right_galley.size().y / 2.0),
        right_galley,
        brand::MUTED,
    );
}

fn detail_row(painter: &egui::Painter, row: Rect, detail: &str) {
    let galley = ellipsized(painter, detail, body(13.0), brand::MUTED, row.width());
    painter.galley(pos2(row.left(), row.center().y - galley.size().y / 2.0), galley, brand::MUTED);
}

fn card_height(has_detail: bool) -> f32 {
    2.0 * PAD_Y
        + NAME_H
        + INNER_GAP
        + BAR_H
        + if has_detail { INNER_GAP - 2.0 + DETAIL_H } else { 0.0 }
}

fn bar_card(ui: &mut Ui, width: f32, card: &BarCard) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, card_height(!card.detail.is_empty())), Sense::hover());
    let painter = ui.painter();
    card_frame(painter, rect, brand::SEGMENT, brand::ITEM_LINE);
    let inner = rect.shrink2(vec2(PAD_X, PAD_Y));
    let name = Rect::from_min_size(inner.min, vec2(inner.width(), NAME_H));
    name_row(painter, name, &card.name, card.right.as_deref().unwrap_or(""));
    let track = Rect::from_min_size(
        pos2(inner.left(), name.bottom() + INNER_GAP),
        vec2(inner.width(), BAR_H),
    );
    painter.rect_filled(track, CornerRadius::same(3), brand::ITEM_LINE);
    let fill =
        Rect::from_min_size(track.min, vec2(track.width() * card.fraction.clamp(0.0, 1.0), BAR_H));
    painter.rect_filled(fill, CornerRadius::same(3), brand::TEAL);
    if !card.detail.is_empty() {
        let row = Rect::from_min_size(
            pos2(inner.left(), track.bottom() + INNER_GAP - 2.0),
            vec2(inner.width(), DETAIL_H),
        );
        detail_row(painter, row, &card.detail);
    }
}

fn segments_card(ui: &mut Ui, width: f32, card: &SegmentsCard) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, card_height(!card.sub.is_empty())), Sense::hover());
    let painter = ui.painter();
    card_frame(painter, rect, brand::SEGMENT, brand::ITEM_LINE);
    let inner = rect.shrink2(vec2(PAD_X, PAD_Y));
    let name = Rect::from_min_size(inner.min, vec2(inner.width(), NAME_H));
    name_row(painter, name, &card.name, &card.label);
    let count = card.segments.len().max(1) as f32;
    let each = ((inner.width() - SEGMENT_GAP * (count - 1.0)) / count).max(1.0);
    let top = name.bottom() + INNER_GAP;
    for (index, segment) in card.segments.iter().enumerate() {
        let left = inner.left() + index as f32 * (each + SEGMENT_GAP);
        let cell = Rect::from_min_size(pos2(left, top), vec2(each, BAR_H));
        let fill = match segment {
            Segment::Done => brand::TEAL,
            Segment::Active => brand::BLUE.gamma_multiply(0.6),
            Segment::Todo => brand::ITEM_LINE,
        };
        painter.rect_filled(cell, CornerRadius::same(3), fill);
    }
    if !card.sub.is_empty() {
        let row = Rect::from_min_size(
            pos2(inner.left(), top + BAR_H + INNER_GAP - 2.0),
            vec2(inner.width(), DETAIL_H),
        );
        detail_row(painter, row, &card.sub);
    }
}

fn quiet_line(ui: &mut Ui, width: f32, line: &QuietLine) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, QUIET_H), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.0, brand::FAINT);
    let elapsed = painter.layout_no_wrap(line.elapsed.clone(), mono(13.0), brand::MUTED);
    let text_x = rect.left() + 30.0;
    let room = (rect.right() - elapsed.size().x - 10.0 - text_x).max(1.0);
    let name = ellipsized(painter, &line.name, body(13.0), brand::MUTED, room);
    painter.galley(pos2(text_x, rect.center().y - name.size().y / 2.0), name, brand::MUTED);
    painter.galley(
        pos2(rect.right() - elapsed.size().x, rect.center().y - elapsed.size().y / 2.0),
        elapsed,
        brand::MUTED,
    );
}

fn stalled_row(ui: &mut Ui, width: f32, card: &StalledCard) -> bool {
    let button_w = button_width(ui, "Ask agent", Size::Compact);
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, Size::Compact.height() + 2.0 * STALLED_PAD_Y),
        Sense::hover(),
    );
    let painter = ui.painter();
    card_frame(
        painter,
        rect,
        brand::ORANGE.gamma_multiply(0.07),
        brand::ORANGE.gamma_multiply(0.3),
    );
    let inner = rect.shrink2(vec2(PAD_X, STALLED_PAD_Y));
    painter.circle_filled(pos2(inner.left() + 4.0, inner.center().y), 4.0, brand::ORANGE);
    let text_x = inner.left() + 20.0;
    let room = (inner.right() - button_w - 12.0 - text_x).max(1.0);
    let line = ellipsized(painter, &card.line, body(13.0), brand::ORANGE_SOFT, room);
    painter.galley(pos2(text_x, inner.center().y - line.size().y / 2.0), line, brand::ORANGE_SOFT);
    let button = Rect::from_center_size(
        pos2(inner.right() - button_w / 2.0, inner.center().y),
        vec2(button_w, Size::Compact.height()),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(button));
    quiet_button(&mut child, "Ask agent", true, Size::Compact).clicked()
}

fn finished_line(ui: &mut Ui, width: f32, line: &FinishedLine) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, FINISHED_H), Sense::hover());
    let painter = ui.painter();
    let ink = match (line.failed, line.ended) {
        (true, _) => brand::RED,
        (false, true) => brand::FAINT,
        (false, false) => brand::GREEN,
    };
    let mark = pos2(rect.left() + 11.0, rect.center().y);
    if line.ended {
        painter
            .line_segment([mark + vec2(-4.0, 0.0), mark + vec2(4.0, 0.0)], Stroke::new(2.0, ink));
    } else if line.failed {
        let arm = 4.0;
        for (a, b) in [(vec2(-arm, -arm), vec2(arm, arm)), (vec2(-arm, arm), vec2(arm, -arm))] {
            painter.line_segment([mark + a, mark + b], Stroke::new(2.0, ink));
        }
    } else {
        painter.add(Shape::line(
            vec![mark + vec2(-4.5, 0.0), mark + vec2(-1.5, 3.5), mark + vec2(4.5, -4.0)],
            Stroke::new(2.0, ink),
        ));
    }
    let detail = painter.layout_no_wrap(line.detail.clone(), mono(13.0), brand::MUTED);
    let text_x = rect.left() + 30.0;
    let room = (rect.right() - detail.size().x - 10.0 - text_x).max(1.0);
    let name = ellipsized(painter, &line.name, body(13.0), brand::MUTED, room);
    painter.galley(pos2(text_x, rect.center().y - name.size().y / 2.0), name, brand::MUTED);
    painter.galley(
        pos2(rect.right() - detail.size().x, rect.center().y - detail.size().y / 2.0),
        detail,
        brand::MUTED,
    );
}

#[cfg(test)]
mod tests;
