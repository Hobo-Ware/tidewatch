//! A chat message cut into the pieces the Reader draws itself: fenced code blocks (with their own
//! copy button), top-level blockquotes (a quiet panel with a quote mark) and, in an agent's
//! message, the `Done` / `Needs you` / `Next` heading lines as captions. Everything between them
//! stays markdown for the renderer. Pure.

use egui::epaint::CubicBezierShape;
use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, Frame, Margin, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use tidewatch_core::{end_block, github_html};

use crate::brand;
use crate::code_block::{self, Look};
use crate::links::{closes_fence, fence_of};
use crate::md_blocks;
use crate::md_view;
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
const QUOTE_TEXT_PX: f32 = 16.0;
const QUOTE_MIN_TEXT: f32 = 80.0;
const BORDER: f32 = 1.0;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Markdown(&'a str),
    Code { lang: Option<&'a str>, text: String },
    Quote(String),
    Image(String),
    Caption(&'static str),
}

/// Cuts `md` at its top-level fences and quotes (up to 3 spaces of indent; a fence inside a quote
/// stays with the renderer, and so do a fence or a quote inside a list item, wherever they sit) and, with
/// `captions`, at the end block's heading-only lines. A fence the message never closes runs to its
/// end, as in CommonMark.
pub(crate) fn pieces(md: &str, captions: bool) -> Vec<Piece<'_>> {
    let heads = if captions { end_block::caption_lines(&outside_fences(md)) } else { Vec::new() };
    let mut out = Vec::new();
    let mut prose = 0..0;
    let mut open: Option<Open<'_>> = None;
    let mut quote: Vec<&str> = Vec::new();
    let lists = md_blocks::list_spans(md);
    let mut at = 0;
    for (index, line) in md.split_inclusive('\n').enumerate() {
        let end = at + line.len();
        let inside = open.is_none() && lists.iter().any(|span| span.contains(&at));
        let lazy = open.is_none() && !inside && continues_quote(&quote, line);
        let quoted = open.is_none() && !inside && (quote_text(line).is_some() || lazy);
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
        } else if inside {
            prose.end = end;
        } else if let Some(text) =
            quote_text(line).or(lazy.then(|| line.trim_end_matches(['\n', '\r'])))
        {
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

/// Whether `line`, with no `>` of its own, carries on the paragraph the quote so far ends in:
/// CommonMark's lazy continuation. A blank line, a line that starts another block and a quote
/// that ends in a heading, a rule or an open fence do not continue.
fn continues_quote(quote: &[&str], line: &str) -> bool {
    let Some(last) = quote.last() else { return false };
    let text = line.trim();
    let starts_block = text.is_empty()
        || text.starts_with(['#', '>', '|', '<', '-', '*', '+', '_', '=', '`', '~'])
        || line.starts_with("    ")
        || text
            .split_once(['.', ')'])
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let tail = last.trim();
    let closed_paragraph = tail.is_empty()
        || tail.starts_with(['#', '`', '~'])
        || tail.chars().all(|c| matches!(c, '-' | '*' | '_' | '='))
        || fence_open(quote);
    !starts_block && !closed_paragraph
}

fn fence_open(quote: &[&str]) -> bool {
    let mut open: Option<(char, usize)> = None;
    for line in quote {
        match open {
            Some(fence) if closes_fence(line, fence) => open = None,
            Some(_) => {}
            None => open = opens(line).map(|fence| (fence.mark, fence.run)),
        }
    }
    open.is_some()
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
pub(crate) fn show(ui: &mut Ui, md: &str, captions: bool, look: Look) {
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
            Piece::Markdown(chunk) => markdown(ui, chunk, look),
            Piece::Code { lang, text } => {
                code_block::show(ui, *lang, text, look);
            }
            Piece::Quote(text) => quote(ui, text, look),
            Piece::Image(label) => image_chip(ui, label),
            Piece::Caption(text) => caption(ui, text),
        });
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct QuoteShown {
    pub(crate) panel: Rect,
    pub(crate) copy: Rect,
}

fn button_reserve(painter: &egui::Painter) -> f32 {
    let button = code_block::button_width(painter);
    (code_block::BUTTON_INSET + button + QUOTE_GAP - f32::from(QUOTE_PAD.right)).max(0.0)
}

pub(crate) fn quote_width(painter: &egui::Painter, text: &str) -> f32 {
    let widest = text
        .lines()
        .map(|line| {
            painter.layout_no_wrap(line.to_owned(), body(QUOTE_TEXT_PX), Color32::WHITE).size().x
        })
        .fold(0.0, f32::max);
    widest.ceil()
        + button_reserve(painter)
        + QUOTE_MARK
        + QUOTE_GAP
        + f32::from(QUOTE_PAD.left + QUOTE_PAD.right)
        + 2.0 * BORDER
}

/// A quote's markdown as written in the message, without the `>` markers and without the blank
/// lines around it: the text to paste into another place that renders markdown.
pub(crate) fn quote_source(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    match first.zip(last) {
        Some((first, last)) => lines[first..=last].join("\n"),
        None => String::new(),
    }
}

fn quote(ui: &mut Ui, md: &str, look: Look) {
    let id = ui.id().with("quote-copy");
    let reserve = button_reserve(ui.painter());
    let framed = Frame::new()
        .fill(brand::SUNKEN)
        .stroke(Stroke::new(BORDER, brand::ITEM_LINE))
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
                    ui.set_width((ui.available_width() - reserve).max(QUOTE_MIN_TEXT));
                    ui.visuals_mut().override_text_color = Some(brand::STEP_TEXT);
                    markdown(ui, md, look);
                });
            });
        });
    let panel = framed.response.rect;
    let copy = code_block::copy_button(ui, id, code_block::corner_anchor(panel), &quote_source(md));
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new("quote-shown"), QuoteShown { panel, copy }));
    #[cfg(not(test))]
    let _ = copy;
}

