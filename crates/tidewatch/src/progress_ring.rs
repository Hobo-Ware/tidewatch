//! The tone colors of the progress screens and the mark glyphs.

use std::f32::consts::TAU;

use egui::{Align2, Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::progress_model::Tone;
use crate::screen_kit::heading;

pub(crate) fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Working => brand::working(),
        Tone::NeedsYou => brand::needs(),
        Tone::Waiting => brand::waiting(),
        Tone::Blocked => brand::blocked(),
        Tone::Done => brand::done(),
    }
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
            painter.add(Shape::line(points, Stroke::new(1.7, brand::green())));
        }
        Mark::Doing => paint_spinner(painter, center, size * 0.8, time, brand::teal()),
        Mark::Skipped => {
            let half = vec2(0.4 * unit, 0.0);
            painter.line_segment([center - half, center + half], Stroke::new(1.5, brand::faint()));
        }
        Mark::Blocked => {
            painter.text(center, Align2::CENTER_CENTER, "!", heading(size), brand::needs());
        }
        Mark::Todo => {
            painter.circle_stroke(center, size * 0.3, Stroke::new(1.3, brand::faint()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tone_has_its_own_color() {
        let tones = [Tone::Working, Tone::Waiting, Tone::Blocked, Tone::Done];
        for (index, tone) in tones.iter().enumerate() {
            for other in &tones[index + 1..] {
                assert_ne!(tone_color(*tone), tone_color(*other));
            }
        }
        assert_eq!(tone_color(Tone::Blocked), brand::red());
    }
}
