//! Draws a workflow tab's header: the stepper (one pill per stage with its state, the loop counter,
//! the gate), the summary and findings of a clicked stage, the question the run asks the user and
//! why it stopped. What each part says is in `stepper`, `workflow_ask` and `workflow_run`; the heights
//! are pure so the layout is a unit test.

use egui::{
    Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Shape, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::runner::StageState;

use crate::brand;
use crate::layout::band_above;
use crate::reader_view::{INSET, ellipsized, field_frame, paint_bar, paint_item};
use crate::screen_kit::{Size, body, heading, primary_button, quiet_button};
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, TEXT_TAG};
use crate::stepper::{self, Detail, Model, Pill};
use crate::workflow_ask::{Ask, Choice};
use crate::workflow_run::{Exec, RunUi, TabRun};

const STEPPER_H: f32 = 48.0;
const PILL_H: f32 = Size::Compact.height();
const PILL_RADIUS: u8 = Size::Compact.radius();
const CONNECTOR_W: f32 = 10.0;
const CARD_PAD: f32 = 12.0;
const OUTER_Y: f32 = 4.0;
const TITLE_H: f32 = 22.0;
const LINE_H: f32 = 16.0;
const BUTTONS_H: f32 = 30.0;
const FIELD_H: f32 = 38.0;
const MAX_DETAIL_LINES: usize = 4;
const DETAIL_MAX_LINES_OPEN: usize = 8;
const CHAR_W: f32 = 6.8;
const STOPPED_H: f32 = 38.0;
const FIELD_MAX_W: f32 = 560.0;

/// The color a stage in `state` is drawn in.
pub(crate) fn state_color(state: StageState) -> Color32 {
    match state {
        StageState::Todo => brand::MUTED,
        StageState::Running => brand::WORKING,
        StageState::NeedsYou => brand::NEEDS,
        StageState::Done => brand::DONE,
        StageState::Failed => brand::FAILED,
    }
}

/// How many lines of `text` fit in `width`, at most `max`: a guess from the character count, close
/// enough to size a card that never overflows.
pub(crate) fn text_lines(text: &str, width: f32, max: usize) -> usize {
    let columns = ((width / CHAR_W) as usize).max(20);
    text.lines()
        .map(|line| line.chars().count().div_ceil(columns).max(1))
        .sum::<usize>()
        .clamp(1, max)
}

/// `text` without its blank lines, which the question card does not draw.
fn compact(text: &str) -> String {
    compact_lines(text).collect::<Vec<_>>().join("\n")
}

fn compact_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.trim().is_empty())
}

/// The height of the question card for `width`: the title, the detail, the field the user types in
/// (a rejection reason or a test command) and the buttons.
pub(crate) fn ask_height(ask: &Ask, ui: &RunUi, width: f32) -> f32 {
    let detail = ask.detail.as_deref().map_or(0.0, |text| {
        text_lines(&compact(text), width - 2.0 * CARD_PAD, MAX_DETAIL_LINES) as f32 * LINE_H
    });
    let field = if ui.rejecting || ask.test_command { FIELD_H } else { 0.0 };
    2.0 * (CARD_PAD + OUTER_Y) + TITLE_H + detail + field + BUTTONS_H
}

/// The height of the summary and findings of a clicked stage.
pub(crate) fn detail_height(detail: &Detail, width: f32) -> f32 {
    let inner = width - 2.0 * CARD_PAD;
    let summary =
        detail.summary.as_deref().map_or(1, |text| text_lines(text, inner, DETAIL_MAX_LINES_OPEN));
    let findings: usize =
        detail.findings.iter().map(|finding| text_lines(&finding.text, inner, 3)).sum();
    let findings = findings.min(DETAIL_MAX_LINES_OPEN) + usize::from(!detail.findings.is_empty());
    2.0 * (CARD_PAD + OUTER_Y) + TITLE_H + (summary + findings) as f32 * LINE_H
}

