//! Reads Claude Code's next-prompt suggestion off its terminal. Claude draws the suggestion as the
//! placeholder of its empty input line: the first character as the inverse cursor, the rest faint
//! (SGR 2), after the `>` prompt, between the top and bottom border of the input box. The parser is
//! pure and returns nothing for any screen that does not look exactly like that, so typed text,
//! history lines and other CLIs never produce a ghost. `Ghost` keeps the last read per tab and
//! looks at the screen again only when the terminal printed something new.

use stdusk_core::terminal::{CellSnap, GridSnap};

use crate::outbox::Screen;

const PROMPTS: [char; 3] = ['>', '\u{276f}', '\u{203a}'];
const RULES: [char; 3] = ['\u{2500}', '\u{2501}', '\u{2550}'];
const RULE_STARTS: [char; 7] =
    ['\u{2500}', '\u{2501}', '\u{2550}', '\u{256d}', '\u{2570}', '\u{250c}', '\u{2514}'];
const SIDES: [char; 2] = ['\u{2502}', '\u{2503}'];
const MIN_RULE: usize = 8;
const MAX_CHARS: usize = 400;

/// What the input box of Claude Code suggests typing next, when the screen shows one.
pub(crate) fn read(snap: &GridSnap) -> Option<String> {
    let cols = snap.cols;
    if cols == 0 || snap.cells.len() < cols * 2 {
        return None;
    }
    let rows: Vec<&[CellSnap]> = snap.cells.chunks(cols).collect();
    let (at, start) = (1..rows.len()).rev().find_map(|at| {
        (is_rule(rows[at - 1])).then(|| prompt_start(rows[at]).map(|s| (at, s)))?
    })?;
    let mut parts = vec![first_line(&rows[at][start..])?];
    let mut closed = false;
    for row in &rows[at + 1..] {
        if is_rule(row) {
            closed = true;
            break;
        }
        let more = faint_text(row, false)?;
        if !more.is_empty() {
            parts.push(more);
        }
    }
    let text = parts.join(" ");
    (closed && !text.is_empty() && text.chars().count() <= MAX_CHARS).then_some(text)
}

/// Whether Claude Code's input box is free to take a message: `Free` when it is empty or holds only
/// its suggestion, `Typed` when something loud sits in it (a draft the user began), `NoBox` when the
/// screen shows no input box (a dialog such as `/model` is open over it).
pub(crate) fn screen_of(snap: &GridSnap) -> Screen {
    let cols = snap.cols;
    if cols == 0 || snap.cells.len() < cols * 2 {
        return Screen::NoBox;
    }
    let rows: Vec<&[CellSnap]> = snap.cells.chunks(cols).collect();
    let found = (1..rows.len())
        .rev()
        .find_map(|at| is_rule(rows[at - 1]).then(|| prompt_start(rows[at]).map(|s| (at, s)))?);
    let Some((at, start)) = found else { return Screen::NoBox };
    let Some(close) = (at + 1..rows.len()).find(|row| is_rule(rows[*row])) else {
        return Screen::NoBox;
    };
    let first = faint_text(&rows[at][start..], true);
    let more = rows[at + 1..close].iter().map(|row| faint_text(row, false));
    if first.is_none() || more.into_iter().any(|row| row.is_none()) {
        Screen::Typed
    } else {
        Screen::Free
    }
}

fn text_of(row: &[CellSnap]) -> impl Iterator<Item = &CellSnap> {
    row.iter().filter(|cell| cell.c != '\0')
}

fn first_non_space(row: &[CellSnap]) -> Option<usize> {
    row.iter().position(|cell| cell.c != ' ' && cell.c != '\0')
}

fn is_rule(row: &[CellSnap]) -> bool {
    let starts = first_non_space(row).is_some_and(|at| RULE_STARTS.contains(&row[at].c));
    starts && text_of(row).filter(|cell| RULES.contains(&cell.c)).count() >= MIN_RULE
}

