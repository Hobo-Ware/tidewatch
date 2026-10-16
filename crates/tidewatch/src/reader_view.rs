//! Draws the Reader: the transcript as rows (markdown for the agent, a small card for the user and
//! for each tool call), the permission strip and the reply box. Only rows near the viewport are
//! drawn; the others take their measured or estimated height. It also holds the small card and
//! field looks the other inline items of the left column share.

use std::path::Path;
use std::sync::Arc;

use egui::collapsing_header::CollapsingState;
use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::style::ScrollAnimation;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Galley, Key, Layout, Margin, Painter,
    Rect, Response, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2,
    vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use stdusk_egui::widgets::chip;
use tidewatch_core::transcript::Source;

use crate::agent_ready::{self, COMPOSER_H, Ready};
use crate::brand;
use crate::reader_keys::{self, Answer};
use crate::reader_rows::{Kind, Row, Rows, Tool};
use crate::reader_scroll::{Autoscroll, Heights};
use crate::screen_kit::{body, mono, primary_button, quiet_button};

/// The side inset every inline item of the left column keeps.
pub(crate) const INSET: f32 = 14.0;
const CARD_RADIUS: u8 = 8;
const OVERSCAN: f32 = 600.0;
const INPUT_H: f32 = 48.0;
const STRIP_H: f32 = 48.0;
const TOOL_HEAD_H: f32 = 20.0;
const BAR_W: f32 = 2.0;
const BAR_INSET: f32 = 7.0;
const JUMP_W: f32 = 140.0;
const JUMP_H: f32 = 30.0;
const SCROLLBAR_GRAB_W: f32 = 16.0;
const OUTPUT_MAX_LINES: usize = 40;

pub(crate) struct ViewState {
    pub(crate) scroll: Autoscroll,
    pub(crate) heights: Heights,
    cache: CommonMarkCache,
    pub(crate) reply: String,
    focus_reply: bool,
    content_h: f32,
    seen_rows: u64,
    stick_pending: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll: Autoscroll::default(),
            heights: Heights::default(),
            cache: CommonMarkCache::default(),
            reply: String::new(),
            focus_reply: true,
            content_h: 0.0,
            seen_rows: 0,
            stick_pending: true,
        }
    }
}

/// What the agent is stuck on, when it waits for something typed into a menu rather than a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    /// A tool call waits for approval.
    Permission,
    /// The agent asked a question.
    Question,
}

/// What the Reader needs to know about the tab beyond its rows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) source: Option<Source>,
    pub(crate) prompt: Option<Prompt>,
    /// Set while the tab waits for its first message: the empty state and the composer show.
    pub(crate) ready: Option<Ready<'a>>,
    /// The folder the agent works in: tool lines show paths relative to it.
    pub(crate) root: Option<&'a Path>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Type this into the agent, then press Enter.
    Reply(String),
    /// Type the keys of this answer into the agent's prompt.
    Answer(Answer),
    SwitchToTerminal,
}

/// The look of an inline item: a card on the window ground.
pub(crate) fn item_frame() -> Frame {
    Frame::new()
        .fill(brand::ITEM)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
}

/// Paints `rect` as an inline item; a hot one (hovered) takes the stronger line.
pub(crate) fn paint_item(painter: &Painter, rect: Rect, hot: bool) {
    let line = if hot { brand::FIELD_LINE } else { brand::ITEM_LINE };
    let radius = CornerRadius::same(CARD_RADIUS);
    painter.rect_filled(rect, radius, brand::ITEM);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
}

/// The short accent bar on the left edge of an inline item that needs attention.
pub(crate) fn paint_bar(painter: &Painter, rect: Rect, color: Color32) {
    let bar = Rect::from_min_max(
        rect.left_top() + vec2(0.0, BAR_INSET),
        rect.left_bottom() + vec2(BAR_W, -BAR_INSET),
    );
    painter.rect_filled(bar, CornerRadius::same(1), color);
}

/// One line of `text` cut with an ellipsis to fit `max_w`.
pub(crate) fn ellipsized(
    painter: &Painter,
    text: &str,
    font: FontId,
    ink: Color32,
    max_w: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
    painter.layout_job(job)
}

