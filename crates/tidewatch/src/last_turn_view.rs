//! The done check of the Progress panel (`show_check`): small rows with a status dot, its two
//! buttons and when it was made. What it says is `LastTurn` and `done_check_view::card_of`; this
//! file only draws it.

use egui::epaint::text::TextFormat;
use egui::{Color32, FontId, Rect, RichText, Sense, Ui, pos2, vec2};
use jiff::tz::TimeZone;
use tidewatch_core::pr_status::Tone;

use crate::brand;
use crate::done_check_glue::Look;
use crate::done_check_view::{self, Action as CheckAction};
use crate::last_turn::LastTurn;
use crate::link_text::Text;
use crate::panel_blocks;
use crate::pr_strip::tone_ink;
use crate::progress_view::Action;
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::{Size, body, heading, primary_button, quiet_button};
use crate::selectable;

const ITEM_FONT: f32 = 14.0;
const ROW_FONT: f32 = 13.0;
const NOTE_FONT: f32 = TEXT_SECONDARY;
const BULLET_W: f32 = 16.0;
const ITEM_GAP: f32 = 6.0;
const CHECK_ROWS: usize = 2;
const BUTTONS_GAP: f32 = 8.0;

pub(crate) fn show_check(
    ui: &mut Ui,
    width: f32,
    turn: &LastTurn,
    age: Option<&str>,
) -> Option<Action> {
    let head = label(ui, "Done check", heading(13.0), brand::SOFT);
    if let Some(age) = age {
        panel_blocks::paint_age(ui.painter(), head.right(), head.center().y - 2.0, age);
    }
    ui.add_space(6.0);
    check(ui, width, &turn.check, turn).map(Action::DoneCheck)
}

fn label(ui: &mut Ui, text: &str, font: FontId, ink: Color32) -> Rect {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y + 4.0), Sense::hover());
    selectable::galley(ui, rect.left_top(), galley, ink);
    rect
}

fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(BUTTONS_GAP, ITEM_GAP);
        ui.add_space(BULLET_W - BUTTONS_GAP);
        add(ui);
    });
}

fn checking_note(ui: &mut Ui, width: f32) -> Option<CheckAction> {
    let painter = ui.painter().clone();
    let galley = painter.layout_no_wrap(
        "Checking that the work landed...".to_owned(),
        body(ROW_FONT),
        brand::MUTED,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + ITEM_GAP), Sense::hover());
    painter.circle_filled(
        pos2(rect.left() + 4.0, rect.top() + galley.size().y / 2.0),
        4.0,
        brand::MUTED,
    );
    selectable::galley(ui, pos2(rect.left() + BULLET_W, rect.top()), galley, brand::MUTED);
    None
}

fn check(ui: &mut Ui, width: f32, look: &Look, turn: &LastTurn) -> Option<CheckAction> {
    let painter = ui.painter().clone();
    let (report, checked, running) = match look {
        Look::Report(report, checked) => (report, Some(*checked), false),
        Look::Checking(Some(report)) => (report, None, true),
        Look::Checking(None) => return checking_note(ui, width),
    };
    let card = done_check_view::card_of(report, turn.today);
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
    let ask_fix = card.ask_fix && turn.owner_asks;
    let stamp =
        checked.map(|made| done_check_view::stamp_text(made.at, made.same, &TimeZone::system()));
    if ask_fix || card.check_again || running || stamp.is_some() {
        button_row(ui, |ui| {
            if ask_fix {
                let can_ask = if running { Err("Checking again") } else { turn.can_ask };
                let sent = primary_button(ui, "Ask agent to fix", can_ask.is_ok(), Size::Compact);
                let sent = match can_ask {
                    Ok(()) => sent,
                    Err(why) => sent.on_hover_text(why),
                };
                if sent.clicked() {
                    picked = Some(CheckAction::AskFix);
                }
            }
            if running {
                quiet_button(ui, "Checking\u{2026}", false, Size::Compact);
            } else if card.check_again
                && quiet_button(ui, "Check again", true, Size::Compact).clicked()
            {
                picked = Some(CheckAction::CheckAgain);
            }
            if let Some(stamp) = stamp {
                let note = RichText::new(stamp).font(body(NOTE_FONT)).color(brand::MUTED);
                ui.add(egui::Label::new(note).wrap_mode(egui::TextWrapMode::Extend));
            }
        });
    }
    picked
}
