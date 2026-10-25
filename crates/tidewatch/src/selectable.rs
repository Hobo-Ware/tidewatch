//! Painted static text that can be selected with the pointer and copied with Cmd+C, through egui's
//! own label selection. Text inside anything clickable (buttons, tabs, chips, rows) stays on
//! `painter`: a selectable run senses drag and click, so it must never sit over a widget whose
//! clicks matter, unless the caller passes the returned `Response` on as the click.

use std::sync::Arc;

use egui::text_selection::LabelSelectionState;
use egui::{Align2, Color32, FontId, Galley, Id, Pos2, Rect, Response, Sense, Stroke, Ui};

fn id_of(ui: &Ui, galley: &Galley, pos: Pos2) -> Id {
    let origin = ui.min_rect().min;
    let at = ((pos.x - origin.x).floor() as i32, (pos.y - origin.y + 0.25).floor() as i32);
    ui.id().with(("selectable-text", galley.text(), at))
}

pub(crate) fn galley(ui: &Ui, pos: Pos2, galley: Arc<Galley>, ink: Color32) -> Response {
    let id = id_of(ui, &galley, pos);
    let rect = Rect::from_min_size(pos, galley.size());
    let response = ui.interact(rect, id, Sense::click_and_drag());
    LabelSelectionState::label_text_selection(ui, &response, pos, galley, ink, Stroke::NONE);
    response
}

pub(crate) fn text(
    ui: &Ui,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    ink: Color32,
) -> Rect {
    let laid = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let rect = anchor.anchor_size(pos, laid.size());
    galley(ui, rect.min, laid, ink);
    rect
}

#[cfg(test)]
mod tests;
