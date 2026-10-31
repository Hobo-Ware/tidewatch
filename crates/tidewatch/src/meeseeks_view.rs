//! Draws a Meeseeks card in the Progress panel: the task with its elapsed time and turn, four stage
//! bars, the status box, and below them the orange struggling card with Take over and Give up.

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Color32, CornerRadius, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::meeseeks_model::{Card, Look, STRUGGLE_BODY, Span, Struggling};
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, heading, mono, quiet_button};
use crate::selectable;

const RADIUS: u8 = 12;
const PAD: f32 = 16.0;
const STRUGGLE_PAD_Y: f32 = 14.0;
const GAP: f32 = 12.0;
const CARDS_GAP: f32 = 14.0;
const BAR_H: f32 = 6.0;
const BAR_GAP: f32 = 6.0;
const BOX_PAD_X: f32 = 12.0;
const BOX_PAD_Y: f32 = 10.0;
const DOT: f32 = 8.0;
const BUTTON_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    TakeOver,
    GiveUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) card: Option<Rect>,
    pub(crate) bars: [Option<Rect>; 4],
    pub(crate) status: Option<Rect>,
    pub(crate) struggle: Option<Rect>,
    pub(crate) take_over: Option<Rect>,
    pub(crate) give_up: Option<Rect>,
}

pub(crate) fn placed_id() -> egui::Id {
    egui::Id::new("meeseeks-card-placed")
}

fn bar_ink(look: Look) -> Color32 {
    match look {
        Look::Done => brand::TEAL,
        Look::Active => brand::BLUE.gamma_multiply(0.6),
        Look::Pending => brand::ITEM_LINE,
        Look::Skipped => brand::ITEM_LINE.gamma_multiply(0.5),
    }
}

fn label_ink(look: Look) -> Color32 {
    match look {
        Look::Active => brand::BLUE_TEXT,
        Look::Skipped => brand::FAINT,
        Look::Done | Look::Pending => brand::MUTED,
    }
}

fn status_job(spans: &[Span], width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for span in spans {
        let ink = if span.strong { brand::TEXT } else { brand::SOFT };
        job.append(&span.text, 0.0, TextFormat::simple(body(13.0), ink));
    }
    job.wrap = TextWrapping::wrap_at_width(width.max(1.0));
    job
}

fn frame(ui: &Ui, rect: Rect, fill: Color32, line: Color32) {
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
}

fn stage_card(ui: &mut Ui, width: f32, card: &Card, placed: &mut Placed) {
    let inner = (width - 2.0 * PAD).max(1.0);
    let painter = ui.painter().clone();
    let meta = painter.layout_no_wrap(card.meta.clone(), mono(12.0), brand::MUTED);
    let title =
        ellipsized(&painter, &card.title, heading(14.0), brand::TEXT, inner - meta.size().x - 10.0);
    let bar_w = (inner - 3.0 * BAR_GAP) / 4.0;
    let labels: Vec<_> = card
        .bars
        .iter()
        .map(|(name, look)| ellipsized(&painter, name, body(12.0), label_ink(*look), bar_w))
        .collect();
    let status = painter.layout_job(status_job(&card.status, inner - 2.0 * BOX_PAD_X));
    let head_h = title.size().y.max(meta.size().y);
    let label_h = labels.iter().map(|galley| galley.size().y).fold(0.0, f32::max);
    let box_h = status.size().y + 2.0 * BOX_PAD_Y;
    let bars_h = if labels.is_empty() { 0.0 } else { BAR_H + GAP + label_h + GAP };
    let height = 2.0 * PAD + head_h + GAP + bars_h + box_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    placed.card = Some(rect);
    frame(ui, rect, brand::ITEM, brand::ITEM_LINE);
    let (left, right) = (rect.left() + PAD, rect.right() - PAD);
    let mut y = rect.top() + PAD;
    selectable::galley(ui, pos2(left, y + (head_h - title.size().y) / 2.0), title, brand::TEXT);
    selectable::galley(
        ui,
        pos2(right - meta.size().x, y + (head_h - meta.size().y) / 2.0),
        meta,
        brand::MUTED,
    );
    y += head_h + GAP;
    if !card.bars.is_empty() {
        for (index, (_, look)) in card.bars.iter().enumerate() {
            let bar = Rect::from_min_size(
                pos2(left + index as f32 * (bar_w + BAR_GAP), y),
                vec2(bar_w, BAR_H),
            );
            painter.rect_filled(bar, CornerRadius::same(3), bar_ink(*look));
            placed.bars[index] = Some(bar);
        }
        y += BAR_H + GAP;
        for (index, (galley, (_, look))) in labels.into_iter().zip(&card.bars).enumerate() {
            let x = left + index as f32 * (bar_w + BAR_GAP);
            selectable::galley(ui, pos2(x, y), galley, label_ink(*look));
        }
        y += label_h + GAP;
    }
    let sunken = Rect::from_min_size(pos2(left, y), vec2(inner, box_h));
    painter.rect_filled(sunken, CornerRadius::same(8), brand::SUNKEN);
    selectable::galley(ui, sunken.min + vec2(BOX_PAD_X, BOX_PAD_Y), status, brand::SOFT);
    placed.status = Some(sunken);
}