fn prompt_start(row: &[CellSnap]) -> Option<usize> {
    let mut at = first_non_space(row)?;
    if SIDES.contains(&row[at].c) {
        at += 1 + row.get(at + 1..)?.iter().position(|cell| cell.c != ' ')?;
    }
    (PROMPTS.contains(&row[at].c) && row.get(at + 1)?.c == ' ').then_some(at + 2)
}

fn trimmed(row: &[CellSnap]) -> Vec<&CellSnap> {
    let mut cells: Vec<&CellSnap> = text_of(row).collect();
    while cells.last().is_some_and(|cell| cell.c == ' ' || SIDES.contains(&cell.c)) {
        cells.pop();
    }
    cells
}

fn is_cursor(cell: &CellSnap, next: Option<&&CellSnap>) -> bool {
    !cell.dim && cell.bg.is_some() && cell.c != ' ' && next.is_some_and(|next| next.dim)
}

fn first_line(content: &[CellSnap]) -> Option<String> {
    let cells = trimmed(content);
    let first = cells.first()?;
    if !first.dim && !is_cursor(first, cells.get(1)) {
        return None;
    }
    faint_text(content, true)
}

/// The faint text of a row, empty for a blank row, `None` when anything on it is not faint. The first
/// character of the first row may be the inverse cursor.
fn faint_text(row: &[CellSnap], first_row: bool) -> Option<String> {
    let cells = trimmed(row);
    if cells.is_empty() {
        return Some(String::new());
    }
    let from = usize::from(first_row && is_cursor(cells[0], cells.get(1)));
    let loud = cells[from..].iter().any(|cell| cell.c != ' ' && !cell.dim);
    if loud {
        return None;
    }
    let text: String = cells.iter().map(|cell| cell.c).collect();
    Some(text.trim().to_owned())
}

/// The suggestion last read from a tab's terminal, and the output count it was read at.
#[derive(Default)]
pub(crate) struct Ghost {
    text: Option<String>,
    seen: Option<u64>,
}

impl Ghost {
    pub(crate) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Reads the screen unless `output` is the count it was last read at. `screen` is only called
    /// when it is needed.
    pub(crate) fn refresh(&mut self, output: u64, screen: impl FnOnce() -> GridSnap) {
        if self.seen == Some(output) {
            return;
        }
        self.seen = Some(output);
        self.text = read(&screen());
    }

