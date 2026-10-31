//! The line above the reply box that says why it is shut: `Agent is not running · Resume`. It stays
//! while the box holds text, so the reason never hides behind a draft. It only draws.

use egui::{Align2, Rect, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::reader_view::{Blocked, INSET};
use crate::screen_kit::{Size, TEXT_SECONDARY, body, button_width, quiet_button};

pub(crate) const HEIGHT: f32 = 40.0;
const DOT: f32 = 8.0;
const GAP: f32 = 8.0;

pub(crate) fn text(blocked: Blocked) -> &'static str {
    match blocked {
        Blocked::Stopped => "Agent is not running",
        Blocked::Failed => "The agent failed",
        Blocked::SettingUp => "The agent is starting up",
        Blocked::Shell => "No agent is running here",
    }
}

/// Whether the tab can run its resume command to fix the reason.
pub(crate) fn offers_resume(blocked: Blocked) -> bool {
    matches!(blocked, Blocked::Stopped | Blocked::Failed)
}

/// Draws the line in `band` and returns whether Resume was pressed.
pub(crate) fn show(ui: &mut Ui, band: Rect, blocked: Blocked) -> bool {
    let y = band.center().y;
    let dot = pos2(band.left() + INSET + DOT / 2.0, y);
    ui.painter().circle_filled(dot, DOT / 2.0, brand::MUTED);
    let at = pos2(dot.x + DOT / 2.0 + GAP, y);
    let galley =
        ui.painter().layout_no_wrap(text(blocked).to_owned(), body(TEXT_SECONDARY), brand::SOFT);
    let end = at.x + galley.size().x;
    ui.painter().galley(pos2(at.x, y - galley.size().y / 2.0), galley, brand::SOFT);
    if !offers_resume(blocked) {
        return false;
    }
    ui.painter().text(
        pos2(end + GAP, y),
        Align2::LEFT_CENTER,
        "\u{b7}",
        body(TEXT_SECONDARY),
        brand::FAINT,
    );
    let width = button_width(ui, "Resume", Size::Compact);
    let rect = Rect::from_min_size(
        pos2(end + 2.0 * GAP + 4.0, y - Size::Compact.height() / 2.0),
        vec2(width, Size::Compact.height()),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    quiet_button(&mut child, "Resume", true, Size::Compact)
        .on_hover_text("Run the resume command in this tab")
        .clicked()
}
