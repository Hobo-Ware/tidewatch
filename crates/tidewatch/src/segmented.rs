//! The segmented control of the window chrome: one bordered container, the active segment filled with a teal tint.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::brand;
use crate::screen_kit::{body, mono};

pub(crate) const HEIGHT: f32 = 28.0;
const EDGE: f32 = 3.0;
const GAP: f32 = 2.0;
const SEGMENT_PAD: f32 = 10.0;
const FONT: f32 = 12.5;
const CHIP_GAP: f32 = 6.0;
const GAP_ROWS: f32 = 6.0;
const KEY_GAP: f32 = 8.0;
const KEY_FONT: f32 = 11.0;

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

/// What each segment says: its label, optionally a key cap after it, optionally an ink of its own.
struct Look<'a> {
    labels: &'a [&'a str],
    keys: &'a [&'a str],
    tones: &'a [Option<Color32>],
}

fn paint_segments(
    ui: &mut Ui,
    id: Id,
    (container, segments): (Rect, Vec<Rect>),
    look: &Look,
    active: usize,
) -> Option<usize> {
    let Look { labels, keys, tones } = *look;
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
        let tone = tones.get(index).copied().flatten();
        let ink = match (on, response.hovered()) {
            (true, _) => tone.unwrap_or(brand::TEAL_TEXT),
            (false, true) => brand::TEXT,
            (false, false) => tone.unwrap_or(brand::SOFT),
        };
        match keys.get(index) {
            Some(key) => {
                let at = pos2(rect.left() + SEGMENT_PAD, rect.center().y);
                painter.text(at, Align2::LEFT_CENTER, *label, body(FONT), ink);
                let at = pos2(rect.right() - SEGMENT_PAD, rect.center().y);
                painter.text(at, Align2::RIGHT_CENTER, *key, mono(KEY_FONT), brand::FAINT);
            }
            None => {
                painter.text(rect.center(), Align2::CENTER_CENTER, *label, body(FONT), ink);
            }
        }
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            clicked = Some(index);
        }
    }
    clicked
}

fn text_widths(ui: &Ui, labels: &[&str]) -> Vec<f32> {
    labels
        .iter()
        .map(|label| {
            ui.painter().layout_no_wrap((*label).to_owned(), body(FONT), brand::SOFT).size().x
        })
        .collect()
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
    let placed = rects(right, center_y, &text_widths(ui, labels));
    let left = placed.0.left();
    let clicked = paint_segments(ui, id, placed, &Look { labels, keys: &[], tones: &[] }, active);
    Output { clicked, left }
}

/// `show` with a dim key cap after each label (`Agent  A`), for a choice a key also picks.
pub(crate) fn show_keyed(
    ui: &mut Ui,
    id: Id,
    right: f32,
    center_y: f32,
    choices: &[(&str, &str)],
    active: usize,
) -> Output {
    let labels: Vec<&str> = choices.iter().map(|(label, _)| *label).collect();
    let keys: Vec<&str> = choices.iter().map(|(_, key)| *key).collect();
    let widths: Vec<f32> = text_widths(ui, &labels)
        .into_iter()
        .zip(&keys)
        .map(|(label, key)| {
            let cap = ui.painter().layout_no_wrap((*key).to_owned(), mono(KEY_FONT), brand::FAINT);
            label + KEY_GAP + cap.size().x
        })
        .collect();
    let placed = rects(right, center_y, &widths);
    let left = placed.0.left();
    let clicked =
        paint_segments(ui, id, placed, &Look { labels: &labels, keys: &keys, tones: &[] }, active);
    Output { clicked, left }
}

/// The same control flowing in the current row; returns the segment clicked that is not the active one.
pub(crate) fn inline(ui: &mut Ui, id: Id, labels: &[&str], active: usize) -> Option<usize> {
    inline_toned(ui, id, labels, &[], active)
}

