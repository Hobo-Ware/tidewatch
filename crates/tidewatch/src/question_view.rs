//! Draws what the Reader shows for questions: the live card (radio rows, checkbox tiles, a
//! "Something else" field, Next, Back, Send answers, Skip), the compact summary after the answer,
//! the quick-reply chips under an agent's last message, and the permission card, which has the
//! same look. The state is `question_card`; this file reads keys and clicks and reports an effect.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Galley, Id, Key, Layout, Margin,
    Modifiers, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder,
    pos2, vec2,
};
use tidewatch_core::question::{QuestionOption, Select};

use crate::brand;
use crate::question_card::{Card, CardKey, Effect, MAX_KEYED, Row};
use crate::quick_reply::Quick;
use crate::reader_keys::Answer;
use crate::reader_rows::{Asked, QuestionRow};
use crate::reader_view::{INSET, ellipsized};
use crate::screen_kit::{Size, body, mono, primary_button, quiet_button};

const CARD_RADIUS: u8 = 12;
const CARD_PAD_X: i8 = 16;
const CARD_PAD_Y: i8 = 14;
const ROW_RADIUS: u8 = 9;
const ROW_GAP: f32 = 8.0;
const ROW_PAD_X: f32 = 12.0;
const ROW_PAD_Y: f32 = 10.0;
const MARK_W: f32 = 26.0;
const NUMBER_W: f32 = 22.0;
const HEADER_H: f32 = 22.0;
const OTHER_FIELD_H: f32 = 30.0;
const ESTIMATED_H: f32 = 300.0;
const MIN_LIST_H: f32 = 120.0;
const CHROME_H: f32 = 150.0;
pub(crate) const PERMISSION_H: f32 = 88.0;
pub(crate) const PERMISSION_COMMAND_H: f32 = 40.0;
const SUMMARY_QUESTION_W: f32 = 260.0;

fn teal_wash() -> Color32 {
    brand::TEAL.gamma_multiply(0.1)
}

/// A live card: the state of the questions in row `seq`, and what the last frame measured.
pub(crate) struct Live {
    pub(crate) seq: u64,
    pub(crate) card: Card,
    height: f32,
    focus_other: bool,
    focused_once: bool,
    /// A press landed on the card: egui drops the focus on a press, so it is asked for again.
    reclaim: bool,
    #[cfg(test)]
    rects: std::cell::RefCell<Vec<Rect>>,
}

impl Live {
    pub(crate) fn new(seq: u64, card: Card) -> Self {
        Self {
            seq,
            card,
            height: ESTIMATED_H,
            focus_other: false,
            focused_once: false,
            reclaim: false,
            #[cfg(test)]
            rects: std::cell::RefCell::default(),
        }
    }

    /// How much room the card took last frame, with the gap above the reply box.
    pub(crate) fn height(&self) -> f32 {
        self.height + ROW_GAP
    }
}

fn command_label() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }
}

fn card_frame(accent: Color32) -> Frame {
    Frame::new()
        .fill(brand::ITEM)
        .stroke(Stroke::new(1.0, accent))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::symmetric(CARD_PAD_X, CARD_PAD_Y))
}

fn paint_glow(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(CARD_RADIUS + 2),
        Stroke::new(4.0, color.gamma_multiply(0.08)),
        StrokeKind::Middle,
    );
}

fn pill(painter: &Painter, at: Pos2, text: &str, ink: Color32, font: FontId) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font, ink);
    let rect = Rect::from_min_size(at, galley.size() + vec2(18.0, 4.0));
    painter.rect_filled(rect, CornerRadius::same(255), ink.gamma_multiply(0.12));
    painter.galley(rect.min + vec2(9.0, 2.0), galley, ink);
    rect
}

