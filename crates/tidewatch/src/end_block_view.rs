//! The Done / Needs you / Next card (PLAN.md, D13): a compact line on the tab, above the panes, that
//! opens into three columns, and the same three segments as one line in an Overview row. What the
//! segments say is plain data; the painters only draw it.

use crate::brand;
use crate::link_text::Text;
use crate::reader_view::{INSET, paint_item};
use crate::screen_kit::{body, mono};
use egui::epaint::text::TextFormat;
use egui::{Align2, Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, Ui, pos2, vec2};
use tidewatch_core::end_block::EndBlock;

pub(crate) const BAND_H: f32 = 40.0;
const LINE_H: f32 = 18.0;
const HEAD_H: f32 = 34.0;
const MAX_ITEMS: usize = 6;
const SEGMENT_GAP: f32 = 18.0;
const PILL_H: f32 = 17.0;
const PILL_PAD: f32 = 8.0;
const CARD_PAD: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Done,
    NeedsYou,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) section: Section,
    pub(crate) label: &'static str,
    pub(crate) text: String,
    pub(crate) items: Vec<String>,
}

fn first_or(items: &[String], none: &str) -> String {
    items.first().map_or_else(|| none.to_owned(), Clone::clone)
}

pub(crate) fn segments(block: &EndBlock) -> [Segment; 3] {
    let needs: Vec<String> = if block.needs_input() { block.needs_you.clone() } else { Vec::new() };
    [
        Segment {
            section: Section::Done,
            label: "Done",
            text: first_or(&block.done, "-"),
            items: block.done.clone(),
        },
        Segment {
            section: Section::NeedsYou,
            label: "Needs you",
            text: first_or(&needs, "nothing"),
            items: needs,
        },
        Segment {
            section: Section::Next,
            label: "Next",
            text: first_or(&block.next, "-"),
            items: block.next.clone(),
        },
    ]
}

fn tone(segment: &Segment) -> Color32 {
    match segment.section {
        Section::Done => brand::DONE,
        Section::NeedsYou if segment.items.is_empty() => brand::MUTED,
        Section::NeedsYou => brand::NEEDS,
        Section::Next => brand::TEAL_TEXT,
    }
}

/// How tall the card is: one band collapsed, the longest column tall when open.
pub(crate) fn height(block: &EndBlock, expanded: bool) -> f32 {
    if !expanded {
        return BAND_H;
    }
    let longest =
        segments(block).iter().map(|s| s.items.len().clamp(1, MAX_ITEMS + 1)).max().unwrap_or(1);
    HEAD_H + LINE_H * longest as f32 + 14.0
}

