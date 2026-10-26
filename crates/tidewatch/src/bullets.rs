//! Live bullet lists in a message box: a line that starts with `- ` or `* ` is drawn as a bullet
//! with a hanging indent while it is typed. The text itself never changes, so what is sent, copied
//! and edited is exactly what was typed. The bullet replaces the marker's first character one for
//! one, so every cursor position of the real text is a valid position in the drawn one.

use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::text::{CCursor, CCursorRange};
use egui::{Color32, FontId, Galley, Ui};

const BULLET: char = '\u{2022}';
const MARKER_LEN: usize = 2;

pub(crate) fn marker_of(line: &str) -> Option<&str> {
    line.get(..MARKER_LEN).filter(|marker| matches!(*marker, "- " | "* "))
}

pub(crate) fn has_bullets(text: &str) -> bool {
    text.split('\n').any(|line| marker_of(line).is_some())
}

/// A run of the text, as byte positions of the real text, drawn in another colour.
pub(crate) type Tint = Option<(Range<usize>, Color32)>;

fn append_tinted(
    job: &mut LayoutJob,
    (text, start): (&str, usize),
    tail: &str,
    look: (&TextFormat, &Tint),
) {
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
            job.append(&format!("{}{tail}", &text[hi..]), 0.0, plain.clone());
        }
        None => job.append(&format!("{text}{tail}"), 0.0, plain.clone()),
    }
}

/// The job for `text`: a section per bullet glyph in `bullet`, the rest in `ink`, a `tint`ed run
/// in its own colour.
pub(crate) fn job(
    text: &str,
    font: &FontId,
    (ink, bullet): (Color32, Color32),
    wrap: f32,
    tint: &Tint,
) -> LayoutJob {
    let plain = TextFormat::simple(font.clone(), ink);
    let marked = TextFormat::simple(font.clone(), bullet);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    let mut start = 0;
    for (at, line) in lines.iter().enumerate() {
        let newline = if at < last { "\n" } else { "" };
        match marker_of(line) {
            Some(_) => {
                job.append(&BULLET.to_string(), 0.0, marked.clone());
                append_tinted(&mut job, (&line[1..], start + 1), newline, (&plain, tint));
            }
            None => append_tinted(&mut job, (line, start), newline, (&plain, tint)),
        }
        start += line.len() + 1;
    }
    job
}

/// How far the text of a bullet line starts from its left edge: the bullet and the space.
pub(crate) fn indent(ui: &Ui, font: &FontId) -> f32 {
    let width = |text: &str| {
        ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
    };
    width(&format!("{BULLET} x")) - width("x")
}

