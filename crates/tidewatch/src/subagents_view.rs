//! The running agents list above the Reader's reply box: one card with a header line (how many
//! run, how many finished, Hide) and a row per agent. It only draws; the caller keeps whether it
//! is folded or opened and carries out what it asks for.

use egui::{Align2, CornerRadius, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::brand;
use crate::reader_view::{INSET, ellipsized};
use crate::screen_kit::{
    TEXT_SECONDARY, TEXT_TAG, TagFill, body, mono, paint_tag, selection_row, tag_width,
};
use crate::subagents_model::{List, Plan};

const PAD: f32 = 12.0;
const HEAD_H: f32 = 28.0;
const ROW_H: f32 = 24.0;
const OUTER: f32 = 6.0;
const FOOT: f32 = 10.0;
const GAP: f32 = 10.0;
const DOT_BOX: f32 = 14.0;
const MIN_WHAT_W: f32 = 96.0;
const RADIUS: u8 = 10;
const ROW_BLEED: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) list: &'a List,
    pub(crate) folded: bool,
    pub(crate) opened: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Fold,
    Open,
    /// A click on the row at this index: read that agent's chat.
    Read(usize),
}

fn chrome() -> f32 {
    2.0 * OUTER + HEAD_H + FOOT
}

fn more_line(plan: Plan) -> bool {
    plan.more > 0 || plan.can_fold_back
}

fn plan_in(facts: Facts<'_>, room: f32) -> Plan {
    let fit = ((room - chrome()) / ROW_H).floor().max(0.0) as usize;
    let plan = facts.list.plan(facts.opened, fit);
    if more_line(plan) && plan.rows >= fit {
        facts.list.plan(facts.opened, fit.saturating_sub(1))
    } else {
        plan
    }
}

/// The height of the card with the gap around it, in at most `room`: its rows beyond what fits
/// stay behind the "more" line.
pub(crate) fn height(facts: Facts<'_>, room: f32) -> f32 {
    if facts.folded {
        return 2.0 * OUTER + HEAD_H;
    }
    let plan = plan_in(facts, room);
    let lines = plan.rows + usize::from(more_line(plan));
    chrome() + lines as f32 * ROW_H
}

struct Columns {
    right_left: f32,
    tag_left: Option<f32>,
}

fn columns(ui: &Ui, inner: Rect, facts: Facts<'_>, rows: usize) -> Columns {
    let painter = ui.painter();
    let shown = &facts.list.rows[..rows];
    let right_w = shown
        .iter()
        .map(|row| painter.layout_no_wrap(row.right.clone(), mono(TEXT_TAG), brand::MUTED).size().x)
        .fold(0.0, f32::max);
    let tag_w = shown
        .iter()
        .filter_map(|row| row.tag.as_deref())
        .map(|tag| tag_width(painter, tag))
        .fold(0.0, f32::max);
    let right_left = inner.right() - right_w;
    let tag_left = inner.left() + DOT_BOX + GAP + MIN_WHAT_W;
    let tag_at = right_left - GAP - tag_w;
    Columns { right_left, tag_left: (tag_w > 0.0 && tag_at >= tag_left).then_some(tag_at) }
}

