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
use crate::screen_kit::{Size, body, button_width, heading, quiet_button};

pub(crate) const GAP: f32 = 12.0;
const PAD_X: f32 = 16.0;
const PAD_Y: f32 = 14.0;
const NAME_H: f32 = 20.0;
const BAR_H: f32 = 6.0;
const DETAIL_H: f32 = 16.0;
const INNER_GAP: f32 = 10.0;
const QUIET_H: f32 = 26.0;
const STALLED_PAD_Y: f32 = 8.0;
const SEGMENT_GAP: f32 = 4.0;
const MARK_W: f32 = 20.0;
const MIN_NAME_ROOM: f32 = 80.0;

/// Draws the cards and returns the job and the message of an Ask agent click.
pub(crate) fn show(ui: &mut Ui, width: f32, stack: bool, cards: &[Card]) -> Option<(u64, String)> {
    let mut action = None;
    for (index, card) in cards.iter().enumerate() {
        if index > 0 {
            let small = matches!(card, Card::Finished(_) | Card::Quiet(_));
            ui.add_space(if small { 4.0 } else { GAP });
        }
        match card {
            Card::Bar(card) => bar_card(ui, width, stack, card),
            Card::Segments(card) => segments_card(ui, width, stack, card),
            Card::Quiet(line) => quiet_line(ui, width, stack, line),
            Card::Stalled(card) => {
                if stalled_row(ui, width, stack, card) {
                    action = Some((card.id, card.ask.clone()));
                }
            }
            Card::Finished(line) => finished_line(ui, width, stack, line),
        }
    }
    action
}

/// Where a line with no frame of its own keeps its content: the inset the cards and the stalled row
/// give theirs, so marks and right-hand values line up with them.
fn row_inner(rect: Rect) -> Rect {
    rect.shrink2(vec2(PAD_X, 0.0))
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

/// Whether a name and its value at the right leave the name `MIN_NAME_ROOM` beside the value.
fn stacks(painter: &egui::Painter, row_w: f32, stack: bool, right: &str) -> bool {
    if right.is_empty() {
        return false;
    }
    let right_w = painter.layout_no_wrap(right.to_owned(), body(13.0), brand::muted()).size().x;
    stack || row_w - right_w - 10.0 < MIN_NAME_ROOM
}

fn name_row(painter: &egui::Painter, row: Rect, name: &str, right: &str, stacked: bool) {
    let right_galley = painter.layout_no_wrap(right.to_owned(), body(13.0), brand::muted());
    if stacked {
        let name = ellipsized(painter, name, heading(TEXT_TITLE), brand::text(), row.width());
        let top = Rect::from_min_size(row.min, vec2(row.width(), NAME_H));
        painter.galley(pos2(top.left(), top.center().y - name.size().y / 2.0), name, brand::text());
        let at = pos2(row.left(), row.top() + NAME_H + (NAME_H - right_galley.size().y) / 2.0);
        painter.galley(at, right_galley, brand::muted());
        return;
    }
    let room = (row.width() - right_galley.size().x - 10.0).max(1.0);
    let name = ellipsized(painter, name, heading(TEXT_TITLE), brand::text(), room);
    painter.galley(pos2(row.left(), row.center().y - name.size().y / 2.0), name, brand::text());
    painter.galley(
        pos2(row.right() - right_galley.size().x, row.center().y - right_galley.size().y / 2.0),
        right_galley,
        brand::muted(),
    );
}

fn detail_row(painter: &egui::Painter, row: Rect, detail: &str) {
    let galley = ellipsized(painter, detail, body(13.0), brand::muted(), row.width());
    painter.galley(
        pos2(row.left(), row.center().y - galley.size().y / 2.0),
        galley,
        brand::muted(),
    );
}

fn card_height(detail_rows: usize, stacked: bool) -> f32 {
    2.0 * PAD_Y
        + NAME_H
        + if stacked { NAME_H } else { 0.0 }
        + INNER_GAP
        + BAR_H
        + if detail_rows > 0 { INNER_GAP - 2.0 } else { 0.0 }
        + detail_rows as f32 * DETAIL_H
}

fn bar_card(ui: &mut Ui, width: f32, stack: bool, card: &BarCard) {
    let right = card.right.as_deref().unwrap_or("");
    let stacked = stacks(ui.painter(), width - 2.0 * PAD_X, stack, right);
    let rows: Vec<&str> = [card.detail.as_str(), card.stats.as_str()]
        .into_iter()
        .filter(|row| !row.is_empty())
        .collect();
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, card_height(rows.len(), stacked)), Sense::hover());
    let painter = ui.painter();
    card_frame(painter, rect, brand::segment(), brand::item_line());
    let inner = rect.shrink2(vec2(PAD_X, PAD_Y));
    let name_h = if stacked { 2.0 * NAME_H } else { NAME_H };
    let name = Rect::from_min_size(inner.min, vec2(inner.width(), name_h));
    name_row(painter, name, &card.name, right, stacked);
    let track = Rect::from_min_size(
        pos2(inner.left(), name.bottom() + INNER_GAP),
        vec2(inner.width(), BAR_H),
    );
    painter.rect_filled(track, CornerRadius::same(3), brand::item_line());
    let fill =
        Rect::from_min_size(track.min, vec2(track.width() * card.fraction.clamp(0.0, 1.0), BAR_H));
    painter.rect_filled(fill, CornerRadius::same(3), brand::teal());
    for (index, text) in rows.into_iter().enumerate() {
        let top = track.bottom() + INNER_GAP - 2.0 + index as f32 * DETAIL_H;
        let row = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), DETAIL_H));
        detail_row(painter, row, text);
    }
}

