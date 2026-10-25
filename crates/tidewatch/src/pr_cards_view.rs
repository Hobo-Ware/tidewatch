//! Draws the pull request cards in the Reader (PLAN.md, D4 follow-up): each [`Model`] as a small
//! card, a sent message as the user's bubble, and the waiting and paused rows at the end.

use std::collections::HashMap;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Painter, Pos2, Rect,
    RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, epaint::text::LayoutJob,
    epaint::text::TextFormat, pos2, vec2,
};
use tidewatch_core::pr_status::SummaryChecks;

use crate::brand;
use crate::pr_card_model::{
    self as model, Block, Bubble, Button, CardAction, Context, Icon, Ink, JobLine, Look, Model,
};
use crate::pr_watch::{Handling, Watch, paused_note};
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, heading, mono, primary_button, quiet_button};

const MAX_CARD_W: f32 = 760.0;
const MAX_BUBBLE_W: f32 = 560.0;
const ICON_BOX: f32 = 16.0;
const INDENT: f32 = 26.0;
const GUESS_H: f32 = 110.0;
const BARS_GAP: f32 = 8.0;

/// What the cards of a tab are drawn from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Data<'a> {
    pub(crate) watch: &'a Watch,
    pub(crate) profile: &'a str,
    pub(crate) handling: Handling,
    pub(crate) policy: tidewatch_core::prefs::GithubPrefs,
    pub(crate) now_ms: i64,
}

impl<'a> Data<'a> {
    pub(crate) fn cards(&self) -> &'a [tidewatch_core::pr_cards::Card] {
        self.watch.cards.cards()
    }

    /// Whether the timeline has anything to draw after the transcript's last row.
    pub(crate) fn has_tail(&self) -> bool {
        !self.watch.pending.is_empty()
            || self.watch.pause.is_some()
            || self.handling == Handling::Paused
    }

    fn context(&self) -> Context<'_> {
        let summary = self.watch.summary();
        Context {
            cards: &self.watch.cards,
            profile: self.profile,
            mode: self.policy.mode,
            handling: self.handling,
            policy: self.policy,
            pending: &self.watch.pending,
            url: summary.map_or("", |summary| summary.url.as_str()),
            checks_green: summary.is_some_and(|summary| summary.checks == SummaryChecks::Passed),
            cleanup: &self.watch.cleanup,
        }
    }
}

/// Measured heights of drawn cards, thrown away when the width changes.
#[derive(Debug, Default)]
pub(crate) struct Heights {
    width: f32,
    measured: HashMap<u64, f32>,
}

impl Heights {
    pub(crate) fn for_width(&mut self, width: f32) {
        if (width - self.width).abs() >= 1.0 {
            self.width = width;
            self.measured.clear();
        }
    }

    pub(crate) fn record(&mut self, id: u64, height: f32) {
        self.measured.insert(id, height);
    }

    pub(crate) fn height(&self, id: u64) -> f32 {
        self.measured.get(&id).copied().unwrap_or(GUESS_H)
    }
}

fn ink_color(ink: Ink) -> Color32 {
    match ink {
        Ink::Teal => brand::TEAL,
        Ink::Green => brand::GREEN,
        Ink::Red => brand::RED,
        Ink::Orange => brand::ORANGE,
        Ink::Amber => brand::AMBER,
        Ink::Quiet => brand::MUTED,
    }
}

/// Draws the card at `index` of the timeline and returns the click it got.
pub(crate) fn card(ui: &mut Ui, data: &Data<'_>, index: usize) -> Option<CardAction> {
    let at_ms = data.cards().get(index)?.at_ms;
    match model::look(index, data.context())? {
        Look::Card(card) => draw_card(ui, &card, &model::age(data.now_ms, at_ms)),
        Look::Bubble(bubble) => {
            draw_bubble(ui, &bubble);
            None
        }
    }
}

/// What follows the last card: messages waiting for the agent, and the paused row.
pub(crate) fn tail(ui: &mut Ui, data: &Data<'_>) -> Option<CardAction> {
    let mut clicked = None;
    for pending in &data.watch.pending {
        ui.add_space(14.0);
        let text = model::waiting_text(data.profile, pending);
        let buttons = model::waiting_buttons(pending);
        clicked = clicked.or(waiting_row(ui, &text, &buttons));
    }
    if let Some(pause) = &data.watch.pause {
        ui.add_space(14.0);
        let (text, fix) = model::paused_text(pause);
        paused_row(ui, &text, fix);
    }
    if data.handling == Handling::Paused {
        ui.add_space(14.0);
        paused_row(ui, &paused_note(), None);
    }
    clicked
}

