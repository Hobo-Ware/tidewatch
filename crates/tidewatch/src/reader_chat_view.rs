//! Draws the Reader's chat: the user's messages as right-aligned bubbles and each agent turn as one
//! left bubble with the tab's face beside it, holding the last thing the agent wrote, a chip over it
//! for the progress notes written before (`12 updates`, a muted list on a click), one row of counted
//! chips under it and, on a click, one mono list of every call of the turn. A turn still running
//! shows a live pill under its latest text, or instead of a bubble until its text arrives. The
//! grouping and the words are `reader_turns`.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Label, Layout, Margin, OutputCommand, Painter,
    Pos2, Rect, Response, RichText, Sense, Shadow, Shape, Stroke, StrokeKind, TextStyle, Ui, pos2,
    vec2,
};
use jiff::tz::TimeZone;
use tidewatch_core::question;
use tidewatch_core::user_markup::{self, TaskResult};

use crate::agent_avatar::{Face, paint_face};
use crate::brand;
use crate::code_block::{self, Look};
use crate::job_model::clock_label;
use crate::link_text::{Laid, Text};
use crate::links;
use crate::pictures;
use crate::progress_ring::paint_spinner;
use crate::reader_markdown::{self, Piece};
use crate::reader_rows::{Kind, Row, ToolKind};
use crate::reader_turns::{self, Body, Call, Group, Section};
use crate::reader_view::{ellipsized, output_preview, split_label};
use crate::screen_kit::{body, mono};

pub(crate) const BODY_PX: f32 = 16.0;
const CODE_EM: f32 = 0.88;
const HEADING_PX: f32 = 22.0;
const USER_SHARE: f32 = 0.62;
const AGENT_SHARE: f32 = 0.82;
const AVATAR: f32 = 32.0;
const AVATAR_GAP: f32 = 12.0;
const META_H: f32 = 16.0;
const QUEUED_OPACITY: f32 = 0.55;
const QUEUED_SPIN: f32 = 11.0;
const QUEUED_GAP: f32 = 6.0;
const META_GAP: f32 = 6.0;
const OUTER: f32 = 10.0;
pub(crate) const CHIP_H: f32 = 26.0;
pub(crate) const CHIP_GAP: f32 = 8.0;
const TOGGLE_H: f32 = 28.0;
const LIVE_H: f32 = 32.0;
const BUBBLE_PAD: Margin = Margin { left: 16, right: 16, top: 12, bottom: 12 };
const USER_PAD: Margin = Margin { left: 16, right: 16, top: 10, bottom: 10 };
const USER_RADIUS: CornerRadius = CornerRadius { nw: 16, ne: 16, se: 4, sw: 16 };
const AGENT_RADIUS: CornerRadius = CornerRadius { nw: 4, ne: 16, se: 16, sw: 16 };
const USER_FILL: Color32 = brand::TEAL_NAV;
pub(crate) const USER_LINE: Color32 = Color32::from_rgb(0x1f, 0x3a, 0x3a);
const USER_CODE_FILL: Color32 = Color32::from_black_alpha(90);
const LIST_SLACK: f32 = 32.0;
const TEXT_SLACK: f32 = 8.0;
const NOTE_GAP: f32 = 6.0;
const ANSWER_QUESTION_PX: f32 = 14.0;
const ANSWER_HEAD_PX: f32 = 12.0;
const ANSWER_PX: f32 = 15.0;
const ANSWER_LEAD: f32 = 22.0;
const ANSWER_PAIR_GAP: f32 = 12.0;
const ANSWER_LINE_GAP: f32 = 6.0;
const JUMP_SHADOW: Shadow =
    Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(140) };
const LIVE_FILL: Color32 = Color32::from_rgb(0x12, 0x20, 0x1f);

/// What a drawn turn needs besides its rows.
pub(crate) struct Scene<'a> {
    pub(crate) root: Option<&'a Path>,
    pub(crate) profile: &'a str,
    pub(crate) face: Option<&'a Face>,
    pub(crate) zone: &'a TimeZone,
    pub(crate) now_ms: i64,
    /// The agent is mid-turn, so the newest open turn is the one it is still on.
    pub(crate) running: bool,
    /// A tool call waits for the user's permission.
    pub(crate) waiting: bool,
}

/// The "Jump to latest" pill, drawn at the top left of `slot`: an opaque fill, a 1 px line and a
/// small shadow, so the text scrolling under it does not bleed through.
pub(crate) fn jump_pill(ui: &mut Ui, slot: Rect, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(13.0), brand::TEAL_TEXT);
    let rect = Rect::from_min_size(slot.min, galley.size() + vec2(24.0, 12.0));
    let response = ui.interact(rect, ui.id().with(("jump-pill", label)), Sense::click());
    let fill = if response.hovered() { brand::ITEM } else { brand::CARD };
    let line = if response.hovered() { brand::FIELD_LINE } else { brand::CARD_LINE };
    let painter = ui.painter();
    let radius = CornerRadius::same(8);
    painter.add(JUMP_SHADOW.as_shape(rect, radius));
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, brand::TEAL_TEXT);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The width a user bubble may take of a column `width` wide.
pub(crate) fn user_width(width: f32) -> f32 {
    (width * USER_SHARE).round()
}

/// The width of an agent turn's column (bubble and chips) beside the face.
pub(crate) fn agent_width(width: f32) -> f32 {
    (width * AGENT_SHARE).round() - AVATAR - AVATAR_GAP
}