fn read_keys(ui: &mut Ui, plain: bool) -> Vec<CardKey> {
    ui.input_mut(|input| {
        let mut keys = Vec::new();
        if input.consume_key(Modifiers::COMMAND, Key::Enter) {
            keys.push(CardKey::CmdEnter);
        }
        if !plain {
            return keys;
        }
        for (key, mapped) in [
            (Key::Enter, CardKey::Enter),
            (Key::Space, CardKey::Space),
            (Key::ArrowUp, CardKey::Up),
            (Key::ArrowDown, CardKey::Down),
        ] {
            if input.consume_key(Modifiers::NONE, key) {
                keys.push(mapped);
            }
        }
        for (key, number) in [
            (Key::Num1, 1),
            (Key::Num2, 2),
            (Key::Num3, 3),
            (Key::Num4, 4),
            (Key::Num5, 5),
            (Key::Num6, 6),
            (Key::Num7, 7),
            (Key::Num8, 8),
            (Key::Num9, 9),
        ] {
            if input.consume_key(Modifiers::NONE, key) {
                keys.push(CardKey::Digit(number));
            }
        }
        keys
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Radio,
    Check,
}

struct OptionLayout {
    label: Arc<Galley>,
    description: Option<Arc<Galley>>,
    recommended: Option<Arc<Galley>>,
    height: f32,
}

fn layout_option(painter: &Painter, option: &QuestionOption, text_w: f32) -> OptionLayout {
    let recommended = option
        .recommended()
        .then(|| painter.layout_no_wrap("Recommended".to_owned(), body(11.0), brand::TEAL_TEXT));
    let label_room = text_w - recommended.as_ref().map_or(0.0, |tag| tag.size().x + 22.0);
    let label =
        painter.layout(option.shown().to_owned(), body(14.0), brand::TEXT, label_room.max(40.0));
    let description = option
        .description
        .as_ref()
        .map(|text| painter.layout(text.clone(), body(13.0), brand::SOFT, text_w));
    let height = ROW_PAD_Y * 2.0
        + label.size().y.max(18.0)
        + description.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    OptionLayout { label, description, recommended, height }
}

fn paint_mark(painter: &Painter, at: Pos2, mark: Mark, selected: bool) {
    let line = if selected { brand::TEAL } else { brand::FIELD_LINE };
    match mark {
        Mark::Radio => {
            painter.circle_stroke(at, 7.0, Stroke::new(1.5, line));
            if selected {
                painter.circle_filled(at, 3.5, brand::TEAL);
            }
        }
        Mark::Check => {
            let rect = Rect::from_center_size(at, vec2(15.0, 15.0));
            let radius = CornerRadius::same(4);
            if selected {
                painter.rect_filled(rect, radius, brand::TEAL);
                let ink = Stroke::new(1.8, brand::GROUND);
                painter.line_segment([at + vec2(-3.6, 0.2), at + vec2(-1.0, 2.9)], ink);
                painter.line_segment([at + vec2(-1.0, 2.9), at + vec2(3.8, -2.8)], ink);
            } else {
                painter.rect_stroke(rect, radius, Stroke::new(1.5, line), StrokeKind::Inside);
            }
        }
    }
}

fn paint_row_box(painter: &Painter, rect: Rect, selected: bool, cursor: bool) {
    let (fill, line) = match (selected, cursor) {
        (true, _) => (brand::TEAL_NAV, brand::TEAL),
        (false, true) => (brand::CARD, brand::SOFT.gamma_multiply(0.5)),
        (false, false) => (brand::CARD, brand::ITEM_LINE),
    };
    painter.rect(
        rect,
        CornerRadius::same(ROW_RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
}

struct Pick {
    selected: bool,
    cursor: bool,
    number: usize,
}

/// One option as a row (radio or checkbox) of `rect`'s width and height.
fn paint_option(
    ui: &Ui,
    rect: Rect,
    layout: &OptionLayout,
    mark: Mark,
    pick: &Pick,
    hovered: bool,
) {
    let painter = ui.painter();
    paint_row_box(painter, rect, pick.selected, pick.cursor || hovered);
    painter.rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(ROW_RADIUS - 1),
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let top = rect.top() + ROW_PAD_Y;
    paint_mark(painter, pos2(rect.left() + ROW_PAD_X + 7.0, top + 9.0), mark, pick.selected);
    let text_x = rect.left() + ROW_PAD_X + MARK_W;
    painter.galley(pos2(text_x, top), layout.label.clone(), brand::TEXT);
    if let Some(tag) = &layout.recommended {
        let at = pos2(text_x + layout.label.size().x + 8.0, top + 1.0);
        let tag_rect = Rect::from_min_size(at, vec2(tag.size().x + 14.0, 16.0));
        painter.rect_stroke(
            tag_rect,
            CornerRadius::same(255),
            Stroke::new(1.0, brand::TEAL.gamma_multiply(0.4)),
            StrokeKind::Inside,
        );
        painter.galley(
            pos2(tag_rect.left() + 7.0, tag_rect.center().y - tag.size().y / 2.0),
            tag.clone(),
            brand::TEAL_TEXT,
        );
    }
    if let Some(text) = &layout.description {
        painter.galley(
            pos2(text_x, top + layout.label.size().y.max(18.0) + 2.0),
            text.clone(),
            brand::SOFT,
        );
    }
    if pick.number <= MAX_KEYED {
        painter.text(
            pos2(rect.right() - ROW_PAD_X, top + 9.0),
            Align2::RIGHT_CENTER,
            pick.number.to_string(),
            mono(12.0),
            brand::FAINT,
        );
    }
}

fn option_text_width(row_w: f32) -> f32 {
    (row_w - 2.0 * ROW_PAD_X - MARK_W - NUMBER_W).max(60.0)
}

struct Drawn {
    clicked: Option<Row>,
    field: Option<Rect>,
}

fn draw_single(ui: &mut Ui, live: &mut Live, id: Id) -> Drawn {
    let width = ui.available_width();
    let mut drawn = Drawn { clicked: None, field: None };
    let question = live.card.question().clone();
    let rows = live.card.rows();
    let cursor = live.card.cursor();
    for (index, row) in rows.iter().enumerate() {
        match row {
            Row::Option(option) => {
                let layout = layout_option(
                    ui.painter(),
                    &question.options[*option],
                    option_text_width(width),
                );
                let (rect, response) =
                    ui.allocate_exact_size(vec2(width, layout.height), Sense::click());
                let pick = Pick {
                    selected: live.card.is_picked(*option),
                    cursor: cursor == index,
                    number: index + 1,
                };
                paint_option(ui, rect, &layout, Mark::Radio, &pick, response.hovered());
                #[cfg(test)]
                live.rects.borrow_mut().push(rect);
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    drawn.clicked = Some(*row);
                }
            }
            Row::Other => {
                let height = ROW_PAD_Y * 2.0 + 18.0 + 6.0 + OTHER_FIELD_H;
                let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
                let on = live.card.other_is_on();
                paint_row_box(ui.painter(), rect, on, cursor == index || response.hovered());
                let top = rect.top() + ROW_PAD_Y;
                paint_mark(
                    ui.painter(),
                    pos2(rect.left() + ROW_PAD_X + 7.0, top + 9.0),
                    Mark::Radio,
                    on,
                );
                let text_x = rect.left() + ROW_PAD_X + MARK_W;
                ui.painter().text(
                    pos2(text_x, top),
                    Align2::LEFT_TOP,
                    "Something else",
                    body(14.0),
                    brand::TEXT,
                );
                ui.painter().text(
                    pos2(rect.right() - ROW_PAD_X, top + 9.0),
                    Align2::RIGHT_CENTER,
                    (index + 1).to_string(),
                    mono(12.0),
                    brand::FAINT,
                );
                let field = Rect::from_min_max(
                    pos2(text_x, top + 18.0 + 6.0),
                    pos2(rect.right() - ROW_PAD_X - NUMBER_W + 6.0, rect.bottom() - ROW_PAD_Y),
                );
                drawn.field = Some(field);
                if response.on_hover_cursor(egui::CursorIcon::Text).clicked() {
                    drawn.clicked = Some(*row);
                }
                other_field(ui, live, id, field);
            }
        }
        ui.add_space(ROW_GAP);
    }
    drawn
}

fn other_field(ui: &mut Ui, live: &mut Live, id: Id, field: Rect) {
    let on = live.card.other_is_on();
    ui.painter().rect(
        field,
        CornerRadius::same(7),
        brand::SEGMENT,
        Stroke::new(1.0, brand::FIELD_LINE),
        StrokeKind::Inside,
    );
    let inner = field.shrink2(vec2(10.0, 4.0));
    let focus_id = id.with("other");
    let mut text = live.card.typed_mut().clone();
    let response = ui.put(
        inner,
        TextEdit::singleline(&mut text)
            .id(focus_id)
            .frame(Frame::NONE)
            .hint_text("Type your own answer")
            .font(body(13.0))
            .text_color(if on { brand::TEXT } else { brand::SOFT })
            .desired_width(inner.width())
            .vertical_align(Align::Center),
    );
    if std::mem::take(&mut live.focus_other) {
        response.request_focus();
    }
    if response.changed() {
        *live.card.typed_mut() = text;
        if !live.card.other_is_on() {
            live.card.choose(Row::Other);
        }
    }
    if response.gained_focus() && !live.card.other_is_on() {
        live.card.choose(Row::Other);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            field,
            CornerRadius::same(7),
            Stroke::new(1.0, brand::TEAL),
            StrokeKind::Inside,
        );
    }
}

fn draw_multi(ui: &mut Ui, live: &Live) -> Option<Row> {
    let width = ui.available_width();
    let card = &live.card;
    let any_description = card.question().options.iter().any(|option| option.description.is_some());
    let columns = if any_description { 1 } else { 2 };
    let tile_w = (width - ROW_GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut clicked = None;
    let options = &card.question().options;
    for (chunk_start, chunk) in options.chunks(columns).enumerate().map(|(i, c)| (i * columns, c)) {
        let layouts: Vec<OptionLayout> = chunk
            .iter()
            .map(|option| layout_option(ui.painter(), option, option_text_width(tile_w)))
            .collect();
        let height = layouts.iter().map(|layout| layout.height).fold(0.0, f32::max);
        let (band, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        for (offset, layout) in layouts.iter().enumerate() {
            let index = chunk_start + offset;
            let rect = Rect::from_min_size(
                pos2(band.left() + offset as f32 * (tile_w + ROW_GAP), band.top()),
                vec2(tile_w, height),
            );
            let response = ui.interact(rect, ui.id().with(("tile", index)), Sense::click());
            let pick = Pick {
                selected: card.is_picked(index),
                cursor: card.cursor() == index,
                number: index + 1,
            };
            paint_option(ui, rect, layout, Mark::Check, &pick, response.hovered());
            #[cfg(test)]
            live.rects.borrow_mut().push(rect);
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(Row::Option(index));
            }
        }
        ui.add_space(ROW_GAP);
    }
    clicked
}

fn header_row(ui: &mut Ui, live: &Live, profile: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let painter = ui.painter();
    let chip = pill(painter, rect.left_top(), &format!("{profile} asks"), brand::TEAL, body(11.5));
    let mut x = chip.right() + 10.0;
    if let Some(header) = &live.card.question().header {
        let galley = painter.layout_no_wrap(header.clone(), body(12.0), brand::MUTED);
        painter.galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            brand::MUTED,
        );
        x += galley.size().x;
    }
    let _ = x;
    let count = live.card.count();
    let multi = live.card.question().select == Select::Multi;
    let text = match (count, multi) {
        (1, false) => String::new(),
        (1, true) => "pick any".to_owned(),
        (n, false) => format!("{} of {n}", live.card.step() + 1),
        (n, true) => format!("{} of {n} \u{b7} pick any", live.card.step() + 1),
    };
    let mut right = rect.right();
    if !text.is_empty() {
        let galley = painter.layout_no_wrap(text, body(12.0), brand::FAINT);
        right -= galley.size().x;
        painter.galley(pos2(right, rect.center().y - galley.size().y / 2.0), galley, brand::FAINT);
        right -= 10.0;
    }
    if count > 1 {
        for index in (0..count).rev() {
            right -= 18.0;
            let color = if index <= live.card.step() { brand::TEAL } else { brand::ITEM_LINE };
            painter.rect_filled(
                Rect::from_min_size(pos2(right, rect.center().y - 2.0), vec2(18.0, 4.0)),
                CornerRadius::same(4),
                color,
            );
            right -= 4.0;
        }
    }
}

fn hint_job(live: &Live) -> LayoutJob {
    let card = &live.card;
    let keys = TextFormat::simple(mono(12.0), brand::SOFT);
    let words = TextFormat::simple(body(12.0), brand::FAINT);
    let mut job = LayoutJob::default();
    let pick = match card.question().select {
        Select::Single => format!("1-{}", card.rows().len().min(MAX_KEYED)),
        Select::Multi => "Space".to_owned(),
    };
    let verb = if card.question().select == Select::Single { " pick" } else { " tick" };
    job.append(&pick, 0.0, keys.clone());
    job.append(verb, 0.0, words.clone());
    job.append(" \u{b7} ", 0.0, words.clone());
    if card.is_last() {
        job.append(&format!("{}+Enter", command_label()), 0.0, keys);
        job.append(" send", 0.0, words);
    } else {
        job.append("Enter", 0.0, keys);
        job.append(" next", 0.0, words);
    }
    job
}

fn footer(ui: &mut Ui, live: &mut Live, profile: &str) -> Effect {
    let mut effect = Effect::Nothing;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let last = live.card.is_last();
        let (label, ready) = if last {
            ("Send answers", live.card.can_send())
        } else {
            ("Next", live.card.answered())
        };
        if primary_button(ui, label, ready, Size::Compact).clicked() {
            if last {
                effect = live.card.send();
            } else {
                live.card.next();
            }
        }
        if live.card.step() > 0 && quiet_button(ui, "Back", true, Size::Compact).clicked() {
            live.card.back();
        }
        if quiet_button(ui, &format!("Skip, let {profile} pick"), true, Size::Compact).clicked() {
            effect = Effect::Skip;
        }
        let galley = ui.painter().layout_job(hint_job(live));
        ui.add_space((ui.available_width() - galley.size().x).max(0.0));
        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        ui.painter().galley(rect.min, galley, brand::FAINT);
    });
    effect
}

