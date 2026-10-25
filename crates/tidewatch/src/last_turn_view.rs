//! The "Last turn" section of the Progress panel: the Done list, the Needs you items with Answer,
//! the Next list and the done check as small rows with a status dot and its two buttons. What each
//! part says is `LastTurn` and `done_check_view::card_of`; this file only draws it.

use egui::epaint::text::TextFormat;
use egui::{Color32, FontId, Sense, Ui, pos2, vec2};
use tidewatch_core::pr_status::Tone;

use crate::brand;
use crate::done_check_glue::Look;
use crate::done_check_view::{self, Action as CheckAction};
use crate::last_turn::LastTurn;
use crate::link_text::Text;
use crate::pr_strip::tone_ink;
use crate::progress_view::Action;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, heading, mono, primary_button, quiet_button};
use crate::selectable;

const ITEM_FONT: f32 = 14.0;
const CODE_FONT: f32 = 13.0;
const ROW_FONT: f32 = 13.0;
const NOTE_FONT: f32 = 12.5;
const BULLET_W: f32 = 16.0;
const ITEM_GAP: f32 = 6.0;
const GROUP_GAP: f32 = 14.0;
const MAX_ITEMS: usize = 5;
const ITEM_ROWS: usize = 3;
const CHECK_ROWS: usize = 2;
const BUTTONS_GAP: f32 = 8.0;

/// Splits `text` at its backticks into spans, each flagged as code; the markers and bold go.
pub(crate) fn spans(text: &str) -> Vec<(String, bool)> {
    let text = text.replace("**", "");
    text.split('`')
        .enumerate()
        .filter(|(_, span)| !span.is_empty())
        .map(|(i, span)| (span.to_owned(), i % 2 == 1))
        .collect()
}

pub(crate) fn show(ui: &mut Ui, width: f32, turn: &LastTurn, offer_answer: bool) -> Option<Action> {
    let mut action = None;
    label(ui, "Last turn", heading(13.0), brand::MUTED);
    ui.add_space(6.0);
    let mut shown = 0;
    let mut caption = |ui: &mut Ui, text: &str| {
        if shown > 0 {
            ui.add_space(GROUP_GAP);
        }
        shown += 1;
        label(ui, text, body(12.0), brand::MUTED);
    };
    if !turn.done.is_empty() {
        caption(ui, "Done");
        items(ui, width, "done", &turn.done, brand::TEXT, brand::DONE);
    }
    if !turn.needs.is_empty() {
        caption(ui, "Needs you");
        items(ui, width, "needs", &turn.needs, brand::TEXT, brand::NEEDS);
        if offer_answer {
            button_row(ui, |ui| {
                if quiet_button(ui, "Answer", true, Size::Compact).clicked() {
                    action = Some(Action::Answer);
                }
            });
        }
    }
    if !turn.next.is_empty() {
        caption(ui, "Next");
        items(ui, width, "next", &turn.next, brand::MUTED, brand::FAINT);
    }
    if let Some(look) = &turn.check {
        caption(ui, "Checks");
        if let Some(picked) = check(ui, width, look, turn.can_ask, turn.owner_asks) {
            action = Some(Action::DoneCheck(picked));
        }
    }
    action
}

fn label(ui: &mut Ui, text: &str, font: FontId, ink: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y + 4.0), Sense::hover());
    selectable::galley(ui, rect.left_top(), galley, ink);
}

fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(BUTTONS_GAP, ITEM_GAP);
        ui.add_space(BULLET_W - BUTTONS_GAP);
        add(ui);
    });
}

fn items(ui: &mut Ui, width: f32, salt: &str, list: &[String], ink: Color32, bullet: Color32) {
    let painter = ui.painter().clone();
    for (index, item) in list.iter().take(MAX_ITEMS).enumerate() {
        let mut text = Text::new();
        for (span, code) in spans(item) {
            let format = if code {
                TextFormat::simple(mono(CODE_FONT), brand::SOFT)
            } else {
                TextFormat::simple(body(ITEM_FONT), ink)
            };
            text.run(&span, &format);
        }
        let laid = text.wrapped((width - BULLET_W).max(40.0), ITEM_ROWS).lay_out(&painter);
        let (rect, _) =
            ui.allocate_exact_size(vec2(width, laid.size().y + ITEM_GAP), Sense::hover());
        let dot = pos2(rect.left() + 4.0, rect.top() + ITEM_FONT / 2.0 + 1.0);
        painter.circle_filled(dot, 2.5, bullet);
        laid.paint_selectable(
            ui,
            ui.id().with((salt, index)),
            pos2(rect.left() + BULLET_W, rect.top()),
            ink,
        );
    }
    let more = list.len().saturating_sub(MAX_ITEMS);
    if more > 0 {
        label(ui, &format!("+{more} more"), body(12.0), brand::FAINT);
    }
}

