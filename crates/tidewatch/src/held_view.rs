//! The messages Tidewatch holds for the agent, drawn as one card at the bottom of the chat: a line
//! that says why they wait, then one row each with the text and Edit, Delete and Send now (and
//! Up and Down while more than one waits). Edit opens the row as a small field where Enter saves
//! and Esc leaves the message as it was. It only draws; the caller carries out what it asks for.

use egui::{
    Align, Align2, Id, Key, Layout, Modifiers, Rect, RichText, ScrollArea, TextEdit, Ui, UiBuilder,
    pos2, vec2,
};

use crate::brand;
use crate::reader_view::{INSET, ellipsized, field_frame, paint_item};
use crate::screen_kit::{
    Glyph, Size, TEXT_BODY, TEXT_TAG, body, button_width, glyph_button, quiet_button,
};
use crate::send_line::{Held, Way};
use tidewatch_core::text::squash;

const PAD: f32 = 12.0;
const HEAD_H: f32 = 28.0;
const ROW_H: f32 = 40.0;
const STACK_H: f32 = 68.0;
const EDIT_H: f32 = 112.0;
const MAX_ROWS: usize = 4;
const GAP: f32 = 6.0;
const OUTER: f32 = 6.0;
const MIN_TEXT_W: f32 = 80.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Edit(u64),
    Save(u64, String),
    Cancel,
    Delete(u64),
    Shift(u64, Way),
    SendNow(u64),
}

pub(crate) struct Shown<'a> {
    pub(crate) items: &'a [Held],
    pub(crate) editing: Option<(u64, &'a mut String)>,
    pub(crate) focus_edit: &'a mut bool,
    pub(crate) note: &'a str,
}

fn buttons_width(ui: &Ui) -> f32 {
    ["Send now", "Delete", "Edit"]
        .iter()
        .map(|label| button_width(ui, label, Size::Compact) + GAP)
        .sum()
}

/// Whether a row of a card in a band `band_w` wide has no room for its text beside its buttons, so
/// the text goes on a line of its own over them.
fn stacked(ui: &Ui, band_w: f32) -> bool {
    let row_w = band_w - 2.0 * INSET - 2.0 * PAD;
    row_w - buttons_width(ui) < MIN_TEXT_W
}

fn row_height(stacked: bool) -> f32 {
    if stacked { STACK_H } else { ROW_H }
}

/// The height of the card for `count` messages in a band `band_w` wide, with the gap around it;
/// nothing for none.
pub(crate) fn height(ui: &Ui, band_w: f32, (count, editing): (usize, bool)) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let each = row_height(stacked(ui, band_w));
    let rows = count.min(MAX_ROWS) as f32 * each;
    let extra = if editing { EDIT_H - each } else { 0.0 };
    HEAD_H + rows + extra + PAD / 2.0 + 2.0 * OUTER
}

fn edit_id(id: u64) -> Id {
    Id::new(("held-edit", id))
}

/// Draws the card in `band` (`height` tall) and returns what was asked for.
pub(crate) fn show(ui: &mut Ui, band: Rect, shown: Shown<'_>) -> Vec<Out> {
    let Shown { items, mut editing, focus_edit, note } = shown;
    let mut out = Vec::new();
    let card = Rect::from_min_max(
        pos2(band.left() + INSET, band.top() + OUTER),
        pos2(band.right() - INSET, band.bottom() - OUTER),
    );
    paint_item(ui.painter(), card, false);
    ui.painter().text(
        pos2(card.left() + PAD, card.top() + HEAD_H / 2.0),
        Align2::LEFT_CENTER,
        note,
        body(TEXT_TAG),
        brand::MUTED,
    );
    let inner = Rect::from_min_max(
        pos2(card.left() + PAD, card.top() + HEAD_H),
        pos2(card.right() - PAD, card.bottom() - PAD / 2.0),
    );
    let stack = stacked(ui, band.width());
    let mut list = ui.new_child(UiBuilder::new().max_rect(inner));
    list.spacing_mut().item_spacing = vec2(GAP, 0.0);
    ScrollArea::vertical().id_salt("held-rows").auto_shrink([false, false]).show(&mut list, |ui| {
        for (index, held) in items.iter().enumerate() {
            let width = ui.available_width();
            let editing_this = editing.as_ref().is_some_and(|(id, _)| *id == held.id);
            if editing_this {
                if let Some((_, buffer)) = editing.as_mut() {
                    edit_row(ui, held.id, buffer, (width, focus_edit), &mut out);
                }
                continue;
            }
            let (rect, _) =
                ui.allocate_exact_size(vec2(width, row_height(stack)), egui::Sense::hover());
            row(ui, rect, held, (index, items.len(), stack), &mut out);
        }
    });
    out
}

