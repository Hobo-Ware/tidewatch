//! The "Due today" card above the Reader's reply box: the item, its steps and the two things the
//! user may do, ask the agent to start or snooze it until tomorrow. It only draws.

use egui::{Align2, FontId, Rect, Ui, UiBuilder, pos2, vec2};
use stdusk_egui::ui::ellipsize;

use crate::brand;
use crate::due::{Due, ask_text};
use crate::progress_ring::paint_mark;
use crate::reader_view::{INSET, ellipsized, paint_item};
use crate::screen_kit::TEXT_BODY;
use crate::screen_kit::TEXT_TITLE;
use crate::screen_kit::{
    Size, TagFill, body, button_width, heading, paint_tag, primary_button, quiet_button,
};

const PAD: f32 = 16.0;
const PILL_H: f32 = Size::Tag.height();
const TITLE_H: f32 = 22.0;
const STEP_H: f32 = 26.0;
const MORE_H: f32 = 20.0;
const GAP: f32 = 10.0;
const BOTTOM_GAP: f32 = 6.0;
const MIN_ROOM: f32 = 150.0;
const MAX_STEPS: usize = 5;
const PROFILE_CHARS: usize = 24;

/// What the card needs beyond the item itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) due: &'a Due,
    /// Other items due in the same tab.
    pub(crate) more: usize,
    pub(crate) profile: &'a str,
    /// Why Ask is off, or `None` while the agent is idle.
    pub(crate) blocked: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Type this into the agent.
    Ask(String),
    /// Hide this item until tomorrow.
    Snooze(String),
}

fn steps_shown(due: &Due) -> usize {
    due.steps.len().min(MAX_STEPS)
}

/// The room the card takes above the reply box, gap included, out of `room`: nothing when it would
/// not fit.
pub(crate) fn height(due: &Due, room: f32) -> f32 {
    let more = if due.steps.len() > MAX_STEPS { MORE_H } else { 0.0 };
    let steps =
        if due.steps.is_empty() { 0.0 } else { GAP + steps_shown(due) as f32 * STEP_H + more };
    let natural = 2.0 * PAD + PILL_H + GAP + TITLE_H + steps + GAP + Size::Compact.height();
    if room < MIN_ROOM.min(natural) || natural > room { 0.0 } else { natural + BOTTOM_GAP }
}

pub(crate) fn ask_label(profile: &str) -> String {
    let (name, _) = ellipsize(profile, PROFILE_CHARS);
    format!("Ask {name} to start")
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, facts: Facts<'_>) -> Option<Action> {
    let Facts { due, more, profile, blocked } = facts;
    let card = Rect::from_min_max(
        pos2(rect.left() + INSET, rect.top()),
        pos2(rect.right() - INSET, rect.bottom() - BOTTOM_GAP),
    );
    paint_item(ui.painter(), card, false);
    let inner = card.shrink(PAD);
    let painter = ui.painter().clone();
    let mut y = inner.top();

    let pill_rect = paint_tag(
        &painter,
        (inner.left(), y + PILL_H / 2.0),
        &due.pill(),
        brand::TEAL_TEXT,
        TagFill::Tint(brand::TEAL.gamma_multiply(0.12)),
    );
    if more > 0 {
        painter.text(
            pos2(inner.right(), pill_rect.center().y),
            Align2::RIGHT_CENTER,
            format!("+{more} more due"),
            FontId::proportional(12.0),
            brand::FAINT,
        );
    }
    y += PILL_H + GAP;

    let title = ellipsized(&painter, &due.what, heading(TEXT_TITLE), brand::TEXT, inner.width());
    painter.galley(pos2(inner.left(), y + (TITLE_H - title.size().y) / 2.0), title, brand::TEXT);
    y += TITLE_H;

    if !due.steps.is_empty() {
        y += GAP;
        for (mark, text) in due.steps.iter().take(MAX_STEPS) {
            let mid = y + STEP_H / 2.0;
            paint_mark(&painter, pos2(inner.left() + 9.0, mid), 14.0, *mark, 0.0);
            let ink = match mark {
                tidewatch_core::progress::Mark::Done => brand::MUTED,
                tidewatch_core::progress::Mark::Skipped => brand::FAINT,
                _ => brand::TEXT,
            };
            let left = inner.left() + 30.0;
            let galley =
                ellipsized(&painter, text, body(TEXT_BODY), ink, (inner.right() - left).max(1.0));
            painter.galley(pos2(left, mid - galley.size().y / 2.0), galley, ink);
            y += STEP_H;
        }
        if due.steps.len() > MAX_STEPS {
            let rest = due.steps.len() - MAX_STEPS;
            painter.text(
                pos2(inner.left() + 30.0, y + MORE_H / 2.0),
                Align2::LEFT_CENTER,
                format!("+{rest} more steps"),
                FontId::proportional(12.0),
                brand::FAINT,
            );
            y += MORE_H;
        }
    }
    y += GAP;

    let label = ask_label(profile);
    let ask_w = button_width(ui, &label, Size::Compact);
    let snooze_w = button_width(ui, "Snooze", Size::Compact);
    let ask_w = ask_w.min((inner.width() - snooze_w - 8.0).max(60.0));
    let mut action = None;
    let ask_rect = Rect::from_min_size(pos2(inner.left(), y), vec2(ask_w, Size::Compact.height()));
    ui.scope_builder(UiBuilder::new().max_rect(ask_rect), |ui| {
        let response = primary_button(ui, &label, blocked.is_none(), Size::Compact);
        if let Some(reason) = blocked {
            response.on_disabled_hover_text(reason);
        } else if response.clicked() {
            action = Some(Action::Ask(ask_text(due)));
        }
    });
    let snooze_rect = Rect::from_min_size(
        pos2(ask_rect.right() + 8.0, y),
        vec2(snooze_w, Size::Compact.height()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(snooze_rect), |ui| {
        let response =
            quiet_button(ui, "Snooze", true, Size::Compact).on_hover_text("Hide it until tomorrow");
        if response.clicked() {
            action = Some(Action::Snooze(due.what.clone()));
        }
    });
    action
}

#[cfg(test)]
mod tests;