/// Runs `add` with the chat's text sizes: 16 px body, inline code and code blocks at 0.88 em.
pub(crate) fn chat_text<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let styles = &mut ui.style_mut().text_styles;
        styles.insert(TextStyle::Body, FontId::proportional(BODY_PX));
        styles.insert(TextStyle::Monospace, FontId::monospace(BODY_PX * CODE_EM));
        styles.insert(TextStyle::Heading, FontId::proportional(HEADING_PX));
        styles.insert(TextStyle::Small, FontId::proportional(13.0));
        ui.visuals_mut().code_bg_color = brand::ITEM;
        add(ui)
    })
    .inner
}

/// An agent's message as markdown with every web link clickable (a bare URL, `localhost:5174`, a
/// URL in inline code) and no other kind of link able to open: a click on a link of another
/// scheme, whichever way it was written, is dropped before it leaves the frame.
pub(crate) fn agent_markdown(ui: &mut Ui, text: &str) {
    ui.scope(|ui| {
        let ink = ui.visuals().text_color();
        let visuals = ui.visuals_mut();
        visuals.override_text_color = None;
        visuals.widgets.noninteractive.fg_stroke.color = ink;
        visuals.hyperlink_color = brand::TEAL_TEXT;
        ui.style_mut().url_in_tooltip = true;
        reader_markdown::show(ui, &links::markdown(text), true, Look::AGENT);
    });
    ui.ctx().output_mut(|out| {
        out.commands.retain(
            |command| !matches!(command, OutputCommand::OpenUrl(open) if !links::is_web_url(&open.url)),
        );
    });
}

fn meta(ui: &mut Ui, text: &str, align: Align) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, META_H), Sense::hover());
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text.to_owned(), body(12.0), brand::FAINT);
    let x = match align {
        Align::Max => rect.right() - galley.size().x,
        _ => rect.left(),
    };
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, brand::FAINT);
}

fn stamped(who: &str, at_ms: Option<i64>, zone: &TimeZone) -> String {
    match at_ms.and_then(|at| clock_label(at, zone)) {
        Some(at) => format!("{who} \u{b7} {at}"),
        None => who.to_owned(),
    }
}

/// A user message: a bubble at the right, `You · 16:02` over it. The text inside is markdown,
/// left-aligned, in a bubble as wide as its text up to the user share of the column.
pub(crate) fn user(ui: &mut Ui, row: &Row, at_ms: Option<i64>, zone: &TimeZone, width: f32) {
    user_bubble(ui, row, Meta::Stamp(at_ms, zone), width);
}

/// A message the user sent while the agent was busy: the same bubble, dimmed, with a small spinner
/// and `Queued` over it, until the transcript has the message.
pub(crate) fn queued(ui: &mut Ui, text: &str, key: u64, time: f64, width: f32) {
    let row = Row { seq: key, kind: Kind::User, text: text.to_owned() };
    user_bubble(ui, &row, Meta::Queued(time), width);
}

#[derive(Clone, Copy)]
enum Meta<'a> {
    Stamp(Option<i64>, &'a TimeZone),
    Queued(f64),
}

fn queued_meta(ui: &mut Ui, time: f64) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, META_H), Sense::hover());
    let painter = ui.painter();
    let galley = painter.layout_no_wrap("Queued".to_owned(), body(12.0), brand::FAINT);
    let x = rect.right() - galley.size().x;
    let y = rect.center().y;
    painter.galley(pos2(x, y - galley.size().y / 2.0), galley, brand::FAINT);
    paint_spinner(
        painter,
        pos2(x - QUEUED_GAP - QUEUED_SPIN / 2.0, y),
        QUEUED_SPIN,
        time,
        brand::TEAL,
    );
}

fn user_bubble(ui: &mut Ui, row: &Row, meta_line: Meta<'_>, width: f32) {
    ui.add_space(OUTER);
    let max_w = user_width(width);
    let (said, pictures) = pictures::split_trailing(&row.text);
    let markdown = user_markup::chat_markdown(&said);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.allocate_ui_with_layout(vec2(max_w, 0.0), Layout::top_down(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let dim = match meta_line {
                Meta::Stamp(at_ms, zone) => {
                    meta(ui, &stamped("You", at_ms, zone), Align::Max);
                    1.0
                }
                Meta::Queued(time) => {
                    queued_meta(ui, time);
                    QUEUED_OPACITY
                }
            };
            ui.set_opacity(dim);
            Frame::new()
                .fill(USER_FILL)
                .stroke(Stroke::new(1.0, USER_LINE))
                .corner_radius(USER_RADIUS)
                .inner_margin(USER_PAD)
                .show(ui, |ui| {
                    let room = max_w - f32::from(USER_PAD.left + USER_PAD.right) - 2.0;
                    if let Some(pairs) = question::parse_answers(&row.text) {
                        answers_block(ui, &pairs, room, row.seq);
                        return;
                    }
                    let natural = ui
                        .painter()
                        .layout(markdown.clone(), FontId::proportional(BODY_PX), brand::TEXT, room)
                        .size()
                        .x;
                    let code = reader_markdown::pieces(&markdown, false)
                        .iter()
                        .filter_map(|piece| match piece {
                            Piece::Code { text, .. } => {
                                Some(code_block::natural_width(ui.painter(), text))
                            }
                            _ => None,
                        })
                        .fold(0.0, f32::max);
                    let text_w =
                        user_text_width(natural, user_markup::has_indented_block(&said), room)
                            .max(code.min(room));
                    let chips_w =
                        if pictures.is_empty() { 0.0 } else { pictures::PLAIN_W.min(room) };
                    let shown_w =
                        if said.trim().is_empty() { chips_w } else { text_w.max(chips_w) };
                    ui.set_width(shown_w.min(room));
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        if !said.trim().is_empty() {
                            chat_text(ui, |ui| {
                                ui.visuals_mut().code_bg_color = USER_CODE_FILL;
                                ui.push_id(row.seq, |ui| {
                                    reader_markdown::show(ui, &markdown, false, Look::USER);
                                });
                            });
                        }
                        sent_pictures(ui, &pictures);
                    });
                });
        });
    });
    ui.add_space(OUTER);
}