/// Lays `text` out with bullets drawn and wrapped rows of a bullet line hung under its text.
pub(crate) fn layout(
    ui: &Ui,
    text: &str,
    font: &FontId,
    inks: (Color32, Color32),
    wrap: f32,
    tint: &Tint,
) -> Arc<Galley> {
    if !has_bullets(text) {
        return ui.painter().layout_job(job(text, font, inks, wrap, tint));
    }
    let hang = indent(ui, font);
    let galley = ui.painter().layout_job(job(text, font, inks, wrap - hang, tint));
    let in_bullet: Vec<bool> = text.split('\n').map(|line| marker_of(line).is_some()).collect();
    let mut hung = (*galley).clone();
    let (mut line, mut first_row) = (0, true);
    for placed in &mut hung.rows {
        if in_bullet.get(line).copied().unwrap_or(false) && !first_row {
            placed.pos.x += hang;
            hung.rect = hung.rect.union(placed.rect());
        }
        first_row = placed.ends_with_newline;
        if placed.ends_with_newline {
            line += 1;
        }
    }
    Arc::new(hung)
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

/// What Enter does with the cursor at char `cursor`: on a bullet line with text it starts the next
/// bullet there, on an empty bullet it removes the marker and ends the list, anywhere else it is
/// left alone (`Plain`). Returns the char position the cursor moves to.
pub(crate) fn enter(text: &mut String, cursor: usize) -> (Enter, usize) {
    let at = char_to_byte(text, cursor);
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[at..].find('\n').map_or(text.len(), |newline| at + newline);
    let Some(marker) = marker_of(&text[start..end]).map(str::to_owned) else {
        return (Enter::Plain, cursor);
    };
    if at < start + MARKER_LEN {
        return (Enter::Plain, cursor);
    }
    if text[start..end] == marker {
        text.replace_range(start..end, "");
        return (Enter::Ended, text[..start].chars().count());
    }
    text.insert_str(at, &format!("\n{marker}"));
    (Enter::Continued, text[..at + 1 + MARKER_LEN].chars().count())
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
    const DOT: Color32 = Color32::GRAY;

    fn font() -> FontId {
        FontId::proportional(16.0)
    }

    fn with_ui<R>(run: impl FnOnce(&Ui) -> R) -> R {
        let ctx = egui::Context::default();
        let (mut run, mut out) = (Some(run), None);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            out = run.take().map(|run| run(ui));
        });
        out.expect("the frame ran")
    }

    #[test]
    fn only_a_dash_or_star_and_a_space_at_the_line_start_is_a_marker() {
        let cases = [
            ("- a", true),
            ("* a", true),
            ("- ", true),
            ("-a", false),
            (" - a", false),
            ("a - b", false),
            ("--", false),
            ("", false),
            ("é", false),
        ];
        for (line, want) in cases {
            assert_eq!(marker_of(line).is_some(), want, "line {line:?}");
        }
    }

    #[test]
    fn the_drawn_text_swaps_one_char_per_marker_and_keeps_the_length() {
        let text = "intro\n- one\n* two\nnot - a bullet\n- ";
        let shown = job(text, &font(), (INK, DOT), 300.0, &None).text;
        assert_eq!(shown, "intro\n\u{2022} one\n\u{2022} two\nnot - a bullet\n\u{2022} ");
        assert_eq!(shown.chars().count(), text.chars().count());
    }

    #[test]
    fn each_bullet_line_gets_a_bullet_section_of_its_own() {
        let text = "intro\n- one\n- two";
        let built = job(text, &font(), (INK, DOT), 300.0, &None);
        assert_eq!(built.text, "intro\n\u{2022} one\n\u{2022} two");
        let bullets: Vec<&str> = built
            .sections
            .iter()
            .filter(|section| section.format.color == DOT)
            .map(|section| &built.text[section.byte_range.start.0..section.byte_range.end.0])
            .collect();
        assert_eq!(bullets, vec!["\u{2022}", "\u{2022}"]);
        let covered: usize = built
            .sections
            .iter()
            .map(|section| section.byte_range.end.0 - section.byte_range.start.0)
            .sum();
        assert_eq!(covered, built.text.len());
    }

    #[test]
    fn a_tinted_run_is_its_own_section_in_text_and_in_bullet_lines() {
        let cases = [
            ("go :smi now", 3..7, ":smi"),
            ("a\n- go :smi", 7..11, ":smi"),
            (":smi", 0..4, ":smi"),
        ];
        for (text, range, want) in cases {
            let tint = Some((range, DOT));
            let built = job(text, &font(), (INK, Color32::RED), 300.0, &tint);
            let tinted: Vec<&str> = built
                .sections
                .iter()
                .filter(|section| section.format.color == DOT)
                .map(|section| &built.text[section.byte_range.start.0..section.byte_range.end.0])
                .collect();
            assert_eq!(tinted, vec![want], "{text:?}");
            assert_eq!(built.text.chars().count(), text.chars().count());
        }
    }

    #[test]
    fn a_tint_outside_the_text_changes_nothing() {
        let tint = Some((40..50, DOT));
        let built = job("short", &font(), (INK, Color32::RED), 300.0, &tint);
        assert_eq!(built.sections.len(), 1);
    }

    #[test]
    fn text_without_bullets_lays_out_as_one_plain_section() {
        let built = job("hello\nworld", &font(), (INK, DOT), 300.0, &None);
        assert_eq!(built.sections.len(), 1);
        assert_eq!(built.text, "hello\nworld");
    }

    #[test]
    fn a_wrapped_bullet_line_hangs_under_its_text() {
        with_ui(|ui| {
            let text = "- one two three four five six seven eight nine ten eleven twelve";
            let galley = layout(ui, text, &font(), (INK, DOT), 140.0, &None);
            assert!(galley.rows.len() > 2, "the line wraps");
            let hang = indent(ui, &font());
            assert!(hang > 4.0);
            assert_eq!(galley.rows[0].pos.x, 0.0);
            for row in &galley.rows[1..] {
                assert_eq!(row.pos.x, hang);
                assert!(row.rect().right() <= 140.0 + 0.5, "a hung row stays inside the wrap");
            }
        });
    }

    #[test]
    fn a_plain_line_after_a_bullet_line_is_not_indented() {
        with_ui(|ui| {
            let galley = layout(ui, "- one\nplain", &font(), (INK, DOT), 300.0, &None);
            assert_eq!(galley.rows.len(), 2);
            assert_eq!(galley.rows[1].pos.x, 0.0);
        });
    }

    #[test]
    fn the_drawn_galley_has_a_cursor_position_for_every_char_of_the_real_text() {
        with_ui(|ui| {
            let text = "- ab\n- cd";
            let galley = layout(ui, text, &font(), (INK, DOT), 300.0, &None);
            assert_eq!(galley.text().chars().count(), text.chars().count());
            let end = galley.end();
            assert_eq!(end.index.0, text.chars().count());
        });
    }

    #[test]
    fn enter_on_a_bullet_line_starts_the_next_bullet_after_the_cursor() {
        let mut text = "- one".to_owned();
        assert_eq!(enter(&mut text, 5), (Enter::Continued, 8));
        assert_eq!(text, "- one\n- ");
        let mut text = "* split here".to_owned();
        assert_eq!(enter(&mut text, 7), (Enter::Continued, 10));
        assert_eq!(text, "* split\n*  here");
    }

    #[test]
    fn enter_on_an_empty_bullet_ends_the_list() {
        let mut text = "- one\n- ".to_owned();
        assert_eq!(enter(&mut text, 8), (Enter::Ended, 6));
        assert_eq!(text, "- one\n");
        let mut text = "- ".to_owned();
        assert_eq!(enter(&mut text, 2), (Enter::Ended, 0));
        assert_eq!(text, "");
    }

    #[test]
    fn enter_off_a_bullet_line_or_before_its_marker_is_plain() {
        let cases = [("hello", 5), ("- a\nplain", 9), ("- a", 1), ("- a", 0), ("", 0)];
        for (start, cursor) in cases {
            let mut text = start.to_owned();
            assert_eq!(enter(&mut text, cursor), (Enter::Plain, cursor), "text {start:?}");
            assert_eq!(text, start);
        }
    }

    #[test]
    fn enter_counts_chars_not_bytes() {
        let mut text = "- caf\u{e9}".to_owned();
        assert_eq!(enter(&mut text, 6), (Enter::Continued, 9));
        assert_eq!(text, "- caf\u{e9}\n- ");
    }
}