fn segments_card(ui: &mut Ui, width: f32, stack: bool, card: &SegmentsCard) {
    let stacked = stacks(ui.painter(), width - 2.0 * PAD_X, stack, &card.label);
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, card_height(usize::from(!card.sub.is_empty()), stacked)),
        Sense::hover(),
    );
    let painter = ui.painter();
    card_frame(painter, rect, brand::segment(), brand::item_line());
    let inner = rect.shrink2(vec2(PAD_X, PAD_Y));
    let name_h = if stacked { 2.0 * NAME_H } else { NAME_H };
    let name = Rect::from_min_size(inner.min, vec2(inner.width(), name_h));
    name_row(painter, name, &card.name, &card.label, stacked);
    let count = card.segments.len().max(1) as f32;
    let each = ((inner.width() - SEGMENT_GAP * (count - 1.0)) / count).max(1.0);
    let top = name.bottom() + INNER_GAP;
    for (index, segment) in card.segments.iter().enumerate() {
        let left = inner.left() + index as f32 * (each + SEGMENT_GAP);
        let cell = Rect::from_min_size(pos2(left, top), vec2(each, BAR_H));
        let fill = match segment {
            Segment::Done => brand::teal(),
            Segment::Active => brand::blue().gamma_multiply(0.6),
            Segment::Todo => brand::item_line(),
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

/// A name and its value at the right, one line or, with no room for both, the value under it.
fn two_part_line(
    ui: &mut Ui,
    width: f32,
    stack: bool,
    (name, value): (&str, &str),
    mark: impl FnOnce(&egui::Painter, Rect),
) {
    let painter = ui.painter().clone();
    let value = painter.layout_no_wrap(value.to_owned(), body(13.0), brand::muted());
    let stacked = stack || width - 2.0 * PAD_X - MARK_W - value.size().x - 10.0 < MIN_NAME_ROOM;
    let height = if stacked { 2.0 * QUIET_H - 6.0 } else { QUIET_H };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let inner = row_inner(rect);
    let first = Rect::from_min_size(rect.min, vec2(width, QUIET_H));
    mark(&painter, first);
    let text_x = inner.left() + MARK_W;
    if stacked {
        let name = ellipsized(
            &painter,
            name,
            body(13.0),
            brand::muted(),
            (inner.right() - text_x).max(1.0),
        );
        painter.galley(pos2(text_x, first.center().y - name.size().y / 2.0), name, brand::muted());
        let y = first.bottom() - 6.0 + (QUIET_H - value.size().y) / 2.0 - 4.0;
        painter.galley(pos2(text_x, y), value, brand::muted());
        return;
    }
    let room = (inner.right() - value.size().x - 10.0 - text_x).max(1.0);
    let name = ellipsized(&painter, name, body(13.0), brand::muted(), room);
    painter.galley(pos2(text_x, rect.center().y - name.size().y / 2.0), name, brand::muted());
    painter.galley(
        pos2(inner.right() - value.size().x, rect.center().y - value.size().y / 2.0),
        value,
        brand::muted(),
    );
}

fn quiet_line(ui: &mut Ui, width: f32, stack: bool, line: &QuietLine) {
    two_part_line(ui, width, stack, (&line.name, &line.elapsed), |painter, first| {
        let inner = row_inner(first);
        painter.circle_filled(pos2(inner.left() + 3.0, first.center().y), 3.0, brand::faint());
    });
}

fn stalled_row(ui: &mut Ui, width: f32, stack: bool, card: &StalledCard) -> bool {
    let button_w = button_width(ui, "Ask agent", Size::Compact);
    let stacked = stack || width - 2.0 * PAD_X - button_w - 32.0 < MIN_NAME_ROOM;
    let rows = if stacked { 2.0 } else { 1.0 };
    let (rect, _) = ui.allocate_exact_size(
        vec2(
            width,
            rows * Size::Compact.height() + 2.0 * STALLED_PAD_Y + if stacked { 4.0 } else { 0.0 },
        ),
        Sense::hover(),
    );
    let painter = ui.painter();
    card_frame(
        painter,
        rect,
        brand::orange().gamma_multiply(0.07),
        brand::orange().gamma_multiply(0.3),
    );
    let inner = rect.shrink2(vec2(PAD_X, STALLED_PAD_Y));
    let line_row = Rect::from_min_size(inner.min, vec2(inner.width(), Size::Compact.height()));
    painter.circle_filled(pos2(inner.left() + 4.0, line_row.center().y), 4.0, brand::orange());
    let text_x = inner.left() + 20.0;
    let room =
        if stacked { inner.right() - text_x } else { inner.right() - button_w - 12.0 - text_x };
    let line = ellipsized(painter, &card.line, body(13.0), brand::orange_soft(), room.max(1.0));
    painter.galley(
        pos2(text_x, line_row.center().y - line.size().y / 2.0),
        line,
        brand::orange_soft(),
    );
    let button = if stacked {
        Rect::from_min_size(
            pos2(text_x, inner.top() + Size::Compact.height() + 4.0),
            vec2(button_w, Size::Compact.height()),
        )
    } else {
        Rect::from_center_size(
            pos2(inner.right() - button_w / 2.0, inner.center().y),
            vec2(button_w, Size::Compact.height()),
        )
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(button));
    quiet_button(&mut child, "Ask agent", true, Size::Compact).clicked()
}

fn finished_line(ui: &mut Ui, width: f32, stack: bool, line: &FinishedLine) {
    let ink = match (line.failed, line.ended) {
        (true, _) => brand::red(),
        (false, true) => brand::faint(),
        (false, false) => brand::green(),
    };
    two_part_line(ui, width, stack, (&line.name, &line.detail), |painter, first| {
        let inner = row_inner(first);
        let mark = pos2(inner.left() + 4.5, first.center().y);
        if line.ended {
            painter.line_segment(
                [mark + vec2(-4.0, 0.0), mark + vec2(4.0, 0.0)],
                Stroke::new(2.0, ink),
            );
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
    });
}

#[cfg(test)]
mod tests;