/// A text input in the field look: ITEM fill, 1 px line, teal while it has the keyboard.
pub(crate) fn field_frame(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let mut prepared = Frame::new()
        .fill(brand::ITEM)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::symmetric(10, 5))
        .begin(ui);
    prepared.content_ui.set_width(prepared.content_ui.available_width());
    let response = add(&mut prepared.content_ui);
    let line = if response.has_focus() { brand::TEAL } else { brand::FIELD_LINE };
    prepared.frame.stroke = Stroke::new(1.0, line);
    prepared.end(ui);
    response
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    rows: &Rows,
    state: &mut ViewState,
    facts: Facts<'_>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.painter().rect_filled(rect, 0.0, brand::GROUND);
    let strip_h = if facts.prompt.is_some() { STRIP_H } else { 0.0 };
    let composing = facts.ready.filter(|_| rows.len() == 0 && facts.prompt.is_none());
    let input_h = if composing.is_some() { COMPOSER_H } else { INPUT_H };
    let input = Rect::from_min_max(pos2(rect.left(), rect.bottom() - input_h), rect.max);
    let strip = Rect::from_min_max(
        pos2(rect.left(), input.top() - strip_h),
        pos2(rect.right(), input.top()),
    );
    let body = Rect::from_min_max(rect.min, pos2(rect.right(), strip.top()));
    if let Some(ready) = composing {
        agent_ready::empty_state(ui, body, ready);
        if let Some(text) =
            agent_ready::composer(ui, input, &mut state.reply, &mut state.focus_reply, ready)
        {
            actions.push(Action::Reply(text));
        }
        return actions;
    }
    transcript(ui, body, rows, state, facts.root);
    if let Some(prompt) = facts.prompt {
        actions.extend(prompt_strip(ui, strip, prompt, facts.source));
    }
    let mut input_ui = ui.new_child(UiBuilder::new().max_rect(input.shrink2(vec2(INSET, 8.0))));
    let enabled = facts.prompt.is_none();
    if let Some(text) = reply_box(&mut input_ui, &mut state.reply, &mut state.focus_reply, enabled)
    {
        actions.push(Action::Reply(text));
    }
    actions
}

fn transcript(ui: &mut Ui, body: Rect, rows: &Rows, state: &mut ViewState, root: Option<&Path>) {
    if rows.len() == 0 {
        ui.painter().text(
            body.center(),
            Align2::CENTER_CENTER,
            "Waiting for the agent's first message...",
            FontId::proportional(14.0),
            brand::MUTED,
        );
        return;
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let scrolled_up =
        ui.input(|i| i.smooth_scroll_delta.y > 0.0) && pointer.is_some_and(|p| body.contains(p));
    let grabbing_bar = ui.input(|i| i.pointer.primary_down())
        && pointer.is_some_and(|p| body.contains(p) && p.x > body.right() - SCROLLBAR_GRAB_W);
    if scrolled_up || grabbing_bar {
        state.scroll.release();
    }
    let width = body.width() - 2.0 * INSET;
    state.heights.for_width(width);
    let newest = rows.newest_seq().map_or(0, |seq| seq + 1);
    if newest != state.seen_rows {
        state.seen_rows = newest;
        state.stick_pending = true;
    }
    let follow = state.scroll.following();
    let scroll_down = follow && std::mem::take(&mut state.stick_pending);
    let ViewState { heights, cache, .. } = state;
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    let out = ScrollArea::vertical().id_salt("reader-scroll").auto_shrink([false, false]).show(
        &mut body_ui,
        |ui| {
            ui.add_space(8.0);
            if rows.dropped() > 0 {
                let text = format!("{} earlier entries are not shown", rows.dropped());
                ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
                });
            }
            let clip = ui.clip_rect();
            for row in rows.iter() {
                let top = ui.next_widget_position().y;
                let height = heights.height(row);
                if top + height < clip.top() - OVERSCAN || top > clip.bottom() + OVERSCAN {
                    ui.allocate_space(vec2(width, height));
                    continue;
                }
                let drawn = ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        draw_row(ui, row, cache, root);
                    })
                });
                heights.record(row.seq, drawn.response.rect.height());
            }
            ui.add_space(12.0);
            if scroll_down {
                ui.scroll_to_cursor_animation(Some(Align::BOTTOM), ScrollAnimation::none());
            }
        },
    );
    let max_offset = (out.content_size.y - out.inner_rect.height()).max(0.0);
    state.scroll.settle(out.state.offset.y, max_offset);
    if (out.content_size.y - state.content_h).abs() > 0.5 {
        state.content_h = out.content_size.y;
        state.stick_pending = true;
        ui.ctx().request_repaint();
    }
    if !state.scroll.following() {
        let slot = Rect::from_center_size(
            pos2(body.center().x, body.bottom() - JUMP_H),
            vec2(JUMP_W, JUMP_H),
        );
        let mut jump_ui = ui.new_child(UiBuilder::new().max_rect(slot));
        if chip(&mut jump_ui, "Jump to latest", true).clicked() {
            state.scroll.jump_to_latest();
            state.stick_pending = true;
            ui.ctx().request_repaint();
        }
    }
}

