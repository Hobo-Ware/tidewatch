//! Draws the Reader's chat: the user's messages as right-aligned bubbles and each agent turn as one
//! left bubble with the tab's face beside it, one row of counted chips under it and, on a click,
//! one mono list of every call of the turn. A turn still running shows a live pill instead of a
//! bubble until its text arrives. The grouping and the words are `reader_turns`.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Label, Layout, Margin, OutputCommand, Painter,
    Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, pos2, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use jiff::tz::TimeZone;
use tidewatch_core::user_markup;

use crate::agent_avatar::{Face, paint_face};
use crate::brand;
use crate::job_model::clock_label;
use crate::links;
use crate::reader_rows::{Kind, Row, ToolKind};
use crate::reader_turns::{self, Chip, Group};
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
const META_GAP: f32 = 6.0;
const OUTER: f32 = 10.0;
const CHIP_H: f32 = 26.0;
const CHIP_GAP: f32 = 8.0;
const LIVE_H: f32 = 32.0;
const LIST_LINE_H: f32 = 18.0;
const LIST_GAP: f32 = 4.0;
const BUBBLE_PAD: Margin = Margin { left: 16, right: 16, top: 12, bottom: 12 };
const USER_PAD: Margin = Margin { left: 16, right: 16, top: 10, bottom: 10 };
const USER_RADIUS: CornerRadius = CornerRadius { nw: 16, ne: 16, se: 4, sw: 16 };
const AGENT_RADIUS: CornerRadius = CornerRadius { nw: 4, ne: 16, se: 16, sw: 16 };
const USER_FILL: Color32 = brand::TEAL_NAV;
const USER_LINE: Color32 = Color32::from_rgb(0x1f, 0x3a, 0x3a);
const USER_CODE_FILL: Color32 = Color32::from_black_alpha(90);
const LIST_SLACK: f32 = 32.0;
const TEXT_SLACK: f32 = 8.0;
const NOTE_GAP: f32 = 6.0;
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
pub(crate) fn agent_markdown(ui: &mut Ui, cache: &mut CommonMarkCache, text: &str) {
    ui.scope(|ui| {
        let ink = ui.visuals().text_color();
        let visuals = ui.visuals_mut();
        visuals.override_text_color = None;
        visuals.widgets.noninteractive.fg_stroke.color = ink;
        visuals.hyperlink_color = brand::TEAL_TEXT;
        ui.style_mut().url_in_tooltip = true;
        CommonMarkViewer::new().show(ui, cache, &links::markdown(text));
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
pub(crate) fn user(
    ui: &mut Ui,
    row: &Row,
    at_ms: Option<i64>,
    zone: &TimeZone,
    (width, cache): (f32, &mut CommonMarkCache),
) {
    ui.add_space(OUTER);
    let max_w = user_width(width);
    let markdown = user_markup::chat_markdown(&row.text);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.allocate_ui_with_layout(vec2(max_w, 0.0), Layout::top_down(Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            meta(ui, &stamped("You", at_ms, zone), Align::Max);
            Frame::new()
                .fill(USER_FILL)
                .stroke(Stroke::new(1.0, USER_LINE))
                .corner_radius(USER_RADIUS)
                .inner_margin(USER_PAD)
                .show(ui, |ui| {
                    let room = max_w - f32::from(USER_PAD.left + USER_PAD.right) - 2.0;
                    let natural = ui
                        .painter()
                        .layout(markdown.clone(), FontId::proportional(BODY_PX), brand::TEXT, room)
                        .size()
                        .x;
                    ui.set_width(user_text_width(
                        natural,
                        user_markup::has_indented_block(&row.text),
                        room,
                    ));
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        chat_text(ui, |ui| {
                            ui.visuals_mut().code_bg_color = USER_CODE_FILL;
                            ui.push_id(row.seq, |ui| {
                                CommonMarkViewer::new().show(ui, cache, &markdown);
                            });
                        });
                    });
                });
        });
    });
    ui.add_space(OUTER);
}

/// The width of a user bubble's text: what the text measured, with room for the bullets and
/// numbers of a list and for the bold and code the measure did not see, never past `room`.
pub(crate) fn user_text_width(natural: f32, indented: bool, room: f32) -> f32 {
    let slack = if indented { LIST_SLACK } else { TEXT_SLACK };
    (natural.ceil() + slack).min(room).max(0.0)
}

/// A slash command the user ran, as a muted mono line.
pub(crate) fn slash_command(ui: &mut Ui, text: &str) {
    ui.add_space(NOTE_GAP);
    ui.add(Label::new(RichText::new(text).font(mono(13.0)).color(brand::MUTED)).truncate());
    ui.add_space(NOTE_GAP);
}

/// What a local command printed, as a muted note.
pub(crate) fn command_output(ui: &mut Ui, text: &str) {
    ui.add_space(NOTE_GAP);
    ui.add(Label::new(RichText::new(text).font(body(13.0)).color(brand::MUTED)));
    ui.add_space(NOTE_GAP);
}