fn sent_pictures(ui: &mut Ui, pictures: &[std::path::PathBuf]) {
    if pictures.is_empty() {
        return;
    }
    ui.add_space(8.0);
    let width = ui.available_width();
    let rows = pictures::chip_rows(pictures.len(), width, pictures::PLAIN_W);
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, pictures::chips_height(rows)), Sense::hover());
    pictures::paint_block(ui, rect.min, width, pictures, false);
}

fn answers_block(ui: &mut Ui, pairs: &[(String, String)], room: f32, seq: u64) {
    let laid = |text: &str, font: FontId, ink: Color32, width: f32| -> Laid {
        let mut line = Text::new();
        line.run(text, &egui::text::TextFormat::simple(font, ink));
        line.wrapped(width, usize::MAX).lay_out(ui.painter())
    };
    let heading_laid = laid("Your answers", body(ANSWER_HEAD_PX), brand::MUTED, room);
    let lines: Vec<(Laid, Laid)> = pairs
        .iter()
        .map(|(question, answer)| {
            (
                laid(question, body(ANSWER_QUESTION_PX), brand::SOFT, room),
                laid(
                    answer,
                    crate::screen_kit::heading(ANSWER_PX),
                    brand::TEXT,
                    room - ANSWER_LEAD,
                ),
            )
        })
        .collect();
    let natural = lines
        .iter()
        .flat_map(|(question, answer)| [question.size().x, answer.size().x + ANSWER_LEAD])
        .fold(heading_laid.size().x, f32::max);
    ui.set_width((natural.ceil() + TEXT_SLACK).min(room).max(0.0));
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let id = ui.id().with(("answers", seq));
        let put = |ui: &mut Ui, laid: &Laid, name: (usize, u8), ink: Color32| -> Rect {
            let (rect, _) = ui.allocate_exact_size(laid.size(), Sense::hover());
            laid.paint(ui, id.with(name), rect.min, ink);
            rect
        };
        put(ui, &heading_laid, (0, 0), brand::MUTED);
        for (index, (question, answer)) in lines.iter().enumerate() {
            ui.add_space(ANSWER_PAIR_GAP);
            if index > 0 {
                let (line, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, USER_LINE);
                ui.add_space(ANSWER_PAIR_GAP);
            }
            put(ui, question, (index, 1), brand::SOFT);
            ui.add_space(ANSWER_LINE_GAP);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (tick, _) = ui.allocate_exact_size(vec2(ANSWER_LEAD, 18.0), Sense::hover());
                let c = tick.left_center() + vec2(6.0, 0.0);
                let stroke = Stroke::new(2.0, brand::TEAL);
                ui.painter().line_segment([c + vec2(-3.5, 0.2), c + vec2(-1.0, 3.0)], stroke);
                ui.painter().line_segment([c + vec2(-1.0, 3.0), c + vec2(3.8, -3.0)], stroke);
                put(ui, answer, (index, 2), brand::TEXT);
            });
        }
    });
}

/// The width of a user bubble's text: what the text measured, with room for the bullets and
/// numbers of a list and for the bold and code the measure did not see, never past `room`.
pub(crate) fn user_text_width(natural: f32, indented: bool, room: f32) -> f32 {
    let slack = if indented { LIST_SLACK } else { TEXT_SLACK };
    (natural.ceil() + slack).min(room).max(0.0)
}

/// A slash command the user ran, as a small teal-outline chip with when.
pub(crate) fn slash_command(ui: &mut Ui, text: &str, time: Option<&str>) {
    ui.add_space(NOTE_GAP);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let galley = ui.painter().layout_no_wrap(text.to_owned(), mono(13.0), brand::TEAL_TEXT);
        let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(18.0, 8.0), Sense::hover());
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, USER_LINE),
            StrokeKind::Inside,
        );
        ui.painter().galley(rect.min + vec2(9.0, 4.0), galley, brand::TEAL_TEXT);
        let said = match time {
            Some(time) => format!("You ran a command \u{b7} {time}"),
            None => "You ran a command".to_owned(),
        };
        ui.label(RichText::new(said).size(12.0).color(brand::FAINT));
    });
    ui.add_space(NOTE_GAP);
}

/// What a local command printed, in a sunken panel; a background task's note is its own slim card.
pub(crate) fn command_output(ui: &mut Ui, text: &str) {
    ui.add_space(NOTE_GAP);
    if let Some((name, result)) = user_markup::background_card(text) {
        background_row(ui, &name, &result);
    } else {
        Frame::new()
            .fill(brand::SUNKEN)
            .stroke(Stroke::new(1.0, brand::ITEM_LINE))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(Label::new(RichText::new(text).font(body(13.0)).color(brand::STEP_TEXT)));
            });
    }
    ui.add_space(NOTE_GAP);
}

