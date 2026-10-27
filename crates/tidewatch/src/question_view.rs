//! Draws what the Reader shows for questions: the live card (radio rows, checkbox tiles, a
//! "Something else" field, Next, Back, Send answers, Skip), the compact summary after the answer,
//! the quick-reply chips under an agent's last message, and the permission card, which has the
//! same look. The state is `question_card`; this file reads keys and clicks and reports an effect.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, Frame, Galley, Id, Key, Layout, Margin, Modifiers, Painter, Pos2,
    Rect, Response, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::question::{Question, QuestionOption, Select};

use crate::brand;
use crate::question_auto;
use crate::question_card::{Card, CardKey, Effect, MAX_KEYED, Row};
use crate::quick_reply::Quick;
use crate::reader_keys::Answer;
use crate::reader_rows::{Asked, QuestionRow, ToolKind};
use crate::reader_turns;
use crate::reader_view::{INSET, ellipsized};
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::{Size, body, heading, mono, primary_button, quiet_button};
use crate::screen_kit::{TEXT_BODY, TEXT_TITLE};

const CARD_RADIUS: u8 = 14;
const CARD_PAD_X: i8 = 22;
const ROW_RADIUS: u8 = 10;
const ROW_GAP: f32 = 8.0;
const ROW_PAD_X: f32 = 14.0;
const ROW_PAD_Y: f32 = 12.0;
const MARK_W: f32 = 28.0;
const NUMBER_W: f32 = 36.0;
const HEADER_H: f32 = 16.0;
const BLOCK_GAP: f32 = 16.0;
const OTHER_H: f32 = 40.0;
const ESTIMATED_H: f32 = 300.0;
const MIN_LIST_H: f32 = 120.0;
const CHROME_H: f32 = 200.0;
pub(crate) const PERMISSION_H: f32 = 80.0 + Size::Compact.height();
pub(crate) const PERMISSION_COMMAND_H: f32 = 57.0;
const SECTION_GAP: f32 = 14.0;
const DOT: f32 = 7.0;
const ANSWER_ROW_PAD_X: i8 = 14;
const ANSWER_ROW_PAD_Y: i8 = 12;
const ROW_INK: Color32 = Color32::from_rgb(0x4a, 0x52, 0x63);

/// A live card: the state of the questions in row `seq`, and what the last frame measured.
pub(crate) struct Live {
    pub(crate) seq: u64,
    pub(crate) card: Card,
    /// How many waiting questions of the tab the agent's picks answer, set before each draw.
    pub(crate) picks: usize,
    /// What is left before Tidewatch answers for the user, in a tab that runs on its own.
    pub(crate) auto_left: Option<f64>,
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
            picks: 0,
            auto_left: None,
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

fn card_frame(top: i8, bottom: i8) -> Frame {
    Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::FIELD_LINE))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin { left: CARD_PAD_X, right: CARD_PAD_X, top, bottom })
}

fn paint_dot(painter: &Painter, at: Pos2, color: Color32) {
    painter.circle_filled(at, DOT / 2.0, color);
}

fn paint_key_badge(painter: &Painter, right_top: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), mono(12.0), brand::FAINT);
    let rect = Rect::from_min_size(
        pos2(right_top.x - galley.size().x - 12.0, right_top.y),
        galley.size() + vec2(12.0, 2.0),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(6.0, 1.0), galley, brand::FAINT);
}

fn paint_tick(painter: &Painter, center: Pos2, ink: Color32) {
    let stroke = Stroke::new(2.0, ink);
    painter.line_segment([center + vec2(-4.5, 0.2), center + vec2(-1.4, 3.4)], stroke);
    painter.line_segment([center + vec2(-1.4, 3.4), center + vec2(4.6, -3.6)], stroke);
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
            (Key::G, CardKey::Picks),
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
    tradeoff: Option<Arc<Galley>>,
    recommended: Option<Arc<Galley>>,
    height: f32,
}