/// Draws the card in `rect` and returns true when it was clicked (open or close it).
pub(crate) fn show_card(ui: &mut Ui, rect: Rect, block: &EndBlock, expanded: bool) -> bool {
    let card = rect.shrink2(vec2(INSET, 4.0));
    let response = ui.interact(card, ui.id().with("end-block"), Sense::click());
    paint_item(ui.painter(), card, response.hovered());
    let painter = ui.painter_at(card);
    let parts = segments(block);
    if expanded {
        let column_w = (card.width() - 2.0 * CARD_PAD) / 3.0;
        for (i, segment) in parts.iter().enumerate() {
            let x = card.left() + CARD_PAD + column_w * i as f32;
            let column =
                Rect::from_min_size(pos2(x, card.top()), vec2(column_w - 8.0, card.height()));
            paint_column(ui, &painter, column, segment);
        }
    } else {
        let id = ui.id().with("end-block-text");
        let mut shown = 0;
        paint_line(&painter, card.shrink2(vec2(CARD_PAD, 0.0)), block, |at, text, room| {
            shown += 1;
            linked_text(ui, id.with(shown), at, text, room);
        });
    }
    let tip = if expanded { "Click to collapse" } else { "Click to expand the agent's summary" };
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

/// Draws a segment's label as a tinted pill at `x`, centered on `y`; returns its right edge.
fn paint_pill(painter: &Painter, x: f32, y: f32, segment: &Segment) -> f32 {
    let ink = tone(segment);
    let galley = painter.layout_no_wrap(segment.label.to_owned(), body(11.5), ink);
    let rect = Rect::from_min_size(
        pos2(x, y - PILL_H / 2.0),
        vec2(galley.size().x + 2.0 * PILL_PAD, PILL_H),
    );
    painter.rect_filled(rect, CornerRadius::same(5), ink.gamma_multiply(0.14));
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
    rect.right()
}

/// A summary item at 12 px with its code spans in mono and its links clickable: `at` is the left
/// middle of the line, cut with an ellipsis at `max_w`.
fn linked_text(ui: &Ui, id: Id, at: Pos2, text: &str, max_w: f32) {
    let mut line = Text::new();
    for (span, code) in spans(text) {
        let (font, ink) = if code { (mono(11.5), brand::SOFT) } else { (body(12.0), brand::TEXT) };
        line.run(&span, &TextFormat::simple(font, ink));
    }
    let laid = line.cut(max_w).lay_out(ui.painter());
    laid.paint(ui, id, pos2(at.x, at.y - laid.size().y / 2.0), brand::TEXT);
}

/// Splits `text` at its backticks into spans, each flagged as code; the markers and bold go.
pub(crate) fn spans(text: &str) -> Vec<(String, bool)> {
    let text = text.replace("**", "");
    text.split('`')
        .enumerate()
        .filter(|(_, span)| !span.is_empty())
        .map(|(i, span)| (span.to_owned(), i % 2 == 1))
        .collect()
}

fn paint_column(ui: &Ui, painter: &Painter, column: Rect, segment: &Segment) {
    paint_pill(painter, column.left(), column.top() + HEAD_H / 2.0 + 1.0, segment);
    let shown =
        if segment.items.is_empty() { vec![segment.text.clone()] } else { segment.items.clone() };
    for (i, item) in shown.iter().take(MAX_ITEMS).enumerate() {
        let y = column.top() + HEAD_H + LINE_H * i as f32 + LINE_H / 2.0;
        let id = ui.id().with(("end-block-item", segment.label, i));
        linked_text(ui, id, pos2(column.left(), y), item, column.width());
    }
    if shown.len() > MAX_ITEMS {
        let y = column.top() + HEAD_H + LINE_H * MAX_ITEMS as f32 + LINE_H / 2.0;
        let more = format!("+{} more", shown.len() - MAX_ITEMS);
        painter.text(pos2(column.left(), y), Align2::LEFT_CENTER, more, body(11.5), brand::MUTED);
    }
}

/// The three segments on one line inside `rect`, each given an equal share of the width.
fn paint_line(
    painter: &Painter,
    rect: Rect,
    block: &EndBlock,
    mut text: impl FnMut(Pos2, &str, f32),
) {
    let parts = segments(block);
    let share = (rect.width() - SEGMENT_GAP * 2.0) / 3.0;
    for (i, segment) in parts.iter().enumerate() {
        let x = rect.left() + (share + SEGMENT_GAP) * i as f32;
        let right = paint_pill(painter, x, rect.center().y, segment);
        let room = x + share - right - 6.0;
        text(pos2(right + 6.0, rect.center().y), &segment.text, room);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(done: &[&str], needs: &[&str], next: &[&str]) -> EndBlock {
        let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        EndBlock { done: own(done), needs_you: own(needs), next: own(next) }
    }

    #[test]
    fn each_segment_shows_its_first_item_and_nothing_to_ask_reads_as_nothing() {
        let parts = segments(&block(&["a", "b"], &["nothing"], &["go"]));
        assert_eq!(parts[0].text, "a");
        assert_eq!(parts[1].text, "nothing");
        assert_eq!(parts[1].items, Vec::<String>::new());
        assert_eq!(parts[2].text, "go");
        let asking = segments(&block(&["a"], &["which base?"], &["wait"]));
        assert_eq!(asking[1].text, "which base?");
        assert_eq!(asking[1].items, ["which base?"]);
    }

    fn click(ctx: &egui::Context, block: &EndBlock, at: egui::Pos2) -> bool {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut clicked = false;
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
        {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 100.0), vec2(800.0, BAND_H));
                clicked |= show_card(ui, rect, block, false);
            });
        }
        clicked
    }

    #[test]
    fn clicking_the_card_asks_to_open_it_and_a_click_elsewhere_does_not() {
        let b = block(&["a"], &["nothing"], &["go"]);
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert!(click(&ctx, &b, pos2(400.0, 120.0)));
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert!(!click(&ctx, &b, pos2(400.0, 20.0)));
    }

    fn press_at(
        ctx: &egui::Context,
        block: &EndBlock,
        expanded: bool,
        at: egui::Pos2,
    ) -> (bool, Vec<String>) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (mut toggled, mut opened) = (false, Vec::new());
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
        {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let rect =
                    Rect::from_min_size(pos2(0.0, 100.0), vec2(800.0, height(block, expanded)));
                toggled |= show_card(ui, rect, block, expanded);
            });
            opened.extend(out.platform_output.commands.into_iter().filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.url),
                _ => None,
            }));
        }
        (toggled, opened)
    }

    #[test]
    fn a_link_in_the_card_opens_without_toggling_it_and_the_rest_of_the_card_still_toggles() {
        let b =
            block(&["Preview at localhost:5174 and `https://x.dev/docs`"], &["nothing"], &["go"]);
        for expanded in [false, true] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
            let y = if expanded { 100.0 + HEAD_H + LINE_H / 2.0 } else { 120.0 };
            let (mut urls, mut toggles) = (Vec::new(), 0);
            for x in (0..800).step_by(4) {
                let (toggled, opened) = press_at(&ctx, &b, expanded, pos2(x as f32, y));
                assert!(!toggled || opened.is_empty(), "x {x} both toggled and opened");
                toggles += usize::from(toggled);
                urls.extend(opened);
            }
            assert!(urls.iter().any(|u| u == "http://localhost:5174"), "{expanded} {urls:?}");
            assert!(urls.iter().any(|u| u == "https://x.dev/docs"), "{expanded} {urls:?}");
            assert!(toggles > 0);
        }
    }

    #[test]
    fn code_spans_keep_their_text_and_lose_their_markers() {
        assert_eq!(
            spans("added `expiry()` in `src/auth.rs:12`"),
            [
                ("added ".to_owned(), false),
                ("expiry()".to_owned(), true),
                (" in ".to_owned(), false),
                ("src/auth.rs:12".to_owned(), true),
            ]
        );
        assert_eq!(spans("**Done** it"), [("Done it".to_owned(), false)]);
        assert_eq!(spans("`only`"), [("only".to_owned(), true)]);
        assert_eq!(spans(""), Vec::<(String, bool)>::new());
    }

    #[test]
    fn an_empty_section_shows_a_dash() {
        let parts = segments(&block(&[], &[], &[]));
        assert_eq!((parts[0].text.as_str(), parts[2].text.as_str()), ("-", "-"));
    }

    #[test]
    fn the_card_is_one_band_until_opened_then_as_tall_as_its_longest_column() {
        let b = block(&["a", "b", "c"], &["nothing"], &["go"]);
        assert_eq!(height(&b, false), BAND_H);
        assert_eq!(height(&b, true), HEAD_H + LINE_H * 3.0 + 14.0);
        let many = block(&["1", "2", "3", "4", "5", "6", "7", "8"], &[], &[]);
        assert_eq!(height(&many, true), HEAD_H + LINE_H * (MAX_ITEMS + 1) as f32 + 14.0);
        assert_eq!(height(&block(&[], &[], &[]), true), HEAD_H + LINE_H + 14.0);
    }
}