fn background_row(ui: &mut Ui, name: &str, result: &TaskResult) {
    Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            let painter = ui.painter();
            let mid = rect.center().y;
            paint_icon(painter, pos2(rect.left() + 8.0, mid), ToolKind::Command, brand::MUTED);
            let tag = painter.layout_no_wrap("background".to_owned(), body(12.0), brand::FAINT);
            let tag_x = rect.right() - tag.size().x;
            painter.galley(pos2(tag_x, mid - tag.size().y / 2.0), tag, brand::FAINT);
            let (ink, text, mark) = match result {
                TaskResult::Done(text) => (brand::GREEN, text.as_str(), Some(true)),
                TaskResult::Failed(text) => (brand::RED, text.as_str(), Some(false)),
                TaskResult::Stopped => (brand::MUTED, "Stopped", None),
            };
            let said = painter.layout_no_wrap(text.to_owned(), body(13.0), ink);
            let lead = if mark.is_some() { 19.0 } else { 0.0 };
            let said_w = said.size().x + lead;
            let said_x = tag_x - 16.0 - said_w;
            let room = said_x - 12.0 - (rect.left() + 28.0);
            let title = ellipsized(painter, name, body(14.0), brand::TEXT, room.max(40.0));
            let title_w = title.size().x;
            painter.galley(
                pos2(rect.left() + 28.0, mid - title.size().y / 2.0),
                title,
                brand::TEXT,
            );
            let at = (rect.left() + 28.0 + title_w + 12.0).min(said_x);
            let stroke = Stroke::new(1.6, ink);
            match mark {
                Some(true) => {
                    let c = pos2(at + 6.5, mid);
                    painter.line_segment([c + vec2(-3.5, 0.2), c + vec2(-1.0, 3.0)], stroke);
                    painter.line_segment([c + vec2(-1.0, 3.0), c + vec2(3.8, -3.0)], stroke);
                }
                Some(false) => {
                    let c = pos2(at + 6.5, mid);
                    painter.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], stroke);
                    painter.line_segment([c + vec2(3.0, -3.0), c + vec2(-3.0, 3.0)], stroke);
                }
                None => {}
            }
            painter.galley(pos2(at + lead, mid - said.size().y / 2.0), said, ink);
        });
}

/// Draws turn `group` of `list`: header, the updates chip, bubble, chips and the open lists. Flips
/// `open` for the turn when a chip is clicked.
pub(crate) fn turn(
    ui: &mut Ui,
    scene: &Scene<'_>,
    list: &[&Row],
    group: &Group,
    open: &mut HashSet<u64>,
    width: f32,
) {
    let live = scene.running && !group.closed;
    let has_text = !group.texts.is_empty();
    let listing = open.contains(&group.key) && !group.calls.is_empty();
    let notes_label = group.notes_label();
    let notes_listing = open.contains(&group.notes_key()) && notes_label.is_some();
    let column = agent_width(width);
    let mut toggled = false;
    let mut notes_toggled = false;
    ui.add_space(OUTER);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let headed = has_text || !live;
        ui.vertical(|ui| {
            if headed {
                ui.add_space(META_H + META_GAP);
            }
            face(ui, scene);
        });
        ui.add_space(AVATAR_GAP);
        ui.vertical(|ui| {
            ui.set_width(column);
            if headed {
                let span = reader_turns::span_label(group.started_ms, group.ended_ms, scene.zone);
                let head = match span {
                    Some(span) => format!("{} \u{b7} {span}", scene.profile),
                    None => scene.profile.to_owned(),
                };
                meta(ui, &head, Align::Min);
                ui.add_space(META_GAP);
            }
            if let Some(label) = &notes_label {
                notes_toggled |= notes_chip(ui, label, notes_listing);
                if notes_listing {
                    ui.add_space(CHIP_GAP);
                    notes_list(ui, list, group, column);
                }
                ui.add_space(CHIP_GAP);
            }
            if has_text {
                bubble(ui, list, group, column);
            }
            if live {
                if has_text {
                    ui.add_space(CHIP_GAP);
                }
                toggled |= live_row(ui, scene, list, group, column);
            } else if !group.chips.is_empty() {
                ui.add_space(CHIP_GAP);
                toggled |= summary_row(ui, group, listing, column);
            }
            if listing {
                ui.add_space(CHIP_GAP);
                call_list(ui, scene.root, list, group, column);
            }
        });
    });
    ui.add_space(OUTER);
    if toggled {
        reader_turns::toggle(open, group.key);
    }
    if notes_toggled {
        reader_turns::toggle(open, group.notes_key());
    }
}

/// The agent has been asked and has written nothing yet: the face and a live pill.
pub(crate) fn pending(ui: &mut Ui, scene: &Scene<'_>, width: f32) {
    ui.add_space(OUTER);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        face(ui, scene);
        ui.add_space(AVATAR_GAP);
        ui.vertical(|ui| {
            ui.set_width(agent_width(width));
            pill(ui, "Working", None, false, agent_width(width));
        });
    });
    ui.add_space(OUTER);
    ui.ctx().request_repaint_after(Duration::from_secs(1));
}

fn face(ui: &mut Ui, scene: &Scene<'_>) {
    let (rect, _) = ui.allocate_exact_size(vec2(AVATAR, AVATAR), Sense::hover());
    let fallback;
    let face = if let Some(face) = scene.face {
        face
    } else {
        fallback = Face::of(scene.profile, "", None);
        &fallback
    };
    paint_face(ui.ctx(), ui.painter(), rect, face);
}