fn row(
    ui: &mut Ui,
    rect: Rect,
    held: &Held,
    (index, count, stacked): (usize, usize, bool),
    out: &mut Vec<Out>,
) {
    let (text_line, button_line) = if stacked {
        let (top, bottom) = rect.split_top_bottom_at_y(rect.top() + 26.0);
        (top, bottom.shrink2(vec2(0.0, 4.0)))
    } else {
        (rect, rect.shrink2(vec2(0.0, 6.0)))
    };
    let mut buttons = ui.new_child(
        UiBuilder::new().max_rect(button_line).layout(Layout::right_to_left(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = GAP;
    let arrows = 2.0 * (Size::Compact.height() + GAP);
    let with_arrows =
        count > 1 && (stacked || rect.width() - buttons_width(ui) - arrows >= MIN_TEXT_W);
    if quiet_button(&mut buttons, "Send now", true, Size::Compact)
        .on_hover_text("Stop the agent's turn and send this now")
        .clicked()
    {
        out.push(Out::SendNow(held.id));
    }
    if quiet_button(&mut buttons, "Delete", true, Size::Compact).clicked() {
        out.push(Out::Delete(held.id));
    }
    if quiet_button(&mut buttons, "Edit", true, Size::Compact).clicked() {
        out.push(Out::Edit(held.id));
    }
    if with_arrows {
        if index + 1 < count
            && glyph_button(&mut buttons, Glyph::Down, "Send this later", Size::Compact).clicked()
        {
            out.push(Out::Shift(held.id, Way::Later));
        }
        if index > 0
            && glyph_button(&mut buttons, Glyph::Up, "Send this earlier", Size::Compact).clicked()
        {
            out.push(Out::Shift(held.id, Way::Earlier));
        }
    }
    let text_w = if stacked {
        text_line.width()
    } else {
        (text_line.width() - buttons.min_rect().width() - GAP).max(MIN_TEXT_W)
    };
    let galley = ellipsized(
        ui.painter(),
        &squash(&held.text),
        egui::FontId::proportional(TEXT_BODY),
        brand::TEXT,
        text_w,
    );
    let y = if stacked { text_line.top() + 13.0 } else { rect.center().y };
    ui.painter().galley(pos2(text_line.left(), y - galley.size().y / 2.0), galley, brand::TEXT);
}

fn edit_row(
    ui: &mut Ui,
    id: u64,
    buffer: &mut String,
    (width, focus): (f32, &mut bool),
    out: &mut Vec<Out>,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, EDIT_H), egui::Sense::hover());
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(0.0, 4.0))));
    let field_h = EDIT_H - 8.0 - Size::Compact.height() - GAP;
    let field_id = edit_id(id);
    if std::mem::take(focus) {
        child.memory_mut(|memory| memory.request_focus(field_id));
    }
    let focused = child.memory(|memory| memory.has_focus(field_id));
    let save_key = focused
        && (child.input(|input| input.events.iter().any(crate::agent_ready::plain_enter))
            || child.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter)));
    let cancel_key =
        focused && child.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    child.allocate_ui(vec2(width, field_h), |ui| {
        field_frame(ui, |ui| {
            ui.add(
                TextEdit::multiline(buffer)
                    .id(field_id)
                    .frame(egui::Frame::NONE)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .return_key(egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                    .font(egui::FontId::proportional(TEXT_BODY))
                    .text_color(brand::TEXT)
                    .hint_text(RichText::new("Edit the message").color(brand::FAINT)),
            )
        });
    });
    child.add_space(GAP);
    child.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let save = quiet_button(ui, "Save", true, Size::Compact).clicked() || save_key;
        let cancel = quiet_button(ui, "Cancel", true, Size::Compact).clicked() || cancel_key;
        if save {
            out.push(Out::Save(id, buffer.clone()));
        } else if cancel {
            out.push(Out::Cancel);
        }
    });
}
