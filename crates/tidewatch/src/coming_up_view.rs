//! The Coming up section of the Overview: a rail with date labels on the left and a card per
//! scheduled item, dashed for what waits on an event, amber for what only you can decide.

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::progress_ring::paint_dot;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, button_width, heading, quiet_button};
use crate::timeline::{Block, Button, Card, Label, Look, Urgency};

const LABEL_W: f32 = 64.0;
const LABEL_GAP: f32 = 18.0;
const CARD_H: f32 = 48.0;
const CARD_GAP: f32 = 8.0;
const CARD_PAD: f32 = 16.0;
const CARD_RADIUS: u8 = 12;
const BLOCK_GAP: f32 = 14.0;
const RAIL_W: f32 = 2.0;
const SEG_W: f32 = 14.0;
const SEG_H: f32 = 5.0;
const SEG_GAP: f32 = 3.0;
const MAX_SEGMENTS: usize = 10;
const TITLE_MIN_W: f32 = 90.0;
const PART_GAP: f32 = 14.0;
const CORNER_STEPS: usize = 6;

pub(crate) fn block_height(block: &Block) -> f32 {
    let cards = block.cards.len() as f32;
    cards * CARD_H + (cards - 1.0).max(0.0) * CARD_GAP
}

/// The parts at the right of a card in the order they give way (button, tab name, step bar): how
/// many of them fit in `width` beside a title that keeps `TITLE_MIN_W`.
pub(crate) fn parts_that_fit(width: f32, parts: &[f32]) -> usize {
    let mut used = TITLE_MIN_W;
    let mut fit = 0;
    for part in parts {
        used += part + PART_GAP;
        if used > width {
            break;
        }
        fit += 1;
    }
    fit
}

