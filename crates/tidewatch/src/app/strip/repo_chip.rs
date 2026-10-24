//! The repo chip: a grey folder, the repo name and a chevron, with no box until it is hovered or its menu is open.
//! Its width is fixed so nothing in the strip moves when the repo changes; a long name is cut with an ellipsis.

use egui::epaint::PathShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, pos2, vec2};

use super::TAB_H;
use crate::brand;

pub(super) const WIDTH: f32 = 180.0;
const PAD: f32 = 8.0;
const GAP: f32 = 8.0;
const ICON: f32 = 18.0;
const CHEVRON: f32 = 12.0;
const LABEL_SIZE: f32 = 13.0;
const RADIUS: u8 = 8;
const FOLDER: [(f32, f32); 6] =
    [(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0)];
const CHEVRON_POINTS: [(f32, f32); 3] = [(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)];

pub(super) fn show(ui: &mut egui::Ui, label: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, TAB_H), Sense::click());
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let painter = ui.painter();
    if resp.hovered() || menu_open {
        painter.rect_filled(rect, CornerRadius::same(RADIUS), brand::ITEM);
    }
    let icon = Rect::from_min_size(
        pos2(rect.left() + PAD, rect.center().y - ICON / 2.0),
        vec2(ICON, ICON),
    );
    stroke_path(painter, &FOLDER, icon, 24.0, Stroke::new(1.5, brand::MUTED), true);
    let chevron = Rect::from_min_size(
        pos2(rect.right() - PAD - CHEVRON, rect.center().y - CHEVRON / 2.0),
        vec2(CHEVRON, CHEVRON),
    );
    stroke_path(painter, &CHEVRON_POINTS, chevron, 24.0, Stroke::new(1.25, brand::FAINT), false);
    let text_left = icon.right() + GAP;
    let text_room = chevron.left() - GAP - text_left;
    let galley = painter.layout_job(cut(label, text_room));
    painter.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, brand::SOFT);
    resp
}

fn cut(label: &str, room: f32) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
        FontId::proportional(LABEL_SIZE),
        brand::SOFT,
    );
    job.wrap = TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    job
}

fn stroke_path(
    painter: &Painter,
    points: &[(f32, f32)],
    into: Rect,
    view_box: f32,
    stroke: Stroke,
    closed: bool,
) {
    let scale = into.width() / view_box;
    let mapped: Vec<Pos2> =
        points.iter().map(|(x, y)| into.min + vec2(x * scale, y * scale)).collect();
    let shape = if closed {
        PathShape::closed_line(mapped, stroke)
    } else {
        PathShape::line(mapped, stroke)
    };
    painter.add(shape);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::RawInput;

    fn width_of(label: &str) -> f32 {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 100.0))),
            ..Default::default()
        };
        let mut width = 0.0;
        let _ = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| width = show(ui, label).rect.width());
        });
        width
    }

    #[test]
    fn the_chip_is_as_wide_for_a_short_name_as_for_a_long_one() {
        let long = "a-repository-with-a-very-long-name-indeed";
        assert_eq!(long.len(), 41);
        assert_eq!(width_of("web"), WIDTH);
        assert_eq!(width_of(long), WIDTH);
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis_inside_the_room() {
        let ctx = crate::fonts::test_context();
        let room = WIDTH - 2.0 * PAD - ICON - CHEVRON - 2.0 * GAP;
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            let long =
                ui.painter().layout_job(cut("a-repository-with-a-very-long-name-indeed", room));
            assert!(long.size().x <= room, "{} > {room}", long.size().x);
            assert!(long.elided);
            let short = ui.painter().layout_job(cut("web", room));
            assert!(!short.elided);
        });
    }
}