fn pill_icon(painter: &egui::Painter, center: egui::Pos2, state: StageState, color: Color32) {
    let r = 4.5;
    let stroke = Stroke::new(1.6, color);
    match state {
        StageState::Todo => {
            painter.circle_stroke(center, r, Stroke::new(1.2, color.gamma_multiply(0.7)));
        }
        StageState::Running | StageState::NeedsYou => {
            painter.circle_filled(center, r, color);
        }
        StageState::Done => {
            painter.add(Shape::line(
                vec![
                    center + vec2(-r, 0.0),
                    center + vec2(-r * 0.3, r * 0.8),
                    center + vec2(r, -r * 0.8),
                ],
                stroke,
            ));
        }
        StageState::Failed => {
            painter.line_segment([center + vec2(-r, -r), center + vec2(r, r)], stroke);
            painter.line_segment([center + vec2(-r, r), center + vec2(r, -r)], stroke);
        }
    }
}

fn pill_text(pill: &Pill) -> String {
    match pill.loops {
        Some((used, max)) => format!("{}  loop {used}/{max}", pill.label),
        None => pill.label.to_owned(),
    }
}

/// Draws the pills from `origin`; returns the clicked stage and where the last pill ends.
fn pills(
    ui: &mut Ui,
    origin: egui::Pos2,
    model: &Model,
    open: Option<usize>,
) -> (Option<usize>, f32) {
    let mut x = origin.x;
    let mut clicked = None;
    for (index, pill) in model.pills.iter().enumerate() {
        let color = state_color(pill.state);
        let label = ui.painter().layout_no_wrap(
            pill_text(pill),
            FontId::proportional(TEXT_SECONDARY),
            brand::TEXT,
        );
        let agent = ui.painter().layout_no_wrap(
            pill.agent_id.clone(),
            FontId::proportional(TEXT_TAG),
            brand::MUTED,
        );
        let width = 10.0 + 14.0 + label.size().x + 8.0 + agent.size().x + 10.0;
        let rect = Rect::from_min_size(pos2(x, origin.y), vec2(width, PILL_H));
        let response = ui
            .interact(rect, ui.id().with(("stage-pill", index)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&pill.tip);
        let selected = open == Some(index);
        let fill = if selected || response.hovered() {
            color.gamma_multiply(0.28)
        } else {
            color.gamma_multiply(0.14)
        };
        let painter = ui.painter();
        let radius = CornerRadius::same(PILL_RADIUS);
        painter.rect_filled(rect, radius, fill);
        let outline =
            if pill.state == StageState::NeedsYou { color } else { color.gamma_multiply(0.4) };
        painter.rect_stroke(rect, radius, Stroke::new(1.0, outline), StrokeKind::Inside);
        pill_icon(painter, pos2(rect.left() + 15.0, rect.center().y), pill.state, color);
        let text_x = rect.left() + 10.0 + 14.0;
        painter.galley(
            pos2(text_x, rect.center().y - label.size().y / 2.0),
            label.clone(),
            brand::TEXT,
        );
        painter.galley(
            pos2(text_x + label.size().x + 8.0, rect.center().y - agent.size().y / 2.0),
            agent,
            brand::MUTED,
        );
        if response.clicked() {
            clicked = Some(index);
        }
        x = rect.right();
        if index + 1 < model.pills.len() {
            let line = Stroke::new(1.0, brand::FIELD_LINE);
            painter.hline(x..=x + CONNECTOR_W, rect.center().y, line);
            x += CONNECTOR_W;
        }
    }
    (clicked, x)
}

fn stepper_band(ui: &mut Ui, rect: Rect, run: &TabRun) -> Option<usize> {
    let model = stepper::model(run.runner(), run.test_timeout());
    let inner = rect.shrink2(vec2(INSET, 0.0));
    let name = ui.painter().layout_no_wrap(model.name.clone(), heading(13.0), brand::TEXT);
    ui.painter().galley(
        pos2(inner.left(), rect.center().y - name.size().y / 2.0),
        name.clone(),
        brand::TEXT,
    );
    let origin = pos2(inner.left() + name.size().x + 16.0, rect.center().y - PILL_H / 2.0);
    let (clicked, pills_right) = pills(ui, origin, &model, run.ui.open_stage);
    let (text, ink) = if let Some(error) = &run.save_error {
        (error.clone(), brand::RED)
    } else {
        let mut facts = vec![model.now.clone()];
        facts.extend(model.gate.as_ref().map(|gate| format!("Gate: {gate}")));
        facts.extend(model.loop_text.clone());
        (facts.join("  \u{b7}  "), brand::MUTED)
    };
    let room = inner.right() - pills_right - 16.0;
    if room > 140.0 {
        let galley = ellipsized(ui.painter(), &text, body(12.0), ink, room);
        let at = pos2(inner.right() - galley.size().x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, ink);
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    clicked
}

fn detail_band(ui: &mut Ui, rect: Rect, detail: &Detail) {
    let card = rect.shrink2(vec2(INSET, OUTER_Y));
    paint_item(ui.painter(), card, false);
    let mut child = ui.new_child(
        UiBuilder::new().max_rect(card.shrink(CARD_PAD)).layout(Layout::top_down(Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 2.0;
    child.label(
        RichText::new(format!("{}  -  {}  -  gate: {}", detail.title, detail.who, detail.gate))
            .size(13.0)
            .strong()
            .color(brand::TEXT),
    );
    match &detail.summary {
        Some(summary) => {
            child.label(RichText::new(plain_text(summary)).size(TEXT_SECONDARY).color(brand::SOFT));
        }
        None => {
            child.label(RichText::new("No summary yet").size(TEXT_SECONDARY).color(brand::MUTED));
        }
    }
    if !detail.findings.is_empty() {
        child.label(RichText::new("Findings").size(12.0).color(brand::MUTED));
        for finding in &detail.findings {
            let (mark, color) =
                if finding.blocking { ("blocking", brand::RED) } else { ("nit", brand::MUTED) };
            child.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).monospace().size(TEXT_TAG).color(color));
                ui.label(
                    RichText::new(plain_line(&finding.text))
                        .size(TEXT_SECONDARY)
                        .color(brand::SOFT),
                );
            });
        }
    }
}

fn plain_text(text: &str) -> String {
    text.lines().map(plain_line).collect::<Vec<_>>().join("\n")
}

/// One line of an agent's markdown as plain text: no emphasis or code markers, bullets as dots.
pub(crate) fn plain_line(line: &str) -> String {
    let line = line.trim_start();
    let line = line.trim_start_matches('#').trim_start();
    let (bullet, rest) = match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        Some(rest) => ("\u{b7} ", rest),
        None => ("", line),
    };
    format!("{bullet}{}", rest.replace(['`', '*'], "").replace("__", ""))
}