fn card_width(ui: &Ui) -> f32 {
    ui.available_width().min(MAX_CARD_W)
}

fn draw_card(ui: &mut Ui, card: &Model, age: &str) -> Option<CardAction> {
    let width = card_width(ui);
    let frame = Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(20, 16));
    let mut clicked = None;
    frame.show(ui, |ui| {
        ui.set_width(width - 42.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        header(ui, card, age);
        for block in &card.body {
            indented(ui, |ui| draw_block(ui, block));
        }
        if !card.buttons.is_empty() || card.note.is_some() {
            indented(ui, |ui| clicked = button_row(ui, &card.buttons, card.note.as_deref()));
        }
    });
    clicked
}

fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_space(INDENT);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}

fn header(ui: &mut Ui, card: &Model, age: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    let ink = ink_color(card.ink);
    paint_icon(painter, pos2(rect.left() + ICON_BOX / 2.0, rect.center().y), card.icon, ink);
    let right = if card.live { "live" } else { age };
    let right_galley = painter.layout_no_wrap(right.to_owned(), body(12.0), brand::FAINT);
    let right_x = rect.right() - right_galley.size().x;
    painter.galley(
        pos2(right_x, rect.center().y - right_galley.size().y / 2.0),
        right_galley,
        brand::FAINT,
    );
    let title = painter.layout_no_wrap(card.title.clone(), heading(15.0), brand::TEXT);
    let title_w = title.size().x;
    let x = rect.left() + INDENT;
    painter.galley(pos2(x, rect.center().y - title.size().y / 2.0), title, brand::TEXT);
    if !card.sub.is_empty() {
        let sub_x = x + title_w + 10.0;
        let room = right_x - 12.0 - sub_x;
        if room > 24.0 {
            let sub = ellipsized(painter, &card.sub, body(12.0), brand::MUTED, room);
            painter.galley(pos2(sub_x, rect.center().y - sub.size().y / 2.0), sub, brand::MUTED);
        }
    }
}

fn draw_block(ui: &mut Ui, block: &Block) {
    match block {
        Block::Mono(text) => {
            ui.label(RichText::new(text).monospace().size(13.0).color(brand::STEP_TEXT));
        }
        Block::Text(text) => {
            ui.label(RichText::new(text).size(14.0).color(brand::STEP_TEXT));
        }
        Block::Jobs(jobs) => {
            Frame::new()
                .fill(brand::SUNKEN)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for job in jobs {
                        job_row(ui, job);
                    }
                });
        }
        Block::Log(lines) => log_box(ui, lines),
        Block::Comments(comments) => comment_list(ui, comments),
        Block::Bars { done, total } => bars(ui, *done, *total),
    }
}

fn job_row(ui: &mut Ui, job: &JobLine) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter();
    let name_ink = if job.passed { brand::SOFT } else { brand::TEXT };
    let mut x = rect.left();
    let took_w = job.took.as_ref().map_or(0.0, |took| {
        let galley = painter.layout_no_wrap(took.clone(), body(12.0), brand::FAINT);
        let width = galley.size().x;
        painter.galley(
            pos2(rect.right() - width, rect.center().y - galley.size().y / 2.0),
            galley,
            brand::FAINT,
        );
        width + 12.0
    });
    let limit = rect.right() - took_w;
    let name = painter.layout_no_wrap(job.name.clone(), mono(13.0), name_ink);
    let name_w = name.size().x;
    painter.galley(pos2(x, rect.center().y - name.size().y / 2.0), name, name_ink);
    x += name_w + 10.0;
    if let Some(at) = &job.at {
        let said = painter.layout_no_wrap("failed at".to_owned(), body(13.0), brand::MUTED);
        let said_w = said.size().x;
        painter.galley(pos2(x, rect.center().y - said.size().y / 2.0), said, brand::MUTED);
        x += said_w + 8.0;
        if limit - x > 20.0 {
            let step = ellipsized(painter, at, mono(13.0), brand::RED, limit - x);
            painter.galley(pos2(x, rect.center().y - step.size().y / 2.0), step, brand::RED);
        }
    }
}

fn log_box(ui: &mut Ui, lines: &[String]) {
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in lines {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 19.0), Sense::hover());
                let galley = ellipsized(ui.painter(), line, mono(12.0), brand::SOFT, rect.width());
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                    galley,
                    brand::SOFT,
                );
            }
        });
}