fn draw_row(ui: &mut Ui, row: &Row, cache: &mut CommonMarkCache, root: Option<&Path>) {
    match &row.kind {
        Kind::User => {
            item_frame().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("You").size(11.5).color(brand::MUTED));
                ui.label(RichText::new(&row.text).color(brand::TEXT));
            });
        }
        Kind::Assistant => {
            ui.scope(|ui| {
                ui.visuals_mut().code_bg_color = brand::ITEM;
                CommonMarkViewer::new().show(ui, cache, &row.text);
            });
        }
        Kind::Tool(tool) => draw_tool(ui, row.seq, tool, root),
        Kind::Divider => draw_divider(ui, &row.text),
        Kind::Notice => {
            Frame::new()
                .fill(brand::RED_FILL)
                .stroke(Stroke::new(1.0, brand::RED_LINE))
                .corner_radius(CornerRadius::same(CARD_RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&row.text).color(brand::RED));
                });
        }
    }
}

fn draw_divider(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(divider_label(text), body(11.5), brand::MUTED);
    let half = galley.size().x / 2.0 + 10.0;
    let line = Stroke::new(1.0, brand::ITEM_LINE);
    painter.hline(rect.left()..=(rect.center().x - half).max(rect.left()), rect.center().y, line);
    painter.hline((rect.center().x + half).min(rect.right())..=rect.right(), rect.center().y, line);
    painter.galley(rect.center() - galley.size() / 2.0, galley, brand::MUTED);
    ui.add_space(6.0);
}

/// A stage divider's text as a quiet label: `Stage 2 \u{b7} Review \u{b7} gemini \u{b7} Read only`.
pub(crate) fn divider_label(text: &str) -> String {
    let text = text.trim_matches(|c: char| c == '-' || c.is_whitespace());
    text.replacen(": ", " \u{b7} ", 1)
        .replacen(" (", " \u{b7} ", 1)
        .replace(", ", " \u{b7} ")
        .trim_end_matches(')')
        .to_owned()
}

fn draw_tool(ui: &mut Ui, seq: u64, tool: &Tool, root: Option<&Path>) {
    let id = ui.make_persistent_id(("reader-tool", seq));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let line = split_label(&tool.label, root);
    let mut clicked = false;
    item_frame().inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (head, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), TOOL_HEAD_H), Sense::click());
        let painter = ui.painter();
        painter.circle_filled(pos2(head.left() + 4.0, head.center().y), 3.5, tool_ink(tool));
        let mut job = LayoutJob::default();
        job.append(line.name, 0.0, TextFormat::simple(body(12.0), brand::SOFT));
        if !line.dir.is_empty() {
            job.append(&line.dir, 8.0, TextFormat::simple(mono(11.5), brand::FAINT));
        }
        if !line.leaf.is_empty() {
            let gap = if line.dir.is_empty() { 8.0 } else { 0.0 };
            job.append(&line.leaf, gap, TextFormat::simple(mono(11.5), brand::TEXT));
        }
        job.wrap = TextWrapping::truncate_at_width((head.width() - 40.0).max(1.0));
        let galley = painter.layout_job(job);
        painter.galley(
            pos2(head.left() + 16.0, head.center().y - galley.size().y / 2.0),
            galley,
            brand::TEXT,
        );
        paint_chevron(painter, pos2(head.right() - 6.0, head.center().y), state.is_open());
        clicked = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&tool.label)
            .clicked();
        if clicked {
            state.set_open(!state.is_open());
        }
        if state.is_open() {
            tool_output(ui, tool);
        }
    });
    if clicked {
        state.store(ui.ctx());
    }
}

fn paint_chevron(painter: &Painter, at: egui::Pos2, open: bool) {
    let stroke = Stroke::new(1.3, brand::FAINT);
    let (a, b, c) = if open {
        (vec2(-3.5, -1.5), vec2(0.0, 2.0), vec2(3.5, -1.5))
    } else {
        (vec2(-1.5, -3.5), vec2(2.0, 0.0), vec2(-1.5, 3.5))
    };
    painter.line_segment([at + a, at + b], stroke);
    painter.line_segment([at + b, at + c], stroke);
}