fn bubble(ui: &mut Ui, list: &[&Row], group: &Group, column: f32) {
    let Some(index) = group.answer() else { return };
    Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(AGENT_RADIUS)
        .inner_margin(BUBBLE_PAD)
        .show(ui, |ui| {
            ui.set_width(column - f32::from(BUBBLE_PAD.left + BUBBLE_PAD.right) - 2.0);
            chat_text(ui, |ui| {
                let row = list[index];
                ui.push_id(row.seq, |ui| {
                    agent_markdown(ui, &row.text);
                });
            });
        });
}

fn summary_row(ui: &mut Ui, group: &Group, open: bool, column: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(column, TOGGLE_H), Sense::hover());
    let painter = ui.painter().clone();
    let label = painter.layout_no_wrap(group.steps_label(), body(12.0), brand::SOFT);
    let chip =
        Rect::from_min_size(rect.min, vec2(11.0 + 12.0 + 6.0 + label.size().x + 11.0, TOGGLE_H));
    let response = ui.interact(chip, ui.id().with(("steps", group.key)), Sense::click());
    let hot = open || response.hovered();
    let (line, ink) =
        if hot { (brand::TEAL, brand::TEAL_TEXT) } else { (brand::FIELD_LINE, brand::SOFT) };
    painter.rect_stroke(chip, CornerRadius::same(255), Stroke::new(1.0, line), StrokeKind::Inside);
    let at = pos2(chip.left() + 11.0 + 6.0, chip.center().y);
    let tip = if open { -3.0 } else { 3.0 };
    let stroke = Stroke::new(1.4, ink);
    painter.line_segment([at + vec2(-4.0, -tip / 2.0), at + vec2(0.0, tip / 2.0)], stroke);
    painter.line_segment([at + vec2(0.0, tip / 2.0), at + vec2(4.0, -tip / 2.0)], stroke);
    painter.galley(
        pos2(chip.left() + 11.0 + 12.0 + 6.0, chip.center().y - label.size().y / 2.0),
        label,
        ink,
    );
    let mut right = rect.right();
    if let Some(took) = group.elapsed_ms() {
        let galley =
            painter.layout_no_wrap(reader_turns::duration_label(took), body(12.0), brand::FAINT);
        right -= galley.size().x;
        painter.galley(pos2(right, rect.center().y - galley.size().y / 2.0), galley, brand::FAINT);
        right -= 10.0;
    }
    let mut x = chip.right() + 10.0;
    let failed = group.failed();
    let tail = (failed > 0).then(|| {
        painter.layout_no_wrap(format!("\u{b7} {failed} failed"), body(12.0), brand::ORANGE)
    });
    let room = right - x - tail.as_ref().map_or(0.0, |galley| galley.size().x + 6.0);
    let galley = ellipsized(&painter, &group.summary(), body(12.0), brand::MUTED, room.max(0.0));
    let width = galley.size().x;
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, brand::MUTED);
    x += width + 6.0;
    if let Some(galley) = tail {
        painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, brand::ORANGE);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The progress notes the agent wrote before its answer, folded: a click opens them.
fn notes_chip(ui: &mut Ui, label: &str, open: bool) -> bool {
    ui.horizontal(|ui| pill_chip(ui, label, (false, open), paint_notes_icon).clicked()).inner
}

