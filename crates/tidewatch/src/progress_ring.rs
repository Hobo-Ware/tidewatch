//! The progress ring (an arc from the top, clockwise, over a faint track) and the mark glyphs.

use std::f32::consts::{PI, TAU};

use egui::{Align2, Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::progress_model::Tone;
use crate::screen_kit::heading;

const STEPS_PER_TURN: f32 = 72.0;

pub(crate) fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Working => brand::WORKING,
        Tone::Waiting => brand::AMBER,
        Tone::Blocked => brand::RED,
        Tone::Done => brand::DONE,
    }
}

/// The stroke of a ring `size` across: a tenth of it, never thinner than 2 px.
pub(crate) fn stroke_of(size: f32) -> f32 {
    (size * 0.105).max(2.0)
}

/// Points of the arc that starts at the top and runs clockwise over `fraction` of a turn. Empty
/// for nothing; a full turn ends where it began.
pub(crate) fn arc_points(center: Pos2, radius: f32, fraction: f32) -> Vec<Pos2> {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return Vec::new();
    }
    let steps = ((STEPS_PER_TURN * fraction).ceil() as usize).max(2);
    (0..=steps)
        .map(|step| {
            let angle = -PI / 2.0 + TAU * fraction * step as f32 / steps as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// Draws a ring of outer diameter `size` centered on `center`. Nothing at all for a fraction of
/// zero: an empty ring says nothing.
pub(crate) fn paint(
    painter: &Painter,
    center: Pos2,
    size: f32,
    fraction: f32,
    color: Color32,
    track: Color32,
) {
    if fraction <= 0.0 {
        return;
    }
    let stroke = stroke_of(size);
    let radius = (size - stroke) / 2.0;
    painter.circle_stroke(center, radius, Stroke::new(stroke, track));
    let points = arc_points(center, radius, fraction);
    let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else {
        return;
    };
    painter.add(Shape::line(points, Stroke::new(stroke, color)));
    painter.circle_filled(first, stroke / 2.0, color);
    painter.circle_filled(last, stroke / 2.0, color);
}

/// The small dot of an item being worked on: a ring with a gap that turns with `time`.
pub(crate) fn paint_spinner(painter: &Painter, center: Pos2, size: f32, time: f64, color: Color32) {
    let stroke = (size * 0.16).max(1.5);
    let radius = (size - stroke) / 2.0;
    let turn = (time * 0.9).rem_euclid(1.0) as f32;
    let points: Vec<Pos2> = (0..=24)
        .map(|step| {
            let angle = TAU * (turn + 0.7 * step as f32 / 24.0);
            pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin())
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(stroke, color)));
}

/// A solid dot with a soft halo, the mark of what is going on right now.
pub(crate) fn paint_dot(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    painter.circle_filled(center, size / 2.0 + 4.0, color.gamma_multiply(0.18));
    painter.circle_filled(center, size / 2.0, color);
}

/// The glyph of a checklist mark, painted in a box of `size` around `center`: a tick, the turning
/// dot, a dash, `!` or an empty circle.
pub(crate) fn paint_mark(painter: &Painter, center: Pos2, size: f32, mark: Mark, time: f64) {
    let unit = size / 2.0;
    match mark {
        Mark::Done => {
            let points = vec![
                center + vec2(-0.45, 0.0) * unit,
                center + vec2(-0.1, 0.4) * unit,
                center + vec2(0.55, -0.4) * unit,
            ];
            painter.add(Shape::line(points, Stroke::new(1.7, brand::GREEN)));
        }
        Mark::Doing => paint_spinner(painter, center, size * 0.8, time, brand::TEAL),
        Mark::Skipped => {
            let half = vec2(0.4 * unit, 0.0);
            painter.line_segment([center - half, center + half], Stroke::new(1.5, brand::FAINT));
        }
        Mark::Blocked => {
            painter.text(center, Align2::CENTER_CENTER, "!", heading(size), brand::RED);
        }
        Mark::Todo => {
            painter.circle_stroke(center, size * 0.3, Stroke::new(1.3, brand::FAINT));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Pos2 = pos2(10.0, 10.0);

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 0.01
    }

    #[test]
    fn the_arc_starts_at_the_top_and_goes_clockwise() {
        let quarter = arc_points(C, 5.0, 0.25);
        assert!(near(quarter[0], pos2(10.0, 5.0)));
        assert!(near(*quarter.last().unwrap(), pos2(15.0, 10.0)));
        let half = arc_points(C, 5.0, 0.5);
        assert!(near(*half.last().unwrap(), pos2(10.0, 15.0)));
    }

    #[test]
    fn a_full_turn_closes_and_nothing_draws_nothing() {
        let full = arc_points(C, 5.0, 1.0);
        assert!(near(full[0], *full.last().unwrap()));
        assert_eq!(arc_points(C, 5.0, 0.0), Vec::<Pos2>::new());
        assert_eq!(arc_points(C, 5.0, -3.0), Vec::<Pos2>::new());
        assert!(near(*arc_points(C, 5.0, 7.0).last().unwrap(), full[0]));
    }

    #[test]
    fn a_ring_at_zero_draws_no_track_and_no_arc() {
        let ctx = egui::Context::default();
        let drawn = |fraction: f32| {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(ui.painter(), C, 20.0, fraction, Color32::WHITE, Color32::GRAY);
            });
            output.shapes.len()
        };
        let (empty, started) = (drawn(0.0), drawn(0.3));
        assert!(started > empty, "{started} shapes at 0.3, {empty} at 0");
        assert_eq!(drawn(-1.0), empty);
    }

    #[test]
    fn a_longer_arc_has_more_points_and_every_point_is_on_the_circle() {
        let (short, long) = (arc_points(C, 5.0, 0.1), arc_points(C, 5.0, 0.9));
        assert!(long.len() > short.len());
        for point in long {
            assert!(((point - C).length() - 5.0).abs() < 0.01);
        }
    }

    #[test]
    fn the_stroke_scales_with_the_ring_and_has_a_floor() {
        assert_eq!(stroke_of(10.0), 2.0);
        assert!((stroke_of(72.0) - 7.56).abs() < 0.01);
        assert!(stroke_of(52.0) > stroke_of(26.0));
    }

    #[test]
    fn each_tone_has_its_own_color() {
        let tones = [Tone::Working, Tone::Waiting, Tone::Blocked, Tone::Done];
        for (index, tone) in tones.iter().enumerate() {
            for other in &tones[index + 1..] {
                assert_ne!(tone_color(*tone), tone_color(*other));
            }
        }
        assert_eq!(tone_color(Tone::Blocked), brand::RED);
    }
}
