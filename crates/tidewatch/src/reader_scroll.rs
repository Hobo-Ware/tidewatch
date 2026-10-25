//! Scrolling rules of the Reader: whether the view follows new rows, and the row heights that let
//! it skip drawing rows that are off screen. Pure numbers, no egui.

use std::collections::HashMap;

use crate::reader_rows::{Kind, Row};

const AT_BOTTOM_PX: f32 = 8.0;
const LINE_H: f32 = 20.0;
const ROW_PAD: f32 = 14.0;
const TOOL_ROW_H: f32 = 26.0;
const REVIEW_BASE_H: f32 = 90.0;
const REVIEW_NOTE_H: f32 = 70.0;
const CHAR_W: f32 = 7.0;
const MIN_COLUMNS: usize = 20;
const WIDTH_STEP: f32 = 1.0;
const CHAT_LINE_H: f32 = 22.0;
const CHAT_CHAR_W: f32 = 8.0;
const USER_SHARE: f32 = 0.62;
const AGENT_SHARE: f32 = 0.82;
const BUBBLE_PAD: f32 = 34.0;
const USER_CHROME_H: f32 = 64.0;
const TURN_CHROME_H: f32 = 70.0;
const TURN_CHIPS_H: f32 = 34.0;
const TURN_LIVE_H: f32 = 40.0;
const TURN_LIST_LINE_H: f32 = 22.0;
const TURN_LIST_PAD_H: f32 = 22.0;
const TURN_NOTES_LINE_H: f32 = 22.0;

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

/// Whether the view was not drawn in the pass before `pass`: it was behind another tab or screen
/// and has just come back (or is drawn for the first time).
pub(crate) fn returned(last_pass: Option<u64>, pass: u64) -> bool {
    last_pass.is_none_or(|last| pass > last.saturating_add(1))
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

    /// The height of the item keyed `key`: measured, else `guess`, worked out only when needed.
    pub(crate) fn height_with(&self, key: u64, guess: impl FnOnce(f32) -> f32) -> f32 {
        self.measured.get(&key).copied().unwrap_or_else(|| guess(self.width))
    }

    /// Forgets rows older than `first_seq`, which were dropped from the list.
    pub(crate) fn keep_from(&mut self, first_seq: u64) {
        self.measured.retain(|seq, _| *seq >= first_seq);
    }

    #[cfg(test)]
    pub(crate) fn measured_count(&self) -> usize {
        self.measured.len()
    }

    #[cfg(test)]
    pub(crate) fn measured(&self, key: u64) -> Option<f32> {
        self.measured.get(&key).copied()
    }
}

/// A guess at a row's height: one line per wrapped line of text. Close enough to place the scroll
/// bar; the real height replaces it the first time the row is drawn.
pub(crate) fn estimate(row: &Row, width: f32) -> f32 {
    if matches!(row.kind, Kind::Tool(_)) {
        return TOOL_ROW_H;
    }
    if let Kind::Review(ids) = &row.kind {
        return REVIEW_BASE_H + ids.len() as f32 * REVIEW_NOTE_H;
    }
    if matches!(row.kind, Kind::User) {
        return estimate_user(&row.text, width);
    }
    let columns = ((width / CHAR_W) as usize).max(MIN_COLUMNS);
    let lines: usize =
        row.text.lines().map(|line| line.chars().count().div_ceil(columns).max(1)).sum();
    lines.max(1) as f32 * LINE_H + ROW_PAD
}

fn chat_lines<'a>(texts: impl IntoIterator<Item = &'a str>, bubble_w: f32) -> usize {
    let columns = (((bubble_w - BUBBLE_PAD) / CHAT_CHAR_W) as usize).max(MIN_COLUMNS);
    texts
        .into_iter()
        .map(|text| {
            text.lines().map(|line| line.chars().count().div_ceil(columns).max(1)).sum::<usize>()
        })
        .sum()
}

/// A guess at a user bubble: its wrapped lines, the name line over it and the gaps around it.
pub(crate) fn estimate_user(text: &str, width: f32) -> f32 {
    let lines = chat_lines([text], width * USER_SHARE).max(1);
    lines as f32 * CHAT_LINE_H + USER_CHROME_H
}

