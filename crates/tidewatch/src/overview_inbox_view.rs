//! Draws the Needs you inbox of the Overview: a panel with one 60 point row per ask (a colored left
//! edge, a tile for its kind, the ask over who waits and for how long) and its answer at the right.
//! A lone question shows its options as buttons that answer through the question card's function;
//! every other ask opens its tab.

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use tidewatch_core::text::clip;

use crate::agent_avatar::{self, Face};
use crate::brand;
use crate::choice::CHAT_LABEL;
use crate::overview::{Board, Row, fmt_elapsed};
use crate::overview_inbox::{Answer, Entry, Go, Kind, summary};
use crate::overview_view::{
    HEAD_GAP, Head, Out, SECTION_HEAD_H, SECTION_PAD, in_rect, section_frame, section_head,
};
use crate::question_compact::Click;
use crate::reader_view::ellipsized;
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TITLE, button_width, heading, primary_button,
    quiet_button,
};

pub(crate) const ROW_H: f32 = 60.0;
const ROW_GAP: f32 = 6.0;
const EDGE_W: f32 = 4.0;
const TILE: f32 = 30.0;
const TILE_X: f32 = 16.0;
const TEXT_X: f32 = TILE_X + TILE + 12.0;
const SMALL_FACE: f32 = 16.0;
const FACE_STEP: f32 = 11.0;
const ANSWER_GAP: f32 = 8.0;
const LABEL_CHARS: usize = 24;
const MORE: &str = "More";
const CHAT: &str = "Chat";
const MIN_ANSWER_ROOM: f32 = 200.0;

pub(crate) fn height(count: usize) -> f32 {
    let rows = count as f32 * ROW_H + count.saturating_sub(1) as f32 * ROW_GAP;
    SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP + rows
}

fn ink_of(kind: Kind) -> Color32 {
    match kind {
        Kind::Failure => brand::failed(),
        Kind::DoThis => brand::needs(),
        Kind::Decision => brand::waiting(),
    }
}

pub(crate) fn show(ui: &mut Ui, width: f32, entries: &[Entry], board: &Board, out: &mut Out) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height(entries.len())), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, true);
    let title = format!("Needs you \u{b7} {}", entries.len());
    let title_w =
        painter.layout_no_wrap(title.clone(), heading(TEXT_TITLE), brand::needs()).size().x;
    let said = summary(entries);
    let said_w = painter
        .layout_no_wrap(said.clone(), FontId::proportional(TEXT_SECONDARY), brand::muted())
        .size()
        .x;
    let fits = title_w + said_w + SECTION_PAD * 2.0 + 40.0 <= width;
    section_head(
        &painter,
        rect,
        &Head {
            title: &title,
            caption: "",
            count: if fits { &said } else { "" },
            dot: Some(brand::needs()),
            ink: brand::needs(),
        },
    );
    let clip_rect = ui.clip_rect();
    let mut y = rect.top() + SECTION_PAD + SECTION_HEAD_H + HEAD_GAP;
    for (index, entry) in entries.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(rect.left() + SECTION_PAD, y),
            vec2(width - SECTION_PAD * 2.0, ROW_H),
        );
        y += ROW_H + ROW_GAP;
        if row.intersects(clip_rect) {
            entry_row(ui, &painter, row, (index, entry), board, out);
        }
    }
}

fn entry_row(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    (index, entry): (usize, &Entry),
    board: &Board,
    out: &mut Out,
) {
    let ink = ink_of(entry.kind);
    let go = first_stop(entry);
    let resp = ui.interact(rect, ui.id().with(("inbox-row", index)), Sense::click());
    let line = if resp.hovered() { brand::field_line() } else { brand::item_line() };
    painter.rect(
        rect,
        CornerRadius::same(10),
        brand::item(),
        Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    let edge = Rect::from_min_size(rect.min, vec2(EDGE_W, rect.height()));
    painter.rect_filled(edge, CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }, ink);
    tile(
        painter,
        Rect::from_center_size(
            pos2(rect.left() + TILE_X + TILE / 2.0, rect.center().y),
            vec2(TILE, TILE),
        ),
        entry.kind,
    );
    let right = answers(ui, rect, entry, out);
    let text_x = rect.left() + TEXT_X;
    let room = (right - text_x - 12.0).max(40.0);
    let ask = ellipsized(painter, &entry.text, heading(TEXT_BODY), brand::text(), room);
    painter.galley(pos2(text_x, rect.top() + 18.0 - ask.size().y / 2.0), ask, brand::text());
    meta(ui, painter, (text_x, rect.top() + 40.0), room, entry, board);
    let tip = match entry.who.as_slice() {
        [only] => only.name.clone(),
        many => many.iter().map(|who| who.name.as_str()).collect::<Vec<_>>().join(", "),
    };
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
        apply(go, out);
    }
}

/// Where a click on the row goes: the first agent's tab, or the rail row of the ask.
fn first_stop(entry: &Entry) -> Go {
    match &entry.answer {
        Answer::Open { go, .. } => *go,
        Answer::Inline { tab_id, .. } => Go::Tab(*tab_id),
    }
}

fn apply(go: Go, out: &mut Out) {
    match go {
        Go::Tab(id) => out.jump = out.jump.or(Some(id)),
        Go::Scheduled(row) => out.agenda = out.agenda.or(Some(row)),
    }
}