/// The ask card. Returns the button the user pressed, a rejection to send back or a way out of one.
enum Pressed {
    Choice(Choice),
    CancelReject,
}

fn ask_card(ui: &mut Ui, rect: Rect, ask: &Ask, state: &mut RunUi) -> Option<Pressed> {
    let card = rect.shrink2(vec2(INSET, OUTER_Y));
    paint_item(ui.painter(), card, false);
    paint_bar(ui.painter(), card, brand::ORANGE);
    let inner = card.shrink2(vec2(CARD_PAD + 2.0, CARD_PAD));
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing = vec2(8.0, 4.0);
    child.label(RichText::new(plain_line(&ask.title)).size(TEXT_BODY).strong().color(brand::TEXT));
    if let Some(detail) = &ask.detail {
        let lines: Vec<&str> = compact_lines(detail).take(MAX_DETAIL_LINES).collect();
        child.label(RichText::new(plain_text(&lines.join("\n"))).size(12.0).color(brand::SOFT));
    }
    let mut pressed = None;
    let width = inner.width().min(FIELD_MAX_W);
    let field = |ui: &mut Ui, text: &mut String, hint: &str, focus: &mut bool| {
        let response = ui
            .allocate_ui(vec2(width, 0.0), |ui| {
                field_frame(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(text)
                            .frame(egui::Frame::NONE)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY)
                            .font(body(14.0))
                            .text_color(brand::TEXT),
                    )
                })
            })
            .inner;
        if std::mem::take(focus) {
            response.request_focus();
        }
        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    };
    let mut submit = false;
    if state.rejecting {
        let RunUi { feedback, focus_field, .. } = state;
        submit = field(
            &mut child,
            feedback,
            "Why? The reason goes to the stage the work returns to",
            focus_field,
        );
    } else if ask.test_command {
        let RunUi { test_command, focus_field, .. } = state;
        submit =
            field(&mut child, test_command, "Test command, for example cargo test", focus_field);
    }
    child.horizontal(|ui| {
        for button in &ask.buttons {
            let label = if state.rejecting && button.choice == Choice::Reject {
                "Send back"
            } else {
                button.label
            };
            let primary = button.primary || (state.rejecting && button.choice == Choice::Reject);
            let enabled = !(state.rejecting
                && button.choice == Choice::Reject
                && state.feedback.trim().is_empty());
            let response = if primary {
                primary_button(ui, label, enabled, Size::Compact)
            } else {
                quiet_button(ui, label, enabled, Size::Compact)
            };
            if response.on_hover_text(button.hint).clicked() {
                pressed = Some(Pressed::Choice(button.choice));
            }
        }
        if state.rejecting && quiet_button(ui, "Cancel", true, Size::Compact).clicked() {
            pressed = Some(Pressed::CancelReject);
        }
    });
    if submit {
        if state.rejecting && !state.feedback.trim().is_empty() {
            pressed = Some(Pressed::Choice(Choice::Reject));
        } else if ask.test_command && !state.test_command.trim().is_empty() {
            pressed = Some(Pressed::Choice(Choice::Retry));
        }
    }
    pressed
}

