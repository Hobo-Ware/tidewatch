//! Live lists in a message box: a line that starts with `- `, `* ` or a number and a dot (`1. `),
//! at any indent, is drawn as a list item while it is typed: a marker in the gutter, its text 30 pt
//! in from the level's edge and every wrapped row under that text. The text itself never changes,
//! so what is sent, copied and edited is exactly what was typed. The indent, marker and space of an
//! item are zero-width glyphs in the drawn text, so every cursor position of the real text is a
//! valid position in the drawn one.

use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::text::{CCursor, CCursorRange};
use egui::{Color32, FontId, Galley, Pos2, Ui, pos2};

use crate::emoji_text;
use crate::md_view::{INDENT, LINE, MARKER_EDGE, Marker, paint_marker};

const TAB_COLUMNS: usize = 4;
const MIN_STEP: usize = 2;
const MAX_DIGITS: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Bullet,
    Number,
}

/// A list line: how deep it sits, and where its indent, marker and text are in the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) kind: Kind,
    pub(crate) level: usize,
    pub(crate) indent: usize,
    pub(crate) marker: usize,
    columns: usize,
    chain: Vec<usize>,
}

impl Item {
    /// Bytes of the line before its text: the indent, the marker and the space after it.
    pub(crate) fn prefix(&self) -> usize {
        self.indent + self.marker + 1
    }
}

fn marked(line: &str) -> Option<(Kind, usize, usize)> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (kind, marker) = match rest.as_bytes().first()? {
        b'-' | b'*' => (Kind::Bullet, 1),
        b'0'..=b'9' if digits <= MAX_DIGITS && rest[digits..].starts_with('.') => {
            (Kind::Number, digits + 1)
        }
        _ => return None,
    };
    rest[marker..].starts_with(' ').then_some((kind, indent, marker))
}

fn columns_of(indent: &str) -> usize {
    indent.chars().map(|space| if space == '\t' { TAB_COLUMNS } else { 1 }).sum()
}

pub(crate) fn has_items(text: &str) -> bool {
    text.split('\n').any(|line| marked(line).is_some())
}

/// The list item of every line, in order. A line sits one level inside the nearest line above it
/// that is indented at least `MIN_STEP` less, and a line that is not an item ends the list.
pub(crate) fn items(text: &str) -> Vec<Option<Item>> {
    let mut chain: Vec<usize> = Vec::new();
    text.split('\n')
        .map(|line| {
            let Some((kind, indent, marker)) = marked(line) else {
                chain.clear();
                return None;
            };
            let columns = columns_of(&line[..indent]);
            while chain.last().is_some_and(|top| top + MIN_STEP > columns) {
                chain.pop();
            }
            let level = chain.len();
            chain.push(columns);
            Some(Item { kind, level, indent, marker, columns, chain: chain.clone() })
        })
        .collect()
}

/// Where the text of an item at `level` starts, from the left edge of the box text.
pub(crate) fn text_edge(level: usize) -> f32 {
    INDENT * (level as f32 + 1.0)
}

/// A run of the text, as byte positions of the real text, drawn in another colour.
pub(crate) type Tint = Option<(Range<usize>, Color32)>;

fn append_tinted(job: &mut LayoutJob, (text, start): (&str, usize), look: (&TextFormat, &Tint)) {
    let (plain, tint) = look;
    let inside = tint.as_ref().and_then(|(range, color)| {
        let (lo, hi) = (
            range.start.max(start) - start,
            range.end.min(start + text.len()).saturating_sub(start),
        );
        (lo < hi && text.is_char_boundary(lo) && text.is_char_boundary(hi))
            .then_some((lo, hi, *color))
    });
    match inside {
        Some((lo, hi, color)) => {
            job.append(&text[..lo], 0.0, plain.clone());
            job.append(&text[lo..hi], 0.0, TextFormat { color, ..plain.clone() });
            job.append(&text[hi..], 0.0, plain.clone());
        }
        None => job.append(text, 0.0, plain.clone()),
    }
}

