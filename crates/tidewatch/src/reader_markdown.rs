//! A chat message cut into the pieces the Reader draws itself: fenced code blocks (with their own
//! copy button), top-level blockquotes (a quiet panel with a quote mark) and, in an agent's
//! message, the `Done` / `Needs you` / `Next` heading lines as captions. Everything between them
//! stays markdown for the renderer. Pure.

use egui::epaint::CubicBezierShape;
use egui::{
    Color32, CornerRadius, Frame, Margin, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use tidewatch_core::{end_block, github_html};

use crate::brand;
use crate::code_block::{self, Look};
use crate::links::{closes_fence, fence_of};
use crate::screen_kit::{body, heading};

const PIECE_GAP: f32 = 8.0;
const CAPTION_PX: f32 = 13.0;
const CAPTION_ABOVE: f32 = 10.0;
const CAPTION_BELOW: f32 = 2.0;
const QUOTE_RADIUS: u8 = 10;
const THUMB_W: f32 = 44.0;
const THUMB_H: f32 = 32.0;
const IMAGE_CHIP_LINE: Color32 = Color32::from_rgb(0x2f, 0x4a, 0x4a);
const QUOTE_PAD: Margin = Margin { left: 16, right: 16, top: 14, bottom: 14 };
const QUOTE_GAP: f32 = 12.0;
const QUOTE_MARK: f32 = 18.0;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Markdown(&'a str),
    Code { lang: Option<&'a str>, text: String },
    Quote(String),
    Image(String),
    Caption(&'static str),
}

/// Cuts `md` at its top-level fences and quotes (up to 3 spaces of indent; a fence inside a quote
/// or deeper in a list stays with the renderer, and so does a quote in a list) and, with
/// `captions`, at the end block's heading-only lines. A fence the message never closes runs to its
/// end, as in CommonMark.
pub(crate) fn pieces(md: &str, captions: bool) -> Vec<Piece<'_>> {
    let heads = if captions { end_block::caption_lines(&outside_fences(md)) } else { Vec::new() };
    let mut out = Vec::new();
    let mut prose = 0..0;
    let mut open: Option<Open<'_>> = None;
    let mut quote: Vec<&str> = Vec::new();
    let mut at = 0;
    for (index, line) in md.split_inclusive('\n').enumerate() {
        let end = at + line.len();
        let quoted = open.is_none() && quote_text(line).is_some();
        if !quoted && !quote.is_empty() {
            out.push(Piece::Quote(std::mem::take(&mut quote).join("\n")));
            prose = at..at;
        }
        if let Some(fence) = &mut open {
            if closes_fence(line, (fence.mark, fence.run)) {
                out.push(open.take().expect("checked open").finish());
                prose = end..end;
            } else {
                fence.lines.push(line);
            }
        } else if let Some(text) = quote_text(line) {
            if quote.is_empty() {
                push_prose(&mut out, &md[prose.clone()]);
            }
            quote.push(text);
            prose = end..end;
        } else if let Some((lead, label)) = image_label(line) {
            prose.end = at + lead;
            push_prose(&mut out, &md[prose.clone()]);
            out.push(Piece::Image(label));
            prose = end..end;
        } else if let Some(fence) = opens(line) {
            push_prose(&mut out, &md[prose.clone()]);
            open = Some(fence);
        } else if let Some((_, caption)) = heads.iter().find(|(i, _)| *i == index) {
            push_prose(&mut out, &md[prose.clone()]);
            out.push(Piece::Caption(caption));
            prose = end..end;
        } else {
            prose.end = end;
        }
        at = end;
    }
    if !quote.is_empty() {
        out.push(Piece::Quote(quote.join("\n")));
    } else if let Some(fence) = open {
        out.push(fence.finish());
    } else {
        push_prose(&mut out, &md[prose]);
    }
    out
}

/// The `Image 1` chip a line ends with (the code chip the user bubble makes of `[Image #1]`) and
/// how many bytes of the line come before it.
fn image_label(line: &str) -> Option<(usize, String)> {
    let line = line.trim_end();
    let body = line.strip_suffix('`')?;
    let start = body.rfind('`')?;
    let label = &body[start + 1..];
    let number = label.strip_prefix("Image ")?;
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        .then(|| (start, label.to_owned()))
}

/// A quote line without its `>` marker and the one space after it; `None` for any other line.
fn quote_text(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = line[indent..].strip_prefix('>').filter(|_| indent <= 3)?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// `md` with the lines of its fenced blocks blanked, so a heading inside code is not a heading.
fn outside_fences(md: &str) -> String {
    let mut open: Option<(char, usize)> = None;
    md.split_inclusive('\n')
        .map(|line| match (open, opens(line)) {
            (Some(fence), _) => {
                if closes_fence(line, fence) {
                    open = None;
                }
                "\n"
            }
            (None, Some(fence)) => {
                open = Some((fence.mark, fence.run));
                "\n"
            }
            (None, None) => line,
        })
        .collect()
}

fn push_prose<'a>(out: &mut Vec<Piece<'a>>, chunk: &'a str) {
    if !chunk.trim().is_empty() {
        out.push(Piece::Markdown(chunk));
    }
}

struct Open<'a> {
    mark: char,
    run: usize,
    indent: usize,
    lang: Option<&'a str>,
    lines: Vec<&'a str>,
}

impl<'a> Open<'a> {
    fn finish(self) -> Piece<'a> {
        let lines: Vec<&str> = self
            .lines
            .iter()
            .map(|line| {
                let line = line.trim_end_matches(['\n', '\r']);
                let strip = line.bytes().take(self.indent).take_while(|b| *b == b' ').count();
                &line[strip..]
            })
            .collect();
        Piece::Code { lang: self.lang, text: lines.join("\n") }
    }
}