/// Draws the live card above the reply box: its bottom is the bottom of `dock`. Returns what the
/// user did. `grab` asks for the keyboard the first time the card shows.
pub(crate) fn live_card(
    ui: &mut Ui,
    dock: Rect,
    live: &mut Live,
    profile: &str,
    grab: bool,
) -> Effect {
    let width = dock.width() - 2.0 * INSET;
    let max_h = dock.height();
    let height = live.height.min(max_h);
    let region =
        Rect::from_min_size(pos2(dock.left() + INSET, dock.bottom() - height), vec2(width, height));
    #[cfg(test)]
    live.rects.borrow_mut().clear();
    let id = ui.id().with(("question-card", live.seq));
    let focus = ui.interact(region, id, Sense::focusable_noninteractive());
    if (grab && !live.focused_once) || std::mem::take(&mut live.reclaim) {
        focus.request_focus();
    }
    live.focused_once = true;
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(region).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing.y = 0.0;
    let frame = card_frame(brand::TEAL);
    let list_max = (max_h - CHROME_H).max(MIN_LIST_H);
    let mut effect = Effect::Nothing;
    let mut drawn = Drawn { clicked: None, field: None };
    let mut multi_click = None;
    let inner = frame.show(&mut child, |ui| {
        ui.set_width(width - 2.0 * f32::from(CARD_PAD_X));
        header_row(ui, live, profile);
        ui.add_space(8.0);
        let question = live.card.question().text.clone();
        ui.label(
            egui::RichText::new(question).font(crate::screen_kit::heading(15.5)).color(brand::TEXT),
        );
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt(("question-list", live.seq, live.card.step()))
            .max_height(list_max)
            .auto_shrink([false, true])
            .show(ui, |ui| match live.card.question().select {
                Select::Single => drawn = draw_single(ui, live, id),
                Select::Multi => multi_click = draw_multi(ui, live),
            });
        ui.add_space(4.0);
        effect = footer(ui, live, profile);
    });
    let rect = inner.response.rect;
    paint_glow(ui.painter(), rect, brand::TEAL);
    if (rect.height() - live.height).abs() > 0.5 {
        live.height = rect.height();
        ui.ctx().request_repaint();
    }
    if let Some(row) = drawn.clicked.or(multi_click) {
        let chosen = live.card.choose(row);
        if chosen == Effect::TypeOther {
            live.focus_other = true;
            ui.ctx().request_repaint();
        }
    }
    let pressed_in = ui.input(|i| i.pointer.any_pressed())
        && ui.input(|i| i.pointer.interact_pos()).is_some_and(|at| {
            rect.contains(at) && !drawn.field.is_some_and(|field| field.contains(at))
        });
    if pressed_in {
        live.reclaim = true;
        ui.ctx().request_repaint();
    }
    let in_field = ui.ctx().memory(egui::Memory::focused) == Some(id.with("other"));
    if focus.has_focus() {
        ui.ctx().memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: false,
                    tab: false,
                    escape: false,
                },
            );
        });
    }
    if focus.has_focus() || in_field {
        for key in read_keys(ui, focus.has_focus()) {
            let keyed = live.card.key(key);
            if keyed == Effect::TypeOther {
                live.focus_other = true;
            }
            if matches!(keyed, Effect::Send(_)) {
                effect = keyed;
            }
        }
        if in_field && ui.input(|i| i.key_pressed(Key::Enter)) && !ui.input(|i| i.modifiers.command)
        {
            if live.card.is_last() {
                if live.card.can_send() {
                    effect = live.card.send();
                }
            } else {
                live.card.next();
            }
        }
    }
    effect
}