fn tile(painter: &Painter, rect: Rect, kind: Kind) {
    let ink = ink_of(kind);
    painter.rect_filled(rect, CornerRadius::same(8), ink.gamma_multiply(0.18));
    match kind {
        Kind::DoThis => {
            let c = rect.center();
            painter.add(Shape::line(
                vec![c + vec2(-5.5, 0.5), c + vec2(-1.5, 4.5), c + vec2(5.5, -4.0)],
                Stroke::new(2.0, ink),
            ));
        }
        Kind::Failure | Kind::Decision => {
            let glyph = if kind == Kind::Failure { "!" } else { "?" };
            painter.text(rect.center(), Align2::CENTER_CENTER, glyph, heading(TEXT_TITLE), ink);
        }
    }
}

fn meta(ui: &Ui, painter: &Painter, (x, mid): (f32, f32), room: f32, entry: &Entry, board: &Board) {
    let mut at = x;
    let shown: Vec<&crate::overview_inbox::Who> = entry.who.iter().take(3).collect();
    for who in &shown {
        let face = who
            .tab_id
            .and_then(|id| board_row(board, id))
            .map(|row| Face::of(&row.profile, &row.agent_id, row.avatar.clone()));
        if let Some(face) = face {
            let tile = Rect::from_center_size(
                pos2(at + SMALL_FACE / 2.0, mid),
                vec2(SMALL_FACE, SMALL_FACE),
            );
            agent_avatar::paint_face(ui.ctx(), painter, tile, &face);
            at += FACE_STEP;
        }
    }
    if at > x {
        at += SMALL_FACE - FACE_STEP + 6.0;
    }
    let who = match entry.who.as_slice() {
        [only] => only.name.clone(),
        many => format!("{} agents", many.len()),
    };
    let waited = entry.waited.map(|secs| format!(" \u{b7} waiting {}", fmt_elapsed(secs)));
    let text = format!("{who}{}", waited.unwrap_or_default());
    let galley = ellipsized(
        painter,
        &text,
        FontId::proportional(TEXT_SECONDARY),
        brand::muted(),
        (x + room - at).max(20.0),
    );
    painter.galley(pos2(at, mid - galley.size().y / 2.0), galley, brand::muted());
}

fn board_row(board: &Board, id: u64) -> Option<&Row> {
    board
        .needs
        .iter()
        .chain(&board.working)
        .chain(&board.waiting)
        .chain(&board.idle)
        .find(|row| row.tab_id == id)
}

/// The buttons at the right end of a row; returns the x where they start.
fn answers(ui: &mut Ui, rect: Rect, entry: &Entry, out: &mut Out) -> f32 {
    let h = Size::Compact.height();
    let right = rect.right() - 14.0;
    let top = rect.center().y - h / 2.0;
    match &entry.answer {
        Answer::Open { label, go } => {
            let w = button_width(ui, label, Size::Compact);
            let at = Rect::from_min_size(pos2(right - w, top), vec2(w, h));
            if in_rect(ui, at, |ui| primary_button(ui, label, true, Size::Compact).clicked()) {
                apply(*go, out);
            }
            at.left()
        }
        Answer::Inline { tab_id, ask } => {
            let buttons = inline_buttons(ui, &ask.options, rect.width() * 0.55);
            let total: f32 = buttons.iter().map(|b| b.width).sum::<f32>()
                + ANSWER_GAP * (buttons.len() - 1) as f32;
            let start = right - total;
            let mut x = start;
            for button in buttons {
                let at = Rect::from_min_size(pos2(x, top), vec2(button.width, h));
                let response = in_rect(ui, at, |ui| {
                    if button.main {
                        primary_button(ui, &button.label, true, Size::Compact)
                    } else {
                        quiet_button(ui, &button.label, true, Size::Compact)
                    }
                });
                let clicked = response.clicked();
                if let Some(tip) = &button.tip {
                    response.on_hover_text(tip);
                }
                if clicked && out.answer.is_none() {
                    out.answer = Some((*tab_id, button.click));
                }
                x += button.width + ANSWER_GAP;
            }
            start
        }
    }
}

struct Button {
    click: Click,
    label: String,
    width: f32,
    main: bool,
    tip: Option<String>,
}

/// The options of a lone question that fit `room`, the first (or the agent's pick) as the main one,
/// then `More` when some do not fit and `Chat`.
fn inline_buttons(ui: &Ui, options: &[tidewatch_core::progress::Choice], room: f32) -> Vec<Button> {
    let choices = crate::ask_model::choices(options);
    let pick = choices.iter().position(|choice| choice.recommended).unwrap_or(0);
    let chat = button_width(ui, CHAT, Size::Compact);
    let more = button_width(ui, MORE, Size::Compact);
    let room = room.max(MIN_ANSWER_ROOM);
    let mut buttons = Vec::new();
    let mut used = chat + ANSWER_GAP;
    let mut cut = false;
    for (index, choice) in choices.iter().enumerate() {
        let label = clip(&choice.text, LABEL_CHARS);
        let width = button_width(ui, &label, Size::Compact);
        let last = index + 1 == choices.len();
        let reserve = if last { 0.0 } else { more + ANSWER_GAP };
        if !buttons.is_empty() && used + width + ANSWER_GAP + reserve > room {
            cut = true;
            break;
        }
        used += width + ANSWER_GAP;
        let tip = (label != choice.text).then(|| choice.text.clone());
        buttons.push(Button {
            click: Click::Option(index),
            label,
            width,
            main: index == pick,
            tip,
        });
    }
    if cut {
        buttons.push(Button {
            click: Click::Open,
            label: MORE.to_owned(),
            width: more,
            main: false,
            tip: Some("Answer it in the tab".to_owned()),
        });
    }
    buttons.push(Button {
        click: Click::Chat,
        label: CHAT.to_owned(),
        width: chat,
        main: false,
        tip: Some(CHAT_LABEL.to_owned()),
    });
    buttons
}
