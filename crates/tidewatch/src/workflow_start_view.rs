//! The card a workflow tab shows until Start run, where its task is typed; nothing runs before it.

use egui::{Frame, Id, Key, Modifiers, Rect, RichText, TextEdit, Ui, UiBuilder, pos2, vec2};
use tidewatch_core::runner::Runner;
use tidewatch_core::test_command;
use tidewatch_core::workflows::Gate;

use crate::brand;
use crate::commands::Command;
use crate::reader_view::{field_frame, paint_item};
use crate::screen_kit::{Size, body, cli_short_name, footer_row, heading, primary_button};
use crate::workflow_run::TabRun;

const COLUMN_W: f32 = 760.0;
const CARD_H: f32 = 56.0;
const CARD_GAP: f32 = 8.0;
const PER_ROW: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) title: &'static str,
    pub(crate) who: String,
    pub(crate) tip: String,
}

fn gate_tip(runner: &Runner, gate: Gate, why: Option<&str>) -> String {
    let gate_line = format!("Gate: {}", gate.label());
    if !gate.runs_tests() {
        return gate_line;
    }
    match (runner.context().test_command.as_deref(), why) {
        (Some(command), Some(why)) => format!("{gate_line}\n{}", test_command::tip(command, why)),
        (None, _) => format!("{gate_line}\n{}", test_command::MISSING_TIP),
        (Some(_), None) => gate_line,
    }
}

pub(crate) fn cards(runner: &Runner, test_why: Option<&str>) -> Vec<Card> {
    (0..runner.stage_count())
        .filter_map(|index| {
            let stage = runner.stage(index)?;
            let who = runner.profile(index).map_or_else(String::new, |profile| {
                format!("{} \u{b7} {}", cli_short_name(&profile.agent), profile.level.label())
            });
            Some(Card {
                title: stage.action.label(),
                who,
                tip: gate_tip(runner, stage.gate, test_why),
            })
        })
        .collect()
}

pub(crate) fn blurb(run: &TabRun) -> String {
    if run.blurb().is_empty() {
        "The stages run one after the other in this worktree. You approve the hand off.".to_owned()
    } else {
        run.blurb().to_owned()
    }
}

pub(crate) fn show(ui: &mut Ui, area: Rect, run: &mut TabRun) -> Option<String> {
    ui.painter().rect_filled(area, 0.0, brand::GROUND);
    let width = COLUMN_W.min(area.width() - 40.0);
    let column = Rect::from_min_size(
        pos2(area.center().x - width / 2.0, area.top() + 24.0),
        vec2(width, area.height() - 24.0),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(column));
    child.spacing_mut().item_spacing.y = 0.0;
    child.add_space(((area.height() - 400.0) / 2.0 - 24.0).max(0.0));
    child.label(
        RichText::new("What should this workflow ship?").font(heading(26.0)).color(brand::TEXT),
    );
    child.add_space(6.0);
    child.add(egui::Label::new(RichText::new(blurb(run)).size(14.0).color(brand::MUTED)).wrap());
    child.add_space(18.0);
    let id = Id::new("workflow-task");
    let focused = child.memory(|m| m.has_focus(id));
    let command_enter =
        focused && child.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    task_box(&mut child, run, id);
    child.add_space(18.0);
    stage_cards(&mut child, &cards(run.runner(), run.test_why()));
    child.add_space(18.0);
    let mut start = command_enter;
    let hint = format!(
        "{} starts  -  edit stages in Settings, Workflows",
        Command::ApproveDelivery.keys()
    );
    footer_row(&mut child, &hint, |ui| {
        let ready = !run.ui.task.trim().is_empty();
        start |= primary_button(ui, "Start run", ready, Size::Regular).clicked();
    });
    (start && !run.ui.task.trim().is_empty()).then(|| run.ui.task.clone())
}

fn task_box(ui: &mut Ui, run: &mut TabRun, id: Id) {
    ui.label(RichText::new("Task").size(12.5).color(brand::MUTED));
    ui.add_space(6.0);
    let field = field_frame(ui, |ui| {
        ui.add(
            TextEdit::multiline(&mut run.ui.task)
                .id(id)
                .frame(Frame::NONE)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Describe the change, in a sentence or two")
                .font(egui::FontId::proportional(16.0))
                .text_color(brand::TEXT),
        )
    });
    if std::mem::take(&mut run.ui.focus_task) {
        field.request_focus();
    }
}

fn stage_cards(ui: &mut Ui, cards: &[Card]) {
    for row in cards.chunks(PER_ROW) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, CARD_H), egui::Sense::hover());
        let card_w = (width - CARD_GAP * (PER_ROW as f32 - 1.0)) / PER_ROW as f32;
        for (i, card) in row.iter().enumerate() {
            let at = Rect::from_min_size(
                pos2(rect.left() + i as f32 * (card_w + CARD_GAP), rect.top()),
                vec2(card_w, CARD_H),
            );
            let painter = ui.painter();
            paint_item(painter, at, false);
            painter.text(
                at.left_top() + vec2(12.0, 11.0),
                egui::Align2::LEFT_TOP,
                card.title,
                heading(13.0),
                brand::TEXT,
            );
            painter.text(
                at.left_top() + vec2(12.0, 31.0),
                egui::Align2::LEFT_TOP,
                &card.who,
                body(12.0),
                brand::MUTED,
            );
            ui.interact(at, ui.id().with(("stage-card", i, card.title)), egui::Sense::hover())
                .on_hover_text(&card.tip);
        }
        ui.add_space(CARD_GAP);
    }
}

#[cfg(test)]
mod tests;
