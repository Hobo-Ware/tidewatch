//! The commit search of the History card: the query, the rows it matched, which row Enter goes to
//! next, and the text for the count on the right of the field. The matching itself is Kelp's
//! `search::matching_rows`; this only decides when to ask and what to do with the answer.

use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(180);

#[derive(Default)]
pub(crate) struct Search {
    pub(crate) query: String,
    answer: Option<Answer>,
    asked: Option<String>,
    edited: Option<Instant>,
    ticket: u64,
    pub(crate) want_focus: Option<Focus>,
    focused_frame: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Caret,
    SelectAll,
}

struct Answer {
    query: String,
    rows: Vec<usize>,
    lit: Vec<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    Nothing,
    Wait(Duration),
    Now { ticket: u64, query: String },
}

pub(crate) fn needle(query: &str) -> &str {
    query.trim()
}

/// The next (`delta` 1) or previous (`delta` -1) match from `current`, wrapping around the ends.
pub(crate) fn step(rows: &[usize], current: Option<usize>, delta: isize) -> Option<usize> {
    let forward = delta >= 0;
    let found = match (current, forward) {
        (Some(row), true) => rows.iter().copied().find(|r| *r > row),
        (Some(row), false) => rows.iter().rev().copied().find(|r| *r < row),
        (None, _) => None,
    };
    found.or_else(|| if forward { rows.first() } else { rows.last() }.copied())
}

pub(crate) fn count_text(matches: Option<usize>, position: Option<usize>) -> String {
    match (matches, position) {
        (None, _) => String::new(),
        (Some(0), _) => "No matches".to_owned(),
        (Some(total), Some(at)) => format!("{} of {total}", at + 1),
        (Some(1), None) => "1 match".to_owned(),
        (Some(total), None) => format!("{total} matches"),
    }
}

impl Search {
    pub(crate) fn drew(&mut self, frame: u64, focused: bool) {
        self.focused_frame = focused.then_some(frame);
    }

    pub(crate) fn was_focused(&self, frame: u64) -> bool {
        self.focused_frame.is_some_and(|drawn| frame <= drawn + 1)
    }