/// The summary card: each question with its answer, or a waiting or skipped note.
pub(crate) fn summary(ui: &mut Ui, row: &QuestionRow, profile: &str) {
    let (pairs, note): (Vec<(String, Option<String>)>, String) = match &row.asked {
        Asked::Answered(pairs) => (
            pairs.iter().map(|(q, a)| (q.clone(), Some(a.clone()))).collect(),
            "Answered".to_owned(),
        ),
        Asked::Sent(pairs) => (
            pairs.iter().map(|(q, a)| (q.clone(), Some(a.clone()))).collect(),
            "Sending your answers...".to_owned(),
        ),
        Asked::Declined => (
            row.questions.iter().map(|q| (q.text.clone(), None)).collect(),
            format!("Skipped, {profile} picks"),
        ),
        Asked::Pending => (
            row.questions.iter().map(|q| (q.text.clone(), None)).collect(),
            "Waiting for your answer".to_owned(),
        ),
    };
    Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            let question_w = SUMMARY_QUESTION_W.min(ui.available_width() * 0.4);
            let answer_w = (ui.available_width() - question_w - 10.0).max(60.0);
            for (question, answer) in &pairs {
                let asked =
                    ui.painter().layout(question.clone(), body(13.0), brand::MUTED, question_w);
                let answered = ui.painter().layout(
                    answer.clone().unwrap_or_else(|| "Skipped".to_owned()),
                    body(14.0),
                    if answer.is_some() { brand::TEXT } else { brand::FAINT },
                    answer_w,
                );
                let height = asked.size().y.max(answered.size().y);
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
                ui.painter().galley(rect.min, asked, brand::MUTED);
                ui.painter().galley(rect.min + vec2(question_w + 10.0, 0.0), answered, brand::TEXT);
            }
            ui.label(egui::RichText::new(note).size(12.0).color(brand::FAINT));
        });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChipClick {
    Send(String),
    Reply,
}