/// The outline of a rounded rectangle as a closed polyline, for the dashed look.
pub(crate) fn rounded_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let corners = [
        (rect.right_top() + vec2(-radius, radius), -std::f32::consts::FRAC_PI_2),
        (rect.right_bottom() + vec2(-radius, -radius), 0.0),
        (rect.left_bottom() + vec2(radius, -radius), std::f32::consts::FRAC_PI_2),
        (rect.left_top() + vec2(radius, radius), std::f32::consts::PI),
    ];
    let mut points = Vec::with_capacity(4 * (CORNER_STEPS + 1) + 1);
    for (center, start) in corners {
        for step in 0..=CORNER_STEPS {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / CORNER_STEPS as f32;
            points.push(center + vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(points[0]);
    points
}

fn segment_ink(mark: Mark) -> Color32 {
    match mark {
        Mark::Done => brand::TEAL,
        Mark::Doing => brand::TEAL.gamma_multiply(0.55),
        Mark::Todo => brand::FIELD_LINE,
        Mark::Skipped => brand::FAINT.gamma_multiply(0.35),
        Mark::Blocked => brand::RED,
    }
}

fn steps_width(steps: &[Mark]) -> f32 {
    let n = steps.len().min(MAX_SEGMENTS) as f32;
    if n == 0.0 { 0.0 } else { n * SEG_W + (n - 1.0) * SEG_GAP }
}

fn label_ink(urgency: Urgency) -> Color32 {
    match urgency {
        Urgency::Ahead => brand::FAINT,
        Urgency::Today => brand::TEAL,
        Urgency::Late => brand::RED,
    }
}

fn paint_label(painter: &egui::Painter, column: Rect, label: &Label) -> f32 {
    let x = column.center().x;
    match label {
        Label::Now => {
            let galley = painter.layout_no_wrap("NOW".to_owned(), heading(12.0), brand::TEAL);
            painter.galley(
                pos2(x - galley.size().x / 2.0, column.top() + 3.0),
                galley.clone(),
                brand::TEAL,
            );
            3.0 + galley.size().y
        }
        Label::Later => {
            let galley = painter.layout_no_wrap(
                "LATER".to_owned(),
                FontId::proportional(12.0),
                brand::FAINT,
            );
            painter.galley(
                pos2(x - galley.size().x / 2.0, column.top() + 3.0),
                galley.clone(),
                brand::FAINT,
            );
            3.0 + galley.size().y
        }
        Label::Date { text, under, urgency } => {
            let ink = if text.starts_with('~') { brand::SOFT } else { brand::TEXT };
            let top = painter.layout_no_wrap(text.clone(), FontId::monospace(12.0), ink);
            let height = top.size().y;
            painter.galley(pos2(x - top.size().x / 2.0, column.top() + 3.0), top, ink);
            let below = painter.layout_no_wrap(
                under.clone(),
                FontId::proportional(11.0),
                label_ink(*urgency),
            );
            let bottom = column.top() + 3.0 + height + 1.0;
            let end = bottom + below.size().y;
            painter.galley(pos2(x - below.size().x / 2.0, bottom), below, label_ink(*urgency));
            end - column.top()
        }
    }
}

fn paint_card_frame(painter: &egui::Painter, rect: Rect, look: Look) {
    let radius = CornerRadius::same(CARD_RADIUS);
    match look {
        Look::Now | Look::Plain | Look::You => {
            let fill = match look {
                Look::Now => brand::TEAL_NAV,
                Look::You => brand::ORANGE.gamma_multiply(0.07),
                _ => brand::CARD,
            };
            painter.rect_filled(rect, radius, fill);
        }
        Look::On | Look::Parked => {
            let ink = if look == Look::On { brand::FIELD_LINE } else { brand::ITEM_LINE };
            let path = rounded_path(rect.shrink(0.5), f32::from(CARD_RADIUS));
            painter.extend(Shape::dashed_line(&path, Stroke::new(1.0, ink), 4.0, 3.0));
        }
    }
}

fn title_ink(look: Look) -> Color32 {
    match look {
        Look::On => brand::SOFT,
        Look::Parked => brand::FAINT,
        Look::Now | Look::Plain | Look::You => brand::TEXT,
    }
}

enum Part {
    Button(Button),
    Tab,
    Steps,
}

fn card(ui: &mut Ui, rect: Rect, card: &Card, id: usize) -> bool {
    let response = ui.interact(rect, ui.id().with(("coming-card", id)), Sense::click());
    let painter = ui.painter().clone();
    paint_card_frame(&painter, rect, card.look);
    if response.hovered() && matches!(card.look, Look::Plain | Look::Now | Look::You) {
        painter.rect_stroke(
            rect,
            CornerRadius::same(CARD_RADIUS),
            Stroke::new(1.0, brand::FIELD_LINE),
            StrokeKind::Inside,
        );
    }
    let inner = rect.shrink2(vec2(CARD_PAD, 0.0));
    let tab = painter.layout_no_wrap(card.tab.clone(), FontId::proportional(12.0), brand::FAINT);
    let mut parts = Vec::new();
    if let Some(button) = card.button {
        parts.push((Part::Button(button), button_width(ui, button.label(), Size::Compact)));
    }
    parts.push((Part::Tab, tab.size().x));
    if !card.steps.is_empty() {
        parts.push((Part::Steps, steps_width(&card.steps)));
    }
    let widths: Vec<f32> = parts.iter().map(|(_, width)| *width).collect();
    let fit = parts_that_fit(inner.width(), &widths);
    let mut right = inner.right();
    let mut clicked = false;
    for (part, width) in parts.into_iter().take(fit) {
        let mid = rect.center().y;
        match part {
            Part::Button(button) => {
                let at = Rect::from_center_size(
                    pos2(right - width / 2.0, mid),
                    vec2(width, Size::Compact.height()),
                );
                ui.scope_builder(UiBuilder::new().max_rect(at), |ui| {
                    clicked = quiet_button(ui, button.label(), true, Size::Compact).clicked();
                });
            }
            Part::Tab => {
                let at = pos2(right - width, mid - tab.size().y / 2.0);
                painter.galley(at, tab.clone(), brand::FAINT);
            }
            Part::Steps => {
                let mut x = right - width;
                for mark in card.steps.iter().take(MAX_SEGMENTS) {
                    let at = Rect::from_min_size(pos2(x, mid - SEG_H / 2.0), vec2(SEG_W, SEG_H));
                    painter.rect_filled(at, CornerRadius::same(3), segment_ink(*mark));
                    x += SEG_W + SEG_GAP;
                }
            }
        }
        right -= width + PART_GAP;
    }
    let mut left = inner.left();
    if card.look == Look::Now {
        paint_dot(&painter, pos2(left + 5.0, rect.center().y), 10.0, brand::TEAL);
        left += 22.0;
    }
    let title = ellipsized(
        &painter,
        &card.title,
        FontId::proportional(14.0),
        title_ink(card.look),
        (right - left).max(1.0),
    );
    painter.galley(pos2(left, rect.center().y - title.size().y / 2.0), title, title_ink(card.look));
    response.on_hover_text(&card.tip).clicked() || clicked
}

/// Draws the section and returns the row of the card that was clicked, or of its button.
pub(crate) fn show(ui: &mut Ui, blocks: &[Block]) -> Option<usize> {
    let width = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    let painter = ui.painter_at(head);
    let title = painter.text(
        head.left_center(),
        Align2::LEFT_CENTER,
        "Coming up",
        heading(16.0),
        brand::TEXT,
    );
    painter.text(
        pos2(title.right() + 14.0, head.center().y + 1.0),
        Align2::LEFT_CENTER,
        "all tabs",
        FontId::proportional(13.0),
        brand::MUTED,
    );
    ui.add_space(12.0);
    let mut clicked = None;
    let count = blocks.len();
    for (index, block) in blocks.iter().enumerate() {
        let height = block_height(block);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let column = Rect::from_min_size(rect.min, vec2(LABEL_W, height));
        let painter = ui.painter().clone();
        let label_h = paint_label(&painter, column, &block.label);
        if index + 1 < count {
            let top = rect.top() + label_h + 8.0;
            let bottom = rect.bottom() + BLOCK_GAP;
            let rail = Rect::from_min_max(
                pos2(column.center().x - RAIL_W / 2.0, top),
                pos2(column.center().x + RAIL_W / 2.0, bottom),
            );
            painter.rect_filled(rail, CornerRadius::ZERO, brand::CARD_LINE);
        }
        let cards_left = rect.left() + LABEL_W + LABEL_GAP;
        for (slot, item) in block.cards.iter().enumerate() {
            let at = pos2(cards_left, rect.top() + slot as f32 * (CARD_H + CARD_GAP));
            let place = Rect::from_min_size(at, vec2(rect.right() - cards_left, CARD_H));
            if card(ui, place, item, index * 100 + slot) {
                clicked = Some(item.row);
            }
        }
        ui.add_space(BLOCK_GAP);
    }
    clicked
}

#[cfg(test)]
mod tests;