    pub(crate) fn edited(&mut self, now: Instant) {
        self.edited = Some(now);
        if needle(&self.query).is_empty() {
            self.clear_answer();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.query.clear();
        self.edited = None;
        self.clear_answer();
    }

    fn clear_answer(&mut self) {
        self.answer = None;
        self.asked = None;
        self.ticket += 1;
    }

    pub(crate) fn history_changed(&mut self) {
        self.asked = None;
    }

    pub(crate) fn ask(&mut self, now: Instant) -> Ask {
        let query = needle(&self.query).to_owned();
        if query.is_empty() || self.asked.as_deref() == Some(query.as_str()) {
            return Ask::Nothing;
        }
        if let Some(edited) = self.edited {
            let waited = now.saturating_duration_since(edited);
            if waited < SETTLE {
                return Ask::Wait(SETTLE.saturating_sub(waited));
            }
        }
        self.edited = None;
        self.ticket += 1;
        self.asked = Some(query.clone());
        Ask::Now { ticket: self.ticket, query }
    }

    pub(crate) fn answered(
        &mut self,
        ticket: u64,
        query: String,
        rows: Vec<usize>,
        history_len: usize,
    ) {
        if ticket != self.ticket {
            return;
        }
        let mut lit = vec![false; history_len];
        for row in &rows {
            if let Some(slot) = lit.get_mut(*row) {
                *slot = true;
            }
        }
        self.answer = Some(Answer { query, rows, lit });
    }

    pub(crate) fn lit(&self, history_len: usize) -> Option<&[bool]> {
        self.answer
            .as_ref()
            .filter(|a| a.query == needle(&self.query) && a.lit.len() == history_len)
            .map(|a| a.lit.as_slice())
    }

    pub(crate) fn rows(&self) -> &[usize] {
        self.answer.as_ref().map_or(&[], |a| a.rows.as_slice())
    }

    pub(crate) fn matches(&self) -> Option<usize> {
        self.answer.as_ref().filter(|a| a.query == needle(&self.query)).map(|a| a.rows.len())
    }

    pub(crate) fn position(&self, selected: Option<usize>) -> Option<usize> {
        let row = selected?;
        self.rows().binary_search(&row).ok()
    }

    pub(crate) fn highlight(&self) -> Option<&str> {
        Some(needle(&self.query)).filter(|text| !text.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_after(search: &mut Search, millis: u64) -> Ask {
        let start = Instant::now();
        search.edited(start);
        search.ask(start + Duration::from_millis(millis))
    }

    #[test]
    fn enter_goes_to_the_next_match_below_and_wraps_to_the_first() {
        let rows = [2, 5, 9];
        let cases = [
            (None, Some(2)),
            (Some(0), Some(2)),
            (Some(2), Some(5)),
            (Some(4), Some(5)),
            (Some(9), Some(2)),
            (Some(12), Some(2)),
        ];
        for (current, want) in cases {
            assert_eq!(step(&rows, current, 1), want, "from {current:?}");
        }
    }

    #[test]
    fn shift_enter_goes_to_the_match_above_and_wraps_to_the_last() {
        let rows = [2, 5, 9];
        let cases = [
            (None, Some(9)),
            (Some(12), Some(9)),
            (Some(9), Some(5)),
            (Some(6), Some(5)),
            (Some(2), Some(9)),
            (Some(0), Some(9)),
        ];
        for (current, want) in cases {
            assert_eq!(step(&rows, current, -1), want, "from {current:?}");
        }
    }

    #[test]
    fn no_matches_means_no_step() {
        assert_eq!(step(&[], Some(3), 1), None);
        assert_eq!(step(&[], None, -1), None);
    }

    #[test]
    fn the_count_reads_the_place_among_the_matches() {
        let cases = [
            (None, None, ""),
            (Some(0), None, "No matches"),
            (Some(0), Some(0), "No matches"),
            (Some(214), Some(2), "3 of 214"),
            (Some(214), None, "214 matches"),
            (Some(1), None, "1 match"),
            (Some(1), Some(0), "1 of 1"),
        ];
        for (matches, position, want) in cases {
            assert_eq!(count_text(matches, position), want, "{matches:?} {position:?}");
        }
    }

    #[test]
    fn typing_waits_for_a_pause_before_asking_and_asks_each_text_once() {
        let mut search = Search { query: "pars".into(), ..Search::default() };
        assert!(matches!(ask_after(&mut search, 50), Ask::Wait(_)));
        let start = Instant::now();
        search.edited(start);
        let Ask::Now { query, .. } = search.ask(start + SETTLE) else { panic!("no ask") };
        assert_eq!(query, "pars");
        assert_eq!(search.ask(start + SETTLE * 2), Ask::Nothing);
    }

    #[test]
    fn a_blank_query_asks_nothing_and_drops_the_answer() {
        let mut search = Search { query: "fix".into(), ..Search::default() };
        let start = Instant::now();
        search.edited(start);
        let Ask::Now { ticket, query } = search.ask(start + SETTLE) else { panic!("no ask") };
        search.answered(ticket, query, vec![1, 3], 5);
        assert_eq!(search.matches(), Some(2));
        search.query = "   ".into();
        search.edited(start);
        assert_eq!(search.ask(start + SETTLE), Ask::Nothing);
        assert_eq!((search.matches(), search.lit(5)), (None, None));
    }

    #[test]
    fn an_answer_to_an_older_text_is_dropped() {
        let mut search = Search { query: "fix".into(), ..Search::default() };
        let start = Instant::now();
        search.edited(start);
        let Ask::Now { ticket, query } = search.ask(start + SETTLE) else { panic!("no ask") };
        search.query = "fixe".into();
        search.edited(start + SETTLE);
        let Ask::Now { ticket: second, query: second_query } = search.ask(start + SETTLE * 2)
        else {
            panic!("no ask")
        };
        search.answered(ticket, query, vec![0], 3);
        assert_eq!(search.matches(), None);
        search.answered(second, second_query, vec![2], 3);
        assert_eq!((search.matches(), search.rows()), (Some(1), &[2][..]));
    }

    #[test]
    fn lit_marks_the_matched_rows_and_follows_the_history_length() {
        let mut search = Search { query: "fix".into(), ..Search::default() };
        let start = Instant::now();
        search.edited(start);
        let Ask::Now { ticket, query } = search.ask(start + SETTLE) else { panic!("no ask") };
        search.answered(ticket, query, vec![1, 3], 5);
        assert_eq!(search.lit(5), Some(&[false, true, false, true, false][..]));
        assert_eq!(search.lit(6), None, "a longer history needs a new answer");
        search.history_changed();
        assert!(matches!(search.ask(start + SETTLE * 2), Ask::Now { .. }));
    }

    #[test]
    fn the_place_among_the_matches_needs_the_selected_row_to_be_one() {
        let mut search = Search { query: "fix".into(), ..Search::default() };
        let start = Instant::now();
        search.edited(start);
        let Ask::Now { ticket, query } = search.ask(start + SETTLE) else { panic!("no ask") };
        search.answered(ticket, query, vec![1, 3, 8], 10);
        assert_eq!(search.position(Some(3)), Some(1));
        assert_eq!(search.position(Some(4)), None);
        assert_eq!(search.position(None), None);
    }

    #[test]
    fn the_field_counts_as_focused_only_for_the_frame_after_it_drew_focused() {
        let mut search = Search::default();
        assert!(!search.was_focused(5));
        search.drew(4, true);
        assert!(search.was_focused(5));
        assert!(!search.was_focused(7), "the card stopped drawing it");
        search.drew(5, false);
        assert!(!search.was_focused(6));
    }

    #[test]
    fn the_highlight_is_the_trimmed_query_and_nothing_for_a_blank_one() {
        let mut search = Search { query: "  Fix ".into(), ..Search::default() };
        assert_eq!(search.highlight(), Some("Fix"));
        search.clear();
        assert_eq!(search.highlight(), None);
    }
}