fn check(
    ui: &mut Ui,
    width: f32,
    look: &Look,
    can_ask: Result<(), &'static str>,
    owner_asks: bool,
) -> Option<CheckAction> {
    let painter = ui.painter().clone();
    let Look::Report(report) = look else {
        let galley = painter.layout_no_wrap(
            "Checking that the work landed...".to_owned(),
            body(ROW_FONT),
            brand::MUTED,
        );
        let (rect, _) =
            ui.allocate_exact_size(vec2(width, galley.size().y + ITEM_GAP), Sense::hover());
        painter.circle_filled(
            pos2(rect.left() + 4.0, rect.top() + galley.size().y / 2.0),
            4.0,
            brand::MUTED,
        );
        selectable::galley(ui, pos2(rect.left() + BULLET_W, rect.top()), galley, brand::MUTED);
        return None;
    };
    let card = done_check_view::card_of(report);
    let ink = tone_ink(card.tone);
    let title = painter.layout_no_wrap(card.title.to_owned(), body(ITEM_FONT), ink);
    let room = (width - BULLET_W - title.size().x - 10.0).max(0.0);
    let summary = ellipsized(&painter, &card.summary, body(NOTE_FONT), brand::MUTED, room);
    let (rect, _) = ui.allocate_exact_size(vec2(width, title.size().y + ITEM_GAP), Sense::hover());
    painter.circle_filled(pos2(rect.left() + 4.0, rect.top() + title.size().y / 2.0), 4.0, ink);
    let summary_at = pos2(
        rect.left() + BULLET_W + title.size().x + 10.0,
        rect.top() + (title.size().y - summary.size().y) / 2.0,
    );
    selectable::galley(ui, pos2(rect.left() + BULLET_W, rect.top()), title, ink);
    selectable::galley(ui, summary_at, summary, brand::MUTED);
    for (index, row) in card.rows.iter().enumerate() {
        let sub = row.text.starts_with("    ");
        let indent = if sub { BULLET_W } else { 0.0 };
        let ink = if row.tone == Tone::Quiet { brand::MUTED } else { tone_ink(row.tone) };
        let mut text = Text::new();
        text.run(row.text.trim_start(), &TextFormat::simple(body(ROW_FONT), ink));
        let room = (width - BULLET_W - indent).max(40.0);
        let laid = text.wrapped(room, CHECK_ROWS).lay_out(&painter);
        let (rect, _) =
            ui.allocate_exact_size(vec2(width, laid.size().y + ITEM_GAP), Sense::hover());
        if !sub {
            let dot = pos2(rect.left() + 4.0, rect.top() + ROW_FONT / 2.0 + 1.0);
            painter.circle_filled(dot, 2.5, ink);
        }
        let at = pos2(rect.left() + BULLET_W + indent, rect.top());
        laid.paint_selectable(ui, ui.id().with(("check-row", index)), at, ink);
    }
    let mut picked = None;
    let ask_fix = card.ask_fix && owner_asks;
    if ask_fix || card.check_again {
        button_row(ui, |ui| {
            if ask_fix {
                let sent = primary_button(ui, "Ask agent to fix", can_ask.is_ok(), Size::Compact);
                let sent = match can_ask {
                    Ok(()) => sent,
                    Err(why) => sent.on_hover_text(why),
                };
                if sent.clicked() {
                    picked = Some(CheckAction::AskFix);
                }
            }
            if card.check_again && quiet_button(ui, "Check again", true, Size::Compact).clicked() {
                picked = Some(CheckAction::CheckAgain);
            }
        });
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_spans_keep_their_text_and_lose_their_markers() {
        assert_eq!(
            spans("added `expiry()` in `src/auth.rs:12`"),
            [
                ("added ".to_owned(), false),
                ("expiry()".to_owned(), true),
                (" in ".to_owned(), false),
                ("src/auth.rs:12".to_owned(), true),
            ]
        );
        assert_eq!(spans("**Done** it"), [("Done it".to_owned(), false)]);
        assert_eq!(spans("`only`"), [("only".to_owned(), true)]);
        assert_eq!(spans(""), Vec::<(String, bool)>::new());
    }
}