fn pill_chip(
    ui: &mut Ui,
    label: &str,
    (failed, open): (bool, bool),
    icon: impl FnOnce(&Painter, Pos2, Color32),
) -> Response {
    let ink = if failed { brand::ORANGE } else { brand::SOFT };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(13.0), ink);
    let icon_w = 14.0;
    let width = 12.0 + icon_w + 6.0 + galley.size().x + 12.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, CHIP_H), Sense::click());
    let line = match (failed, response.hovered() || open) {
        (true, true) => brand::ORANGE,
        (true, false) => brand::ORANGE.gamma_multiply(0.55),
        (false, true) => brand::FIELD_LINE,
        (false, false) => brand::CARD_LINE,
    };
    let painter = ui.painter();
    let radius = CornerRadius::same(255);
    if failed {
        painter.rect_filled(rect, radius, brand::ORANGE.gamma_multiply(0.08));
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
    let icon_ink = if failed { brand::ORANGE } else { brand::MUTED };
    icon(painter, pos2(rect.left() + 12.0 + icon_w / 2.0, rect.center().y), icon_ink);
    let text_at = pos2(rect.left() + 12.0 + icon_w + 6.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn icon_point(center: Pos2, x: f32, y: f32) -> Pos2 {
    let s = 14.0 / 24.0;
    center + vec2((x - 12.0) * s, (y - 12.0) * s)
}

fn paint_notes_icon(painter: &Painter, center: Pos2, ink: Color32) {
    let stroke = Stroke::new(1.4, ink);
    for (y, end) in [(7.0, 20.0), (12.0, 20.0), (17.0, 13.0)] {
        painter.line_segment([icon_point(center, 4.0, y), icon_point(center, end, y)], stroke);
    }
}

fn paint_icon(painter: &Painter, center: Pos2, kind: ToolKind, ink: Color32) {
    let stroke = Stroke::new(1.4, ink);
    let at = |x, y| icon_point(center, x, y);
    let open = |points: &[(f32, f32)]| {
        painter.add(Shape::line(points.iter().map(|(x, y)| at(*x, *y)).collect(), stroke));
    };
    let closed = |points: &[(f32, f32)]| {
        painter.add(Shape::closed_line(points.iter().map(|(x, y)| at(*x, *y)).collect(), stroke));
    };
    match kind {
        ToolKind::Command => {
            open(&[(4.0, 7.0), (10.0, 12.0), (4.0, 17.0)]);
            open(&[(12.0, 19.0), (20.0, 19.0)]);
        }
        ToolKind::Edit => {
            closed(&[(4.0, 20.0), (8.0, 20.0), (19.0, 9.0), (15.0, 5.0), (4.0, 16.0)]);
        }
        ToolKind::Read => {
            closed(&[
                (2.0, 12.0),
                (6.0, 7.5),
                (12.0, 5.0),
                (18.0, 7.5),
                (22.0, 12.0),
                (18.0, 16.5),
                (12.0, 19.0),
                (6.0, 16.5),
            ]);
            painter.circle_stroke(at(12.0, 12.0), 3.0 * 14.0 / 24.0, stroke);
        }
        ToolKind::Search => {
            painter.circle_stroke(at(10.0, 10.0), 6.0 * 14.0 / 24.0, stroke);
            open(&[(15.0, 15.0), (21.0, 21.0)]);
        }
        ToolKind::Agent => {
            closed(&[(5.0, 9.0), (19.0, 9.0), (19.0, 19.0), (5.0, 19.0)]);
            open(&[(12.0, 4.0), (12.0, 9.0)]);
            painter.circle_filled(at(9.5, 14.0), 1.2, ink);
            painter.circle_filled(at(14.5, 14.0), 1.2, ink);
        }
        ToolKind::Other => {
            for x in [6.0, 12.0, 18.0] {
                painter.circle_filled(at(x, 12.0), 1.4, ink);
            }
        }
    }
}

/// The live row of a turn: a teal pill with the call it is on and how far it has got. A click on
/// the count opens the list.
fn live_row(ui: &mut Ui, scene: &Scene<'_>, list: &[&Row], group: &Group, column: f32) -> bool {
    let tool = group.current.and_then(|index| match &list[index].kind {
        Kind::Tool(tool) => Some(tool),
        _ => None,
    });
    let waiting = scene.waiting && tool.is_some_and(|tool| !tool.done);
    let (verb, command) = match tool {
        Some(tool) => {
            let line = split_label(&tool.label, scene.root);
            let verb =
                if waiting { "Waiting for permission" } else { reader_turns::live_verb(tool.kind) };
            (verb, Some(format!("{}{}", line.dir, line.leaf)))
        }
        None => ("Working", None),
    };
    let elapsed = group.started_ms.map(|from| (scene.now_ms - from).max(0));
    let so_far = match (group.so_far(), elapsed) {
        (Some(count), Some(took)) => {
            Some(format!("{} \u{b7} {count}", reader_turns::duration_label(took)))
        }
        (Some(count), None) => Some(count),
        (None, Some(took)) => Some(reader_turns::duration_label(took)),
        (None, None) => None,
    };
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
        let count_w = so_far.as_ref().map_or(0.0, |text| {
            ui.painter().layout_no_wrap(text.clone(), body(13.0), brand::FAINT).size().x + 10.0
        });
        pill(ui, verb, command.as_deref(), waiting, (column - count_w).max(120.0));
        if let Some(text) = so_far {
            let galley = ui.painter().layout_no_wrap(text, body(13.0), brand::FAINT);
            let (rect, response) =
                ui.allocate_exact_size(vec2(galley.size().x, LIVE_H), Sense::click());
            let ink = if response.hovered() { brand::SOFT } else { brand::FAINT };
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                galley,
                ink,
            );
            clicked = response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        }
    });
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    clicked
}

fn pill(ui: &mut Ui, verb: &str, command: Option<&str>, waiting: bool, max_w: f32) {
    let (ink, fill, line) = if waiting {
        (brand::ORANGE, brand::ORANGE.gamma_multiply(0.08), brand::ORANGE.gamma_multiply(0.4))
    } else {
        (brand::TEAL_TEXT, LIVE_FILL, USER_LINE)
    };
    let verb_galley = ui.painter().layout_no_wrap(verb.to_owned(), body(13.0), ink);
    let lead = 12.0 + 8.0 + 8.0;
    let room = (max_w - lead - verb_galley.size().x - 8.0 - 12.0).max(0.0);
    let command_galley = command
        .filter(|text| !text.is_empty() && room > 24.0)
        .map(|text| ellipsized(ui.painter(), text, mono(12.0), brand::SOFT, room));
    let command_w = command_galley.as_ref().map_or(0.0, |galley| galley.size().x + 8.0);
    let width = lead + verb_galley.size().x + command_w + 12.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, LIVE_H), Sense::hover());
    let body_rect = Rect::from_center_size(rect.center(), vec2(width, CHIP_H));
    let painter = ui.painter();
    let radius = CornerRadius::same(255);
    painter.rect_filled(body_rect, radius, fill);
    painter.rect_stroke(body_rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
    painter.circle_filled(pos2(body_rect.left() + 12.0 + 4.0, body_rect.center().y), 4.0, ink);
    let mut x = body_rect.left() + lead;
    painter.galley(
        pos2(x, body_rect.center().y - verb_galley.size().y / 2.0),
        verb_galley.clone(),
        ink,
    );
    x += verb_galley.size().x + 8.0;
    if let Some(galley) = command_galley {
        painter.galley(pos2(x, body_rect.center().y - galley.size().y / 2.0), galley, brand::SOFT);
    }
}

fn notes_list(ui: &mut Ui, list: &[&Row], group: &Group, column: f32) {
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::SIDEBAR_LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(column - 26.0);
            ui.spacing_mut().item_spacing.y = NOTE_GAP;
            for index in group.notes() {
                ui.add(Label::new(
                    RichText::new(list[*index].text.trim()).font(body(14.0)).color(brand::MUTED),
                ));
            }
        });
}