fn tool_output(ui: &mut Ui, tool: &Tool) {
    ui.add_space(4.0);
    let at = ui.available_rect_before_wrap();
    ui.painter().hline(at.x_range(), at.top(), Stroke::new(1.0, brand::ITEM_LINE));
    ui.add_space(6.0);
    if tool.output.is_empty() {
        let text = if tool.done { "No output" } else { "Running..." };
        ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
        return;
    }
    let ink = if tool.error { brand::RED } else { brand::MUTED };
    ui.label(RichText::new(output_preview(&tool.output)).monospace().size(11.5).color(ink));
}

/// The dot of a tool row: running, done or failed.
pub(crate) fn tool_ink(tool: &Tool) -> Color32 {
    match (tool.done, tool.error) {
        (_, true) => brand::FAILED,
        (true, false) => brand::DONE.gamma_multiply(0.6),
        (false, false) => brand::WORKING,
    }
}

/// A tool row's label split for drawing: the tool, then the folder and leaf of its path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolLine<'a> {
    pub(crate) name: &'a str,
    pub(crate) dir: String,
    pub(crate) leaf: String,
}

/// Splits a `Name  argument` label, with the worktree's own folder taken off every path.
pub(crate) fn split_label<'a>(label: &'a str, root: Option<&Path>) -> ToolLine<'a> {
    let (name, argument) = label.split_once("  ").unwrap_or((label, ""));
    let argument = root.and_then(Path::to_str).map_or_else(
        || argument.to_owned(),
        |root| {
            root_forms(root).iter().fold(argument.to_owned(), |text, root| {
                if text == *root { ".".to_owned() } else { text.replace(&format!("{root}/"), "") }
            })
        },
    );
    let is_path = argument.contains('/') && !argument.contains(char::is_whitespace);
    match argument.rfind('/').filter(|_| is_path) {
        Some(at) => {
            ToolLine { name, dir: argument[..=at].to_owned(), leaf: argument[at + 1..].to_owned() }
        }
        None => ToolLine { name, dir: String::new(), leaf: argument },
    }
}

/// A folder as written with and without macOS's `/private` in front, which `/tmp` and `/var` hide.
fn root_forms(root: &str) -> Vec<String> {
    let root = root.trim_end_matches('/');
    let other = match root.strip_prefix("/private") {
        Some(rest) if rest.starts_with('/') => rest.to_owned(),
        _ => format!("/private{root}"),
    };
    let mut forms = vec![root.to_owned(), other];
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms
}

/// The first lines of a tool's output, with a count of what was cut.
pub(crate) fn output_preview(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= OUTPUT_MAX_LINES {
        return output.to_owned();
    }
    let text = lines[..OUTPUT_MAX_LINES].join("\n");
    let hidden = lines.len() - OUTPUT_MAX_LINES;
    format!("{text}\n... {hidden} more lines")
}

