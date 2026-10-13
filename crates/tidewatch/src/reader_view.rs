//! Draws the Reader: the transcript as rows (markdown for the agent, a bubble for the user, one
//! collapsed line per tool call), the permission buttons and the reply box. Only rows near the
//! viewport are drawn; the others take their measured or estimated height.

use egui::style::ScrollAnimation;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Rect, RichText, ScrollArea,
    Ui, UiBuilder, pos2, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, chip, text_field};
use tidewatch_core::transcript::Source;

use crate::reader_keys::{self, Answer};
use crate::reader_rows::{Kind, Row, Rows, Tool};
use crate::reader_scroll::{Autoscroll, Heights};

const SIDE_PAD: f32 = 16.0;
const OVERSCAN: f32 = 600.0;
const INPUT_H: f32 = 48.0;
const STRIP_H: f32 = 40.0;
const SEND_W: f32 = 64.0;
const JUMP_W: f32 = 140.0;
const JUMP_H: f32 = 30.0;
const SCROLLBAR_GRAB_W: f32 = 16.0;
const OUTPUT_MAX_LINES: usize = 40;
const BUBBLE_CORNER: u8 = 8;

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
pub(crate) struct Facts {
    pub(crate) source: Option<Source>,
    pub(crate) prompt: Option<Prompt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Type this into the agent, then press Enter.
    Reply(String),
    /// Type the keys of this answer into the agent's prompt.
    Answer(Answer),
    SwitchToTerminal,
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    rows: &Rows,
    state: &mut ViewState,
    facts: Facts,
) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.painter().rect_filled(rect, 0.0, colors::bg());
    let strip_h = if facts.prompt.is_some() { STRIP_H } else { 0.0 };
    let input = Rect::from_min_max(pos2(rect.left(), rect.bottom() - INPUT_H), rect.max);
    let strip = Rect::from_min_max(
        pos2(rect.left(), input.top() - strip_h),
        pos2(rect.right(), input.top()),
    );
    let body = Rect::from_min_max(rect.min, pos2(rect.right(), strip.top()));
    transcript(ui, body, rows, state);
    if let Some(prompt) = facts.prompt {
        actions.extend(prompt_strip(ui, strip, prompt, facts.source));
    }
    let mut input_ui = ui.new_child(UiBuilder::new().max_rect(input.shrink2(vec2(SIDE_PAD, 6.0))));
    let enabled = facts.prompt.is_none();
    if let Some(text) = reply_box(&mut input_ui, &mut state.reply, &mut state.focus_reply, enabled)
    {
        actions.push(Action::Reply(text));
    }
    actions
}

fn transcript(ui: &mut Ui, body: Rect, rows: &Rows, state: &mut ViewState) {
    if rows.len() == 0 {
        ui.painter().text(
            body.center(),
            Align2::CENTER_CENTER,
            "Waiting for the agent's first message...",
            FontId::proportional(14.0),
            colors::dim(),
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
    let width = body.width() - 2.0 * SIDE_PAD;
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
                    ui.add_space(SIDE_PAD);
                    ui.label(RichText::new(text).size(12.0).color(colors::dim()));
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
                    ui.add_space(SIDE_PAD);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        draw_row(ui, row, cache);
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

fn draw_row(ui: &mut Ui, row: &Row, cache: &mut CommonMarkCache) {
    match &row.kind {
        Kind::User => {
            Frame::new()
                .fill(colors::elevated())
                .corner_radius(CornerRadius::same(BUBBLE_CORNER))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("You").size(11.0).color(colors::dim()));
                    ui.label(RichText::new(&row.text).color(colors::fg()));
                });
        }
        Kind::Assistant => {
            CommonMarkViewer::new().show(ui, cache, &row.text);
        }
        Kind::Tool(tool) => draw_tool(ui, row.seq, tool),
        Kind::Notice => {
            Frame::new()
                .fill(colors::red().gamma_multiply(0.18))
                .corner_radius(CornerRadius::same(BUBBLE_CORNER))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&row.text).color(colors::red()));
                });
        }
    }
}

fn draw_tool(ui: &mut Ui, seq: u64, tool: &Tool) {
    let (mark, color) = tool_mark(tool);
    let title =
        RichText::new(format!("{mark}  {}", tool.label)).monospace().size(12.0).color(color);
    let header = egui::CollapsingHeader::new(title).id_salt(("reader-tool", seq));
    header.show(ui, |ui| {
        if tool.output.is_empty() {
            let text = if tool.done { "No output" } else { "Running..." };
            ui.label(RichText::new(text).size(12.0).color(colors::dim()));
            return;
        }
        let shown = output_preview(&tool.output);
        ui.label(RichText::new(shown).monospace().size(12.0).color(if tool.error {
            colors::red()
        } else {
            colors::dim()
        }));
    });
}

/// The mark and color of a tool row: running, done or failed.
pub(crate) fn tool_mark(tool: &Tool) -> (&'static str, Color32) {
    match (tool.done, tool.error) {
        (_, true) => ("x", colors::red()),
        (true, false) => ("+", colors::dim()),
        (false, false) => ("...", colors::accent()),
    }
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
    ui.painter().rect_filled(strip, 0.0, colors::elevated());
    let mut strip_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(strip.shrink2(vec2(SIDE_PAD, 4.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let answers = match prompt {
        Prompt::Permission => reader_keys::answers(source),
        Prompt::Question => &[],
    };
    strip_ui.label(RichText::new(prompt_text(prompt, !answers.is_empty())).color(colors::fg()));
    for (answer, _) in answers {
        if action_button(&mut strip_ui, answer.label(), *answer == Answer::Allow).clicked() {
            actions.push(Action::Answer(*answer));
        }
    }
    if action_button(&mut strip_ui, "Switch to terminal", false).clicked() {
        actions.push(Action::SwitchToTerminal);
    }
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
        ui.horizontal(|ui| {
            let width = ui.available_width() - SEND_W - ui.spacing().item_spacing.x;
            let hint = if enabled { "Reply to the agent" } else { "Answer the prompt above first" };
            let field = text_field(ui, text, hint, width, colors::fg());
            if *focus {
                field.request_focus();
                *focus = false;
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let clicked = action_button(ui, "Send", true).clicked();
            if (enter || clicked) && !text.trim().is_empty() {
                sent = Some(std::mem::take(text));
                *focus = true;
            }
        })
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
        assert_eq!(tool_mark(&tool(false, false)).0, "...");
        assert_eq!(tool_mark(&tool(true, false)).0, "+");
        assert_eq!(tool_mark(&tool(true, true)).0, "x");
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
                show(ui, rect, rows, &mut state, Facts { source: None, prompt: None });
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
                let facts =
                    Facts { source: Some(Source::Claude), prompt: Some(Prompt::Permission) };
                let actions = show(ui, rect, &rows, &mut state, facts);
                assert_eq!(actions, Vec::<Action>::new());
            });
        }
        assert_eq!(state.heights.measured_count(), 3);
    }
}
