//! The repo picker: the repo's name in 15 px semibold with a small chevron, no box until it is hovered or its menu is open.
//! Its width fits the widest repo name the window holds, up to `WIDTH`, so nothing in the strip moves when the repo
//! changes; a longer name is cut with an ellipsis.

use egui::epaint::PathShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{CornerRadius, Painter, Pos2, Rect, Response, Sense, Stroke, pos2, vec2};
use stdusk_egui::colors;

use super::TAB_H;
use crate::brand;

pub(super) const WIDTH: f32 = 130.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
const CHEVRON: f32 = 14.0;
const LABEL_SIZE: f32 = 15.0;
const RADIUS: u8 = 8;
const CHEVRON_POINTS: [(f32, f32); 3] = [(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)];

/// The chip's width for the widest name's text width: its padding, the gap and the chevron around it,
/// never past `WIDTH`.
fn fit(widest_text: f32) -> f32 {
    (widest_text + 2.0 * PAD + GAP + CHEVRON).min(WIDTH)
}

/// The width every repo's chip takes in this window: the same for all of them, set by the widest.
pub(super) fn width_for<'a>(ui: &egui::Ui, labels: impl Iterator<Item = &'a str>) -> f32 {
    let widest = labels
        .map(|label| {
            ui.painter()
                .layout_no_wrap(
                    label.to_owned(),
                    crate::screen_kit::heading(LABEL_SIZE),
                    brand::TEXT,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max);
    fit(widest)
}

pub(super) fn show(ui: &mut egui::Ui, label: &str, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TAB_H), Sense::click());
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let painter = ui.painter();
    let galley = painter.layout_job(cut(label, text_room(width)));
    let text_left = rect.left() + PAD;
    let chevron = Rect::from_min_size(
        pos2(text_left + galley.size().x + GAP, rect.center().y - CHEVRON / 2.0),
        vec2(CHEVRON, CHEVRON),
    );
    if resp.hovered() || menu_open {
        let tint = Rect::from_min_max(rect.min, pos2(chevron.right() + PAD, rect.max.y));
        painter.rect_filled(tint, CornerRadius::same(RADIUS), colors::hover());
    }
    painter.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    stroke_path(painter, &CHEVRON_POINTS, chevron, 24.0, Stroke::new(1.5, brand::MUTED));
    resp
}

fn text_room(width: f32) -> f32 {
    width - 2.0 * PAD - GAP - CHEVRON
}

fn cut(label: &str, room: f32) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
        crate::screen_kit::heading(LABEL_SIZE),
        brand::TEXT,
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
) {
    let scale = into.width() / view_box;
    let mapped: Vec<Pos2> =
        points.iter().map(|(x, y)| into.min + vec2(x * scale, y * scale)).collect();
    painter.add(PathShape::line(mapped, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::RawInput;

    const LONG: &str = "a-repository-with-a-very-long-name-indeed";

    fn widths(labels: &[&str], shown: &str) -> (f32, f32) {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 100.0))),
            ..Default::default()
        };
        let (mut fit, mut width) = (0.0, 0.0);
        let _ = ctx.run_ui(raw, |ui| {
            fit = width_for(ui, labels.iter().copied());
            ui.horizontal(|ui| width = show(ui, shown, fit).rect.width());
        });
        (fit, width)
    }

    #[test]
    fn the_chip_is_as_wide_as_its_widest_name_and_never_past_the_cap() {
        assert_eq!(LONG.len(), 41);
        let (short, _) = widths(&["web"], "web");
        let (mid, _) = widths(&["web", "tidewatch"], "web");
        let (long, _) = widths(&["web", LONG], "web");
        assert!(short < mid && mid < WIDTH, "{short} {mid}");
        assert_eq!(long, WIDTH);
    }

    #[test]
    fn every_repo_of_the_window_gets_the_same_width_so_switching_moves_nothing() {
        let labels = ["web", "tidewatch", "api"];
        let sizes: Vec<f32> = labels.iter().map(|shown| widths(&labels, shown).1).collect();
        assert_eq!(sizes[0], sizes[1]);
        assert_eq!(sizes[1], sizes[2]);
        assert!(sizes[0] < WIDTH);
    }

    #[test]
    fn the_fit_adds_padding_gap_and_chevron_and_caps_at_the_width() {
        let around = 2.0 * PAD + GAP + CHEVRON;
        let cases = [(0.0, around), (40.0, 40.0 + around), (500.0, WIDTH)];
        for (text, want) in cases {
            assert_eq!(fit(text), want, "{text}");
        }
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis_inside_the_room() {
        let ctx = crate::fonts::test_context();
        let room = text_room(WIDTH);
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
