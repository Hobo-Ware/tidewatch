//! Draws the job cards on top of the Progress panel: a bar card with the big percent and a rate
//! sparkline, a segmented card for a small total, an orange card for a job that stopped moving
//! (with Ask agent), and one line for each job that finished.

use egui::{Color32, CornerRadius, Rect, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::job_model::{BarCard, Card, FinishedLine, Segment, SegmentsCard, StalledCard};
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, button_width, heading, mono, quiet_button};

pub(crate) const GAP: f32 = 12.0;
const PAD: f32 = 16.0;
const SMALL_PAD_Y: f32 = 14.0;
const DOT: f32 = 8.0;
const HEAD_H: f32 = 20.0;
const BIG_H: f32 = 38.0;
const BAR_H: f32 = 8.0;
const SPARK_H: f32 = 38.0;
const STAT_H: f32 = 34.0;
const SEGMENT_H: f32 = 6.0;
const FINISHED_H: f32 = 26.0;

/// Draws the cards and returns the job and the message of an Ask agent click.
pub(crate) fn show(ui: &mut Ui, width: f32, cards: &[Card]) -> Option<(u64, String)> {
    let mut action = None;
    for (index, card) in cards.iter().enumerate() {
        if index > 0 {
            ui.add_space(if matches!(card, Card::Finished(_)) { 4.0 } else { GAP });
        }
        match card {
            Card::Bar(card) => bar_card(ui, width, card),
            Card::Segments(card) => segments_card(ui, width, card),
            Card::Stalled(card) => {
                if stalled_card(ui, width, card) {
                    action = Some((card.id, card.ask.clone()));
                }
            }
            Card::Finished(line) => finished_line(ui, width, line),
        }
    }
    action
}

fn dot(painter: &egui::Painter, center: egui::Pos2, ink: Color32, glow: bool) {
    if glow {
        painter.circle_filled(center, DOT / 2.0 + 4.0, ink.gamma_multiply(0.15));
    }
    painter.circle_filled(center, DOT / 2.0, ink);
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

fn header(painter: &egui::Painter, row: Rect, name: &str, right: &str, ink: Color32, glow: bool) {
    dot(painter, pos2(row.left() + DOT / 2.0 + 4.0, row.center().y), ink, glow);
    let right_galley = painter.layout_no_wrap(right.to_owned(), body(12.0), brand::MUTED);
    let name_x = row.left() + DOT + 12.0;
    let room = (row.right() - right_galley.size().x - 8.0 - name_x).max(1.0);
    let name = ellipsized(painter, name, heading(14.0), brand::TEXT, room);
    painter.galley(pos2(name_x, row.center().y - name.size().y / 2.0), name, brand::TEXT);
    painter.galley(
        pos2(row.right() - right_galley.size().x, row.center().y - right_galley.size().y / 2.0),
        right_galley,
        brand::MUTED,
    );
}

fn bar_card(ui: &mut Ui, width: f32, card: &BarCard) {
    let spark = card.series.len() >= 2;
    let inner_h = HEAD_H
        + GAP
        + BIG_H
        + if card.fraction.is_some() { GAP + BAR_H } else { 0.0 }
        + if spark { GAP + SPARK_H } else { 0.0 }
        + GAP
        + STAT_H;
    let (rect, _) = ui.allocate_exact_size(vec2(width, inner_h + 2.0 * PAD), Sense::hover());
    let painter = ui.painter();
    card_frame(painter, rect, brand::SEGMENT, brand::TEAL.gamma_multiply(0.2));
    let inner = rect.shrink(PAD);
    let mut y = inner.top();
    let right = match (card.phase.is_empty(), &card.elapsed) {
        (true, elapsed) => elapsed.clone(),
        (false, elapsed) => format!("{} \u{b7} {elapsed}", card.phase),
    };
    let head = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), HEAD_H));
    header(painter, head, &card.name, &right, brand::TEAL, true);
    y += HEAD_H + GAP;
    paint_big(
        painter,
        Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), BIG_H)),
        card,
    );
    y += BIG_H;
    if let Some(fraction) = card.fraction {
        y += GAP;
        let track = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), BAR_H));
        painter.rect_filled(track, CornerRadius::same(4), brand::ITEM_LINE);
        let fill =
            Rect::from_min_size(track.min, vec2(track.width() * fraction.clamp(0.0, 1.0), BAR_H));
        painter.rect_filled(fill, CornerRadius::same(4), brand::TEAL);
        y += BAR_H;
    }
    if spark {
        y += GAP;
        sparkline(
            painter,
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), SPARK_H)),
            &card.series,
        );
        y += SPARK_H;
    }
    y += GAP;
    stats(painter, Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), STAT_H)), card);
}

fn paint_big(painter: &egui::Painter, row: Rect, card: &BarCard) {
    let main = card.percent.clone().unwrap_or_else(|| card.counts.clone());
    let size = if card.percent.is_some() { 32.0 } else { 24.0 };
    let big = painter.layout_no_wrap(main, mono(size), brand::TEXT);
    let base = row.center().y + big.size().y / 2.0;
    let x = row.left() + big.size().x + 10.0;
    painter.galley(pos2(row.left(), row.center().y - big.size().y / 2.0), big, brand::TEXT);
    if card.percent.is_some() {
        let counts =
            ellipsized(painter, &card.counts, mono(13.0), brand::MUTED, (row.right() - x).max(1.0));
        painter.galley(pos2(x, base - counts.size().y - 3.0), counts, brand::MUTED);
    }
}