/// Draws the card in `band` (`height` tall) and returns what was asked for.
pub(crate) fn show(ui: &mut Ui, band: Rect, facts: Facts<'_>) -> Vec<Out> {
    let mut out = Vec::new();
    let card = Rect::from_min_max(
        pos2(band.left() + INSET, band.top() + OUTER),
        pos2(band.right() - INSET, band.bottom() - OUTER),
    );
    let radius = CornerRadius::same(RADIUS);
    ui.painter().rect_filled(card, radius, brand::ITEM);
    ui.painter().rect_stroke(card, radius, Stroke::new(1.0, brand::ITEM_LINE), StrokeKind::Inside);
    let inner = Rect::from_min_max(
        pos2(card.left() + PAD, card.top()),
        pos2(card.right() - PAD, card.bottom()),
    );
    let head_mid = card.top() + HEAD_H / 2.0 + 2.0;
    ui.painter().text(
        pos2(inner.left(), head_mid),
        Align2::LEFT_CENTER,
        facts.list.head(),
        body(TEXT_TAG),
        brand::MUTED,
    );
    let word = if facts.folded { "Show" } else { "Hide" };
    let word_galley = ui.painter().layout_no_wrap(word.to_owned(), body(TEXT_TAG), brand::SOFT);
    let word_rect = Rect::from_min_size(
        pos2(inner.right() - word_galley.size().x, head_mid - word_galley.size().y / 2.0),
        word_galley.size(),
    );
    let word_hit =
        ui.interact(word_rect.expand2(vec2(6.0, 6.0)), Id::new("agents-fold"), Sense::click());
    if word_hit.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        out.push(Out::Fold);
    }
    let hot = ui.rect_contains_pointer(word_rect.expand2(vec2(6.0, 6.0)));
    ui.painter().galley(
        word_rect.min,
        word_galley,
        if hot { brand::TEAL_TEXT } else { brand::SOFT },
    );
    if let Some(note) = facts.list.finished_note() {
        ui.painter().text(
            pos2(word_rect.left(), head_mid),
            Align2::RIGHT_CENTER,
            format!("{note} \u{b7} "),
            body(TEXT_TAG),
            brand::MUTED,
        );
    }
    if facts.folded {
        return out;
    }
    let room = band.height();
    let plan = plan_in(facts, room);
    let columns = columns(ui, inner, facts, plan.rows);
    for (index, row) in facts.list.rows[..plan.rows].iter().enumerate() {
        let top = card.top() + OUTER + HEAD_H + index as f32 * ROW_H;
        let hit = Rect::from_min_size(
            pos2(inner.left() - ROW_BLEED, top),
            vec2(inner.width() + 2.0 * ROW_BLEED, ROW_H),
        );
        let response = ui.interact(hit, Id::new(("agents-row", &row.id)), Sense::click());
        selection_row(ui.painter(), hit, false, response.hovered());
        paint_row(ui, inner, top, row, &columns);
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            out.push(Out::Read(index));
        }
    }
    if more_line(plan) {
        let top = card.top() + OUTER + HEAD_H + plan.rows as f32 * ROW_H;
        let text =
            if plan.more > 0 { format!("+{} more", plan.more) } else { "Show fewer".to_owned() };
        let galley = ui.painter().layout_no_wrap(text, body(TEXT_SECONDARY), brand::SOFT);
        let rect = Rect::from_min_size(
            pos2(inner.left() + DOT_BOX + GAP, top + (ROW_H - galley.size().y) / 2.0),
            galley.size(),
        );
        let hit = rect.expand2(vec2(6.0, 4.0));
        let response = ui.interact(hit, Id::new("agents-more"), Sense::click());
        let ink = if response.hovered() { brand::TEAL_TEXT } else { brand::SOFT };
        ui.painter().galley(rect.min, galley, ink);
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            out.push(Out::Open);
        }
    }
    if facts.list.running > 0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    out
}

fn paint_row(ui: &Ui, inner: Rect, top: f32, row: &crate::subagents_model::Row, columns: &Columns) {
    let painter = ui.painter();
    let mid = top + ROW_H / 2.0;
    let (what_ink, tag_ink, right_ink, dot) = if row.running {
        (brand::TEXT, brand::SOFT, brand::MUTED, brand::TEAL)
    } else {
        (brand::FAINT, brand::FAINT, brand::FAINT, brand::GREEN)
    };
    let centre = pos2(inner.left() + 4.0, mid);
    if row.running {
        painter.circle_filled(centre, 7.0, dot.gamma_multiply(0.13));
    }
    painter.circle_filled(centre, 4.0, dot);
    let what_left = inner.left() + DOT_BOX + GAP - 4.0;
    let what_right = columns.tag_left.unwrap_or(columns.right_left) - GAP;
    let what =
        ellipsized(painter, &row.what, body(TEXT_SECONDARY), what_ink, what_right - what_left);
    painter.galley(pos2(what_left, mid - what.size().y / 2.0), what, what_ink);
    if let (Some(left), Some(tag)) = (columns.tag_left, row.tag.as_deref()) {
        paint_tag(painter, (left, mid), tag, tag_ink, TagFill::Line(brand::ITEM_LINE));
    }
    painter.text(
        pos2(inner.right(), mid),
        Align2::RIGHT_CENTER,
        &row.right,
        mono(TEXT_TAG),
        right_ink,
    );
}

#[cfg(test)]
mod tests;