fn call_list(ui: &mut Ui, root: Option<&Path>, list: &[&Row], group: &Group, column: f32) {
    let calls: Vec<Call<'_>> = group
        .calls
        .iter()
        .filter_map(|index| match &list[*index].kind {
            Kind::Tool(tool) => {
                let line = split_label(&tool.label, root);
                Some(Call { tool, name: line.name, argument: format!("{}{}", line.dir, line.leaf) })
            }
            _ => None,
        })
        .collect();
    let sections = reader_turns::sections(&calls);
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            ui.set_width(column - 2.0);
            for (position, section) in sections.iter().enumerate() {
                let shown = Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    section_head(ui, section);
                    ui.horizontal_top(|ui| {
                        ui.add_space(21.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing = vec2(18.0, 5.0);
                            section_body(ui, section, &calls);
                        });
                    });
                });
                if position + 1 < sections.len() {
                    let rect = shown.response.rect;
                    ui.painter().hline(
                        rect.x_range(),
                        rect.bottom(),
                        Stroke::new(1.0, brand::SIDEBAR_LINE),
                    );
                }
            }
        });
}

fn section_head(ui: &mut Ui, section: &Section) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    paint_icon(ui.painter(), pos2(rect.left() + 6.5, rect.center().y), section.kind, brand::MUTED);
    let galley = ui.painter().layout_no_wrap(section.head.clone(), body(12.0), brand::MUTED);
    ui.painter().galley(
        pos2(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0),
        galley,
        brand::MUTED,
    );
}

fn mono_job(parts: &[(&str, Color32)]) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (text, ink) in parts {
        job.append(text, 0.0, TextFormat::simple(mono(12.0), *ink));
    }
    job
}

