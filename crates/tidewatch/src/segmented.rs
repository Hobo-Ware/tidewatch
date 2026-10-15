//! The segmented control of the window chrome: one bordered container, the active segment filled with a teal tint.

use egui::{Align2, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::brand;
use crate::screen_kit::body;

pub(crate) const HEIGHT: f32 = 28.0;
const EDGE: f32 = 3.0;
const GAP: f32 = 2.0;
const SEGMENT_PAD: f32 = 10.0;
const FONT: f32 = 12.5;

/// The container and then each segment, for text widths `text` and the control's right edge.
fn rects(right: f32, center_y: f32, text: &[f32]) -> (Rect, Vec<Rect>) {
    let widths: Vec<f32> = text.iter().map(|w| w + 2.0 * SEGMENT_PAD).collect();
    let inner = widths.iter().sum::<f32>() + GAP * text.len().saturating_sub(1) as f32;
    let width = inner + 2.0 * EDGE;
    let top = center_y - HEIGHT / 2.0;
    let container = Rect::from_min_size(pos2(right - width, top), vec2(width, HEIGHT));
    let mut x = container.left() + EDGE;
    let segments = widths
        .iter()
        .map(|w| {
            let rect = Rect::from_min_size(pos2(x, top + EDGE), vec2(*w, HEIGHT - 2.0 * EDGE));
            x += w + GAP;
            rect
        })
        .collect();
    (container, segments)
}

pub(crate) struct Output {
    /// The segment that was clicked and is not the active one.
    pub(crate) clicked: Option<usize>,
    /// The left edge of the drawn control.
    pub(crate) left: f32,
}

/// Draws the control with its right edge at `right`, centered on `center_y`.
pub(crate) fn show(
    ui: &mut Ui,
    id: Id,
    right: f32,
    center_y: f32,
    labels: &[&str],
    active: usize,
) -> Output {
    let galleys: Vec<_> = labels
        .iter()
        .map(|label| ui.painter().layout_no_wrap((*label).to_owned(), body(FONT), brand::SOFT))
        .collect();
    let text: Vec<f32> = galleys.iter().map(|g| g.size().x).collect();
    let (container, segments) = rects(right, center_y, &text);
    let painter = ui.painter().clone();
    painter.rect(
        container,
        CornerRadius::same(7),
        brand::SEGMENT,
        Stroke::new(1.0, brand::SEGMENT_LINE),
        StrokeKind::Inside,
    );
    let mut clicked = None;
    for (index, (rect, label)) in segments.into_iter().zip(labels).enumerate() {
        let response = ui.interact(rect, id.with(index), Sense::click());
        let on = index == active;
        if on {
            painter.rect_filled(rect, 5.0, brand::SEGMENT_ON);
        }
        let ink = match (on, response.hovered()) {
            (true, _) => brand::TEAL_TEXT,
            (false, true) => brand::TEXT,
            (false, false) => brand::SOFT,
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, *label, body(FONT), ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            clicked = Some(index);
        }
    }
    Output { clicked, left: container.left() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_sit_inside_the_container_with_a_gap_and_the_right_edge_where_asked() {
        let (container, segments) = rects(300.0, 20.0, &[50.0, 40.0]);
        assert_eq!(container.right(), 300.0);
        assert_eq!(container.height(), HEIGHT);
        assert_eq!(container.center().y, 20.0);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].width(), 50.0 + 2.0 * SEGMENT_PAD);
        assert_eq!(segments[1].left() - segments[0].right(), GAP);
        assert_eq!(segments[1].right() + EDGE, container.right());
        assert_eq!(segments[0].left() - EDGE, container.left());
    }

    #[test]
    fn one_segment_has_no_gap() {
        let (container, segments) = rects(100.0, 0.0, &[30.0]);
        assert_eq!(container.width(), 30.0 + 2.0 * SEGMENT_PAD + 2.0 * EDGE);
        assert_eq!(segments.len(), 1);
    }
}