/// `inline` with an ink of its own for some segments, such as red for a dangerous choice.
pub(crate) fn inline_toned(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    active: usize,
) -> Option<usize> {
    let text = text_widths(ui, labels);
    let width = rects(0.0, 0.0, &text).0.width();
    let (slot, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let placed = rects(slot.right(), slot.center().y, &text);
    paint_segments(ui, id, placed, &Look { labels, keys: &[], tones }, active)
}

/// Where each chip of widths `widths` goes in rows of at most `avail`, and the height the rows take.
fn flow(widths: &[f32], avail: f32) -> (Vec<Rect>, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut placed = Vec::with_capacity(widths.len());
    for width in widths {
        if x > 0.0 && x + width > avail {
            x = 0.0;
            y += HEIGHT + GAP_ROWS;
        }
        placed.push(Rect::from_min_size(pos2(x, y), vec2(*width, HEIGHT)));
        x += width + CHIP_GAP;
    }
    (placed, if widths.is_empty() { 0.0 } else { y + HEIGHT })
}

/// Chips that wrap onto more rows, for a choice with more options than a segmented control holds.
pub(crate) fn wrap_chips(ui: &mut Ui, id: Id, labels: &[&str], active: usize) -> Option<usize> {
    chips(ui, id, labels, &[], active)
}

fn chip_widths(ui: &Ui, labels: &[&str]) -> Vec<f32> {
    text_widths(ui, labels).into_iter().map(|w| w + 2.0 * SEGMENT_PAD).collect()
}

fn fits_inline(ui: &Ui, labels: &[&str], avail: f32) -> bool {
    rects(0.0, 0.0, &text_widths(ui, labels)).0.width() <= avail
}

pub(crate) fn inline_or_chips_height(ui: &Ui, labels: &[&str], avail: f32) -> f32 {
    if fits_inline(ui, labels, avail) { HEIGHT } else { flow(&chip_widths(ui, labels), avail).1 }
}

/// The segmented control when it fits the row, else chips that wrap; `tones` give a segment its own ink.
pub(crate) fn inline_or_chips(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    active: usize,
) -> Option<usize> {
    if fits_inline(ui, labels, ui.available_width()) {
        inline_toned(ui, id, labels, tones, active)
    } else {
        chips(ui, id, labels, tones, active)
    }
}

fn chips(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    active: usize,
) -> Option<usize> {
    let widths = chip_widths(ui, labels);
    let (placed, height) = flow(&widths, ui.available_width());
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter().clone();
    let mut clicked = None;
    for (index, (offset, label)) in placed.into_iter().zip(labels).enumerate() {
        let rect = offset.translate(area.min.to_vec2());
        let response = ui.interact(rect, id.with(index), Sense::click());
        let on = index == active;
        let (fill, line) = if on {
            (brand::SEGMENT_ON, brand::SEGMENT_LINE)
        } else {
            (brand::SEGMENT, brand::SEGMENT_LINE)
        };
        painter.rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, line), StrokeKind::Inside);
        let tone = tones.get(index).copied().flatten();
        let ink = match (on, response.hovered()) {
            (true, _) => tone.unwrap_or(brand::TEAL_TEXT),
            (false, true) => brand::TEXT,
            (false, false) => tone.unwrap_or(brand::SOFT),
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, *label, body(FONT), ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            clicked = Some(index);
        }
    }
    clicked
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

    #[test]
    fn chips_wrap_to_a_new_row_when_the_next_one_does_not_fit() {
        let (placed, height) = flow(&[50.0, 50.0, 50.0], 120.0);
        assert_eq!(placed[1].left(), 50.0 + CHIP_GAP);
        assert_eq!((placed[2].left(), placed[2].top()), (0.0, HEIGHT + GAP_ROWS));
        assert_eq!(height, 2.0 * HEIGHT + GAP_ROWS);
    }

    #[test]
    fn a_chip_wider_than_the_row_still_gets_a_row_of_its_own() {
        let (placed, height) = flow(&[200.0, 30.0], 100.0);
        assert_eq!((placed[0].left(), placed[1].left()), (0.0, 0.0));
        assert_eq!(height, 2.0 * HEIGHT + GAP_ROWS);
        assert_eq!(flow(&[], 100.0).1, 0.0);
    }
}
