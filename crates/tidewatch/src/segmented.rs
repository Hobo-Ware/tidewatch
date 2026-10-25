//! The segmented control of the window chrome: one bordered container, the active segment filled with a teal tint.

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

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
const BADGE_W: f32 = 24.0;
const BADGE_H: f32 = 16.0;
const BADGE_GAP: f32 = 6.0;
const BADGE_FONT: f32 = 11.0;
const BADGE_MOST: usize = 99;

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
    pub(crate) segments: Vec<Rect>,
}

/// What each segment says: its label, optionally a key cap after it, optionally an ink of its own,
/// optionally the reason it cannot be picked (the segment is dimmed and the reason is its tooltip).
struct Look<'a> {
    labels: &'a [&'a str],
    keys: &'a [&'a str],
    tones: &'a [Option<Color32>],
    refused: &'a [Option<String>],
    badges: &'a [Option<usize>],
}

fn refusal(refused: &[Option<String>], index: usize) -> Option<&str> {
    refused.get(index).and_then(Option::as_deref)
}

fn pointer_over(response: Response, reason: Option<&str>) -> Response {
    match reason {
        Some(reason) => {
            response.on_hover_text(reason).on_hover_cursor(egui::CursorIcon::NotAllowed)
        }
        None => response.on_hover_cursor(egui::CursorIcon::PointingHand),
    }
}

fn paint_segments(
    ui: &mut Ui,
    id: Id,
    (container, segments): (Rect, Vec<Rect>),
    look: &Look,
    active: usize,
) -> Option<usize> {
    let Look { labels, keys, tones, refused, badges } = *look;
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
        let reason = refusal(refused, index);
        let tone = tones.get(index).copied().flatten();
        let ink = match (on, reason.is_some(), response.hovered()) {
            (true, _, _) => tone.unwrap_or(brand::TEAL_TEXT),
            (false, true, _) => brand::FAINT,
            (false, false, true) => brand::TEXT,
            (false, false, false) => tone.unwrap_or(brand::SOFT),
        };
        let count = badges.get(index).copied().flatten().filter(|&count| count > 0);
        let key = keys.get(index);
        let (at, align) = label_anchor(rect, count.is_some() || key.is_some());
        painter.text(at, align, *label, body(FONT), ink);
        if let Some(count) = count {
            paint_badge(&painter, rect, count);
        } else if let Some(key) = key {
            let at = pos2(rect.right() - SEGMENT_PAD, rect.center().y);
            painter.text(at, Align2::RIGHT_CENTER, *key, mono(KEY_FONT), brand::FAINT);
        }
        if pointer_over(response, reason).clicked() && !on {
            clicked = Some(index);
        }
    }
    clicked
}

fn label_anchor(rect: Rect, shares: bool) -> (egui::Pos2, Align2) {
    if shares {
        (pos2(rect.left() + SEGMENT_PAD, rect.center().y), Align2::LEFT_CENTER)
    } else {
        (rect.center(), Align2::CENTER_CENTER)
    }
}

fn badge_label(count: usize) -> String {
    if count > BADGE_MOST { format!("{BADGE_MOST}+") } else { count.to_string() }
}

fn paint_badge(painter: &egui::Painter, segment: Rect, count: usize) {
    let pill = Rect::from_center_size(
        pos2(segment.right() - SEGMENT_PAD - BADGE_W / 2.0, segment.center().y),
        vec2(BADGE_W, BADGE_H),
    );
    painter.rect_filled(pill, CornerRadius::same(255), brand::TEAL.gamma_multiply(0.22));
    painter.text(
        pill.center(),
        Align2::CENTER_CENTER,
        badge_label(count),
        mono(BADGE_FONT),
        brand::TEAL_TEXT,
    );
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
    let segments = placed.1.clone();
    let clicked = paint_segments(
        ui,
        id,
        placed,
        &Look { labels, keys: &[], tones: &[], refused: &[], badges: &[] },
        active,
    );
    Output { clicked, left, segments }
}