fn layout_option(painter: &Painter, option: &QuestionOption, text_w: f32) -> OptionLayout {
    let recommended = option
        .recommended()
        .then(|| painter.layout_no_wrap("Recommended".to_owned(), body(12.0), brand::TEAL_TEXT));
    let label_room = text_w - recommended.as_ref().map_or(0.0, |tag| tag.size().x + 6.0);
    let label = painter.layout(
        option.shown().to_owned(),
        body(TEXT_BODY),
        brand::TEXT,
        label_room.max(40.0),
    );
    let description = option
        .description
        .as_ref()
        .map(|text| painter.layout(text.clone(), body(13.0), brand::MUTED, text_w));
    let tradeoff = option
        .tradeoff
        .as_ref()
        .map(|text| painter.layout(text.clone(), body(TEXT_SECONDARY), brand::MUTED, text_w));
    let height = ROW_PAD_Y * 2.0
        + label.size().y.max(20.0)
        + description.as_ref().map_or(0.0, |text| 3.0 + text.size().y)
        + tradeoff.as_ref().map_or(0.0, |text| 3.0 + text.size().y)
        + 2.0;
    OptionLayout { label, description, tradeoff, recommended, height }
}

fn paint_mark(painter: &Painter, at: Pos2, mark: Mark, selected: bool) {
    let line = if selected { brand::TEAL } else { ROW_INK };
    match mark {
        Mark::Radio => {
            painter.circle_stroke(at, 7.75, Stroke::new(1.5, line));
            if selected {
                painter.circle_filled(at, 4.0, brand::TEAL);
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
    let line = match (selected, cursor) {
        (true, _) => brand::TEAL,
        (false, true) => brand::SOFT.gamma_multiply(0.5),
        (false, false) => brand::ITEM_LINE,
    };
    painter.rect_stroke(
        rect,
        CornerRadius::same(ROW_RADIUS),
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
    let top = rect.top() + ROW_PAD_Y + 1.0;
    paint_mark(painter, pos2(rect.left() + ROW_PAD_X + 8.0, top + 10.0), mark, pick.selected);
    let text_x = rect.left() + ROW_PAD_X + MARK_W;
    painter.galley(pos2(text_x, top), layout.label.clone(), brand::TEXT);
    if let Some(tag) = &layout.recommended {
        let at = pos2(
            text_x + layout.label.size().x + 6.0,
            top + (layout.label.size().y - tag.size().y) / 2.0 + 1.0,
        );
        painter.galley(at, tag.clone(), brand::TEAL_TEXT);
    }
    let mut below = top + layout.label.size().y.max(20.0);
    if let Some(text) = &layout.description {
        painter.galley(pos2(text_x, below + 3.0), text.clone(), brand::MUTED);
        below += 3.0 + text.size().y;
    }
    if let Some(text) = &layout.tradeoff {
        painter.galley(pos2(text_x, below + 3.0), text.clone(), brand::MUTED);
    }
    if pick.number <= MAX_KEYED {
        paint_key_badge(painter, pos2(rect.right() - ROW_PAD_X, top), &pick.number.to_string());
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
                let (rect, response) = ui.allocate_exact_size(vec2(width, OTHER_H), Sense::click());
                let on = live.card.other_is_on();
                paint_other_box(ui.painter(), rect, on, cursor == index || response.hovered());
                let mid = rect.center().y;
                let label = ui.painter().layout_no_wrap(
                    "Something else".to_owned(),
                    body(14.0),
                    if on { brand::TEXT } else { brand::MUTED },
                );
                let text_x = rect.left() + ROW_PAD_X;
                ui.painter().galley(
                    pos2(text_x, mid - label.size().y / 2.0),
                    label.clone(),
                    brand::MUTED,
                );
                if index < MAX_KEYED {
                    paint_key_badge(
                        ui.painter(),
                        pos2(rect.right() - ROW_PAD_X, mid - 9.0),
                        &(index + 1).to_string(),
                    );
                }
                let field = Rect::from_min_max(
                    pos2(text_x + label.size().x + 12.0, rect.top() + 4.0),
                    pos2(rect.right() - ROW_PAD_X - NUMBER_W, rect.bottom() - 4.0),
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

fn rounded_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let mut points = Vec::new();
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -90.0_f32),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
        (pos2(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    for (center, start) in corners {
        for step in 0..=6 {
            let angle = (start + 15.0 * step as f32).to_radians();
            points.push(center + vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(points[0]);
    points
}

fn paint_other_box(painter: &Painter, rect: Rect, on: bool, hot: bool) {
    if on {
        paint_row_box(painter, rect, true, false);
        return;
    }
    let line = if hot { brand::SOFT.gamma_multiply(0.5) } else { brand::FIELD_LINE };
    painter.extend(Shape::dashed_line(
        &rounded_path(rect.shrink(0.5), f32::from(ROW_RADIUS)),
        Stroke::new(1.0, line),
        5.0,
        4.0,
    ));
}

fn other_field(ui: &mut Ui, live: &mut Live, id: Id, field: Rect) {
    let inner = field;
    let focus_id = id.with("other");
    let mut text = live.card.typed_mut().clone();
    let response = ui.put(
        inner,
        TextEdit::singleline(&mut text)
            .id(focus_id)
            .frame(Frame::NONE)
            .hint_text("Type your own answer")
            .font(body(14.0))
            .text_color(brand::TEXT)
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
}

fn draw_multi(ui: &mut Ui, live: &Live) -> Option<Row> {
    let width = ui.available_width();
    let card = &live.card;
    let any_description = card
        .question()
        .options
        .iter()
        .any(|option| option.description.is_some() || option.tradeoff.is_some());
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

/// The big line of the card and the smaller one under it: the agent's decision sentence over its
/// question, or the question alone.
fn headlines(question: &Question) -> (String, Option<String>) {
    match &question.decision {
        Some(decision) if *decision != question.text => {
            (decision.clone(), Some(question.text.clone()))
        }
        Some(decision) => (decision.clone(), None),
        None => (question.text.clone(), None),
    }
}

fn header_row(ui: &mut Ui, live: &Live, profile: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let painter = ui.painter();
    paint_dot(painter, pos2(rect.left() + DOT / 2.0, rect.center().y), brand::ORANGE);
    let text = match &live.card.question().header {
        Some(topic) => format!("{profile} asks \u{b7} {topic}"),
        None => format!("{profile} asks"),
    };
    let galley = painter.layout_no_wrap(text, body(12.0), brand::MUTED);
    painter.galley(
        pos2(rect.left() + DOT + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        brand::MUTED,
    );
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
    if live.picks > 0 {
        job.append("G", 0.0, keys.clone());
        job.append(" picks \u{b7} ", 0.0, words.clone());
    }
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
            (
                if live.card.count() > 1 { "Send answers" } else { "Send answer" },
                live.card.can_send(),
            )
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
        let hint = if live.picks > 0 {
            format!("Answer every waiting question with {profile}'s pick")
        } else {
            format!("Skip the questions and let {profile} decide")
        };
        if quiet_button(ui, &format!("Let {profile} pick"), true, Size::Compact)
            .on_hover_text(hint)
            .clicked()
        {
            effect = if live.picks > 0 { Effect::GoWithPicks } else { Effect::Skip };
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
    let frame = card_frame(20, 20);
    let list_max = (max_h - CHROME_H).max(MIN_LIST_H);
    let mut effect = Effect::Nothing;
    let mut drawn = Drawn { clicked: None, field: None };
    let mut multi_click = None;
    let inner = frame.show(&mut child, |ui| {
        ui.set_width(width - 2.0 * f32::from(CARD_PAD_X));
        header_row(ui, live, profile);
        ui.add_space(BLOCK_GAP);
        let (headline, detail) = headlines(live.card.question());
        ui.label(
            egui::RichText::new(headline).font(crate::screen_kit::heading(16.0)).color(brand::TEXT),
        );
        if let Some(detail) = detail {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(detail).font(body(13.0)).color(brand::MUTED));
        }
        ui.add_space(BLOCK_GAP);
        egui::ScrollArea::vertical()
            .id_salt(("question-list", live.seq, live.card.step()))
            .max_height(list_max)
            .auto_shrink([false, true])
            .show(ui, |ui| match live.card.question().select {
                Select::Single => drawn = draw_single(ui, live, id),
                Select::Multi => multi_click = draw_multi(ui, live),
            });
        ui.add_space(12.0);
        effect = footer(ui, live, profile);
        if let Some(left) = live.auto_left {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(question_auto::line(
                    left,
                    std::slice::from_ref(live.card.question()),
                ))
                .font(body(12.0))
                .color(brand::FAINT),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    });
    let rect = inner.response.rect;
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
            if matches!(keyed, Effect::Send(_))
                || matches!(keyed, Effect::GoWithPicks) && live.picks > 0
            {
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

fn paint_question_icon(painter: &Painter, center: Pos2, ink: Color32) {
    let scale = 14.0 / 24.0;
    let at = |x: f32, y: f32| center + vec2(x - 12.0, y - 12.0) * scale;
    let stroke = Stroke::new(1.3, ink);
    painter.circle_stroke(at(12.0, 12.0), 9.0 * scale, stroke);
    let arc: Vec<Pos2> = (0..=16)
        .map(|step| {
            let angle = (180.0_f32 + 15.4 * step as f32).to_radians();
            at(12.0 + 2.5 * angle.cos(), 9.5 + 2.5 * angle.sin())
        })
        .collect();
    painter.add(Shape::line(arc, stroke));
    painter.add(Shape::line(vec![at(13.0, 11.8), at(12.0, 13.3), at(12.0, 14.0)], stroke));
    painter.circle_filled(at(12.0, 17.2), 0.8, ink);
}

/// One answer in its own row: a teal check and the answer (or a faint note), with `label` at the
/// right.
fn answer_row(ui: &mut Ui, answer: Option<&str>, note: &str, label: &str) {
    let width = ui.available_width();
    let painter = ui.painter().clone();
    let label_galley = (!label.is_empty())
        .then(|| painter.layout_no_wrap(label.to_owned(), body(12.0), brand::FAINT));
    let lead = if answer.is_some() { 26.0 } else { 0.0 };
    let side = f32::from(ANSWER_ROW_PAD_X) * 2.0 + 2.0;
    let label_w = label_galley.as_ref().map_or(0.0, |galley| galley.size().x + 12.0);
    let (text, ink, font) = match answer {
        Some(answer) => (answer, brand::TEXT, heading(TEXT_TITLE)),
        None => (note, brand::FAINT, body(14.0)),
    };
    let galley =
        painter.layout(text.to_owned(), font, ink, (width - side - lead - label_w).max(60.0));
    let pad = f32::from(ANSWER_ROW_PAD_Y) * 2.0 + 2.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, galley.size().y.max(16.0) + pad), Sense::hover());
    painter.rect(
        rect,
        CornerRadius::same(ROW_RADIUS),
        brand::SUNKEN,
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
    let left = rect.left() + f32::from(ANSWER_ROW_PAD_X) + 1.0;
    if answer.is_some() {
        paint_tick(&painter, pos2(left + 8.0, rect.center().y), brand::TEAL);
    }
    painter.galley(pos2(left + lead, rect.center().y - galley.size().y / 2.0), galley, ink);
    if let Some(galley) = label_galley {
        painter.galley(
            pos2(
                rect.right() - f32::from(ANSWER_ROW_PAD_X) - 1.0 - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            brand::FAINT,
        );
    }
}

/// The summary card: each question with its answer in its own row, or a waiting or skipped note.
pub(crate) fn summary(
    ui: &mut Ui,
    row: &QuestionRow,
    profile: &str,
    time: Option<&str>,
    auto: bool,
) {
    let (pairs, note, label): (Vec<(String, Option<String>)>, String, &str) = match &row.asked {
        Asked::Answered(pairs) => (
            pairs.iter().map(|(q, a)| (q.clone(), Some(a.clone()))).collect(),
            String::new(),
            if auto { "Picked by Tidewatch" } else { "Your answer" },
        ),
        Asked::Sent(pairs) => (
            pairs.iter().map(|(q, a)| (q.clone(), Some(a.clone()))).collect(),
            String::new(),
            if auto { "Picked by Tidewatch" } else { "Sending..." },
        ),
        Asked::Declined => (
            row.questions.iter().map(|q| (q.text.clone(), None)).collect(),
            format!("Skipped, {profile} picks"),
            "",
        ),
        Asked::Pending => (
            row.questions.iter().map(|q| (q.text.clone(), None)).collect(),
            "Waiting for your answer".to_owned(),
            "",
        ),
    };
    card_frame(18, 20).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = SECTION_GAP;
        let (head, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
        let painter = ui.painter();
        paint_question_icon(painter, pos2(head.left() + 7.0, head.center().y), brand::MUTED);
        let galley = painter.layout_no_wrap("Question".to_owned(), body(12.0), brand::MUTED);
        painter.galley(
            pos2(head.left() + 22.0, head.center().y - galley.size().y / 2.0),
            galley,
            brand::MUTED,
        );
        if let Some(time) = time {
            let galley = painter.layout_no_wrap(time.to_owned(), body(12.0), brand::FAINT);
            painter.galley(
                pos2(head.right() - galley.size().x, head.center().y - galley.size().y / 2.0),
                galley,
                brand::FAINT,
            );
        }
        for (question, answer) in &pairs {
            ui.label(
                egui::RichText::new(question)
                    .font(body(TEXT_BODY))
                    .color(brand::SOFT)
                    .line_height(Some(23.0)),
            );
            answer_row(ui, answer.as_deref(), &note, label);
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChipClick {
    Send(String),
    Reply,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChipLook {
    Yes,
    Plain,
    Write,
}

fn chip(ui: &mut Ui, label: &str, look: ChipLook) -> Response {
    let (ink, line) = match look {
        ChipLook::Yes => (brand::TEAL_TEXT, brand::TEAL),
        ChipLook::Plain => (brand::STEP_TEXT, brand::FIELD_LINE),
        ChipLook::Write => (brand::MUTED, brand::FIELD_LINE),
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(Size::Compact.font()), ink);
    let size = vec2(galley.size().x + 2.0 * Size::Compact.pad(), Size::Compact.height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let radius = CornerRadius::same(Size::Compact.radius());
    if response.hovered() {
        ui.painter().rect_filled(rect, radius, line.gamma_multiply(0.12));
    }
    if look == ChipLook::Write {
        ui.painter().extend(Shape::dashed_line(
            &rounded_path(rect.shrink(0.5), f32::from(Size::Compact.radius())),
            Stroke::new(1.0, line),
            5.0,
            4.0,
        ));
    } else {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The chips under the agent's last message: the quick replies, then "Write a reply".
pub(crate) fn quick_chips(ui: &mut Ui, quick: &[Quick]) -> Option<ChipClick> {
    let mut clicked = None;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        for (index, reply) in quick.iter().enumerate() {
            let first =
                index == 0 || reply.label.chars().next().is_some_and(|c| c.is_ascii_digit());
            let look = if first { ChipLook::Yes } else { ChipLook::Plain };
            if chip(ui, &reply.label, look).clicked() {
                clicked = Some(ChipClick::Send(reply.send.clone()));
            }
        }
        if chip(ui, "Write a reply", ChipLook::Write).clicked() {
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

/// The permission card in the strip above the reply box: what needs an OK in plain words, the
/// command, and the answers the CLI takes. `answers` is empty for a CLI whose keys are not verified.
pub(crate) fn permission_card(
    ui: &mut Ui,
    strip: Rect,
    tool: Option<(&str, &str)>,
    time: Option<&str>,
    answers: &[Answer],
) -> Option<PermissionClick> {
    let card = strip.shrink2(vec2(INSET, 0.0));
    let mut clicked = None;
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing.y = SECTION_GAP;
    card_frame(18, 20).show(&mut child, |ui| {
        ui.set_width(card.width() - 2.0 * f32::from(CARD_PAD_X) - 2.0);
        let (head, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
        let painter = ui.painter();
        paint_dot(painter, pos2(head.left() + DOT / 2.0, head.center().y), brand::ORANGE);
        let label = painter.layout_no_wrap("Needs your OK".to_owned(), body(12.0), brand::ORANGE);
        let mut x = head.left() + DOT + 8.0;
        painter.galley(
            pos2(x, head.center().y - label.size().y / 2.0),
            label.clone(),
            brand::ORANGE,
        );
        x += label.size().x + 6.0;
        if let Some((name, _)) = tool {
            let words = format!("\u{b7} {}", reader_turns::permission_words(ToolKind::of(name)));
            let galley = painter.layout_no_wrap(words, body(12.0), brand::FAINT);
            painter.galley(pos2(x, head.center().y - galley.size().y / 2.0), galley, brand::FAINT);
        }
        if let Some(time) = time {
            let galley = painter.layout_no_wrap(time.to_owned(), body(12.0), brand::FAINT);
            painter.galley(
                pos2(head.right() - galley.size().x, head.center().y - galley.size().y / 2.0),
                galley,
                brand::FAINT,
            );
        }
        if let Some((name, command)) = tool.filter(|(_, command)| !command.is_empty()) {
            let (rect, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), PERMISSION_COMMAND_H - SECTION_GAP),
                Sense::hover(),
            );
            ui.painter().rect(
                rect,
                CornerRadius::same(ROW_RADIUS),
                brand::SUNKEN,
                Stroke::new(1.0, brand::ITEM_LINE),
                StrokeKind::Inside,
            );
            let mut x = rect.left() + 15.0;
            if ToolKind::of(name) == ToolKind::Command {
                let dollar = ui.painter().layout_no_wrap("$".to_owned(), mono(13.0), brand::FAINT);
                ui.painter().galley(
                    pos2(x, rect.center().y - dollar.size().y / 2.0),
                    dollar.clone(),
                    brand::FAINT,
                );
                x += dollar.size().x + 10.0;
            }
            let galley =
                ellipsized(ui.painter(), command, mono(13.0), brand::TEXT, rect.right() - 15.0 - x);
            ui.painter().galley(
                pos2(x, rect.center().y - galley.size().y / 2.0),
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
                if answers.is_empty() { "Answer it in the terminal" } else { "Open in terminal" };
            let width = crate::screen_kit::button_width(ui, label, Size::Compact);
            ui.add_space((ui.available_width() - width).max(0.0));
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

    fn option(label: &str) -> QuestionOption {
        QuestionOption::new(label)
    }

    fn live() -> Live {
        Live::new(
            7,
            Card::new(vec![
                Question {
                    text: "Where?".into(),
                    header: Some("Retry".into()),
                    decision: None,
                    select: Select::Single,
                    options: vec![option("Stage"), option("Run")],
                },
                Question {
                    text: "Which?".into(),
                    header: None,
                    decision: None,
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

    fn picks_screen() -> Screen {
        let mut screen = Screen::new();
        screen.live = Live::new(
            8,
            Card::new(vec![Question {
                text: "Which storage?".into(),
                header: None,
                decision: Some("Pick where the retry count is kept.".into()),
                select: Select::Single,
                options: vec![
                    QuestionOption {
                        tradeoff: Some("Simple, but every stage shares one count.".into()),
                        ..option("On the run")
                    },
                    QuestionOption {
                        tradeoff: Some("A bit more code, each stage counts alone.".into()),
                        top_pick: true,
                        ..option("On the stage")
                    },
                ],
            }]),
        );
        screen
    }

    #[test]
    fn the_pick_is_preselected_and_enter_with_the_command_key_sends_it_untouched() {
        let mut screen = picks_screen();
        screen.settle();
        assert!(screen.live.card.is_picked(1));
        assert_eq!(screen.live.card.cursor(), 1);
        screen.frame(vec![key(Key::Enter, Modifiers::COMMAND)]);
        let Some(Effect::Send(given)) =
            screen.effects.iter().find(|e| matches!(e, Effect::Send(_)))
        else {
            panic!("no send")
        };
        assert_eq!(given[0].picked, [1]);
    }

    #[test]
    fn g_goes_with_the_picks_only_when_the_tab_has_some_waiting() {
        let mut screen = picks_screen();
        screen.settle();
        screen.frame(vec![key(Key::G, Modifiers::NONE)]);
        assert!(!screen.effects.contains(&Effect::GoWithPicks));
        screen.live.picks = 2;
        screen.frame(vec![key(Key::G, Modifiers::NONE)]);
        assert!(screen.effects.contains(&Effect::GoWithPicks));
    }

    #[test]
    fn the_decision_and_the_tradeoffs_make_the_card_taller_than_the_bare_one() {
        let mut decided = picks_screen();
        decided.settle();
        let bare = {
            let mut screen = picks_screen();
            screen.live = Live::new(
                9,
                Card::new(vec![Question {
                    text: "Which storage?".into(),
                    header: None,
                    decision: None,
                    select: Select::Single,
                    options: vec![option("On the run"), option("On the stage")],
                }]),
            );
            screen.settle();
            screen.live.height
        };
        assert!(decided.live.height > bare, "{} vs {bare}", decided.live.height);
    }

    #[test]
    fn the_headline_is_the_decision_over_the_question_or_the_question_alone() {
        let mut q = Question {
            text: "Which storage?".into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: vec![option("a"), option("b")],
        };
        assert_eq!(headlines(&q), ("Which storage?".to_owned(), None));
        q.decision = Some("Pick where the count lives.".into());
        assert_eq!(
            headlines(&q),
            ("Pick where the count lives.".to_owned(), Some("Which storage?".to_owned()))
        );
        q.decision = Some("Which storage?".into());
        assert_eq!(headlines(&q), ("Which storage?".to_owned(), None));
    }

    #[test]
    fn a_card_in_a_short_window_scrolls_its_options_instead_of_growing_past_the_dock() {
        let mut screen = Screen::new();
        screen.settle();
        assert!(screen.live.height <= 740.0);
    }

    #[test]
    fn the_answered_card_stacks_the_question_over_the_answer_row() {
        let ctx = crate::fonts::test_context();
        let row = QuestionRow {
            questions: Vec::new(),
            asked: Asked::Answered(vec![("Which retry?".into(), "Backoff".into())]),
            origin: crate::reader_rows::Origin::Claude,
        };
        let mut texts = Vec::new();
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 400.0))),
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| summary(ui, &row, "Builder", Some("12:41"), false));
            texts = out
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                    _ => None,
                })
                .collect();
        }
        let at = |want: &str| texts.iter().find(|(text, _)| text == want).map(|(_, pos)| *pos);
        let (question, answer) = (at("Which retry?").unwrap(), at("Backoff").unwrap());
        assert!(answer.y > question.y + 10.0, "the answer sits below the question");
        assert!(at("Question").is_some() && at("12:41").is_some() && at("Your answer").is_some());
    }

    fn drawn_texts(row: &QuestionRow, auto: bool) -> Vec<String> {
        let ctx = crate::fonts::test_context();
        let mut texts = Vec::new();
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 400.0))),
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| summary(ui, row, "Builder", None, auto));
            texts = out
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        texts
    }

    #[test]
    fn an_answer_tidewatch_picked_says_so() {
        let row = QuestionRow {
            questions: Vec::new(),
            asked: Asked::Answered(vec![("Which retry?".into(), "Backoff".into())]),
            origin: crate::reader_rows::Origin::Claude,
        };
        assert!(drawn_texts(&row, true).contains(&"Picked by Tidewatch".to_owned()));
        assert!(!drawn_texts(&row, true).contains(&"Your answer".to_owned()));
        assert!(drawn_texts(&row, false).contains(&"Your answer".to_owned()));
    }
}