fn comment_list(ui: &mut Ui, comments: &[(String, String)]) {
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, (place, text)) in comments.iter().enumerate() {
                if index > 0 {
                    let (line, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, CornerRadius::ZERO, brand::SIDEBAR_LINE);
                }
                Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(place).monospace().size(12.0).color(brand::MUTED));
                    ui.label(RichText::new(text).size(14.0).color(brand::TEXT));
                });
            }
        });
}

fn bars(ui: &mut Ui, done: usize, total: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    let count = total.max(1);
    let gaps = BARS_GAP * (count - 1) as f32;
    let each = ((rect.width() - gaps) / count as f32).max(2.0);
    for n in 0..count {
        let bar = Rect::from_min_size(
            pos2(rect.left() + n as f32 * (each + BARS_GAP), rect.top()),
            vec2(each, 4.0),
        );
        ui.painter().rect_filled(bar, CornerRadius::same(2), brand::ITEM_LINE);
        let (fill, share) = if n < done { (brand::GREEN, 1.0) } else { (brand::AMBER, 0.4) };
        let lit = Rect::from_min_size(bar.min, vec2(bar.width() * share, 4.0));
        ui.painter().rect_filled(lit, CornerRadius::same(2), fill);
    }
}

fn button_row(ui: &mut Ui, buttons: &[Button], note: Option<&str>) -> Option<CardAction> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for button in buttons {
            let response = if button.primary {
                primary_button(ui, &button.label, true, Size::Compact)
            } else {
                quiet_button(ui, &button.label, true, Size::Compact)
            };
            if response.clicked() {
                clicked = Some(button.action.clone());
            }
        }
        if let Some(note) = note {
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let galley = ellipsized(ui.painter(), note, body(12.0), brand::FAINT, rect.width());
            let x = (rect.right() - galley.size().x).max(rect.left());
            ui.painter().galley(
                pos2(x, rect.center().y - galley.size().y / 2.0),
                galley,
                brand::FAINT,
            );
        }
    });
    clicked
}

pub(crate) fn draw_bubble(ui: &mut Ui, bubble: &Bubble) {
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        Frame::new()
            .fill(brand::ITEM)
            .stroke(Stroke::new(1.0, brand::ITEM_LINE))
            .corner_radius(CornerRadius { nw: 12, ne: 12, sw: 12, se: 4 })
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_max_width(MAX_BUBBLE_W.min(ui.available_width()));
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut who = LayoutJob::default();
                who.append(bubble.who, 0.0, TextFormat::simple(body(11.0), brand::MUTED));
                who.append(&bubble.tag, 8.0, TextFormat::simple(body(11.0), brand::TEAL));
                ui.label(who);
                let mut text = LayoutJob::default();
                text.wrap.max_width = ui.available_width();
                text.append(&bubble.headline, 0.0, TextFormat::simple(body(13.5), brand::TEXT));
                if let Some(more) = &bubble.more {
                    text.append(more, 8.0, TextFormat::simple(body(13.0), brand::FAINT));
                }
                ui.label(text);
            });
    });
}

fn waiting_row(ui: &mut Ui, text: &str, buttons: &[Button; 2]) -> Option<CardAction> {
    let (rect, _) = ui.allocate_exact_size(vec2(card_width(ui), 40.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), brand::CARD);
    dashed_rect(painter, rect, Stroke::new(1.0, brand::FIELD_LINE));
    painter.circle_filled(pos2(rect.left() + 18.0, rect.center().y), 4.0, brand::AMBER);
    let mut clicked = None;
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    right.spacing_mut().item_spacing.x = 6.0;
    for button in buttons.iter().rev() {
        if quiet_button(&mut right, &button.label, true, Size::Compact).clicked() {
            clicked = Some(button.action.clone());
        }
    }
    let left = rect.left() + 32.0;
    let galley = ellipsized(
        ui.painter(),
        text,
        body(13.5),
        brand::TEXT,
        right.min_rect().left() - 12.0 - left,
    );
    ui.painter().galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    clicked
}