fn struggle_card(
    ui: &mut Ui,
    width: f32,
    struggling: &Struggling,
    placed: &mut Placed,
) -> Option<Action> {
    let inner = (width - 2.0 * PAD).max(1.0);
    let painter = ui.painter().clone();
    let reason = painter.layout_no_wrap(struggling.reason.clone(), body(12.0), brand::ORANGE_SOFT);
    let title = ellipsized(
        &painter,
        &struggling.title,
        heading(14.0),
        brand::TEXT,
        inner - DOT - 10.0 - reason.size().x - 10.0,
    );
    let text = painter.layout(STRUGGLE_BODY.to_owned(), body(13.0), brand::ORANGE_SOFT, inner);
    let head_h = title.size().y.max(reason.size().y);
    let button_h = Size::Compact.height();
    let height = 2.0 * STRUGGLE_PAD_Y + head_h + 10.0 + text.size().y + 10.0 + button_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    placed.struggle = Some(rect);
    frame(ui, rect, brand::STRUGGLE_FILL, brand::STRUGGLE_LINE);
    let (left, right) = (rect.left() + PAD, rect.right() - PAD);
    let mut y = rect.top() + STRUGGLE_PAD_Y;
    painter.circle_filled(pos2(left + DOT / 2.0, y + head_h / 2.0), DOT / 2.0, brand::ORANGE);
    selectable::galley(
        ui,
        pos2(left + DOT + 10.0, y + (head_h - title.size().y) / 2.0),
        title,
        brand::TEXT,
    );
    selectable::galley(
        ui,
        pos2(right - reason.size().x, y + (head_h - reason.size().y) / 2.0),
        reason,
        brand::ORANGE_SOFT,
    );
    y += head_h + 10.0;
    let text_h = text.size().y;
    selectable::galley(ui, pos2(left, y), text, brand::ORANGE_SOFT);
    y += text_h + 10.0;
    let row = Rect::from_min_size(pos2(left, y), vec2(inner, button_h));
    let mut action = None;
    ui.scope_builder(
        UiBuilder::new().max_rect(row).layout(Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            let take = quiet_button(ui, "Take over", true, Size::Compact);
            let give = quiet_button(ui, "Give up", true, Size::Compact);
            placed.take_over = Some(take.rect);
            placed.give_up = Some(give.rect);
            if take.clicked() {
                action = Some(Action::TakeOver);
            } else if give.clicked() {
                action = Some(Action::GiveUp);
            }
        },
    );
    action
}

pub(crate) fn show(ui: &mut Ui, width: f32, card: &Card) -> Option<Action> {
    let mut placed = Placed::default();
    stage_card(ui, width, card, &mut placed);
    let mut action = None;
    if let Some(struggling) = &card.struggling {
        ui.add_space(CARDS_GAP);
        action = struggle_card(ui, width, struggling, &mut placed);
    }
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(), placed));
    action
}

#[cfg(test)]
mod tests;