/// The job for `text`, which starts at byte `start` of the real text, with a `tint`ed run in its own
/// colour.
pub(crate) fn job(
    (text, start): (&str, usize),
    font: &FontId,
    ink: Color32,
    wrap: f32,
    tint: &Tint,
) -> LayoutJob {
    let plain = TextFormat::simple(font.clone(), ink);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    append_tinted(&mut job, (text, start), (&plain, tint));
    job
}

fn indented(ui: &Ui, galley: &Galley, (chars, edge): (usize, f32), font: &FontId) -> Arc<Galley> {
    let blanks = ui.painter().layout_no_wrap(" ".repeat(chars), font.clone(), Color32::WHITE);
    let mut lead = blanks.rows.first().map(|row| row.glyphs.clone()).unwrap_or_default();
    for glyph in &mut lead {
        glyph.pos.x = 0.0;
        glyph.advance_width = 0.0;
    }
    let mut hung = galley.clone();
    for placed in &mut hung.rows {
        placed.pos.x += edge;
    }
    if let Some(first) = hung.rows.first_mut() {
        Arc::make_mut(&mut first.row).glyphs.splice(0..0, lead);
    }
    Arc::new(hung)
}

/// Lays `text` out with each list item's text at its level's edge and its wrapped rows under it.
pub(crate) fn layout(
    ui: &Ui,
    text: &str,
    font: &FontId,
    ink: Color32,
    (wrap, tint): (f32, &Tint),
) -> Arc<Galley> {
    let whole = emoji_text::slot_job(ui.ctx(), job((text, 0), font, ink, wrap, tint));
    if !has_items(text) {
        return ui.painter().layout_job(whole);
    }
    let mut start = 0;
    let galleys: Vec<Arc<Galley>> = text
        .split('\n')
        .zip(items(text))
        .map(|(line, item)| {
            let at = start;
            start += line.len() + 1;
            let Some(item) = item else {
                return laid_out(ui, job((line, at), font, ink, wrap, tint));
            };
            let edge = text_edge(item.level);
            let prefix = item.prefix();
            let width = (wrap - edge).max(font.size);
            let body = laid_out(ui, job((&line[prefix..], at + prefix), font, ink, width, tint));
            indented(ui, &body, (line[..prefix].chars().count(), edge), font)
        })
        .collect();
    let merged = Galley::concat(Arc::new(whole), &galleys, ui.ctx().pixels_per_point());
    Arc::new(merged)
}

/// Draws the marker of every item line of `galley` (the layout of its text, drawn from `origin`):
/// a dot, a ring or a square by level, or the number as typed.
fn laid_out(ui: &Ui, job: LayoutJob) -> Arc<Galley> {
    ui.painter().layout_job(emoji_text::slot_job(ui.ctx(), job))
}