fn paused_row(ui: &mut Ui, text: &str, fix: Option<&str>) {
    let (rect, _) = ui.allocate_exact_size(vec2(card_width(ui), 52.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(8),
        brand::ITEM,
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
    let boxed = Rect::from_center_size(
        pos2(rect.left() + 14.0 + ICON_BOX / 2.0, rect.center().y),
        vec2(ICON_BOX, ICON_BOX),
    );
    painter.rect_filled(boxed, CornerRadius::same(8), brand::BUTTON);
    paint_icon(painter, boxed.center(), Icon::Bang, brand::MUTED);
    let mut right = rect.right() - 14.0;
    if let Some(fix) = fix {
        let galley = painter.layout_no_wrap(fix.to_owned(), mono(12.5), brand::SOFT);
        right -= galley.size().x;
        painter.galley(pos2(right, rect.center().y - galley.size().y / 2.0), galley, brand::SOFT);
        right -= 12.0;
    }
    let left = boxed.right() + 10.0;
    let galley = ellipsized(painter, text, body(13.5), brand::TEXT, right - left);
    painter.galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, brand::TEXT);
}

fn dashed_rect(painter: &Painter, rect: Rect, stroke: Stroke) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for n in 0..4 {
        painter.extend(Shape::dashed_line(&[corners[n], corners[(n + 1) % 4]], stroke, 4.0, 3.0));
    }
}

/// The glyphs of the 28 px box, drawn from a 16 by 16 design centered on `center`.
pub(crate) fn paint_icon(painter: &Painter, center: Pos2, icon: Icon, ink: Color32) {
    let at = |x: f32, y: f32| center + vec2(x - 8.0, y - 8.0);
    let stroke = Stroke::new(1.5, ink);
    let ring = |x: f32, y: f32, r: f32| painter.circle_stroke(at(x, y), r, stroke);
    let line = |points: &[(f32, f32)]| {
        painter.add(Shape::line(points.iter().map(|(x, y)| at(*x, *y)).collect(), stroke));
    };
    match icon {
        Icon::Opened => {
            ring(4.0, 3.5, 1.8);
            ring(4.0, 12.5, 1.8);
            ring(12.0, 12.5, 1.8);
            line(&[(4.0, 5.3), (4.0, 10.7)]);
            line(&[(12.0, 10.7), (12.0, 7.5), (10.5, 5.0), (7.5, 4.5)]);
            line(&[(9.0, 3.0), (7.5, 4.5), (9.0, 6.0)]);
        }
        Icon::Running => {
            let points: Vec<Pos2> = (0..=18)
                .map(|n| {
                    let angle = -0.5 + n as f32 * (std::f32::consts::TAU * 0.75 / 18.0);
                    at(8.0 + 5.0 * angle.cos(), 8.0 + 5.0 * angle.sin())
                })
                .collect();
            painter.add(Shape::line(points, Stroke::new(2.0, ink)));
        }
        Icon::Failed => {
            ring(8.0, 8.0, 6.2);
            line(&[(5.8, 5.8), (10.2, 10.2)]);
            line(&[(10.2, 5.8), (5.8, 10.2)]);
        }
        Icon::Passed => {
            ring(8.0, 8.0, 6.2);
            line(&[(5.3, 8.2), (7.2, 10.1), (10.8, 6.3)]);
        }
        Icon::Review => {
            painter.add(Shape::closed_line(
                [
                    (2.5, 3.5),
                    (13.5, 3.5),
                    (13.5, 10.5),
                    (7.5, 10.5),
                    (4.5, 13.0),
                    (4.5, 10.5),
                    (2.5, 10.5),
                ]
                .iter()
                .map(|(x, y)| at(*x, *y))
                .collect(),
                stroke,
            ));
        }
        Icon::Comment(initial) => {
            painter.text(center, Align2::CENTER_CENTER, initial, FontId::proportional(12.0), ink);
        }
        Icon::Conflict => {
            line(&[(4.0, 2.5), (4.0, 13.5)]);
            line(&[(12.0, 2.5), (12.0, 6.5), (11.1, 8.6), (9.0, 9.5), (4.0, 9.5)]);
        }
        Icon::Approved => line(&[(3.0, 8.5), (6.0, 11.5), (13.0, 4.5)]),
        Icon::Merged => {
            ring(4.0, 3.5, 1.8);
            ring(4.0, 12.5, 1.8);
            ring(12.0, 8.0, 1.8);
            line(&[(4.0, 5.3), (4.0, 10.7)]);
            line(&[(5.5, 4.5), (7.0, 7.0), (9.0, 8.0), (10.2, 8.0)]);
        }
        Icon::Closed => {
            ring(4.0, 3.5, 1.8);
            ring(4.0, 12.5, 1.8);
            line(&[(4.0, 5.3), (4.0, 10.7)]);
            line(&[(10.0, 4.0), (14.0, 8.0)]);
            line(&[(14.0, 4.0), (10.0, 8.0)]);
        }
        Icon::Bang => {
            painter.text(center, Align2::CENTER_CENTER, "!", FontId::proportional(14.0), ink);
        }
    }
}

#[cfg(test)]
mod tests;