fn chip(ui: &mut Ui, label: &str, ink: Color32, fill: Color32, line: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(13.0), ink);
    let size = galley.size() + vec2(28.0, 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hot = response.hovered();
    ui.painter().rect(
        rect,
        CornerRadius::same(255),
        if hot { fill.gamma_multiply(1.8) } else { fill },
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The chips under the agent's last message: the quick replies, then "Reply...".
pub(crate) fn quick_chips(ui: &mut Ui, quick: &[Quick]) -> Option<ChipClick> {
    let mut clicked = None;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        for (index, reply) in quick.iter().enumerate() {
            let first =
                index == 0 || reply.label.chars().next().is_some_and(|c| c.is_ascii_digit());
            let (ink, fill, line) = if first {
                (brand::TEAL_TEXT, teal_wash(), brand::TEAL.gamma_multiply(0.35))
            } else {
                (brand::TEXT, Color32::TRANSPARENT, brand::FIELD_LINE)
            };
            if chip(ui, &reply.label, ink, fill, line).clicked() {
                clicked = Some(ChipClick::Send(reply.send.clone()));
            }
        }
        if chip(ui, "Reply\u{2026}", brand::SOFT, Color32::TRANSPARENT, brand::FIELD_LINE).clicked()
        {
            clicked = Some(ChipClick::Reply);
        }
    });
    clicked
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionClick {
    Answer(Answer),
    SwitchToTerminal,
}

/// The permission card in the strip above the reply box: a "Needs your OK" chip with the tool, its
/// command, and the answers the CLI takes. `answers` is empty for a CLI whose keys are not verified.
pub(crate) fn permission_card(
    ui: &mut Ui,
    strip: Rect,
    tool: Option<(&str, &str)>,
    answers: &[Answer],
) -> Option<PermissionClick> {
    let card = strip.shrink2(vec2(INSET, 0.0));
    let mut clicked = None;
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing.y = 10.0;
    card_frame(brand::ORANGE).show(&mut child, |ui| {
        ui.set_width(card.width() - 2.0 * f32::from(CARD_PAD_X));
        let (head, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
        let chip = pill(ui.painter(), head.left_top(), "Needs your OK", brand::ORANGE, body(11.5));
        if let Some((name, _)) = tool {
            let galley = ui.painter().layout_no_wrap(name.to_owned(), mono(12.5), brand::SOFT);
            ui.painter().galley(
                pos2(chip.right() + 10.0, head.center().y - galley.size().y / 2.0),
                galley,
                brand::SOFT,
            );
        }
        if let Some((_, command)) = tool.filter(|(_, command)| !command.is_empty()) {
            let (rect, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), PERMISSION_COMMAND_H - 10.0),
                Sense::hover(),
            );
            ui.painter().rect(
                rect,
                CornerRadius::same(8),
                brand::SEGMENT,
                Stroke::new(1.0, brand::CARD_LINE),
                StrokeKind::Inside,
            );
            let galley =
                ellipsized(ui.painter(), command, mono(12.5), brand::TEXT, rect.width() - 24.0);
            ui.painter().galley(
                pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
                galley,
                brand::TEXT,
            );
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for answer in answers {
                let response = match answer {
                    Answer::Allow => primary_button(ui, "Allow once", true, Size::Compact),
                    Answer::Deny => crate::screen_kit::danger_button(ui, "Deny", Size::Compact),
                };
                if response.clicked() {
                    clicked = Some(PermissionClick::Answer(*answer));
                }
            }
            let label =
                if answers.is_empty() { "Answer it in the terminal" } else { "Switch to terminal" };
            if quiet_button(ui, label, true, Size::Compact).clicked() {
                clicked = Some(PermissionClick::SwitchToTerminal);
            }
        });
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};
    use tidewatch_core::question::Question;

    fn option(label: &str) -> QuestionOption {
        QuestionOption { label: label.into(), description: None }
    }

    fn live() -> Live {
        Live::new(
            7,
            Card::new(vec![
                Question {
                    text: "Where?".into(),
                    header: Some("Retry".into()),
                    select: Select::Single,
                    options: vec![option("Stage (Recommended)"), option("Run")],
                },
                Question {
                    text: "Which?".into(),
                    header: None,
                    select: Select::Multi,
                    options: vec![option("Unit"), option("Lints"), option("UI")],
                },
            ]),
        )
    }

    struct Screen {
        ctx: egui::Context,
        live: Live,
        composer: String,
        composer_focus: bool,
        grab: bool,
        effects: Vec<Effect>,
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    impl Screen {
        fn new() -> Self {
            Self {
                ctx: crate::fonts::test_context(),
                live: live(),
                composer: String::new(),
                composer_focus: false,
                grab: true,
                effects: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 800.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let Self { ctx, live, composer, composer_focus, grab, effects } = self;
            let _ = ctx.run_ui(raw, |ui| {
                let dock = Rect::from_min_max(pos2(0.0, 0.0), pos2(700.0, 740.0));
                let field = Rect::from_min_size(pos2(14.0, 750.0), vec2(600.0, 30.0));
                let response =
                    ui.put(field, TextEdit::singleline(composer).id(Id::new("composer")));
                if std::mem::take(composer_focus) {
                    response.request_focus();
                }
                let effect = live_card(ui, dock, live, "Builder", *grab);
                effects.push(effect);
            });
            self.grab = false;
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame(vec![]);
            }
        }

        fn click(&mut self, at: Pos2) {
            self.frame(vec![Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }]);
            }
        }

        fn rects(&self) -> Vec<Rect> {
            self.live.rects.borrow().clone()
        }

        fn sent(&self) -> bool {
            self.effects.iter().any(|effect| matches!(effect, Effect::Send(_)))
        }
    }

    #[test]
    fn a_click_picks_an_option_and_the_number_keys_work_once_the_card_has_the_keyboard() {
        let mut screen = Screen::new();
        screen.settle();
        let rects = screen.rects();
        assert_eq!(rects.len(), 2, "two options are drawn");
        screen.click(rects[1].center());
        assert!(screen.live.card.is_picked(1));
        screen.frame(vec![key(Key::Num1, Modifiers::NONE)]);
        assert!(screen.live.card.is_picked(0) && !screen.live.card.is_picked(1));
    }

    #[test]
    fn keys_go_to_the_composer_and_never_to_the_card_while_it_has_focus() {
        let mut screen = Screen::new();
        screen.grab = false;
        screen.composer_focus = true;
        screen.settle();
        screen.frame(vec![key(Key::Num1, Modifiers::NONE), Event::Text("1".into())]);
        screen.frame(vec![key(Key::Enter, Modifiers::COMMAND)]);
        assert_eq!(screen.composer, "1");
        assert_eq!(screen.live.card.step(), 0);
        assert!(!screen.live.card.is_picked(0));
        assert!(!screen.sent());
    }

    #[test]
    fn the_card_takes_the_keyboard_when_it_appears_and_enter_and_command_enter_work() {
        let mut screen = Screen::new();
        screen.settle();
        screen.frame(vec![key(Key::Num2, Modifiers::NONE)]);
        assert!(screen.live.card.is_picked(1));
        screen.frame(vec![key(Key::Enter, Modifiers::NONE)]);
        assert_eq!(screen.live.card.step(), 1);
        screen.frame(vec![key(Key::Space, Modifiers::NONE)]);
        screen.frame(vec![key(Key::Enter, Modifiers::COMMAND)]);
        assert!(screen.sent(), "send needs the multiple choice ticked");
        let Some(Effect::Send(given)) =
            screen.effects.iter().find(|e| matches!(e, Effect::Send(_)))
        else {
            panic!("no send")
        };
        assert_eq!(given[0].picked, [1]);
        assert_eq!(given[1].picked, [0]);
    }

    #[test]
    fn clicking_outside_the_card_gives_the_keyboard_back() {
        let mut screen = Screen::new();
        screen.settle();
        screen.click(pos2(300.0, 765.0));
        screen.frame(vec![key(Key::Num1, Modifiers::NONE)]);
        assert!(!screen.live.card.is_picked(0));
    }

    #[test]
    fn typing_in_something_else_picks_it_and_the_digits_land_in_the_field() {
        let mut screen = Screen::new();
        screen.settle();
        screen.frame(vec![key(Key::Num3, Modifiers::NONE)]);
        screen.settle();
        screen.frame(vec![Event::Text("45".into())]);
        assert!(screen.live.card.other_is_on());
        assert_eq!(screen.live.card.typed_mut(), "45");
        assert!(!screen.live.card.is_picked(0));
    }

    #[test]
    fn a_card_in_a_short_window_scrolls_its_options_instead_of_growing_past_the_dock() {
        let mut screen = Screen::new();
        screen.settle();
        assert!(screen.live.height <= 740.0);
    }
}