fn prompt_strip(ui: &mut Ui, strip: Rect, prompt: Prompt, source: Option<Source>) -> Vec<Action> {
    let mut actions = Vec::new();
    let card = strip.shrink2(vec2(INSET, 4.0));
    paint_item(ui.painter(), card, false);
    paint_bar(ui.painter(), card, brand::ORANGE);
    let mut buttons = ui.new_child(
        UiBuilder::new()
            .max_rect(card.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = 6.0;
    let answers = match prompt {
        Prompt::Permission => reader_keys::answers(source),
        Prompt::Question => &[],
    };
    if quiet_button(&mut buttons, "Switch to terminal", true).clicked() {
        actions.push(Action::SwitchToTerminal);
    }
    for (answer, _) in answers.iter().rev() {
        let clicked = if *answer == Answer::Allow {
            primary_button(&mut buttons, answer.label(), true)
        } else {
            quiet_button(&mut buttons, answer.label(), true)
        }
        .clicked();
        if clicked {
            actions.push(Action::Answer(*answer));
        }
    }
    let left = card.left() + 16.0;
    let galley = ellipsized(
        ui.painter(),
        prompt_text(prompt, !answers.is_empty()),
        body(13.0),
        brand::TEXT,
        buttons.min_rect().left() - 12.0 - left,
    );
    ui.painter().galley(pos2(left, card.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    actions
}

/// What the strip above the reply box says. A reply typed into a menu would pick its default
/// option on Enter, so the reply box is off while a prompt waits.
pub(crate) fn prompt_text(prompt: Prompt, has_buttons: bool) -> &'static str {
    match (prompt, has_buttons) {
        (Prompt::Permission, true) => "The agent is asking for permission",
        (Prompt::Permission, false) => {
            "The agent is asking for permission. Answer it in the terminal."
        }
        (Prompt::Question, _) => "The agent asked a question. Answer it in the terminal.",
    }
}

/// The reply row. Enter sends a non-empty text and clears it. Asks for focus once, the first
/// time it is drawn, and again after a send. Off while `enabled` is false.
pub(crate) fn reply_box(
    ui: &mut Ui,
    text: &mut String,
    focus: &mut bool,
    enabled: bool,
) -> Option<String> {
    let mut sent = None;
    ui.add_enabled_ui(enabled, |ui| {
        let hint = if enabled { "Reply to the agent" } else { "Answer the prompt above first" };
        let mut clicked = false;
        let field = field_frame(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = quiet_button(ui, "Send", !text.trim().is_empty()).clicked();
                ui.add(
                    TextEdit::singleline(text)
                        .frame(Frame::NONE)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(14.0))
                        .text_color(brand::TEXT),
                )
            })
            .inner
        });
        if *focus {
            field.request_focus();
            *focus = false;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if (enter || clicked) && !text.trim().is_empty() {
            sent = Some(std::mem::take(text));
            *focus = true;
        }
    });
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, RawInput};

    fn frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        text: &mut String,
        focus: &mut bool,
    ) -> Option<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut sent = None;
        let _ = ctx.run_ui(raw, |ui| {
            sent = reply_box(ui, text, focus, true);
        });
        sent
    }

    fn enter() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn enter_in_the_reply_box_sends_the_text_and_clears_it() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        assert_eq!(frame(&ctx, vec![], &mut text, &mut focus), None);
        assert_eq!(
            frame(&ctx, vec![Event::Text("run the tests".into())], &mut text, &mut focus),
            None
        );
        assert_eq!(text, "run the tests");
        let sent = frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert_eq!(sent.as_deref(), Some("run the tests"));
        assert_eq!(text, "");
    }

    #[test]
    fn enter_on_an_empty_or_blank_reply_sends_nothing() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        frame(&ctx, vec![], &mut text, &mut focus);
        frame(&ctx, vec![Event::Text("   ".into())], &mut text, &mut focus);
        assert_eq!(frame(&ctx, vec![enter()], &mut text, &mut focus), None);
    }

    #[test]
    fn focus_is_asked_for_once_and_again_after_a_send() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        frame(&ctx, vec![], &mut text, &mut focus);
        assert!(!focus);
        frame(&ctx, vec![Event::Text("hi".into())], &mut text, &mut focus);
        frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert!(focus);
    }

    #[test]
    fn a_tool_row_shows_running_done_or_failed() {
        let tool = |done, error| Tool { label: "Bash".into(), output: String::new(), error, done };
        let inks = [
            tool_ink(&tool(false, false)),
            tool_ink(&tool(true, false)),
            tool_ink(&tool(true, true)),
        ];
        assert_ne!(inks[0], inks[1]);
        assert_ne!(inks[1], inks[2]);
        assert_ne!(inks[0], inks[2]);
        assert_eq!(inks[2], brand::FAILED);
    }

    fn line(name: &'static str, dir: &str, leaf: &str) -> ToolLine<'static> {
        ToolLine { name, dir: dir.into(), leaf: leaf.into() }
    }

    #[test]
    fn a_tool_path_is_shown_from_the_worktree_with_its_folder_split_off() {
        let root = Path::new("/h/.tidewatch/acme/upbeat-harbor");
        let label = "Edit  /h/.tidewatch/acme/upbeat-harbor/src/auth.rs";
        assert_eq!(split_label(label, Some(root)), line("Edit", "src/", "auth.rs"));
        assert_eq!(split_label(label, None).dir, "/h/.tidewatch/acme/upbeat-harbor/src/");
        assert_eq!(split_label("Read  src/a/b.rs", Some(root)), line("Read", "src/a/", "b.rs"));
        assert_eq!(split_label("Read  Cargo.toml", Some(root)), line("Read", "", "Cargo.toml"));
    }

    #[test]
    fn a_worktree_is_found_with_or_without_the_private_prefix() {
        for (root, label) in [
            ("/tmp/w/tree", "Grep  /private/tmp/w/tree"),
            ("/private/tmp/w/tree", "Grep  /tmp/w/tree"),
            ("/private/tmp/w/tree", "Grep  /private/tmp/w/tree"),
        ] {
            assert_eq!(split_label(label, Some(Path::new(root))), line("Grep", "", "."), "{root}");
        }
        let root = Some(Path::new("/tmp/w/tree"));
        assert_eq!(
            split_label("Edit  /private/tmp/w/tree/src/a.rs", root),
            line("Edit", "src/", "a.rs")
        );
        assert_eq!(
            split_label("Edit  /private/tmp/w/other/a.rs", root).dir,
            "/private/tmp/w/other/"
        );
    }

    #[test]
    fn a_stage_divider_reads_as_a_quiet_label() {
        assert_eq!(
            divider_label("--- Stage 1: Work (claude opus, Auto edit) ---"),
            "Stage 1 \u{b7} Work \u{b7} claude opus \u{b7} Auto edit"
        );
        assert_eq!(
            divider_label("--- Stage 2: Review (gemini, Read only, round 2) ---"),
            "Stage 2 \u{b7} Review \u{b7} gemini \u{b7} Read only \u{b7} round 2"
        );
        assert_eq!(divider_label("Stage 3"), "Stage 3");
    }

    #[test]
    fn a_command_stays_whole_and_loses_only_the_worktree_prefix() {
        let root = Path::new("/w/tree");
        assert_eq!(split_label("Bash  git status", Some(root)), line("Bash", "", "git status"));
        assert_eq!(
            split_label("Bash  cat /w/tree/src/a.rs | wc", Some(root)),
            line("Bash", "", "cat src/a.rs | wc")
        );
        assert_eq!(split_label("TodoWrite", Some(root)), line("TodoWrite", "", ""));
    }

    #[test]
    fn long_output_is_cut_with_a_count() {
        let output = (0..60).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let preview = output_preview(&output);
        assert_eq!(preview.lines().count(), OUTPUT_MAX_LINES + 1);
        assert!(preview.ends_with("... 20 more lines"));
        assert_eq!(output_preview("a\nb"), "a\nb");
    }

    fn repaint_delays(rows: &Rows, frames: usize) -> Vec<std::time::Duration> {
        let ctx = egui::Context::default();
        let mut state = ViewState::default();
        let mut delays = Vec::new();
        for frame in 0..frames {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
                focused: true,
                time: Some(frame as f64 * 0.016),
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                show(
                    ui,
                    rect,
                    rows,
                    &mut state,
                    Facts { source: None, prompt: None, ready: None, root: None },
                );
            });
            delays.extend(out.viewport_output.values().map(|v| v.repaint_delay));
        }
        delays
    }

    #[test]
    fn a_settled_reader_asks_for_no_more_frames_than_the_cursor_blink() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        for n in 0..60 {
            let text = format!("# Title {n}\n\n- a\n- b\n\n```rust\nfn x() {{}}\n```\n\nwords");
            rows.push(Entry { role: Role::Assistant, text, tool: None, ts: None });
        }
        let delays = repaint_delays(&rows, 24);
        let busy = delays.iter().rev().take(8).filter(|d| d.is_zero()).count();
        assert_eq!(busy, 0, "{delays:?}");
    }

    #[test]
    fn the_reader_view_draws_every_kind_of_row_headless() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        rows.push_all(vec![
            Entry { role: Role::User, text: "hi".into(), tool: None, ts: None },
            Entry {
                role: Role::Assistant,
                text: "# Title\n\n- a\n- b\n\n```rust\nfn x() {}\n```".into(),
                tool: None,
                ts: None,
            },
            Entry { role: Role::System, text: "boom".into(), tool: None, ts: None },
        ]);
        let ctx = egui::Context::default();
        let mut state = ViewState::default();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
            focused: true,
            ..RawInput::default()
        };
        for _ in 0..2 {
            let _ = ctx.run_ui(raw.clone(), |ui| {
                let rect = ui.max_rect();
                let facts = Facts {
                    source: Some(Source::Claude),
                    prompt: Some(Prompt::Permission),
                    ready: None,
                    root: None,
                };
                let actions = show(ui, rect, &rows, &mut state, facts);
                assert_eq!(actions, Vec::<Action>::new());
            });
        }
        assert_eq!(state.heights.measured_count(), 3);
    }
}