/// A guess at an agent turn: the wrapped lines of its answer (none while it only runs tools), its
/// chip row or live row, its open list of `calls` lines, and the updates chip over the bubble when
/// `notes` has texts (with their lines when that list is open).
pub(crate) fn estimate_turn(
    texts: &[&str],
    calls: usize,
    (live, listing): (bool, bool),
    (notes, notes_listing): (&[&str], bool),
    width: f32,
) -> f32 {
    let bubble_w = width * AGENT_SHARE - 44.0;
    let notes_h = match (notes.is_empty(), notes_listing) {
        (true, _) => 0.0,
        (false, false) => TURN_CHIPS_H,
        (false, true) => {
            let lines = chat_lines(notes.iter().copied(), bubble_w);
            TURN_CHIPS_H + lines as f32 * TURN_NOTES_LINE_H + TURN_LIST_PAD_H
        }
    };
    let text_h = if texts.is_empty() {
        0.0
    } else {
        chat_lines(texts.iter().copied(), bubble_w) as f32 * CHAT_LINE_H + 24.0
    };
    let under = match (live, calls) {
        (true, _) => TURN_LIVE_H,
        (false, 0) => 0.0,
        (false, _) => TURN_CHIPS_H,
    };
    let list_h = if listing { calls as f32 * TURN_LIST_LINE_H + TURN_LIST_PAD_H } else { 0.0 };
    let chrome = if texts.is_empty() && live { 20.0 } else { TURN_CHROME_H };
    text_h + under + list_h + notes_h + chrome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader_rows::Tool;

    fn row(seq: u64, text: &str) -> Row {
        Row { seq, kind: Kind::Assistant, text: text.into() }
    }

    #[test]
    fn a_view_has_returned_when_a_pass_went_by_without_drawing_it() {
        let cases = [
            (None, 7, true),
            (Some(6), 7, false),
            (Some(7), 7, false),
            (Some(5), 7, true),
            (Some(0), 40, true),
            (Some(u64::MAX), u64::MAX, false),
        ];
        for (last, pass, want) in cases {
            assert_eq!(returned(last, pass), want, "last {last:?} pass {pass}");
        }
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
                kind: crate::reader_rows::ToolKind::Command,
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
    fn a_review_row_is_guessed_by_its_notes_not_by_its_message() {
        let review =
            |ids: Vec<u64>| Row { seq: 1, kind: Kind::Review(ids), text: "a\nb\nc".into() };
        let two = estimate(&review(vec![1, 2]), 800.0);
        let four = estimate(&review(vec![1, 2, 3, 4]), 800.0);
        assert_eq!(four - two, 2.0 * REVIEW_NOTE_H);
    }

    #[test]
    fn a_user_bubble_and_a_turn_grow_with_their_lines_and_a_list_with_its_calls() {
        let one = estimate_user("one", 800.0);
        let three = estimate_user("a\nb\nc", 800.0);
        assert_eq!(three - one, 2.0 * CHAT_LINE_H);
        let closed = estimate_turn(&["done"], 6, (false, false), (&[], false), 800.0);
        let open = estimate_turn(&["done"], 6, (false, true), (&[], false), 800.0);
        assert_eq!(open - closed, 6.0 * TURN_LIST_LINE_H + TURN_LIST_PAD_H);
        let bare = estimate_turn(&["done"], 0, (false, false), (&[], false), 800.0);
        assert_eq!(closed - bare, TURN_CHIPS_H);
        assert!(estimate_turn(&[], 2, (true, false), (&[], false), 800.0) < closed);
    }

    #[test]
    fn folded_updates_add_a_chip_row_and_their_lines_only_when_open() {
        let notes = ["one", "two", "three"];
        let none = estimate_turn(&["done"], 0, (false, false), (&[], false), 800.0);
        let folded = estimate_turn(&["done"], 0, (false, false), (&notes, false), 800.0);
        let open = estimate_turn(&["done"], 0, (false, false), (&notes, true), 800.0);
        assert_eq!(folded - none, TURN_CHIPS_H);
        assert_eq!(open - folded, 3.0 * TURN_NOTES_LINE_H + TURN_LIST_PAD_H);
    }

    #[test]
    fn a_measured_turn_height_wins_over_its_guess() {
        let mut heights = Heights::default();
        heights.for_width(800.0);
        assert_eq!(heights.height_with(9, |_| 123.0), 123.0);
        heights.record(9, 77.0);
        assert_eq!(heights.height_with(9, |_| 123.0), 77.0);
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