fn opens(line: &str) -> Option<Open<'_>> {
    let (mark, run, info) = fence_of(line)?;
    if mark == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next();
    Some(Open {
        mark,
        run,
        indent: line.len() - line.trim_start_matches(' ').len(),
        lang,
        lines: Vec::new(),
    })
}

/// Draws `md` as a chat message: markdown where the renderer is good, and the pieces above where
/// the Reader draws its own. `look` is what the code blocks sit on.
pub(crate) fn show(ui: &mut Ui, cache: &mut CommonMarkCache, md: &str, captions: bool, look: Look) {
    let md = github_html::scrub(md);
    let parts = pieces(&md, captions);
    let column = ui.max_rect().x_range();
    ui.shrink_clip_rect(Rect::from_x_y_ranges(column, ui.clip_rect().y_range()));
    for (index, piece) in parts.iter().enumerate() {
        if index > 0
            && !matches!(piece, Piece::Caption(_))
            && !matches!(parts[index - 1], Piece::Caption(_))
        {
            ui.add_space(PIECE_GAP);
        }
        ui.push_id(index, |ui| match piece {
            Piece::Markdown(chunk) => markdown(ui, cache, chunk),
            Piece::Code { lang, text } => {
                code_block::show(ui, *lang, text, look);
            }
            Piece::Quote(text) => quote(ui, cache, text),
            Piece::Image(label) => image_chip(ui, label),
            Piece::Caption(text) => caption(ui, text),
        });
    }
}

fn quote(ui: &mut Ui, cache: &mut CommonMarkCache, md: &str) {
    Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(QUOTE_RADIUS))
        .inner_margin(QUOTE_PAD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = QUOTE_GAP;
                let (rect, _) =
                    ui.allocate_exact_size(vec2(QUOTE_MARK, QUOTE_MARK), Sense::hover());
                paint_quote_mark(ui, rect.translate(vec2(0.0, 2.0)));
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.visuals_mut().override_text_color = Some(brand::STEP_TEXT);
                    markdown(ui, cache, md);
                });
            });
        });
}