/// `show` with a count pill after the label of every segment that has `Some` badge. The slot is
/// always as wide as the pill, so the control keeps its width whether the count is 0 or 99+.
pub(crate) fn show_badged(
    ui: &mut Ui,
    id: Id,
    right: f32,
    center_y: f32,
    (labels, badges): (&[&str], &[Option<usize>]),
    active: usize,
) -> Output {
    let widths: Vec<f32> = text_widths(ui, labels)
        .into_iter()
        .enumerate()
        .map(|(index, width)| match badges.get(index).copied().flatten() {
            Some(_) => width + BADGE_GAP + BADGE_W,
            None => width,
        })
        .collect();
    let placed = rects(right, center_y, &widths);
    let left = placed.0.left();
    let segments = placed.1.clone();
    let clicked = paint_segments(
        ui,
        id,
        placed,
        &Look { labels, keys: &[], tones: &[], refused: &[], badges },
        active,
    );
    Output { clicked, left, segments }
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
    let segments = placed.1.clone();
    let clicked = paint_segments(
        ui,
        id,
        placed,
        &Look { labels: &labels, keys: &keys, tones: &[], refused: &[], badges: &[] },
        active,
    );
    Output { clicked, left, segments }
}

/// The width of the control `inline` draws for `labels`.
pub(crate) fn inline_width(ui: &Ui, labels: &[&str]) -> f32 {
    rects(0.0, 0.0, &text_widths(ui, labels)).0.width()
}

/// The centre of each segment of a control with its right edge at `right`, for a headless test to aim at.
#[cfg(test)]
pub(crate) fn centers(ui: &Ui, labels: &[&str], right: f32, center_y: f32) -> Vec<egui::Pos2> {
    rects(right, center_y, &text_widths(ui, labels)).1.iter().map(Rect::center).collect()
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
    inline_gated(ui, id, labels, tones, &[], active)
}

fn inline_gated(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    refused: &[Option<String>],
    active: usize,
) -> Option<usize> {
    let text = text_widths(ui, labels);
    let width = rects(0.0, 0.0, &text).0.width();
    let (slot, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    let placed = rects(slot.right(), slot.center().y, &text);
    paint_segments(ui, id, placed, &Look { labels, keys: &[], tones, refused, badges: &[] }, active)
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
    chips(ui, id, labels, &[], &[], active)
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

/// The segmented control when it fits the row, else chips that wrap; `tones` give a segment its own
/// ink, and a segment with a `refused` reason is dimmed and says it on hover. A refused segment still
/// reports its click, so the caller can show the reason.
pub(crate) fn inline_or_chips(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    refused: &[Option<String>],
    active: usize,
) -> Option<usize> {
    if fits_inline(ui, labels, ui.available_width()) {
        inline_gated(ui, id, labels, tones, refused, active)
    } else {
        chips(ui, id, labels, tones, refused, active)
    }
}

fn chips(
    ui: &mut Ui,
    id: Id,
    labels: &[&str],
    tones: &[Option<Color32>],
    refused: &[Option<String>],
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
        let reason = refusal(refused, index);
        let tone = tones.get(index).copied().flatten();
        let ink = match (on, reason.is_some(), response.hovered()) {
            (true, _, _) => tone.unwrap_or(brand::TEAL_TEXT),
            (false, true, _) => brand::FAINT,
            (false, false, true) => brand::TEXT,
            (false, false, false) => tone.unwrap_or(brand::SOFT),
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, *label, body(FONT), ink);
        if pointer_over(response, reason).clicked() && !on {
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

    #[test]
    fn a_count_past_99_reads_99_plus() {
        let cases = [(1, "1"), (42, "42"), (99, "99"), (100, "99+"), (4000, "99+")];
        for (count, want) in cases {
            assert_eq!(badge_label(count), want, "{count}");
        }
    }

    #[test]
    fn a_segment_with_no_count_centres_its_label() {
        let rect = Rect::from_min_size(pos2(100.0, 0.0), vec2(120.0, 28.0));
        assert_eq!(label_anchor(rect, false), (rect.center(), Align2::CENTER_CENTER));
        assert_eq!(label_anchor(rect, true).1, Align2::LEFT_CENTER);
    }

    #[test]
    fn the_badge_slot_keeps_the_control_as_wide_with_no_count_as_with_a_big_one() {
        let ctx = crate::fonts::test_context();
        let mut lefts = Vec::new();
        for count in [0, 1, 99, 400] {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let out = show_badged(
                    ui,
                    Id::new("badged"),
                    400.0,
                    20.0,
                    (&["Progress", "Changes", "History"], &[None, Some(count), None]),
                    2,
                );
                lefts.push(out.left);
            });
        }
        assert!(lefts.windows(2).all(|pair| pair[0] == pair[1]), "{lefts:?}");
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let plain =
                show(ui, Id::new("plain"), 400.0, 20.0, &["Progress", "Changes", "History"], 2);
            assert!(lefts[0] < plain.left, "the slot makes room for the pill");
            assert_eq!(plain.left - lefts[0], BADGE_GAP + BADGE_W);
        });
    }
}
