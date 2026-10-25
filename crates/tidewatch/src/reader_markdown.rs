//! A chat message cut into the pieces the Reader draws itself: fenced code blocks (with their own
//! copy button) and, in an agent's message, the `Done` / `Needs you` / `Next` heading lines as
//! captions. Everything between them stays markdown for the renderer. Pure.

use egui::Ui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use tidewatch_core::end_block;

use crate::brand;
use crate::code_block::{self, Look};
use crate::links::{closes_fence, fence_of};
use crate::screen_kit::heading;

const PIECE_GAP: f32 = 8.0;
const CAPTION_PX: f32 = 13.0;
const CAPTION_ABOVE: f32 = 10.0;
const CAPTION_BELOW: f32 = 2.0;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Markdown(&'a str),
    Code { lang: Option<&'a str>, text: String },
    Caption(&'static str),
}

/// Cuts `md` at its top-level fences (up to 3 spaces of indent; a fence inside a quote or deeper
/// in a list stays with the renderer) and, with `captions`, at the end block's heading-only lines.
/// A fence the message never closes runs to its end, as in CommonMark.
pub(crate) fn pieces(md: &str, captions: bool) -> Vec<Piece<'_>> {
    let heads = if captions { end_block::caption_lines(&outside_fences(md)) } else { Vec::new() };
    let mut out = Vec::new();
    let mut prose = 0..0;
    let mut open: Option<Open<'_>> = None;
    let mut at = 0;
    for (index, line) in md.split_inclusive('\n').enumerate() {
        let end = at + line.len();
        if let Some(fence) = &mut open {
            if closes_fence(line, (fence.mark, fence.run)) {
                out.push(open.take().expect("checked open").finish());
                prose = end..end;
            } else {
                fence.lines.push(line);
            }
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
    if let Some(fence) = open {
        out.push(fence.finish());
    } else {
        push_prose(&mut out, &md[prose]);
    }
    out
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
    let parts = pieces(md, captions);
    for (index, piece) in parts.iter().enumerate() {
        if index > 0
            && !matches!(piece, Piece::Caption(_))
            && !matches!(parts[index - 1], Piece::Caption(_))
        {
            ui.add_space(PIECE_GAP);
        }
        ui.push_id(index, |ui| match piece {
            Piece::Markdown(chunk) => {
                CommonMarkViewer::new().show(ui, cache, chunk);
            }
            Piece::Code { lang, text } => {
                code_block::show(ui, *lang, text, look);
            }
            Piece::Caption(text) => caption(ui, text),
        });
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
}
