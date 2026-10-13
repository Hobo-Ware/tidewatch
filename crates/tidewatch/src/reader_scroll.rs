//! Scrolling rules of the Reader: whether the view follows new rows, and the row heights that let
//! it skip drawing rows that are off screen. Pure numbers, no egui.

use std::collections::HashMap;

use crate::reader_rows::{Kind, Row};

const AT_BOTTOM_PX: f32 = 8.0;
const LINE_H: f32 = 20.0;
const ROW_PAD: f32 = 14.0;
const TOOL_ROW_H: f32 = 26.0;
const CHAR_W: f32 = 7.0;
const MIN_COLUMNS: usize = 20;
const WIDTH_STEP: f32 = 1.0;

/// Whether the view sticks to the newest row. It follows until the user scrolls up, and follows again
/// once they reach the bottom or press "jump to latest".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Autoscroll {
    following: bool,
}

impl Default for Autoscroll {
    fn default() -> Self {
        Self { following: true }
    }
}

impl Autoscroll {
    pub(crate) fn following(self) -> bool {
        self.following
    }

    /// The user scrolled up or grabbed the scrollbar.
    pub(crate) fn release(&mut self) {
        self.following = false;
    }

    /// Looks at where the view ended up after a frame.
    pub(crate) fn settle(&mut self, offset: f32, max_offset: f32) {
        if max_offset - offset <= AT_BOTTOM_PX {
            self.following = true;
        }
    }

    pub(crate) fn jump_to_latest(&mut self) {
        self.following = true;
    }
}

/// Measured heights of drawn rows, kept per row and thrown away when the width changes (text wraps
/// differently). A row never drawn gets an estimate from its text.
#[derive(Debug, Default)]
pub(crate) struct Heights {
    width: f32,
    measured: HashMap<u64, f32>,
}

impl Heights {
    pub(crate) fn for_width(&mut self, width: f32) {
        if (width - self.width).abs() >= WIDTH_STEP {
            self.width = width;
            self.measured.clear();
        }
    }

    pub(crate) fn record(&mut self, seq: u64, height: f32) {
        self.measured.insert(seq, height);
    }

    pub(crate) fn height(&self, row: &Row) -> f32 {
        self.measured.get(&row.seq).copied().unwrap_or_else(|| estimate(row, self.width))
    }

    /// Forgets rows older than `first_seq`, which were dropped from the list.
    pub(crate) fn keep_from(&mut self, first_seq: u64) {
        self.measured.retain(|seq, _| *seq >= first_seq);
    }

    #[cfg(test)]
    pub(crate) fn measured_count(&self) -> usize {
        self.measured.len()
    }
}

/// A guess at a row's height: one line per wrapped line of text. Close enough to place the scroll
/// bar; the real height replaces it the first time the row is drawn.
pub(crate) fn estimate(row: &Row, width: f32) -> f32 {
    if matches!(row.kind, Kind::Tool(_)) {
        return TOOL_ROW_H;
    }
    let columns = ((width / CHAR_W) as usize).max(MIN_COLUMNS);
    let lines: usize =
        row.text.lines().map(|line| line.chars().count().div_ceil(columns).max(1)).sum();
    lines.max(1) as f32 * LINE_H + ROW_PAD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader_rows::Tool;

    fn row(seq: u64, text: &str) -> Row {
        Row { seq, kind: Kind::Assistant, text: text.into() }
    }

    #[test]
    fn a_new_view_follows() {
        assert!(Autoscroll::default().following());
    }

    #[test]
    fn scrolling_up_stops_following_and_reaching_the_bottom_resumes_it() {
        let mut scroll = Autoscroll::default();
        scroll.release();
        scroll.settle(100.0, 500.0);
        assert!(!scroll.following());
        scroll.settle(496.0, 500.0);
        assert!(scroll.following());
    }

    #[test]
    fn new_content_alone_does_not_resume_following() {
        let mut scroll = Autoscroll::default();
        scroll.release();
        scroll.settle(100.0, 100.0 + 400.0);
        scroll.settle(100.0, 100.0 + 900.0);
        assert!(!scroll.following());
    }

    #[test]
    fn jump_to_latest_follows_again() {
        let mut scroll = Autoscroll::default();
        scroll.release();
        scroll.jump_to_latest();
        assert!(scroll.following());
    }

    #[test]
    fn a_tool_row_is_one_line_and_text_rows_grow_with_their_lines() {
        let tool = Row {
            seq: 0,
            kind: Kind::Tool(Tool {
                label: "Bash".into(),
                output: String::new(),
                error: false,
                done: false,
            }),
            text: String::new(),
        };
        assert_eq!(estimate(&tool, 800.0), TOOL_ROW_H);
        let short = estimate(&row(1, "one"), 800.0);
        let long = estimate(&row(2, "a\nb\nc"), 800.0);
        assert_eq!(long - short, 2.0 * LINE_H);
    }

    #[test]
    fn long_lines_wrap_in_the_estimate() {
        let wide = estimate(&row(1, &"x".repeat(400)), 1400.0);
        let narrow = estimate(&row(1, &"x".repeat(400)), 280.0);
        assert!(narrow > wide);
    }

    #[test]
    fn a_measured_height_wins_until_the_width_changes() {
        let mut heights = Heights::default();
        heights.for_width(800.0);
        let r = row(7, "hello");
        let guess = heights.height(&r);
        heights.record(7, 99.0);
        assert_eq!(heights.height(&r), 99.0);
        heights.for_width(800.4);
        assert_eq!(heights.height(&r), 99.0);
        heights.for_width(640.0);
        assert_eq!(heights.measured_count(), 0);
        assert_ne!(heights.height(&r), 99.0);
        assert_eq!(heights.height(&r), estimate(&r, 640.0));
        assert_ne!(guess, 99.0);
    }

    #[test]
    fn dropped_rows_lose_their_heights() {
        let mut heights = Heights::default();
        heights.record(1, 10.0);
        heights.record(5, 10.0);
        heights.keep_from(3);
        assert_eq!(heights.measured_count(), 1);
    }
}
