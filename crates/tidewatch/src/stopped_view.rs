//! The bar of an agent tab whose agent is not running: one line and the Resume button that runs the
//! tab's resume command. It only draws.

use egui::{Align2, FontId, Rect, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::reader_view::{INSET, paint_item};
use crate::screen_kit::{Size, button_width, primary_button};

const BAR_H: f32 = 40.0;
const GAP: f32 = 6.0;
const PAD: f32 = 12.0;
const DOT: f32 = 8.0;
const LABEL: &str = "Agent is not running";
const BUTTON: &str = "Resume";

pub(crate) const HEIGHT: f32 = BAR_H + 2.0 * GAP;

/// Draws the bar in `band` (`HEIGHT` tall) and returns whether Resume was pressed.
pub(crate) fn show(ui: &mut Ui, band: Rect) -> bool {
    let card = Rect::from_min_max(
        pos2(band.left() + INSET, band.top() + GAP),
        pos2(band.right() - INSET, band.bottom() - GAP),
    );
    paint_item(ui.painter(), card, false);
    let dot = pos2(card.left() + PAD + DOT / 2.0, card.center().y);
    ui.painter().circle_filled(dot, DOT / 2.0, brand::MUTED);
    ui.painter().text(
        pos2(dot.x + DOT / 2.0 + 8.0, card.center().y),
        Align2::LEFT_CENTER,
        LABEL,
        FontId::proportional(13.5),
        brand::TEXT,
    );
    let width = button_width(ui, BUTTON, Size::Compact);
    let button = Rect::from_center_size(
        pos2(card.right() - PAD - width / 2.0, card.center().y),
        vec2(width, Size::Compact.height()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(button), |ui| {
        primary_button(ui, BUTTON, true, Size::Compact)
            .on_hover_text("Run the resume command in this tab")
            .clicked()
    })
    .inner
}

#[cfg(test)]
mod tests;