fn sparkline(painter: &egui::Painter, area: Rect, series: &[f32]) {
    let steps = (series.len() - 1).max(1) as f32;
    let low = series.iter().copied().fold(f32::MAX, f32::min);
    let high = series.iter().copied().fold(0.0_f32, f32::max);
    let stretch = |value: f32| if high > low { (value - low) / (high - low) } else { 0.5 };
    let points: Vec<egui::Pos2> = series
        .iter()
        .enumerate()
        .map(|(index, value)| {
            pos2(
                area.left() + area.width() * index as f32 / steps,
                area.bottom() - 4.0 - (area.height() - 8.0) * stretch(*value).clamp(0.0, 1.0),
            )
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.0, brand::TEAL.gamma_multiply(0.55))));
}

fn stats(painter: &egui::Painter, row: Rect, card: &BarCard) {
    let estimating = card.eta.is_none();
    let cells = [
        card.rate.as_deref().map(|text| ("RATE", text, false)),
        Some(("ETA", card.eta.as_deref().unwrap_or("estimating"), estimating)),
        card.done_at.as_deref().map(|text| ("DONE AT", text, false)),
    ];
    let mut x = row.left();
    for (label, value, faint) in cells.into_iter().flatten() {
        let ink = if faint { brand::FAINT } else { brand::TEXT };
        let label_galley = painter.layout_no_wrap(label.to_owned(), body(11.5), brand::FAINT);
        let value_galley = painter.layout_no_wrap(value.to_owned(), mono(13.0), ink);
        let width = label_galley.size().x.max(value_galley.size().x);
        if x + width > row.right() + 0.5 {
            break;
        }
        painter.galley(pos2(x, row.top()), label_galley, brand::FAINT);
        painter.galley(pos2(x, row.top() + 16.0), value_galley, ink);
        x += width + 18.0;
    }
}

fn segments_card(ui: &mut Ui, width: f32, card: &SegmentsCard) {
    let sub_h = if card.sub.is_empty() { 0.0 } else { 10.0 + 16.0 };
    let inner_h = HEAD_H + 10.0 + SEGMENT_H + sub_h;
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, inner_h + 2.0 * SMALL_PAD_Y), Sense::hover());
    let painter = ui.painter();
    card_frame(painter, rect, brand::SEGMENT, brand::ITEM_LINE);
    let inner = Rect::from_min_max(
        pos2(rect.left() + PAD, rect.top() + SMALL_PAD_Y),
        pos2(rect.right() - PAD, rect.bottom() - SMALL_PAD_Y),
    );
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), HEAD_H));
    header(painter, head, &card.name, &card.label, brand::BLUE, false);
    let count = card.segments.len().max(1) as f32;
    let gap = 4.0;
    let each = ((inner.width() - gap * (count - 1.0)) / count).max(1.0);
    let top = inner.top() + HEAD_H + 10.0;
    for (index, segment) in card.segments.iter().enumerate() {
        let left = inner.left() + index as f32 * (each + gap);
        let cell = Rect::from_min_size(pos2(left, top), vec2(each, SEGMENT_H));
        let fill = match segment {
            Segment::Done => brand::TEAL,
            Segment::Active => brand::BLUE.gamma_multiply(0.6),
            Segment::Todo => brand::ITEM_LINE,
        };
        painter.rect_filled(cell, CornerRadius::same(3), fill);
    }
    if !card.sub.is_empty() {
        let sub = ellipsized(painter, &card.sub, body(12.5), brand::MUTED, inner.width());
        painter.galley(pos2(inner.left(), top + SEGMENT_H + 10.0), sub, brand::MUTED);
    }
}

fn stalled_card(ui: &mut Ui, width: f32, card: &StalledCard) -> bool {
    let button_w = button_width(ui, "Ask agent", Size::Compact);
    let inner_h = 40.0_f32.max(Size::Compact.height());
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, inner_h + 2.0 * SMALL_PAD_Y), Sense::hover());
    let painter = ui.painter();
    card_frame(
        painter,
        rect,
        brand::ORANGE.gamma_multiply(0.07),
        brand::ORANGE.gamma_multiply(0.3),
    );
    let inner = rect.shrink2(vec2(PAD, SMALL_PAD_Y));
    dot(painter, pos2(inner.left() + DOT / 2.0, inner.center().y), brand::ORANGE, false);
    let text_x = inner.left() + DOT + 12.0;
    let room = (inner.right() - button_w - 12.0 - text_x).max(1.0);
    let name = ellipsized(painter, &card.name, heading(14.0), brand::TEXT, room);
    let line = ellipsized(painter, &card.line, body(12.5), brand::ORANGE_SOFT, room);
    let block = name.size().y + 2.0 + line.size().y;
    let top = inner.center().y - block / 2.0;
    let line_top = top + name.size().y + 2.0;
    painter.galley(pos2(text_x, top), name, brand::TEXT);
    painter.galley(pos2(text_x, line_top), line, brand::ORANGE_SOFT);
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
    let ink = if line.failed { brand::RED } else { brand::TEAL };
    let mark = pos2(rect.left() + 11.0, rect.center().y);
    if line.failed {
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
