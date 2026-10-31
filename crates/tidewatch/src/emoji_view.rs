//! Draws the emoji list above the message box: a selected row with a teal bar like the command
//! palette's, the typed letters of each short name in bold teal, and the Tab hint on the picked row.

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Area, CornerRadius, FontId, Frame, Id, Margin, Order, Pos2, Rect, Sense, Stroke, Ui,
    pos2, vec2,
};

use crate::brand;
use crate::emoji_paint;
use crate::emoji_pick::Hit;
use crate::screen_kit::{TEXT_BODY, TEXT_TITLE};
use crate::screen_kit::{heading, selected_ink, selection_row};

pub(crate) const ROW_H: f32 = 38.0;
pub(crate) const WIDTH: f32 = 320.0;
const GAP: f32 = 8.0;
const PAD: i8 = 6;
const GLYPH_PX: f32 = 20.0;
const GLYPH_COLUMN: f32 = 24.0;
const ROW_PAD: f32 = 10.0;
const COLUMN_GAP: f32 = 12.0;

fn menu_id(id: Id) -> Id {
    id.with("emoji-menu")
}

/// The name as it is shown, `:smile:`, and the byte range of it that the typed letters light up.
pub(crate) fn shown_name(hit: &Hit) -> (String, std::ops::Range<usize>) {
    let lit = if hit.prefix { 0..hit.lit.end + 1 } else { hit.lit.start + 1..hit.lit.end + 1 };
    (format!(":{}:", hit.code), lit)
}

fn name_job(hit: &Hit, selected: bool) -> LayoutJob {
    let (name, lit) = shown_name(hit);
    let ink = selected_ink(selected, brand::TEXT);
    let plain = TextFormat::simple(FontId::proportional(TEXT_BODY), ink);
    let bold = TextFormat::simple(heading(TEXT_TITLE), brand::TEAL_TEXT);
    let mut job = LayoutJob::default();
    job.append(&name[..lit.start], 0.0, plain.clone());
    job.append(&name[lit.clone()], 0.0, bold);
    job.append(&name[lit.end..], 0.0, plain);
    job
}

fn paint_row(ui: &Ui, rect: Rect, hit: &Hit, selected: bool, hovered: bool) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, selected, hovered);
    let glyph = Rect::from_center_size(
        pos2(rect.left() + ROW_PAD + GLYPH_COLUMN / 2.0, rect.center().y),
        vec2(GLYPH_PX, GLYPH_PX),
    );
    if !emoji_paint::paint_in(ui, Id::new(("emoji-row", hit.code)), glyph, hit.emoji.as_str()) {
        painter.text(
            glyph.center(),
            Align2::CENTER_CENTER,
            hit.emoji.as_str(),
            FontId::proportional(GLYPH_PX),
            brand::TEXT,
        );
    }
    let galley = painter.layout_job(name_job(hit, selected));
    let x = rect.left() + ROW_PAD + GLYPH_COLUMN + COLUMN_GAP;
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    if selected {
        painter.text(
            pos2(rect.right() - ROW_PAD, rect.center().y),
            Align2::RIGHT_CENTER,
            "Tab",
            FontId::proportional(12.0),
            brand::MUTED,
        );
    }
}

/// While the list is open the box keeps Tab, the arrows and Esc, so none moves the focus away.
pub(crate) fn hold_keys(ui: &Ui, id: Id) {
    crate::slash_view::hold_keys(ui, id);
}

/// Draws the list with its left edge at `at.x` and its bottom `at.y` above the box. Returns the row
/// that was clicked.
pub(crate) fn show(ui: &Ui, id: Id, at: Pos2, rows: &[Hit], picked: usize) -> Option<usize> {
    let mut clicked = None;
    Area::new(menu_id(id))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(at.x, at.y - GAP))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(PAD))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, hit) in rows.iter().enumerate() {
                        let size = vec2(WIDTH - 2.0 * f32::from(PAD), ROW_H);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        paint_row(ui, rect, hit, index == picked, response.hovered());
                        if response.clicked() {
                            clicked = Some(index);
                        }
                    }
                });
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emoji_pick::{Usage, find};

    #[test]
    fn a_prefix_match_lights_the_colon_and_the_letters_and_another_match_only_the_letters() {
        let cases = [
            ("smi", "smile", ":smi", "le:"),
            ("miling", "slightly_smiling_face", "miling", "_face:"),
        ];
        for (term, code, lit, tail) in cases {
            let hit =
                find(term, &Usage::default(), 40).into_iter().find(|hit| hit.code == code).unwrap();
            let (name, range) = shown_name(&hit);
            assert_eq!(&name[range.clone()], lit, "{code}");
            assert!(name.ends_with(tail), "{name}");
        }
    }
}