#[derive(Debug, PartialEq, Eq)]
enum Chunk<'a> {
    Prose(&'a str),
    Table(&'a str),
}

fn is_table_divider(line: &str) -> bool {
    let line = line.trim();
    line.contains('-')
        && line.contains('|')
        && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Cuts `md` at its tables: a header line, a divider line and the rows under them.
fn table_chunks(md: &str) -> Vec<Chunk<'_>> {
    let lines: Vec<&str> = md.split_inclusive('\n').collect();
    let mut out = Vec::new();
    let mut prose_from = 0;
    let mut index = 0;
    let offset = |line: usize| lines[..line].iter().map(|l| l.len()).sum::<usize>();
    while index < lines.len() {
        let table = lines[index].contains('|')
            && lines.get(index + 1).is_some_and(|next| is_table_divider(next));
        if !table {
            index += 1;
            continue;
        }
        let end = (index + 2..lines.len())
            .find(|row| !lines[*row].contains('|') || lines[*row].trim().is_empty())
            .unwrap_or(lines.len());
        let (from, to) = (offset(index), offset(end));
        if from > prose_from {
            out.push(Chunk::Prose(&md[prose_from..from]));
        }
        out.push(Chunk::Table(&md[from..to]));
        prose_from = to;
        index = end;
    }
    if prose_from < md.len() {
        out.push(Chunk::Prose(&md[prose_from..]));
    }
    out
}

fn markdown(ui: &mut Ui, cache: &mut CommonMarkCache, md: &str) {
    for (index, chunk) in table_chunks(md).into_iter().enumerate() {
        ui.push_id(index, |ui| match chunk {
            Chunk::Prose(text) => {
                CommonMarkViewer::new().show(ui, cache, text);
            }
            Chunk::Table(text) => {
                ui.scope(|ui| {
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
                        CommonMarkViewer::new().show(ui, cache, text);
                    });
                });
            }
        });
    }
}

fn image_chip(ui: &mut Ui, label: &str) {
    ui.horizontal(|ui| {
        let galley = ui.painter().layout_no_wrap(label.to_owned(), body(13.0), brand::STEP_TEXT);
        let size = vec2(6.0 + THUMB_W + 10.0 + galley.size().x + 12.0, THUMB_H + 12.0);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter();
        painter.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.0, IMAGE_CHIP_LINE),
            StrokeKind::Inside,
        );
        let thumb = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(THUMB_W, THUMB_H));
        painter.rect(
            thumb,
            CornerRadius::same(6),
            brand::SUNKEN,
            Stroke::new(1.0, brand::ITEM_LINE),
            StrokeKind::Inside,
        );
        paint_image_glyph(painter, thumb.center());
        painter.galley(
            pos2(thumb.right() + 10.0, rect.center().y - galley.size().y / 2.0),
            galley,
            brand::STEP_TEXT,
        );
    });
}

fn paint_image_glyph(painter: &egui::Painter, center: Pos2) {
    let at = |x: f32, y: f32| center + vec2(x - 12.0, y - 12.0) * (16.0 / 24.0);
    let stroke = Stroke::new(1.3, brand::FAINT);
    painter.add(Shape::closed_line(
        vec![at(3.0, 5.0), at(21.0, 5.0), at(21.0, 19.0), at(3.0, 19.0)],
        stroke,
    ));
    painter.circle_stroke(at(9.0, 10.0), 1.6 * 16.0 / 24.0, stroke);
    painter.add(Shape::line(vec![at(21.0, 16.0), at(16.0, 11.0), at(8.0, 19.0)], stroke));
}