    /// Forgets the suggestion, so the next `refresh` reads the screen again.
    pub(crate) fn clear(&mut self) {
        self.text = None;
        self.seen = None;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use egui::Color32;

    use super::*;

    const COLS: usize = 40;

    #[derive(Clone, Copy, PartialEq)]
    enum Ink {
        Plain,
        Faint,
        Cursor,
    }

    fn cell(c: char, ink: Ink) -> CellSnap {
        CellSnap {
            c,
            fg: Color32::WHITE,
            bg: (ink == Ink::Cursor).then_some(Color32::LIGHT_GRAY),
            selected: false,
            wide: false,
            bold: false,
            dim: ink == Ink::Faint,
        }
    }

    fn row(parts: &[(&str, Ink)]) -> Vec<CellSnap> {
        let mut cells: Vec<CellSnap> =
            parts.iter().flat_map(|(text, ink)| text.chars().map(|c| cell(c, *ink))).collect();
        cells.resize_with(COLS, || cell(' ', Ink::Plain));
        cells
    }

    fn screen(rows: &[Vec<CellSnap>]) -> GridSnap {
        GridSnap {
            cols: COLS,
            rows: rows.len(),
            cells: rows.iter().flatten().map(|c| CellSnap { ..*c }).collect(),
            cursor: None,
            top_line: 0,
        }
    }

    fn rule() -> Vec<CellSnap> {
        row(&[(&"\u{2500}".repeat(COLS), Ink::Plain)])
    }

    fn boxed(line: Vec<CellSnap>) -> GridSnap {
        screen(&[row(&[("Done. All tests pass.", Ink::Plain)]), rule(), line, rule(), row(&[])])
    }

    fn suggestion(line: Vec<CellSnap>) -> Option<String> {
        read(&boxed(line))
    }

    #[test]
    fn the_faint_run_after_the_prompt_is_the_suggestion() {
        let cases: [(&str, Vec<CellSnap>, Option<&str>); 9] = [
            (
                "faint after the prompt, terminal not focused",
                row(&[("> ", Ink::Plain), ("run the tests", Ink::Faint)]),
                Some("run the tests"),
            ),
            (
                "cursor on the first letter",
                row(&[("> ", Ink::Plain), ("r", Ink::Cursor), ("un the tests", Ink::Faint)]),
                Some("run the tests"),
            ),
            (
                "the pointer glyph",
                row(&[("\u{276f} ", Ink::Plain), ("commit this", Ink::Faint)]),
                Some("commit this"),
            ),
            (
                "an empty box has no suggestion",
                row(&[("> ", Ink::Plain), (" ", Ink::Cursor)]),
                None,
            ),
            (
                "typed text is never a suggestion",
                row(&[("> ", Ink::Plain), ("fix the bug", Ink::Plain)]),
                None,
            ),
            (
                "typed text with the cursor behind it",
                row(&[("> ", Ink::Plain), ("fix", Ink::Plain), (" ", Ink::Cursor)]),
                None,
            ),
            (
                "faint text followed by loud text",
                row(&[("> ", Ink::Plain), ("run", Ink::Faint), (" it", Ink::Plain)]),
                None,
            ),
            (
                "a bare cursor letter with nothing faint behind it",
                row(&[("> ", Ink::Plain), ("r", Ink::Cursor)]),
                None,
            ),
            (
                "a side border around the box",
                row(&[("\u{2502} > ", Ink::Plain), ("go on", Ink::Faint)]),
                Some("go on"),
            ),
        ];
        for (name, line, want) in cases {
            assert_eq!(suggestion(line).as_deref(), want, "{name}");
        }
    }

    #[test]
    fn the_input_box_is_free_typed_or_covered() {
        let cases = [
            ("an empty box", boxed(row(&[("> ", Ink::Plain), (" ", Ink::Cursor)])), Screen::Free),
            (
                "only its suggestion",
                boxed(row(&[("> ", Ink::Plain), ("r", Ink::Cursor), ("un it", Ink::Faint)])),
                Screen::Free,
            ),
            (
                "a draft the user began",
                boxed(row(&[("> ", Ink::Plain), ("fix the", Ink::Plain), (" ", Ink::Cursor)])),
                Screen::Typed,
            ),
            (
                "one typed letter under the cursor",
                boxed(row(&[("> ", Ink::Plain), ("r", Ink::Cursor)])),
                Screen::Typed,
            ),
            (
                "a draft on the second line",
                screen(&[
                    rule(),
                    row(&[("> ", Ink::Plain), ("run the", Ink::Faint)]),
                    row(&[("  ", Ink::Plain), ("suite", Ink::Plain)]),
                    rule(),
                ]),
                Screen::Typed,
            ),
            (
                "a menu instead of the box",
                screen(&[
                    row(&[("Select model", Ink::Plain)]),
                    row(&[("  1. Default", Ink::Plain)]),
                    row(&[("  2. Opus", Ink::Plain)]),
                ]),
                Screen::NoBox,
            ),
            ("a screen with nothing on it", screen(&[]), Screen::NoBox),
        ];
        for (name, snap, want) in cases {
            assert_eq!(screen_of(&snap), want, "{name}");
        }
    }

    #[test]
    fn a_suggestion_that_wraps_is_joined_with_a_space() {
        let snap = screen(&[
            rule(),
            row(&[("> ", Ink::Plain), ("r", Ink::Cursor), ("un the whole", Ink::Faint)]),
            row(&[("  ", Ink::Plain), ("test suite", Ink::Faint)]),
            rule(),
        ]);
        assert_eq!(read(&snap).as_deref(), Some("run the whole test suite"));
    }

    #[test]
    fn a_wrapped_line_with_loud_text_is_nothing() {
        let snap = screen(&[
            rule(),
            row(&[("> ", Ink::Plain), ("run the", Ink::Faint)]),
            row(&[("  ", Ink::Plain), ("suite", Ink::Plain)]),
            rule(),
        ]);
        assert_eq!(read(&snap), None);
    }

    #[test]
    fn a_prompt_line_outside_a_box_is_nothing() {
        let faint = || row(&[("> ", Ink::Plain), ("run the tests", Ink::Faint)]);
        let no_top = screen(&[row(&[("hello", Ink::Plain)]), faint(), rule()]);
        let no_bottom = screen(&[rule(), faint(), row(&[("footer", Ink::Plain)])]);
        let last_row = screen(&[rule(), faint()]);
        assert_eq!(read(&no_top), None);
        assert_eq!(read(&no_bottom), None);
        assert_eq!(read(&last_row), None);
    }

    #[test]
    fn the_input_box_at_the_bottom_wins_over_a_quoted_prompt_in_the_history() {
        let snap = screen(&[
            rule(),
            row(&[("> ", Ink::Plain), ("old suggestion", Ink::Faint)]),
            rule(),
            row(&[("reply", Ink::Plain)]),
            rule(),
            row(&[("> ", Ink::Plain), ("r", Ink::Cursor), ("un it", Ink::Faint)]),
            rule(),
        ]);
        assert_eq!(read(&snap).as_deref(), Some("run it"));
    }

    #[test]
    fn a_rule_with_a_label_still_closes_the_box() {
        let mut top = row(&[("\u{2500}\u{2500} claude ", Ink::Plain)]);
        top.truncate(COLS - 12);
        top.extend(rule().into_iter().take(12));
        let snap = screen(&[top, row(&[("> ", Ink::Plain), ("ship it", Ink::Faint)]), rule()]);
        assert_eq!(read(&snap).as_deref(), Some("ship it"));
    }

    #[test]
    fn an_empty_or_degenerate_grid_is_nothing() {
        let empty = GridSnap { cols: 0, rows: 0, cells: Vec::new(), cursor: None, top_line: 0 };
        let one_row = screen(&[rule()]);
        assert_eq!(read(&empty), None);
        assert_eq!(read(&one_row), None);
    }

    #[test]
    fn a_very_long_suggestion_is_nothing() {
        let long = "x".repeat(MAX_CHARS + 1);
        let mut line = row(&[("> ", Ink::Plain)]);
        line.extend(long.chars().map(|c| cell(c, Ink::Faint)));
        let cols = line.len();
        let snap = GridSnap {
            cols,
            rows: 3,
            cells: [rule_of(cols), line, rule_of(cols)].into_iter().flatten().collect(),
            cursor: None,
            top_line: 0,
        };
        assert_eq!(read(&snap), None);
    }

    fn rule_of(cols: usize) -> Vec<CellSnap> {
        (0..cols).map(|_| cell('\u{2500}', Ink::Plain)).collect()
    }

    #[test]
    fn the_screen_is_read_again_only_when_the_output_count_moved() {
        let reads = Cell::new(0);
        let mut ghost = Ghost::default();
        let look = |ghost: &mut Ghost, output| {
            ghost.refresh(output, || {
                reads.set(reads.get() + 1);
                boxed(row(&[("> ", Ink::Plain), ("run it", Ink::Faint)]))
            });
        };
        look(&mut ghost, 7);
        look(&mut ghost, 7);
        look(&mut ghost, 7);
        assert_eq!((reads.get(), ghost.text()), (1, Some("run it")));
        look(&mut ghost, 8);
        assert_eq!(reads.get(), 2);
        ghost.clear();
        assert_eq!(ghost.text(), None);
        look(&mut ghost, 8);
        assert_eq!(reads.get(), 3);
    }
}
