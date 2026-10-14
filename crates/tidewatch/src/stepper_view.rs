//! Draws a workflow tab's header: the stepper (one pill per stage with its state, the loop counter,
//! the gate), the summary and findings of a clicked stage, the question the run asks the user and
//! why it stopped. What each part says is in `stepper`, `workflow_ask` and `workflow_run`; the heights
//! are pure so the layout is a unit test.

use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, text_field};
use tidewatch_core::runner::StageState;

use crate::brand;
use crate::layout::band_above;
use crate::stepper::{self, Detail, Model, Pill};
use crate::workflow_ask::{Ask, Choice};
use crate::workflow_run::{Exec, RunUi, TabRun};

const STEPPER_H: f32 = 56.0;
const PAD: f32 = 10.0;
const PILL_H: f32 = 26.0;
const PILL_GAP: f32 = 6.0;
const CARD_PAD: f32 = 12.0;
const TITLE_H: f32 = 22.0;
const LINE_H: f32 = 16.0;
const BUTTONS_H: f32 = 36.0;
const FIELD_H: f32 = 38.0;
const MAX_DETAIL_LINES: usize = 4;
const DETAIL_MAX_LINES_OPEN: usize = 8;
const CHAR_W: f32 = 6.8;
const STOPPED_H: f32 = 30.0;

/// The color a stage in `state` is drawn in.
pub(crate) fn state_color(state: StageState) -> Color32 {
    match state {
        StageState::Todo => colors::dim(),
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

/// The height of the question card for `width`: the title, the detail, the field the user types in
/// (a rejection reason or a test command) and the buttons.
pub(crate) fn ask_height(ask: &Ask, ui: &RunUi, width: f32) -> f32 {
    let detail = ask.detail.as_deref().map_or(0.0, |text| {
        text_lines(text, width - 2.0 * CARD_PAD, MAX_DETAIL_LINES) as f32 * LINE_H
    });
    let field = if ui.rejecting || ask.test_command { FIELD_H } else { 0.0 };
    2.0 * CARD_PAD + TITLE_H + detail + field + BUTTONS_H
}

/// The height of the summary and findings of a clicked stage.
pub(crate) fn detail_height(detail: &Detail, width: f32) -> f32 {
    let inner = width - 2.0 * CARD_PAD;
    let summary =
        detail.summary.as_deref().map_or(1, |text| text_lines(text, inner, DETAIL_MAX_LINES_OPEN));
    let findings: usize =
        detail.findings.iter().map(|finding| text_lines(&finding.text, inner, 3)).sum();
    let findings = findings.min(DETAIL_MAX_LINES_OPEN) + usize::from(!detail.findings.is_empty());
    2.0 * CARD_PAD + TITLE_H + (summary + findings) as f32 * LINE_H
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

/// Draws the pills left to right from `origin`. Returns the index of a clicked stage.
fn pills(ui: &mut Ui, origin: egui::Pos2, model: &Model, open: Option<usize>) -> Option<usize> {
    let mut x = origin.x;
    let mut clicked = None;
    for (index, pill) in model.pills.iter().enumerate() {
        let color = state_color(pill.state);
        let label =
            ui.painter().layout_no_wrap(pill_text(pill), FontId::proportional(12.5), colors::fg());
        let agent = ui.painter().layout_no_wrap(
            pill.agent_id.clone(),
            FontId::proportional(11.0),
            colors::dim(),
        );
        let width = 10.0 + 12.0 + label.size().x + 8.0 + agent.size().x + 10.0;
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
        painter.rect_filled(rect, 13.0, fill);
        let outline =
            if pill.state == StageState::NeedsYou { color } else { color.gamma_multiply(0.4) };
        painter.rect_stroke(rect, 13.0, Stroke::new(1.0, outline), StrokeKind::Inside);
        pill_icon(painter, pos2(rect.left() + 15.0, rect.center().y), pill.state, color);
        let text_x = rect.left() + 10.0 + 12.0;
        painter.galley(
            pos2(text_x, rect.center().y - label.size().y / 2.0),
            label.clone(),
            colors::fg(),
        );
        painter.galley(
            pos2(text_x + label.size().x + 8.0, rect.center().y - agent.size().y / 2.0),
            agent,
            colors::dim(),
        );
        if response.clicked() {
            clicked = Some(index);
        }
        x = rect.right();
        if index + 1 < model.pills.len() {
            painter.text(
                pos2(x + PILL_GAP, rect.center().y),
                Align2::LEFT_CENTER,
                ">",
                FontId::proportional(12.0),
                colors::dim(),
            );
            x += 2.0 * PILL_GAP + 6.0;
        }
    }
    clicked
}

fn stepper_band(ui: &mut Ui, rect: Rect, run: &TabRun) -> Option<usize> {
    let model = stepper::model(run.runner(), run.test_timeout());
    ui.painter().rect_filled(rect, 0.0, colors::elevated());
    let inner = rect.shrink2(vec2(PAD, 6.0));
    let name =
        ui.painter().layout_no_wrap(model.name.clone(), FontId::proportional(13.0), colors::fg());
    ui.painter().galley(
        pos2(inner.left(), inner.top() + (PILL_H - name.size().y) / 2.0),
        name.clone(),
        colors::fg(),
    );
    let origin = pos2(inner.left() + name.size().x + 14.0, inner.top());
    let clicked = pills(ui, origin, &model, run.ui.open_stage);
    let mut facts = vec![model.now.clone()];
    facts.extend(model.gate.as_ref().map(|gate| format!("Gate: {gate}")));
    facts.extend(model.loop_text.clone());
    let line = facts.join("   -   ");
    ui.painter().text(
        pos2(inner.left(), inner.top() + PILL_H + 8.0),
        Align2::LEFT_TOP,
        line,
        FontId::proportional(12.0),
        colors::dim(),
    );
    if let Some(error) = &run.save_error {
        ui.painter().text(
            pos2(inner.right(), inner.top() + PILL_H + 8.0),
            Align2::RIGHT_TOP,
            error,
            FontId::proportional(12.0),
            brand::RED,
        );
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, colors::border()),
    );
    clicked
}

fn detail_band(ui: &mut Ui, rect: Rect, detail: &Detail) {
    ui.painter().rect_filled(rect, 0.0, colors::bg());
    let inner = rect.shrink(CARD_PAD);
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing.y = 2.0;
    child.label(
        RichText::new(format!("{}  -  {}  -  gate: {}", detail.title, detail.who, detail.gate))
            .size(13.0)
            .strong()
            .color(colors::fg()),
    );
    match &detail.summary {
        Some(summary) => {
            child.label(RichText::new(summary).size(12.5).color(colors::fg()));
        }
        None => {
            child.label(RichText::new("No summary yet").size(12.5).color(colors::dim()));
        }
    }
    if !detail.findings.is_empty() {
        child.label(RichText::new("Findings").size(12.0).color(colors::dim()));
        for finding in &detail.findings {
            let (mark, color) =
                if finding.blocking { ("blocking", brand::RED) } else { ("nit", colors::dim()) };
            child.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).monospace().size(11.5).color(color));
                ui.label(RichText::new(&finding.text).size(12.5).color(colors::fg()));
            });
        }
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, colors::border()),
    );
}