fn markdown(ui: &mut Ui, md: &str, look: Look) {
    md_view::show(ui, md, look);
}

const CHIP_FRAME_W: f32 = 6.0 + THUMB_W + 10.0 + 12.0;

fn image_chip_galley(
    painter: &egui::Painter,
    label: &str,
    room: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(label.to_owned(), body(13.0), brand::STEP_TEXT);
    job.wrap = TextWrapping::truncate_at_width((room - CHIP_FRAME_W).max(0.0));
    painter.layout_job(job)
}

/// How wide the image chip for `label` draws: the chip a bubble has to make room for.
pub(crate) fn image_chip_width(painter: &egui::Painter, label: &str) -> f32 {
    CHIP_FRAME_W + image_chip_galley(painter, label, f32::INFINITY).size().x
}

fn image_chip(ui: &mut Ui, label: &str) {
    ui.horizontal(|ui| {
        let galley = image_chip_galley(ui.painter(), label, ui.available_width());
        let size = vec2(CHIP_FRAME_W + galley.size().x, THUMB_H + 12.0);
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
    fn a_fence_or_a_quote_inside_a_list_item_stays_in_the_item_whatever_its_indent() {
        let cases = [
            "- item\n\n  ```sh\n  cargo test\n  ```\n- next",
            "- item\n  > quoted in the item",
            "1. step\n\n   ```\n   code\n   ```",
        ];
        for input in cases {
            assert_eq!(pieces(input, false), [Piece::Markdown(input)], "input {input:?}");
        }
        assert_eq!(
            pieces("- a\n\n```\nafter\n```", false),
            [Piece::Markdown("- a\n\n"), code(None, "after")]
        );
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
        let mut out = None;
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 900.0))),
                ..RawInput::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                ui.set_width(500.0);
                crate::reader_chat_view::chat_text(ui, |ui| show(ui, md, captions, Look::AGENT));
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

    fn widest(md: &str, width: f32) -> f32 {
        let ctx = crate::fonts::test_context();
        let mut widest = 0.0_f32;
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))),
                ..RawInput::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                ui.set_max_width(width);
                let shown = ui.scope(|ui| show(ui, md, false, Look::AGENT));
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

    const DRAFT: &str = "> @Sam the sign-in change is in: `acme://auth` now opens the app from a cold start, \
        and the token is kept across restarts.\n>\n> Two things left on my side: the **retry** path and \
        the [rollout notes](https://example.com/notes).\n>\n> I will ping you once `acme://auth/callback` \
        is verified on a clean install.";

    fn quote_of(md: &str) -> String {
        let parts = pieces(md, false);
        let quotes: Vec<String> = parts
            .iter()
            .filter_map(|piece| match piece {
                Piece::Quote(text) => Some(quote_source(text)),
                _ => None,
            })
            .collect();
        assert_eq!(quotes.len(), 1, "one quote in {md:?}: {parts:?}");
        quotes.into_iter().next().expect("one quote")
    }

    #[test]
    fn the_copied_quote_is_its_markdown_without_the_markers() {
        let cases = [
            ("> a\n>\n> b", "a\n\nb"),
            (
                "> use `acme://auth` and **bold** [l](https://x.dev)",
                "use `acme://auth` and **bold** [l](https://x.dev)",
            ),
            ("> - a\n>   - b\n> - c\n>\n> 1. one\n> 2. two", "- a\n  - b\n- c\n\n1. one\n2. two"),
            ("> first\nlazy second\n\nafter", "first\nlazy second"),
            ("> ```sh\n> a > b\n> echo \"> c\"\n> ```", "```sh\na > b\necho \"> c\"\n```"),
            ("> outer\n> > inner", "outer\n> inner"),
            (">\n> padded\n>\n", "padded"),
            ("   >tight\n>  two spaces kept", "tight\n two spaces kept"),
            ("> a\r\n> b\r\n", "a\nb"),
            ("> keeps \u{2014} and \u{2013} as written", "keeps \u{2014} and \u{2013} as written"),
        ];
        for (input, want) in cases {
            assert_eq!(quote_of(input), want, "input {input:?}");
        }
    }

    #[test]
    fn a_lazy_line_joins_the_quote_only_while_a_paragraph_is_open() {
        let stays_out = [
            "> a\n\nb",
            "> a\n- b",
            "> a\n# b",
            "> a\n```\nb",
            "> # head\nb",
            "> ---\nb",
            "> ```\n> code\n> ```\nb",
            "> a\n1. b",
            "> a\n    b",
        ];
        for input in stays_out {
            let parts = pieces(input, false);
            assert!(
                matches!(&parts[0], Piece::Quote(text) if !text.contains('b')),
                "input {input:?}: {parts:?}"
            );
        }
        assert_eq!(pieces("> a\nb\nc\n> d", false), [Piece::Quote("a\nb\nc\nd".to_owned())]);
        assert_eq!(pieces("plain\nlines", false), [Piece::Markdown("plain\nlines")]);
    }

    const SCREEN: f32 = 1000.0;

    fn quote_frame(
        ctx: &egui::Context,
        (width, md): (f32, &str),
        events: Vec<egui::Event>,
        time: f64,
    ) -> (FullOutput, QuoteShown) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(SCREEN, 900.0))),
            events,
            focused: true,
            time: Some(time),
            ..RawInput::default()
        };
        let mut shown = None;
        let out = ctx.run_ui(raw, |ui| {
            ui.set_width(width);
            show(ui, md, false, Look::AGENT);
            shown = ui.data(|data| data.get_temp::<QuoteShown>(egui::Id::new("quote-shown")));
        });
        (out, shown.expect("the quote recorded where it drew"))
    }

    fn quote_settled(ctx: &egui::Context, quote: (f32, &str), time: f64) -> QuoteShown {
        let _ = quote_frame(ctx, quote, vec![], time);
        quote_frame(ctx, quote, vec![], time).1
    }

    fn rows(out: &FullOutput) -> Vec<(String, Rect)> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos, &text.galley)),
                _ => None,
            })
            .flat_map(|(label, pos, galley)| {
                galley
                    .rows
                    .iter()
                    .map(move |row| (label.clone(), row.rect().translate(pos.to_vec2())))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn the_copy_button_sits_in_the_quote_corner_like_the_code_blocks_and_no_text_runs_under_it() {
        let ctx = crate::fonts::test_context();
        for width in [240.0, 320.0, 480.0, 720.0] {
            for md in [
                DRAFT,
                "> short",
                "> one\n>\n> two",
                "> `acme://auth`",
                "> a very long unbroken word: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ] {
                let shown = quote_settled(&ctx, (width, md), 1.0);
                let (out, _) = quote_frame(&ctx, (width, md), vec![], 1.0);
                let room = format!("width {width}, {md:?}: {shown:?}");
                assert!((shown.panel.width() - width).abs() < 0.5, "{room}");
                assert_eq!(
                    shown.panel.right() - shown.copy.right(),
                    BORDER + code_block::BUTTON_INSET,
                    "{room}"
                );
                assert_eq!(shown.copy.top() - shown.panel.top(), BORDER + 4.0, "{room}");
                assert!(shown.copy.bottom() <= shown.panel.bottom(), "{room}");
                for (label, rect) in rows(&out) {
                    if label == "Copy" || label == "Copied" {
                        continue;
                    }
                    assert!(
                        !rect.intersects(shown.copy),
                        "{label:?} {rect:?} runs under the button, {room}"
                    );
                }
            }
        }
    }

    fn code_block_corner(ctx: &egui::Context, block: (f32, &str)) -> (Rect, Rect) {
        let raw = |time| RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(SCREEN, 900.0))),
            focused: true,
            time: Some(time),
            ..RawInput::default()
        };
        let mut shown = None;
        for _ in 0..2 {
            let _ = ctx.run_ui(raw(1.0), |ui| {
                ui.set_width(block.0);
                code_block::show(ui, Some("sh"), block.1, Look::AGENT);
                shown = ui.data(|data| {
                    data.get_temp::<code_block::Shown>(ui.id().with("code-block").with("shown"))
                });
            });
        }
        let shown = shown.expect("the code block recorded where it drew");
        (shown.frame, shown.copy)
    }

    fn press(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(ctx: &egui::Context, quote: (f32, &str), at: Pos2, time: f64) -> Vec<String> {
        let steps =
            [vec![egui::Event::PointerMoved(at)], vec![press(at, true)], vec![press(at, false)]];
        let mut copied = Vec::new();
        for (step, events) in steps.into_iter().enumerate() {
            let (out, _) = quote_frame(ctx, quote, events, time + step as f64 * 0.01);
            copied.extend(out.platform_output.commands.into_iter().filter_map(|command| {
                match command {
                    egui::OutputCommand::CopyText(text) => Some(text),
                    _ => None,
                }
            }));
        }
        copied
    }

    fn labels(out: &FullOutput) -> Vec<String> {
        rows(out).into_iter().map(|(label, _)| label).collect()
    }

    #[test]
    fn a_click_copies_the_quote_source_and_reads_copied_for_a_moment() {
        let ctx = crate::fonts::test_context();
        let quote = (480.0, DRAFT);
        let shown = quote_settled(&ctx, quote, 1.0);
        assert!(labels(&quote_frame(&ctx, quote, vec![], 1.0).0).contains(&"Copy".to_owned()));
        let copied = click(&ctx, quote, shown.copy.center(), 1.0);
        let want = quote_of(DRAFT);
        assert!(want.contains("\n\n") && want.contains("`acme://auth`") && !want.contains("> "));
        assert_eq!(copied, [want]);
        let during = labels(&quote_frame(&ctx, quote, vec![], 1.2).0);
        assert!(during.contains(&"Copied".to_owned()) && !during.contains(&"Copy".to_owned()));
        let later = labels(&quote_frame(&ctx, quote, vec![], 3.0).0);
        assert!(later.contains(&"Copy".to_owned()) && !later.contains(&"Copied".to_owned()));
        assert_eq!(quote_frame(&ctx, quote, vec![], 3.0).1.copy, shown.copy);
    }

    #[test]
    fn a_click_on_the_quote_text_copies_nothing() {
        let ctx = crate::fonts::test_context();
        let quote = (480.0, DRAFT);
        let shown = quote_settled(&ctx, quote, 1.0);
        let at = pos2(shown.panel.left() + 60.0, shown.panel.center().y);
        assert_eq!(click(&ctx, quote, at, 1.0), Vec::<String>::new());
    }

    #[test]
    fn a_quote_in_the_users_bubble_gets_the_button_too() {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(SCREEN, 900.0))),
            ..RawInput::default()
        };
        let mut seen = Vec::new();
        for _ in 0..2 {
            let out = ctx.run_ui(raw.clone(), |ui| {
                ui.set_width(400.0);
                show(ui, "> from me", false, Look::USER);
            });
            seen = labels(&out);
        }
        assert!(seen.contains(&"Copy".to_owned()), "{seen:?}");
    }

    #[test]
    fn a_quote_is_as_wide_as_its_text_plus_the_mark_and_the_button() {
        let ctx = crate::fonts::test_context();
        let width = ctx.run_ui(RawInput::default(), |ui| {
            let wide = quote_width(ui.painter(), "a much longer line than the other\nshort");
            let narrow = quote_width(ui.painter(), "short");
            assert!(wide > narrow);
            assert!(narrow > code_block::button_width(ui.painter()) + QUOTE_MARK);
        });
        drop(width);
    }

    #[test]
    fn the_quote_button_has_the_same_inset_and_size_as_the_code_blocks() {
        let ctx = crate::fonts::test_context();
        let (frame, copy) = code_block_corner(&ctx, (400.0, "x"));
        let shown = quote_settled(&ctx, (400.0, "> x"), 1.0);
        assert_eq!(shown.copy.size(), copy.size());
        assert_eq!(shown.panel.right() - shown.copy.right(), frame.right() - copy.right());
        assert_eq!(shown.copy.top() - shown.panel.top(), copy.top() - frame.top());
    }
}