pub(crate) fn paint_markers(ui: &Ui, galley: &Galley, origin: Pos2, font: &FontId) {
    let text = galley.text();
    if !has_items(text) {
        return;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let found = items(text);
    let (mut line, mut first_row) = (0, true);
    for placed in &galley.rows {
        if first_row && let Some(Some(item)) = found.get(line) {
            let label = &lines[line][item.indent..item.indent + item.marker];
            let marker = match item.kind {
                Kind::Bullet => Marker::Bullet(item.level),
                Kind::Number => Marker::Number(label),
            };
            let edge =
                pos2(origin.x + text_edge(item.level) - MARKER_EDGE, origin.y + placed.pos.y);
            paint_marker(ui, &marker, edge, (font.size * LINE, font.clone()));
        }
        first_row = placed.ends_with_newline;
        if placed.ends_with_newline {
            line += 1;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Enter {
    Plain,
    Continued,
    Ended,
}

fn char_to_byte(text: &str, at: usize) -> usize {
    text.char_indices().nth(at).map_or(text.len(), |(byte, _)| byte)
}

fn chars_before(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

struct Spot {
    line: usize,
    span: Range<usize>,
    item: Item,
}

fn spot(text: &str, at: usize) -> Option<Spot> {
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[at..].find('\n').map_or(text.len(), |newline| at + newline);
    let line = text[..start].matches('\n').count();
    let item = items(text).into_iter().nth(line)??;
    Some(Spot { line, span: start..end, item })
}

pub(crate) fn on_list_line(text: &str, cursor: usize) -> bool {
    has_items(text) && spot(text, char_to_byte(text, cursor)).is_some()
}

fn next_prefix(prefix: &str, item: &Item) -> String {
    match item.kind {
        Kind::Bullet => prefix.to_owned(),
        Kind::Number => {
            let (indent, rest) = prefix.split_at(item.indent);
            let digits = &rest[..item.marker - 1];
            let next = digits.parse::<u64>().map_or(1, |number| number.saturating_add(1));
            format!("{indent}{next}. ")
        }
    }
}

/// What Enter does with the cursor at char `cursor`: on a list line with text it starts the next
/// item there (same indent, the next number on a numbered one), on an empty item it clears the
/// line and ends the list, anywhere else it is left alone (`Plain`). Returns the char position the
/// cursor moves to.
pub(crate) fn enter(text: &mut String, cursor: usize) -> (Enter, usize) {
    let at = char_to_byte(text, cursor);
    let Some(Spot { span, item, .. }) = spot(text, at) else {
        return (Enter::Plain, cursor);
    };
    let text_start = span.start + item.prefix();
    if at < text_start {
        return (Enter::Plain, cursor);
    }
    if span.end == text_start {
        text.replace_range(span.clone(), "");
        return (Enter::Ended, chars_before(text, span.start));
    }
    let next = next_prefix(&text[span.start..text_start], &item);
    text.insert_str(at, &format!("\n{next}"));
    (Enter::Continued, chars_before(text, at + 1 + next.len()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shift {
    In,
    Out,
}

fn indent_for(shift: Shift, line: usize, item: &Item, all: &[Option<Item>]) -> Option<usize> {
    match shift {
        Shift::Out => match item.level {
            0 => (item.indent > 0).then_some(0),
            level => Some(item.chain[level - 1]),
        },
        Shift::In => {
            let above = all.get(line.checked_sub(1)?)?.as_ref()?;
            let deeper = item.level + 1;
            if above.level >= deeper {
                Some(above.chain[deeper])
            } else {
                (above.level == item.level).then_some(above.columns + above.marker + 1)
            }
        }
    }
}

/// Moves the list line the cursor (char `cursor`) is on one level in or out by setting its indent:
/// in puts it under the item above (at that item's text, so a Markdown reader nests it too), out
/// onto the level of the item it sat under. `None` off a list line; `Some` with the cursor
/// to keep otherwise, also when the line cannot move.
pub(crate) fn shift(text: &mut String, cursor: usize, way: Shift) -> Option<usize> {
    let at = char_to_byte(text, cursor);
    let Spot { line, span, item } = spot(text, at)?;
    let all = items(text);
    let Some(columns) = indent_for(way, line, &item, &all) else {
        return Some(cursor);
    };
    let old = span.start..span.start + item.indent;
    let before = text[old.clone()].chars().count();
    text.replace_range(old, &" ".repeat(columns));
    let start = chars_before(text, span.start);
    Some((cursor + columns).saturating_sub(before).max(start))
}

pub(crate) fn place_cursor(ui: &Ui, id: egui::Id, at: usize) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
    state.store(ui.ctx(), id);
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Color32 = Color32::WHITE;
    const TEAL: Color32 = Color32::GRAY;

    fn font() -> FontId {
        FontId::proportional(16.0)
    }

    fn with_ui<R>(run: impl FnOnce(&Ui) -> R) -> R {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let (mut run, mut out) = (Some(run), None);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            out = run.take().map(|run| run(ui));
        });
        out.expect("the frame ran")
    }

    fn laid(ui: &Ui, text: &str, wrap: f32) -> Arc<Galley> {
        layout(ui, text, &font(), INK, (wrap, &None))
    }

    fn levels(text: &str) -> Vec<Option<(Kind, usize)>> {
        items(text).into_iter().map(|item| item.map(|item| (item.kind, item.level))).collect()
    }

    #[test]
    fn a_marker_is_a_dash_star_or_number_and_a_space_after_any_indent() {
        let cases = [
            ("- a", Some((Kind::Bullet, 0, 1))),
            ("* a", Some((Kind::Bullet, 0, 1))),
            ("- ", Some((Kind::Bullet, 0, 1))),
            ("1. a", Some((Kind::Number, 0, 2))),
            ("12. a", Some((Kind::Number, 0, 3))),
            ("123456789. a", Some((Kind::Number, 0, 10))),
            ("  - a", Some((Kind::Bullet, 2, 1))),
            ("\t- a", Some((Kind::Bullet, 1, 1))),
            ("    3. a", Some((Kind::Number, 4, 2))),
            ("1234567890. a", None),
            ("-a", None),
            ("1.a", None),
            ("1 a", None),
            ("1) a", None),
            ("--", None),
            ("a - b", None),
            ("", None),
            ("  ", None),
            ("\u{e9}", None),
        ];
        for (line, want) in cases {
            assert_eq!(marked(line), want, "line {line:?}");
        }
    }

    #[test]
    fn two_to_four_spaces_or_a_tab_is_one_level() {
        let cases = [
            ("- a\n  - b\n    - c", vec![0, 1, 2]),
            ("- a\n    - b\n        - c", vec![0, 1, 2]),
            ("- a\n   - b", vec![0, 1]),
            ("- a\n\t- b\n\t\t- c", vec![0, 1, 2]),
            ("- a\n - b", vec![0, 0]),
            ("- a\n    - b\n  - c", vec![0, 1, 1]),
            ("- a\n    - b\n- c", vec![0, 1, 0]),
            ("  - a\n- b", vec![0, 0]),
        ];
        for (text, want) in cases {
            let got: Vec<usize> =
                items(text).into_iter().map(|item| item.expect("an item").level).collect();
            assert_eq!(got, want, "text {text:?}");
        }
    }

    #[test]
    fn a_line_that_is_not_an_item_ends_the_list() {
        assert_eq!(
            levels("- a\n    - b\nplain\n    - c"),
            vec![Some((Kind::Bullet, 0)), Some((Kind::Bullet, 1)), None, Some((Kind::Bullet, 0))]
        );
        assert_eq!(levels("- a\n\n  - b")[2], Some((Kind::Bullet, 0)));
    }

    #[test]
    fn numbers_and_bullets_nest_in_each_other() {
        assert_eq!(
            levels("1. a\n   - b\n     1. c\n2. d"),
            vec![
                Some((Kind::Number, 0)),
                Some((Kind::Bullet, 1)),
                Some((Kind::Number, 2)),
                Some((Kind::Number, 0))
            ]
        );
    }

    #[test]
    fn the_text_edge_steps_30_a_level() {
        assert_eq!(text_edge(0), 30.0);
        assert_eq!(text_edge(1), 60.0);
        assert_eq!(text_edge(2), 90.0);
    }

    #[test]
    fn the_drawn_text_is_the_typed_text_and_a_tint_is_its_own_section() {
        let cases = [
            ("go :smi now", 3..7, ":smi"),
            ("a\n- go :smi", 7..11, ":smi"),
            (":smi", 0..4, ":smi"),
        ];
        for (text, range, want) in cases {
            let tint = Some((range, TEAL));
            let built = job((text, 0), &font(), INK, 300.0, &tint);
            assert_eq!(built.text, text);
            let tinted: Vec<&str> = built
                .sections
                .iter()
                .filter(|section| section.format.color == TEAL)
                .map(|section| &built.text[section.byte_range.start.0..section.byte_range.end.0])
                .collect();
            assert_eq!(tinted, vec![want], "{text:?}");
        }
    }

    #[test]
    fn a_tint_outside_the_text_changes_nothing() {
        let tint = Some((40..50, TEAL));
        assert_eq!(job(("short", 0), &font(), INK, 300.0, &tint).sections.len(), 1);
    }

    #[test]
    fn the_tint_follows_the_text_of_an_item_line_in_the_laid_out_galley() {
        with_ui(|ui| {
            let text = "a\n    - go :smi";
            let tint = Some((13..17, TEAL));
            let galley = layout(ui, text, &font(), INK, (300.0, &tint));
            assert_eq!(galley.text(), text);
            let rows: Vec<Color32> = galley
                .rows
                .iter()
                .flat_map(|row| row.visuals.mesh.vertices.iter().map(|vertex| vertex.color))
                .collect();
            assert!(rows.contains(&TEAL), "the tinted run is drawn in its colour");
        });
    }

    #[test]
    fn an_item_text_starts_at_its_levels_edge_and_a_plain_line_at_zero() {
        with_ui(|ui| {
            let text = "plain\n- one\n    - two\n        - three\n1. four\nplain";
            let galley = laid(ui, text, 400.0);
            assert_eq!(galley.rows.len(), 6);
            let starts: Vec<f32> = galley
                .rows
                .iter()
                .map(|row| {
                    row.pos.x
                        + row.glyphs.iter().find(|g| g.advance_width > 0.0).map_or(0.0, |g| g.pos.x)
                })
                .collect();
            assert_eq!(starts[0], 0.0);
            assert_eq!(starts[1], 30.0);
            assert_eq!(starts[2], 60.0);
            assert_eq!(starts[3], 90.0);
            assert_eq!(starts[4], 30.0);
            assert_eq!(starts[5], 0.0);
        });
    }

    #[test]
    fn every_wrapped_row_of_an_item_starts_at_its_text_edge_inside_the_wrap() {
        with_ui(|ui| {
            let long = "one two three four five six seven eight nine ten eleven twelve";
            let text = format!("- {long}\n    - {long}");
            let galley = laid(ui, &text, 220.0);
            let rows = &galley.rows;
            assert!(rows.len() > 4, "both lines wrap");
            let split = rows.iter().position(|row| row.ends_with_newline).unwrap() + 1;
            for row in &rows[..split] {
                assert_eq!(row.pos.x, 30.0);
                assert!(row.rect().right() <= 220.5, "{:?}", row.rect());
            }
            for row in &rows[split..] {
                assert_eq!(row.pos.x, 60.0);
                assert!(row.rect().right() <= 220.5, "{:?}", row.rect());
            }
        });
    }

    #[test]
    fn the_row_pitch_is_the_same_for_plain_and_item_lines() {
        with_ui(|ui| {
            let plain = laid(ui, "a\nb\nc", 300.0);
            let list = laid(ui, "- a\n    - b\n1. c", 300.0);
            let tops =
                |galley: &Galley| galley.rows.iter().map(|row| row.pos.y).collect::<Vec<_>>();
            assert_eq!(tops(&plain), tops(&list));
            assert_eq!(plain.size().y, list.size().y);
        });
    }

    #[test]
    fn the_drawn_galley_has_a_cursor_position_for_every_char_of_the_real_text() {
        with_ui(|ui| {
            let text = "- ab\n    12. cd\n\n  * ";
            let galley = laid(ui, text, 300.0);
            assert_eq!(galley.text(), text);
            assert_eq!(galley.end().index.0, text.chars().count());
            let chars: usize =
                galley.rows.iter().map(|row| row.char_count_including_newline().0).sum();
            assert_eq!(chars, text.chars().count());
        });
    }

    #[test]
    fn the_cursor_in_an_item_sits_at_its_text_edge_and_an_empty_item_keeps_its_row_height() {
        with_ui(|ui| {
            let text = "- ab\n    - ";
            let galley = laid(ui, text, 300.0);
            let at = |chars: usize| galley.pos_from_cursor(CCursor::new(chars));
            assert_eq!(at(2).min.x, 30.0, "after the marker");
            assert_eq!(at(11).min.x, 60.0, "after the nested marker of an empty item");
            assert_eq!(at(11).height(), at(2).height());
            assert_eq!(galley.rows[1].pos.y, galley.rows[0].row.size.y);
        });
    }

    #[test]
    fn text_without_items_lays_out_as_one_plain_galley() {
        with_ui(|ui| {
            let galley = laid(ui, "hello\nworld", 300.0);
            assert_eq!(galley.text(), "hello\nworld");
            assert_eq!(galley.rows.len(), 2);
            assert!(galley.rows.iter().all(|row| row.pos.x == 0.0));
        });
    }

    fn entered(start: &str, cursor: usize) -> (Enter, usize, String) {
        let mut text = start.to_owned();
        let (outcome, at) = enter(&mut text, cursor);
        (outcome, at, text)
    }

    #[test]
    fn enter_on_an_item_starts_the_next_one_at_the_same_level() {
        let cases = [
            ("- one", 5, "- one\n- ", 8),
            ("* one", 5, "* one\n* ", 8),
            ("* split here", 7, "* split\n*  here", 10),
            ("    - deep", 10, "    - deep\n    - ", 17),
            ("\t- tab", 6, "\t- tab\n\t- ", 10),
            ("1. one", 6, "1. one\n2. ", 10),
            ("9. nine", 7, "9. nine\n10. ", 12),
            ("  7. up", 7, "  7. up\n  8. ", 13),
            ("a\n- b\nc", 5, "a\n- b\n- \nc", 8),
        ];
        for (start, cursor, want, at) in cases {
            assert_eq!(
                entered(start, cursor),
                (Enter::Continued, at, want.to_owned()),
                "{start:?}"
            );
        }
    }

    #[test]
    fn enter_on_an_empty_item_clears_the_line_and_ends_the_list() {
        let cases = [
            ("- one\n- ", 8, "- one\n", 6),
            ("- ", 2, "", 0),
            ("1. one\n2. ", 10, "1. one\n", 7),
            ("- a\n    - b\n    - ", 18, "- a\n    - b\n", 12),
            ("- a\n\t* ", 7, "- a\n", 4),
        ];
        for (start, cursor, want, at) in cases {
            assert_eq!(entered(start, cursor), (Enter::Ended, at, want.to_owned()), "{start:?}");
        }
    }

    #[test]
    fn enter_off_an_item_or_before_its_text_is_plain() {
        let cases = [
            ("hello", 5),
            ("- a\nplain", 9),
            ("- a", 1),
            ("- a", 0),
            ("    - a", 5),
            ("1. a", 2),
            ("", 0),
        ];
        for (start, cursor) in cases {
            assert_eq!(
                entered(start, cursor),
                (Enter::Plain, cursor, start.to_owned()),
                "{start:?}"
            );
        }
    }

    #[test]
    fn enter_counts_chars_not_bytes() {
        assert_eq!(entered("- caf\u{e9}", 6), (Enter::Continued, 9, "- caf\u{e9}\n- ".to_owned()));
        assert_eq!(
            entered("\u{e9}\n1. \u{e9}", 6),
            (Enter::Continued, 10, "\u{e9}\n1. \u{e9}\n2. ".to_owned())
        );
    }

    fn shifted(start: &str, cursor: usize, way: Shift) -> (Option<usize>, String) {
        let mut text = start.to_owned();
        let at = shift(&mut text, cursor, way);
        (at, text)
    }

    #[test]
    fn tab_puts_an_item_under_the_one_above_at_that_items_text() {
        let cases = [
            ("- a\n- b", 8, "- a\n  - b", 10),
            ("- a\n  - b\n  - c", 17, "- a\n  - b\n    - c", 19),
            ("1. a\n2. b", 10, "1. a\n   2. b", 13),
            ("- a\n    - b\n- c", 16, "- a\n    - b\n    - c", 20),
            ("- a\n- b", 5, "- a\n  - b", 7),
            ("- a\n    - b\n    - c\n- d", 23, "- a\n    - b\n    - c\n    - d", 27),
        ];
        for (start, cursor, want, at) in cases {
            assert_eq!(shifted(start, cursor, Shift::In), (Some(at), want.to_owned()), "{start:?}");
        }
    }

    #[test]
    fn tab_cannot_go_deeper_than_one_level_under_the_item_above() {
        let cases = [("- a\n  - b", 10), ("- a", 3), ("- a\nplain\n- b", 14)];
        for (start, cursor) in cases {
            assert_eq!(
                shifted(start, cursor, Shift::In),
                (Some(cursor), start.to_owned()),
                "{start:?}"
            );
        }
    }

    #[test]
    fn shift_tab_puts_an_item_on_the_level_it_sat_under() {
        let cases = [
            ("- a\n  - b", 10, "- a\n- b", 8),
            ("- a\n    - b\n        - c", 27, "- a\n    - b\n    - c", 23),
            ("- a\n\t- b", 8, "- a\n- b", 7),
            ("  - a", 5, "- a", 3),
            ("1. a\n   - b", 11, "1. a\n- b", 8),
            ("- a\n    - b", 6, "- a\n- b", 4),
        ];
        for (start, cursor, want, at) in cases {
            assert_eq!(
                shifted(start, cursor, Shift::Out),
                (Some(at), want.to_owned()),
                "{start:?}"
            );
        }
    }

    #[test]
    fn shift_tab_on_a_first_level_item_changes_nothing() {
        assert_eq!(shifted("- a", 3, Shift::Out), (Some(3), "- a".to_owned()));
    }

    #[test]
    fn tab_and_shift_tab_leave_other_lines_alone() {
        for way in [Shift::In, Shift::Out] {
            assert_eq!(shifted("hello", 5, way), (None, "hello".to_owned()));
            assert_eq!(shifted("- a\nplain", 9, way), (None, "- a\nplain".to_owned()));
        }
    }

    #[test]
    fn the_levels_after_a_tab_and_after_a_shift_tab_are_one_more_and_one_less() {
        fn at_end(text: &mut String, way: Shift) {
            let end = text.chars().count();
            shift(text, end, way);
        }
        let mut text = "- a\n- b\n- c".to_owned();
        at_end(&mut text, Shift::In);
        assert_eq!(levels(&text)[2], Some((Kind::Bullet, 1)));
        at_end(&mut text, Shift::In);
        assert_eq!(
            levels(&text)[2],
            Some((Kind::Bullet, 1)),
            "no deeper than under the item above"
        );
        at_end(&mut text, Shift::Out);
        assert_eq!(levels(&text)[2], Some((Kind::Bullet, 0)));
        shift(&mut text, 7, Shift::In);
        at_end(&mut text, Shift::In);
        let kinds: Vec<usize> =
            items(&text).into_iter().map(|item| item.expect("an item").level).collect();
        assert_eq!(kinds, vec![0, 1, 1]);
    }

    #[test]
    fn on_a_list_line_is_true_only_with_the_cursor_on_an_item() {
        assert!(on_list_line("- a", 3));
        assert!(on_list_line("x\n  1. a", 8));
        assert!(!on_list_line("x\n- a", 1));
        assert!(!on_list_line("plain", 2));
        assert!(!on_list_line("", 0));
    }
}
