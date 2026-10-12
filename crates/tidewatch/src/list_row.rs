//! One selectable row of a list (profile pickers in the new-tab dialog and the settings editor):
//! a name, a dim detail after it, a tag on the right, selected and hover fills.

use egui::{Align2, FontId, Sense, Stroke, Vec2};
use stdusk_egui::colors;

pub(crate) const ROW_HEIGHT: f32 = 30.0;

pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) detail_color: egui::Color32,
    pub(crate) tag: String,
    pub(crate) dimmed: bool,
}

pub(crate) fn pick_row(ui: &mut egui::Ui, row: &Row, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 7.0, colors::selection());
        painter.rect_stroke(
            rect,
            7.0,
            Stroke::new(1.0, colors::accent()),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 7.0, colors::hover_elevated());
    }
    let name_color = if row.dimmed { colors::dim() } else { colors::fg() };
    let name = painter.text(
        rect.left_center() + Vec2::new(10.0, 0.0),
        Align2::LEFT_CENTER,
        &row.name,
        FontId::proportional(14.0),
        name_color,
    );
    painter.text(
        egui::pos2(name.right() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        &row.detail,
        FontId::proportional(12.0),
        row.detail_color,
    );
    if !row.tag.is_empty() {
        painter.text(
            rect.right_center() - Vec2::new(10.0, 0.0),
            Align2::RIGHT_CENTER,
            &row.tag,
            FontId::proportional(12.0),
            colors::dim(),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