fn paint_quote_mark(ui: &Ui, rect: Rect) {
    let at = |x: f32, y: f32| -> Pos2 { rect.min + vec2(x, y) * (QUOTE_MARK / 24.0) };
    let stroke = Stroke::new(1.5, brand::FAINT);
    for shift in [0.0, 8.0] {
        ui.painter().add(Shape::line(
            vec![at(7.0 + shift, 7.0), at(11.0 + shift, 7.0), at(11.0 + shift, 11.0)],
            stroke,
        ));
        ui.painter().add(CubicBezierShape::from_points_stroke(
            [
                at(11.0 + shift, 11.0),
                at(11.0 + shift, 14.0),
                at(9.5 + shift, 16.0),
                at(7.0 + shift, 17.0),
            ],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    }
}

fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(CAPTION_ABOVE);
    ui.label(egui::RichText::new(text).font(heading(CAPTION_PX)).color(brand::MUTED));
    ui.add_space(CAPTION_BELOW);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{FullOutput, RawInput, Shape, pos2, vec2};

    fn code(lang: Option<&str>, text: &str) -> Piece<'static> {
        Piece::Code {
            lang: lang.map(|lang| &*Box::leak(lang.to_owned().into_boxed_str())),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_message_is_cut_at_its_top_level_fences() {
        let cases: [(&str, Vec<Piece>); 11] = [
            ("just words", vec![Piece::Markdown("just words")]),
            ("", vec![]),
            (
                "Run:\n\n```sh\ncargo test\n```\n\nDone then.",
                vec![
                    Piece::Markdown("Run:\n\n"),
                    code(Some("sh"), "cargo test"),
                    Piece::Markdown("\nDone then."),
                ],
            ),
            ("```\na\n\nb\n```", vec![code(None, "a\n\nb")]),
            ("~~~rust extra\nfn x() {}\n~~~", vec![code(Some("rust"), "fn x() {}")]),
            ("````\n```\ninner\n```\n````", vec![code(None, "```\ninner\n```")]),
            ("```sh\nno end\nstill code", vec![code(Some("sh"), "no end\nstill code")]),
            ("   ```\n   a\n    b\n   ```", vec![code(None, "a\n b")]),
            ("```\n```", vec![code(None, "")]),
            ("``` not `a fence` ```\ntext", vec![Piece::Markdown("``` not `a fence` ```\ntext")]),
            (
                "    ```\n    indented four is code anyway\n    ```",
                vec![Piece::Markdown("    ```\n    indented four is code anyway\n    ```")],
            ),
        ];
        for (input, want) in cases {
            assert_eq!(pieces(input, false), want, "input {input:?}");
        }
    }

    #[test]
    fn the_end_block_headings_become_captions_only_for_the_agent() {
        let text = "Shipped it.\n\n**Done**\n- a\n\n**Needs you**\n- nothing\n\n**Next**\n- b\n";
        let with = pieces(text, true);
        assert_eq!(
            with,
            [
                Piece::Markdown("Shipped it.\n\n"),
                Piece::Caption("Done"),
                Piece::Markdown("- a\n\n"),
                Piece::Caption("Needs you"),
                Piece::Markdown("- nothing\n\n"),
                Piece::Caption("Next"),
                Piece::Markdown("- b\n"),
            ]
        );
        assert_eq!(pieces(text, false), [Piece::Markdown(text)]);
    }

    #[test]
    fn a_sentence_or_a_fenced_line_that_looks_like_a_heading_is_no_caption() {
        let cases = [
            "Done with the refactor.\n\nNext time I will test more.",
            "**Done:** x\n**Needs you:** y\n**Next:** z",
            "**Done**\n```\n**Needs you**\n```\n**Next**\n- z",
        ];
        for input in cases {
            assert!(
                pieces(input, true).iter().all(|piece| !matches!(piece, Piece::Caption(_))),
                "input {input:?}"
            );
        }
    }

    fn draw(md: &str, captions: bool) -> FullOutput {
        let ctx = crate::fonts::test_context();
        let mut cache = CommonMarkCache::default();
        let mut out = None;
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 900.0))),
                ..RawInput::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                ui.set_width(500.0);
                show(ui, &mut cache, md, captions, Look::AGENT);
            }));
        }
        out.expect("two frames ran")
    }

    fn galleys(out: &FullOutput) -> Vec<std::sync::Arc<egui::Galley>> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some(text.galley.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn captions_are_13_px_semibold_muted_and_the_body_stays_plain() {
        let md = "Shipped.\n\n**Done**\n- one\n\n**Needs you**\n- nothing\n\n**Next**\n- two\n";
        let seen = galleys(&draw(md, true));
        for name in ["Done", "Needs you", "Next"] {
            let galley = seen.iter().find(|g| g.text() == name).unwrap_or_else(|| panic!("{name}"));
            let format = &galley.job.sections[0].format;
            assert_eq!(format.font_id, heading(13.0), "{name}");
            assert_eq!(format.color, brand::MUTED, "{name}");
        }
        let plain = galleys(&draw(md, false));
        assert!(plain.iter().all(|g| g.job.sections[0].format.font_id != heading(13.0)));
    }

    #[test]
    fn a_caption_has_space_above_it() {
        let tight =
            galleys(&draw("a\n\n**Done**\n- x\n\n**Needs you**\n- y\n\n**Next**\n- z", true));
        let top = |text: &str| {
            tight.iter().position(|g| g.text() == text).unwrap_or_else(|| panic!("{text}"))
        };
        assert!(top("Done") > top("a"));
        let out = draw("a\n\n**Done**\n- x\n\n**Needs you**\n- y\n\n**Next**\n- z", true);
        let ys: Vec<f32> = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) if matches!(text.galley.text(), "a" | "Done") => Some(text.pos.y),
                _ => None,
            })
            .collect();
        assert_eq!(ys.len(), 2);
        assert!(ys[1] - ys[0] > 16.0 + CAPTION_ABOVE - 1.0, "{ys:?}");
    }

    #[test]
    fn a_fence_in_a_message_is_drawn_by_the_code_block_with_a_copy_button() {
        let seen: Vec<String> = galleys(&draw("Run it:\n\n```sh\ncargo test\n```\n", true))
            .iter()
            .map(|g| g.text().to_owned())
            .collect();
        assert!(seen.iter().any(|text| text == "cargo test"), "{seen:?}");
        assert!(seen.contains(&"Copy".to_owned()));
        assert!(seen.contains(&"sh".to_owned()));
    }

    #[test]
    fn a_top_level_quote_is_one_piece_with_its_markers_taken_off() {
        let cases: [(&str, Vec<Piece>); 5] = [
            (
                "Before:\n\n> one\n> two\n\nAfter.",
                vec![
                    Piece::Markdown("Before:\n\n"),
                    Piece::Quote("one\ntwo".to_owned()),
                    Piece::Markdown("\nAfter."),
                ],
            ),
            ("> only", vec![Piece::Quote("only".to_owned())]),
            ("   >tight\n>\n> after a gap", vec![Piece::Quote("tight\n\nafter a gap".to_owned())]),
            ("- item\n    > deep in a list", vec![Piece::Markdown("- item\n    > deep in a list")]),
            ("```\n> in code\n```", vec![code(None, "> in code")]),
        ];
        for (input, want) in cases {
            assert_eq!(pieces(input, false), want, "input {input:?}");
        }
    }

    #[test]
    fn a_line_that_ends_in_an_image_chip_gives_the_chip_its_own_piece() {
        assert_eq!(
            pieces("See:\n\n`Image 1`\n", false),
            vec![Piece::Markdown("See:\n\n"), Piece::Image("Image 1".to_owned())],
        );
        assert_eq!(
            pieces("See the run: `Image 2`", false),
            vec![Piece::Markdown("See the run: "), Piece::Image("Image 2".to_owned())],
        );
        assert_eq!(
            pieces("use `Image 1` here", false),
            vec![Piece::Markdown("use `Image 1` here")]
        );
    }

    #[test]
    fn tables_are_cut_out_of_their_prose() {
        let md = "Before\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n| 3 | 4 |\n\nAfter";
        assert_eq!(
            table_chunks(md),
            [
                Chunk::Prose("Before\n\n"),
                Chunk::Table("| a | b |\n|---|:-:|\n| 1 | 2 |\n| 3 | 4 |\n"),
                Chunk::Prose("\nAfter"),
            ]
        );
        assert_eq!(table_chunks("a | b\nno divider"), [Chunk::Prose("a | b\nno divider")]);
        assert_eq!(table_chunks("| a |\n|---|"), [Chunk::Table("| a |\n|---|")]);
    }

    fn widest(md: &str, width: f32) -> f32 {
        let ctx = crate::fonts::test_context();
        let mut cache = CommonMarkCache::default();
        let mut widest = 0.0_f32;
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
                ..RawInput::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                ui.set_max_width(width);
                let shown = ui.scope(|ui| show(ui, &mut cache, md, false, Look::AGENT));
                widest = shown.response.rect.width();
            });
        }
        widest
    }

    #[test]
    fn a_wide_table_scrolls_inside_the_column_and_never_widens_it() {
        let cell = "a very long cell that would never fit in a narrow pane at all";
        let md = format!("| one | two | three |\n|---|---|---|\n| {cell} | {cell} | {cell} |\n");
        assert!(widest(&md, 360.0) <= 360.5);
        let quoted = md
            .lines()
            .collect::<Vec<_>>()
            .iter()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(widest(&quoted, 360.0) <= 360.5);
    }

    #[test]
    fn github_markup_in_a_message_never_reaches_the_screen() {
        let md = "<!-- codex-review-state:v1 {\"a\":1} -->\n<details><summary>More</summary>\n\nat <relative-time datetime=\"2026-10-25T10:00:00Z\">Oct 25</relative-time><br/>done\n</details>";
        let seen: String = galleys(&draw(md, false)).iter().map(|g| g.text().to_owned()).collect();
        assert!(seen.contains("More") && seen.contains("Oct 25") && seen.contains("done"));
        for raw in ["<!--", "<details", "<summary", "<br", "relative-time", "datetime"] {
            assert!(!seen.contains(raw), "{raw} showed in {seen:?}");
        }
    }
}