/// Draws turn `group` of `list`: header, bubble, chips and the open list. Flips `open` for the
/// turn when a chip is clicked.
pub(crate) fn turn(
    ui: &mut Ui,
    scene: &Scene<'_>,
    list: &[&Row],
    group: &Group,
    (open, cache): (&mut HashSet<u64>, &mut CommonMarkCache),
    width: f32,
) {
    let live = scene.running && !group.closed;
    let has_text = !group.texts.is_empty();
    let listing = open.contains(&group.key) && !group.calls.is_empty();
    let column = agent_width(width);
    let mut toggled = false;
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
            if has_text {
                bubble(ui, list, group, cache, column);
            }
            if live {
                if has_text {
                    ui.add_space(CHIP_GAP);
                }
                toggled |= live_row(ui, scene, list, group, column);
            } else if !group.chips.is_empty() {
                ui.add_space(CHIP_GAP);
                toggled |= chips(ui, group, listing);
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

fn bubble(ui: &mut Ui, list: &[&Row], group: &Group, cache: &mut CommonMarkCache, column: f32) {
    Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(AGENT_RADIUS)
        .inner_margin(BUBBLE_PAD)
        .show(ui, |ui| {
            ui.set_width(column - f32::from(BUBBLE_PAD.left + BUBBLE_PAD.right) - 2.0);
            chat_text(ui, |ui| {
                for (n, index) in group.texts.iter().enumerate() {
                    if n > 0 {
                        ui.add_space(8.0);
                    }
                    let row = list[*index];
                    ui.push_id(row.seq, |ui| {
                        agent_markdown(ui, cache, &row.text);
                    });
                }
            });
        });
}

fn chips(ui: &mut Ui, group: &Group, open: bool) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(CHIP_GAP, CHIP_GAP);
        for chip in &group.chips {
            clicked |= chip_pill(ui, chip, open).clicked();
        }
        if let Some(took) = group.elapsed_ms() {
            duration(ui, &reader_turns::duration_label(took));
        }
    });
    clicked
}

fn duration(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), body(13.0), brand::FAINT);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 8.0, CHIP_H), Sense::hover());
    let at = pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, brand::FAINT);
}

fn chip_pill(ui: &mut Ui, chip: &Chip, open: bool) -> Response {
    let failed = chip.failed > 0;
    let label = chip.label();
    let ink = if failed { brand::ORANGE } else { brand::SOFT };
    let galley = ui.painter().layout_no_wrap(label, body(13.0), ink);
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
    paint_icon(
        painter,
        pos2(rect.left() + 12.0 + icon_w / 2.0, rect.center().y),
        chip.kind,
        icon_ink,
    );
    let text_at = pos2(rect.left() + 12.0 + icon_w + 6.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn icon_point(center: Pos2, x: f32, y: f32) -> Pos2 {
    let s = 14.0 / 24.0;
    center + vec2((x - 12.0) * s, (y - 12.0) * s)
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
            Some(format!("{count} \u{b7} {}", reader_turns::duration_label(took)))
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

fn call_list(ui: &mut Ui, root: Option<&Path>, list: &[&Row], group: &Group, column: f32) {
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::SIDEBAR_LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(column - 26.0);
            ui.spacing_mut().item_spacing.y = LIST_GAP;
            for index in &group.calls {
                let Kind::Tool(tool) = &list[*index].kind else { continue };
                let line = split_label(&tool.label, root);
                let text =
                    reader_turns::list_line(tool, line.name, &format!("{}{}", line.dir, line.leaf));
                let ink = if tool.error { brand::RED } else { brand::SOFT };
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), LIST_LINE_H), Sense::hover());
                let galley = ellipsized(ui.painter(), &text, mono(12.0), ink, rect.width());
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                    galley,
                    ink,
                );
                response.on_hover_ui(|ui| {
                    ui.set_max_width(480.0);
                    ui.label(RichText::new(&tool.label).font(mono(12.0)).color(brand::TEXT));
                    if !tool.output.is_empty() {
                        let ink = if tool.error { brand::RED } else { brand::MUTED };
                        ui.label(
                            RichText::new(output_preview(&tool.output)).font(mono(11.5)).color(ink),
                        );
                    }
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    const TEXT: &str = "Preview at localhost:5174 now, [docs](https://x.dev/docs) and \
        [bad](javascript:alert(1)) or <file:///etc/passwd> or [ref][r].\n\n[r]: javascript:alert(2)";

    fn frame(ctx: &egui::Context, cache: &mut CommonMarkCache, events: Vec<Event>) -> Vec<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            ui.set_width(600.0);
            ui.visuals_mut().override_text_color = Some(Color32::WHITE);
            chat_text(ui, |ui| agent_markdown(ui, cache, TEXT));
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

    fn click(ctx: &egui::Context, cache: &mut CommonMarkCache, at: Pos2) -> Vec<String> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut opened = Vec::new();
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            opened.extend(frame(ctx, cache, events));
        }
        opened
    }

    fn every_click(y: f32) -> Vec<(f32, Vec<String>)> {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let _ = frame(&ctx, &mut cache, vec![]);
        let _ = frame(&ctx, &mut cache, vec![]);
        (0..640)
            .step_by(4)
            .map(|x| (x as f32, click(&ctx, &mut cache, pos2(x as f32, y))))
            .collect()
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
        let mut cache = CommonMarkCache::default();
        let mut inks = Vec::new();
        for _ in 0..2 {
            let out = ctx.run_ui(RawInput::default(), |ui| {
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                chat_text(ui, |ui| agent_markdown(ui, &mut cache, "plain localhost:5174 end"));
            });
            inks = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.fallback_color, t.override_text_color))
                    }
                    _ => None,
                })
                .collect();
        }
        let ink =
            |word: &str| inks.iter().find(|(text, ..)| text.contains(word)).map(|i| (i.1, i.2));
        assert_eq!(ink("localhost").map(|i| i.0), Some(brand::TEAL_TEXT));
        let plain = ink("plain").unwrap();
        assert_eq!(plain.1.unwrap_or(plain.0), Color32::WHITE);
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
}