fn stopped_band(ui: &mut Ui, rect: Rect, text: &str) {
    let card = rect.shrink2(vec2(INSET, OUTER_Y));
    let radius = CornerRadius::same(8);
    ui.painter().rect_filled(card, radius, brand::RED_FILL);
    ui.painter().rect_stroke(card, radius, Stroke::new(1.0, brand::RED_LINE), StrokeKind::Inside);
    paint_bar(ui.painter(), card, brand::RED);
    let left = card.left() + CARD_PAD + 2.0;
    let galley =
        ellipsized(ui.painter(), text, body(13.0), brand::RED, card.right() - CARD_PAD - left);
    ui.painter().galley(pos2(left, card.center().y - galley.size().y / 2.0), galley, brand::RED);
}

/// Takes the header bands off the top of `terminal`, draws them and returns what the user's clicks
/// asked the run to do.
pub(crate) fn show(ui: &mut Ui, terminal: &mut Rect, run: &mut TabRun) -> Vec<Exec> {
    let pad = crate::layout::TERMINAL_PAD;
    let mut area = Rect::from_min_max(
        pos2(terminal.left() - pad, terminal.top() - pad),
        pos2(terminal.right() + pad, terminal.bottom()),
    );
    let width = area.width() - 2.0 * INSET;
    let stepper = band_above(&mut area, STEPPER_H);
    let detail = run.ui.open_stage.and_then(|index| stepper::detail(run.runner(), index));
    let detail_rect = detail.as_ref().map(|d| band_above(&mut area, detail_height(d, width)));
    let ask = run.ask();
    let ask_rect = ask.as_ref().map(|a| band_above(&mut area, ask_height(a, &run.ui, width)));
    let stopped = run.stopped();
    let stopped_rect = stopped.as_ref().map(|_| band_above(&mut area, STOPPED_H));
    terminal.min.y = (area.top() + pad).min(terminal.max.y - 1.0);

    let mut execs = Vec::new();
    if let Some(index) = stepper_band(ui, stepper, run) {
        run.toggle_stage(index);
    }
    if let (Some(detail), Some(rect)) = (&detail, detail_rect) {
        detail_band(ui, rect, detail);
    }
    if let (Some(ask), Some(rect)) = (&ask, ask_rect) {
        match ask_card(ui, rect, ask, &mut run.ui) {
            Some(Pressed::Choice(choice)) => execs = run.choose(choice),
            Some(Pressed::CancelReject) => run.cancel_reject(),
            None => {}
        }
    }
    if let (Some(text), Some(rect)) = (&stopped, stopped_rect) {
        stopped_band(ui, rect, text);
    }
    execs
}

#[cfg(test)]
mod tests;