/// The ask card. Returns the button the user pressed, a rejection to send back or a way out of one.
enum Pressed {
    Choice(Choice),
    CancelReject,
}

fn ask_card(ui: &mut Ui, rect: Rect, ask: &Ask, state: &mut RunUi) -> Option<Pressed> {
    let accent = brand::ORANGE;
    ui.painter().rect_filled(rect, 0.0, accent.gamma_multiply(0.10));
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, accent);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, colors::border()),
    );
    let inner = rect.shrink2(vec2(CARD_PAD + 3.0, CARD_PAD));
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    child.spacing_mut().item_spacing = vec2(8.0, 4.0);
    child.label(RichText::new(&ask.title).size(13.5).strong().color(colors::fg()));
    if let Some(detail) = &ask.detail {
        let lines: Vec<&str> = detail.lines().take(MAX_DETAIL_LINES).collect();
        child.label(RichText::new(lines.join("\n")).size(12.0).color(colors::dim()));
    }
    let mut pressed = None;
    let width = inner.width().min(560.0);
    let field = |ui: &mut Ui, text: &mut String, hint: &str, focus: &mut bool| {
        let response = text_field(ui, text, hint, width, colors::fg());
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
            let response = ui.add_enabled_ui(enabled, |ui| action_button(ui, label, primary)).inner;
            if response.on_hover_text(button.hint).clicked() {
                pressed = Some(Pressed::Choice(button.choice));
            }
        }
        if state.rejecting && action_button(ui, "Cancel", false).clicked() {
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
    ui.painter().rect_filled(rect, 0.0, brand::RED.gamma_multiply(0.12));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
        0.0,
        brand::RED,
    );
    ui.painter().text(
        pos2(rect.left() + CARD_PAD + 3.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        brand::RED,
    );
}

/// Takes the header bands off the top of `terminal`, draws them and returns what the user's clicks
/// asked the run to do.
pub(crate) fn show(ui: &mut Ui, terminal: &mut Rect, run: &mut TabRun) -> Vec<Exec> {
    let pad = crate::layout::TERMINAL_PAD;
    let mut area = Rect::from_min_max(
        pos2(terminal.left() - pad, terminal.top() - pad),
        pos2(terminal.right() + pad, terminal.bottom()),
    );
    let width = area.width();
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