fn section_body(ui: &mut Ui, section: &Section, calls: &[Call<'_>]) {
    match &section.body {
        Body::Files(files) => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(18.0, 4.0);
                for file in files {
                    let at = file.rfind('/').map_or(0, |at| at + 1);
                    let (dir, leaf) = file.split_at(at);
                    ui.add(Label::new(mono_job(&[(dir, brand::FAINT), (leaf, brand::STEP_TEXT)])));
                }
            });
        }
        Body::Lines(indices) => {
            for index in indices {
                let call = &calls[*index];
                let ink = if call.tool.error { brand::RED } else { brand::STEP_TEXT };
                let job = if call.tool.kind == ToolKind::Command {
                    mono_job(&[("$ ", brand::FAINT), (&call.argument, ink)])
                } else {
                    let text = reader_turns::list_line(call.tool, call.name, &call.argument);
                    mono_job(&[(&text, ink)])
                };
                ui.add(Label::new(job).truncate()).on_hover_ui(|ui| {
                    ui.set_max_width(480.0);
                    ui.label(RichText::new(&call.tool.label).font(mono(12.0)).color(brand::TEXT));
                    if !call.tool.output.is_empty() {
                        let ink = if call.tool.error { brand::RED } else { brand::MUTED };
                        ui.label(
                            RichText::new(output_preview(&call.tool.output))
                                .font(mono(11.5))
                                .color(ink),
                        );
                    }
                });
            }
        }
        Body::Servers(servers) => {
            for (server, tools) in servers {
                let mut job = LayoutJob::default();
                if tools.is_empty() {
                    job.append(server, 0.0, TextFormat::simple(body(12.0), brand::STEP_TEXT));
                } else {
                    job.append(server, 0.0, TextFormat::simple(body(12.0), brand::MUTED));
                    job.append(
                        &format!(" {}", tools.join(", ")),
                        0.0,
                        TextFormat::simple(body(12.0), brand::STEP_TEXT),
                    );
                }
                ui.add(Label::new(job));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    const TEXT: &str = "Preview at localhost:5174 now, [docs](https://x.dev/docs) and \
        [bad](javascript:alert(1)) or <file:///etc/passwd> or [ref][r].\n\n[r]: javascript:alert(2)";

    fn frame(ctx: &egui::Context, events: Vec<Event>) -> Vec<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            ui.set_width(600.0);
            ui.visuals_mut().override_text_color = Some(Color32::WHITE);
            chat_text(ui, |ui| agent_markdown(ui, TEXT));
        });
        out.platform_output
            .commands
            .into_iter()
            .filter_map(|c| match c {
                OutputCommand::OpenUrl(open) => Some(open.url),
                _ => None,
            })
            .collect()
    }

    fn click(ctx: &egui::Context, at: Pos2) -> Vec<String> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut opened = Vec::new();
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            opened.extend(frame(ctx, events));
        }
        opened
    }

    fn every_click(y: f32) -> Vec<(f32, Vec<String>)> {
        let ctx = egui::Context::default();
        let _ = frame(&ctx, vec![]);
        let _ = frame(&ctx, vec![]);
        (0..640).step_by(4).map(|x| (x as f32, click(&ctx, pos2(x as f32, y)))).collect()
    }

    fn painted_text(text: &str) -> Vec<String> {
        let ctx = crate::fonts::test_context();
        let row = Row { seq: 1, kind: Kind::User, text: text.to_owned() };
        let zone = TimeZone::UTC;
        let mut texts = Vec::new();
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                user(ui, &row, None, &zone, 800.0);
            });
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
    fn the_bubble_draws_answers_as_a_heading_and_pairs_without_their_q_and_a_markers() {
        let questions =
            [("Where should the retry count live?", "On the stage"), ("Which?", "Unit")];
        let message = format!(
            "{}\n\n{}",
            "My answers to your earlier questions:",
            questions
                .iter()
                .map(|(q, a)| format!("Q: {q}\nA: {a}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        );
        let drawn = painted_text(&message);
        for want in ["Your answers", questions[0].0, questions[0].1, questions[1].0, questions[1].1]
        {
            assert!(drawn.iter().any(|text| text == want), "{want}: {drawn:?}");
        }
        assert!(drawn.iter().all(|text| !text.contains("Q:") && !text.contains("A:")), "{drawn:?}");
    }

    #[test]
    fn a_message_that_only_mentions_q_in_prose_is_still_drawn_as_text() {
        let drawn = painted_text("Is it fine? Q: where\nA: here, I think");
        assert!(!drawn.iter().any(|text| text == "Your answers"), "{drawn:?}");
        assert!(drawn.iter().any(|text| text.contains("Q: where")), "{drawn:?}");
    }

    #[test]
    fn a_click_on_a_bare_address_opens_it_and_a_click_on_plain_text_does_not() {
        let hits = every_click(14.0);
        let opened: Vec<_> = hits.iter().filter(|(_, urls)| !urls.is_empty()).collect();
        let urls: Vec<_> = opened.iter().flat_map(|(_, urls)| urls.iter().cloned()).collect();
        assert!(urls.iter().any(|u| u == "http://localhost:5174"), "{urls:?}");
        assert!(urls.iter().all(|u| links::is_web_url(u)), "{urls:?}");
        assert_eq!(hits[1].1.len(), 0, "a click on the first word");
        assert_eq!(hits[0].1.len(), 0);
    }

    #[test]
    fn only_web_links_ever_open_whatever_is_clicked() {
        let mut all = Vec::new();
        for y in [14.0, 36.0, 58.0, 80.0] {
            all.extend(every_click(y).into_iter().flat_map(|(_, urls)| urls));
        }
        assert!(all.iter().any(|url| url == "https://x.dev/docs"));
        assert!(all.iter().all(|url| links::is_web_url(url)), "{all:?}");
    }

    #[test]
    fn links_are_teal_and_other_text_keeps_the_ink_the_theme_gives_it() {
        let ctx = egui::Context::default();
        let mut inks = Vec::new();
        for _ in 0..2 {
            let out = ctx.run_ui(RawInput::default(), |ui| {
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                chat_text(ui, |ui| agent_markdown(ui, "plain localhost:5174 end"));
            });
            inks = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        t.fallback_color,
                        t.override_text_color,
                        t.galley.clone(),
                    )),
                    _ => None,
                })
                .collect();
        }
        let ink = |word: &str| {
            let galley = &inks.iter().find(|(text, ..)| text.contains("plain")).expect("text").3;
            galley
                .job
                .sections
                .iter()
                .find(|section| {
                    let range =
                        usize::from(section.byte_range.start)..usize::from(section.byte_range.end);
                    galley.job.text[range].contains(word)
                })
                .map(|section| section.format.color)
        };
        assert_eq!(ink("localhost"), Some(brand::TEAL_TEXT));
        assert_eq!(ink("plain"), Some(Color32::WHITE));
    }

    #[test]
    fn a_sent_message_with_pictures_shows_chips_and_never_the_raw_paths() {
        let message = "look at these\n/nope/shot-1793011211356.png\n/nope/shot-1793011218705.png";
        let texts = painted_text(message);
        assert!(texts.iter().any(|text| text == "look at these"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "Screenshot 1"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "Screenshot 2"), "{texts:?}");
        assert!(!texts.iter().any(|text| text.contains("/nope/")), "{texts:?}");
    }

    #[test]
    fn a_message_of_only_pictures_has_no_empty_text_and_a_path_in_a_sentence_stays_text() {
        let only = painted_text("/nope/shot-1.png");
        assert!(only.iter().any(|text| text == "Screenshot 1"), "{only:?}");
        let inline = painted_text("open /nope/shot-1.png please");
        assert!(inline.iter().any(|text| text.contains("/nope/shot-1.png")), "{inline:?}");
        assert!(!inline.iter().any(|text| text == "Screenshot 1"), "{inline:?}");
    }

    #[test]
    fn a_user_bubble_is_as_wide_as_its_text_and_never_past_the_room() {
        let cases = [
            (120.2, false, 500.0, 129.0),
            (120.2, true, 500.0, 153.0),
            (480.0, false, 400.0, 400.0),
            (0.0, false, 400.0, 8.0),
            (10.0, true, 20.0, 20.0),
        ];
        for (natural, indented, room, want) in cases {
            assert_eq!(user_text_width(natural, indented, room), want, "{natural} {indented}");
        }
    }

    #[test]
    fn the_jump_pill_is_an_opaque_card_with_a_line_and_a_shadow_under_it() {
        let ctx = crate::fonts::test_context();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))),
            ..egui::RawInput::default()
        };
        let slot = Rect::from_min_size(pos2(50.0, 50.0), vec2(140.0, 30.0));
        let out = ctx.run_ui(raw, |ui| {
            jump_pill(ui, slot, "Jump to latest");
        });
        let rects: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Rect(rect) => Some(rect.clone()),
                _ => None,
            })
            .collect();
        let card = rects.iter().position(|rect| rect.fill == brand::CARD).unwrap();
        assert_eq!(rects[card].fill.a(), 255);
        assert!(
            rects
                .iter()
                .any(|rect| rect.stroke.width == 1.0 && rect.stroke.color == brand::CARD_LINE)
        );
        assert!(rects[..card].iter().any(|rect| rect.fill.a() < 255), "the shadow is under it");
    }
}
