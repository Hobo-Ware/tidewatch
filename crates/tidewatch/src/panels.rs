//! Placeholder panels for the History and Diff areas until Phase 5 brings the Kelp widgets.

use egui::{Align2, FontId, Rect, Stroke, StrokeKind};
use stdusk_egui::colors;

const NOTE: &str = "Coming in Phase 5";

pub(crate) fn placeholder(ui: &egui::Ui, rect: Rect, title: &str) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors::elevated());
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, colors::border()), StrokeKind::Inside);
    painter.text(
        rect.left_top() + egui::vec2(14.0, 12.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(14.0),
        colors::fg(),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        NOTE,
        FontId::proportional(13.0),
        colors::dim(),
    );
}
