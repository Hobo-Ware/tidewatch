//! Draws a chat message's markdown blocks (`md_blocks`) the way GitHub lays them out: a list sits
//! one step in from the text above it and every nested list one more, bullets change shape per
//! level, a wrapped line of an item lines up with the item's text, quotes carry a left bar and a
//! table scrolls inside the column.

use egui::epaint::text::TextFormat;
use egui::style::ScrollStyle;
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Grid, Layout, Margin, Rect, ScrollArea, Sense,
    Shape, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, Vec2, pos2, vec2,
};
use pulldown_cmark::Alignment;

use crate::brand;
use crate::code_block::{self, Look};
use crate::link_text::Text;
use crate::links;
use crate::md_blocks::{self, Block, Inline, Item, Kind, Note, Run};
use crate::screen_kit::{body, heading, mono};

pub(crate) const LINE: f32 = 1.6;
const BLOCK_GAP: f32 = 10.0;
const TIGHT_GAP: f32 = 4.0;
const HEADING_ABOVE: f32 = 16.0;
const HEADING_BELOW: f32 = 6.0;
pub(crate) const INDENT: f32 = 30.0;
pub(crate) const MARKER_EDGE: f32 = 8.0;
const BULLET_R: f32 = 3.0;
const BOX: f32 = 15.0;
const LIFT: f32 = 2.0;
const QUOTE_INDENT: f32 = 16.0;
const QUOTE_BAR: f32 = 3.0;
const CODE_EM: f32 = 0.88;
const NOTE_EM: f32 = 0.72;
const HEADING_EM: [f32; 6] = [1.7, 1.45, 1.25, 1.1, 1.0, 0.94];
const CELL: Margin = Margin { left: 10, right: 10, top: 6, bottom: 6 };
const TABLE_RADIUS: u8 = 6;

#[derive(Clone, Copy)]
struct Ctx {
    look: Look,
    depth: usize,
}

pub(crate) enum Marker<'a> {
    Bullet(usize),
    Number(&'a str),
    Task(bool),
}

pub(crate) fn show(ui: &mut Ui, md: &str, look: Look) {
    let tree = md_blocks::parse(md);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        blocks(ui, &tree, Ctx { look, depth: 0 });
    });
}

fn blocks(ui: &mut Ui, list: &[Block], ctx: Ctx) {
    for (index, block) in list.iter().enumerate() {
        if index > 0 {
            ui.add_space(gap(&list[index - 1], block));
        }
        ui.push_id(index, |ui| draw(ui, block, ctx));
    }
}

fn gap(prev: &Block, next: &Block) -> f32 {
    let tight = |block: &Block| matches!(block, Block::Paragraph { tight: true, .. });
    match (prev, next) {
        (Block::Heading { .. }, _) => HEADING_BELOW,
        (_, Block::Heading { .. }) => HEADING_ABOVE,
        _ if tight(prev) || tight(next) => TIGHT_GAP,
        _ => BLOCK_GAP,
    }
}

fn draw(ui: &mut Ui, block: &Block, ctx: Ctx) {
    match block {
        Block::Paragraph { runs, .. } => {
            text(ui, runs, 1.0, false, ink(ui), TextWrapMode::Wrap);
        }
        Block::Heading { level, runs } => heading_block(ui, *level, runs),
        Block::Rule => rule(ui),
        Block::Code { lang, text } => {
            code_block::show(ui, lang.as_deref(), text, ctx.look);
        }
        Block::Quote(inner) => quote(ui, inner, ctx),
        Block::List { start, items } => list(ui, *start, items, ctx),
        Block::Table { aligns, head, rows } => table(ui, aligns, head, rows),
        Block::Notes(notes) => notes_block(ui, notes, ctx),
    }
}

fn ink(ui: &Ui) -> Color32 {
    ui.visuals().text_color()
}

fn base_px(ui: &Ui) -> f32 {
    ui.style().text_styles.get(&TextStyle::Body).map_or(16.0, |font| font.size)
}

fn text(
    ui: &mut Ui,
    runs: &Inline,
    scale: f32,
    semibold: bool,
    color: Color32,
    wrap: TextWrapMode,
) {
    let size = base_px(ui) * scale;
    let mut line = Text::new();
    for run in runs {
        let format = format_of(ui, run, size, semibold, color);
        let label = match run.kind {
            Kind::Image if run.text.is_empty() => "Image".to_owned(),
            Kind::Image => format!("Image: {}", run.text),
            _ => run.text.clone(),
        };
        match run.link.as_deref().filter(|url| links::is_web_url(url)) {
            Some(url) => line.linked(&label, &format, url),
            None => line.raw(&label, &format),
        }
    }
    line.show(ui, ui.id().with("text"), color, wrap);
}

fn format_of(ui: &Ui, run: &Run, size: f32, semibold: bool, color: Color32) -> TextFormat {
    let linked = run.link.as_deref().is_some_and(links::is_web_url);
    let mut format = TextFormat::simple(body(size), color);
    format.line_height = Some(size * LINE);
    if run.kind == Kind::Note {
        format.font_id = body(size * NOTE_EM);
        format.valign = Align::TOP;
        format.color = brand::TEAL_TEXT;
        return format;
    }
    if run.kind == Kind::Code {
        let around = if run.style.bold || semibold { heading(size) } else { body(size) };
        let code = mono(size * CODE_EM);
        let lift = ascent(ui, around) - ascent(ui, code.clone());
        format.font_id = code;
        format.background = ui.visuals().code_bg_color;
        format.line_height = Some(size * LINE - lift);
        format.valign = Align::BOTTOM;
    } else if run.style.bold || semibold {
        format.font_id = heading(size);
    }
    format.italics = run.style.italic;
    if run.style.strike {
        format.strikethrough = Stroke::new(1.0, color);
    }
    if run.kind == Kind::Image {
        format.background = brand::ITEM;
        format.color = brand::TEAL_TEXT;
    }
    if linked {
        format.color = brand::TEAL_TEXT;
        format.underline = Stroke::new(1.0, brand::link_rest());
    }
    format
}

/// Where a font puts its baseline below the top of its row. epaint places each glyph at its own
/// font's ascent, so a smaller code face would sit higher than the words beside it.
fn ascent(ui: &Ui, font: egui::FontId) -> f32 {
    let galley = ui.painter().layout_no_wrap("x".to_owned(), font, Color32::WHITE);
    galley.rows.first().and_then(|row| row.glyphs.first()).map_or(0.0, |glyph| glyph.pos.y)
}

fn heading_block(ui: &mut Ui, level: u8, runs: &Inline) {
    let index = usize::from(level.clamp(1, 6)) - 1;
    let color = if level == 6 { brand::MUTED } else { ink(ui) };
    text(ui, runs, HEADING_EM[index], true, color, TextWrapMode::Wrap);
    if level <= 2 {
        ui.add_space(4.0);
        rule_line(ui);
    }
}

fn rule(ui: &mut Ui) {
    ui.add_space(4.0);
    rule_line(ui);
    ui.add_space(4.0);
}

fn rule_line(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, brand::ITEM_LINE);
}

fn quote(ui: &mut Ui, inner: &[Block], ctx: Ctx) {
    let left = ui.cursor().min.x;
    let shown = ui.horizontal_top(|ui| {
        ui.add_space(QUOTE_INDENT);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.visuals_mut().override_text_color = Some(brand::SOFT);
            blocks(ui, inner, ctx);
        });
    });
    let rect = shown.response.rect;
    let bar = Rect::from_min_max(
        pos2(left + 2.0, rect.top()),
        pos2(left + 2.0 + QUOTE_BAR, rect.bottom()),
    );
    ui.painter().rect_filled(bar, CornerRadius::same(2), brand::FAINT);
}

fn list(ui: &mut Ui, start: Option<u64>, items: &[Item], ctx: Ctx) {
    let labels: Vec<String> = start
        .map(|first| (first..).zip(items).map(|(number, _)| format!("{number}.")).collect())
        .unwrap_or_default();
    let gutter = gutter_for(ui, &labels);
    let loose = items.iter().any(|item| {
        item.blocks.iter().any(|block| matches!(block, Block::Paragraph { tight: false, .. }))
    });
    let inner = Ctx { depth: ctx.depth + 1, ..ctx };
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            ui.add_space(if loose { BLOCK_GAP } else { TIGHT_GAP });
        }
        let marker = match (item.task, labels.get(index)) {
            (Some(done), _) => Marker::Task(done),
            (None, Some(label)) => Marker::Number(label),
            (None, None) => Marker::Bullet(ctx.depth),
        };
        ui.push_id(index, |ui| entry(ui, &marker, gutter, &item.blocks, inner));
    }
}

fn gutter_for(ui: &Ui, labels: &[String]) -> f32 {
    let size = base_px(ui);
    let widest = labels
        .iter()
        .map(|label| {
            ui.painter().layout_no_wrap(label.clone(), body(size), Color32::WHITE).size().x
        })
        .fold(0.0, f32::max);
    INDENT.max((widest + 2.0 * MARKER_EDGE).ceil())
}

fn entry(ui: &mut Ui, marker: &Marker, gutter: f32, inner: &[Block], ctx: Ctx) {
    let origin = ui.cursor().min;
    ui.horizontal_top(|ui| {
        ui.add_space(gutter);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            blocks(ui, inner, ctx);
        });
    });
    let first_line = base_px(ui) * LINE;
    let edge = origin.x + gutter - MARKER_EDGE;
    paint_marker(ui, marker, pos2(edge, origin.y), (first_line, body(base_px(ui))));
}

pub(crate) fn paint_marker(
    ui: &Ui,
    marker: &Marker,
    edge_top: egui::Pos2,
    (line, number_font): (f32, FontId),
) {
    let painter = ui.painter();
    let edge_center = edge_top + vec2(0.0, line / 2.0 - LIFT);
    let center = edge_center - vec2(BULLET_R + 3.0, 0.0);
    let color = brand::SOFT;
    match marker {
        Marker::Bullet(depth) => match depth % 3 {
            0 => {
                painter.circle_filled(center, BULLET_R, color);
            }
            1 => {
                painter.circle_stroke(center, BULLET_R, Stroke::new(1.3, color));
            }
            _ => {
                let square = Rect::from_center_size(center, vec2(5.5, 5.5));
                painter.rect_filled(square, CornerRadius::same(1), color);
            }
        },
        Marker::Number(label) => {
            let mut format = TextFormat::simple(number_font, color);
            format.line_height = Some(line);
            let mut job = egui::text::LayoutJob::default();
            job.append(label, 0.0, format);
            let galley = painter.layout_job(job);
            painter.galley(edge_top - vec2(galley.size().x, 0.0), galley, color);
        }
        Marker::Task(done) => paint_box(ui, edge_center - vec2(BOX / 2.0, 0.0), *done),
    }
}

fn paint_box(ui: &Ui, center: egui::Pos2, done: bool) {
    let rect = Rect::from_center_size(center, vec2(BOX, BOX));
    let painter = ui.painter();
    let radius = CornerRadius::same(4);
    if done {
        painter.rect(
            rect,
            radius,
            brand::TEAL_NAV,
            Stroke::new(1.3, brand::TEAL),
            StrokeKind::Inside,
        );
        let at = |x: f32, y: f32| rect.min + vec2(x, y) * (BOX / 15.0);
        painter.add(Shape::line(
            vec![at(3.8, 7.8), at(6.4, 10.6), at(11.2, 4.8)],
            Stroke::new(1.7, brand::TEAL_TEXT),
        ));
    } else {
        painter.rect_stroke(rect, radius, Stroke::new(1.3, brand::FAINT), StrokeKind::Inside);
    }
}

fn notes_block(ui: &mut Ui, notes: &[Note], ctx: Ctx) {
    ui.add_space(BLOCK_GAP);
    rule_line(ui);
    ui.add_space(BLOCK_GAP);
    ui.scope(|ui| {
        ui.visuals_mut().override_text_color = Some(brand::SOFT);
        let labels: Vec<String> = notes.iter().map(|note| format!("{}.", note.label)).collect();
        let gutter = gutter_for(ui, &labels);
        for (index, note) in notes.iter().enumerate() {
            if index > 0 {
                ui.add_space(TIGHT_GAP);
            }
            ui.push_id(index, |ui| {
                entry(ui, &Marker::Number(&labels[index]), gutter, &note.blocks, ctx);
            });
        }
    });
}

fn table(ui: &mut Ui, aligns: &[Alignment], head: &[Inline], rows: &[Vec<Inline>]) {
    let id = ui.id().with("table");
    ui.scope(|ui| {
        ui.spacing_mut().scroll = ScrollStyle::solid();
        ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            Frame::new()
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(CornerRadius::same(TABLE_RADIUS))
                .show(ui, |ui| {
                    Grid::new(id)
                        .spacing(Vec2::ZERO)
                        .with_row_color(|row, _| (row == 0).then_some(brand::ITEM))
                        .show(ui, |ui| {
                            cells(ui, aligns, head, true);
                            ui.end_row();
                            for row in rows {
                                cells(ui, aligns, row, false);
                                ui.end_row();
                            }
                        });
                });
        });
    });
}

fn cells(ui: &mut Ui, aligns: &[Alignment], row: &[Inline], header: bool) {
    for (column, cell) in row.iter().enumerate() {
        let side = match aligns.get(column) {
            Some(Alignment::Right) => Align::Max,
            Some(Alignment::Center) => Align::Center,
            _ => Align::Min,
        };
        ui.push_id(column, |ui| {
            Frame::new().inner_margin(CELL).show(ui, |ui| {
                ui.with_layout(Layout::top_down(side), |ui| {
                    let color = ink(ui);
                    text(ui, cell, 1.0, header, color, TextWrapMode::Extend);
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, FullOutput, Modifiers, OutputCommand, PointerButton, RawInput, Shape, pos2};

    const WIDTH: f32 = 420.0;

    fn run_frames(md: &str, frames: Vec<Vec<Event>>) -> Vec<FullOutput> {
        let ctx = crate::fonts::test_context();
        let mut outs = Vec::new();
        let mut steps = vec![vec![], vec![]];
        steps.extend(frames);
        for events in steps {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            outs.push(ctx.run_ui(raw, |ui| {
                ui.set_width(WIDTH);
                crate::reader_chat_view::chat_text(ui, |ui| show(ui, md, Look::AGENT));
            }));
        }
        outs
    }

    fn draw(md: &str) -> FullOutput {
        run_frames(md, vec![]).pop().expect("frames ran")
    }

    fn texts(out: &FullOutput) -> Vec<(String, egui::Pos2, f32)> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.pos,
                    text.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size),
                )),
                _ => None,
            })
            .collect()
    }

    fn at(out: &FullOutput, text: &str) -> egui::Pos2 {
        texts(out)
            .into_iter()
            .find(|(shown, ..)| shown.starts_with(text))
            .unwrap_or_else(|| panic!("{text} not drawn"))
            .1
    }

    fn baselines(out: &FullOutput, needle: &str) -> Vec<(bool, f32)> {
        let text = out
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.text().contains(needle) => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{needle} not drawn"));
        let row = &text.galley.rows[0];
        let smallest =
            row.glyphs.iter().map(|glyph| glyph.font_height).fold(f32::INFINITY, f32::min);
        row.glyphs
            .iter()
            .filter(|glyph| !glyph.chr.is_whitespace())
            .map(|glyph| ((glyph.font_height - smallest).abs() < 0.01, row.pos.y + glyph.pos.y))
            .collect()
    }

    #[test]
    fn inline_code_sits_on_the_same_baseline_as_the_words_around_it() {
        for md in ["Pushed `feat/jellyfin` back to origin", "**Done: `feat/x` is back on origin**"]
        {
            let found = baselines(&draw(md), "back");
            let code: Vec<f32> = found.iter().filter(|g| g.0).map(|g| g.1).collect();
            let words: Vec<f32> = found.iter().filter(|g| !g.0).map(|g| g.1).collect();
            assert!(!code.is_empty() && !words.is_empty(), "{md}: {found:?}");
            for y in &code {
                assert!((y - words[0]).abs() <= 1.0, "{md}: code at {y}, words at {}", words[0]);
            }
        }
    }

    #[test]
    fn a_list_sits_in_from_the_paragraph_and_each_level_one_step_more() {
        let out = draw("para\n\n- one\n  - two\n    - three\n      - four");
        let xs: Vec<f32> =
            ["para", "one", "two", "three", "four"].iter().map(|word| at(&out, word).x).collect();
        for pair in xs.windows(2) {
            assert!(pair[1] - pair[0] >= INDENT - 0.5, "{xs:?}");
        }
        assert!((xs[2] - xs[1] - INDENT).abs() < 0.5, "{xs:?}");
        assert!((xs[3] - xs[2] - INDENT).abs() < 0.5, "{xs:?}");
    }

    #[test]
    fn wrapped_lines_of_an_item_line_up_with_its_text_not_its_bullet() {
        let long = "word ".repeat(40);
        let out = draw(&format!("- {long}\n  1. {long}"));
        let rows: Vec<egui::Pos2> = texts(&out)
            .into_iter()
            .filter(|(text, ..)| text.starts_with("word"))
            .map(|(_, pos, _)| pos)
            .collect();
        assert_eq!(rows.len(), 2, "one galley per item");
        let first = at(&out, "word").x;
        assert!((rows[0].x - first).abs() < 0.1);
        assert!(rows[1].x - rows[0].x >= INDENT - 0.5);
        let galleys: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.text().starts_with("word") => {
                    Some(text.galley.clone())
                }
                _ => None,
            })
            .collect();
        for galley in galleys {
            assert!(galley.rows.len() > 1, "the long item wraps");
            let lefts: Vec<f32> = galley.rows.iter().map(|row| row.pos.x).collect();
            assert!(lefts.iter().all(|x| x.abs() < 0.1), "{lefts:?}");
        }
    }

    #[test]
    fn a_numbered_list_draws_its_own_numbers_from_the_start() {
        let out = draw("3. a\n4. b\n5. c");
        let numbers: Vec<String> = texts(&out)
            .into_iter()
            .map(|(text, ..)| text)
            .filter(|text| text.ends_with('.'))
            .collect();
        assert_eq!(numbers, ["3.", "4.", "5."]);
    }

    fn markers(out: &FullOutput) -> (usize, usize, usize) {
        let mut filled = 0;
        let mut ring = 0;
        let mut square = 0;
        for clipped in &out.shapes {
            match &clipped.shape {
                Shape::Circle(c) if c.fill != Color32::TRANSPARENT && c.radius < 4.0 => filled += 1,
                Shape::Circle(c) if c.fill == Color32::TRANSPARENT && c.radius < 4.0 => ring += 1,
                Shape::Rect(r) if (r.rect.width() - 5.5).abs() < 0.1 => square += 1,
                _ => {}
            }
        }
        (filled, ring, square)
    }

    #[test]
    fn bullets_are_a_dot_a_ring_and_a_square_by_level_and_then_repeat() {
        let out = draw("- a\n  - b\n    - c\n      - d");
        assert_eq!(markers(&out), (2, 1, 1));
    }

    #[test]
    fn task_items_draw_boxes_instead_of_bullets() {
        let out = draw("- [x] done\n- [ ] open\n- plain");
        assert_eq!(markers(&out), (1, 0, 0));
        let boxes = out
            .shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, Shape::Rect(r) if (r.rect.width() - BOX).abs() < 0.1)
            })
            .count();
        assert_eq!(boxes, 2);
    }

    #[test]
    fn headings_get_smaller_from_one_to_six() {
        let out = draw("# a\n\n## b\n\n### c\n\n#### d\n\n##### e\n\n###### f");
        let sizes: Vec<f32> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|name| {
                texts(&out)
                    .into_iter()
                    .find(|(text, ..)| text == name)
                    .unwrap_or_else(|| panic!("{name}"))
                    .2
            })
            .collect();
        for pair in sizes.windows(2) {
            assert!(pair[0] > pair[1] - 0.001, "{sizes:?}");
        }
        assert!(sizes[0] > sizes[3] + 4.0, "{sizes:?}");
    }

    #[test]
    fn a_quote_draws_a_left_bar_and_a_nested_quote_a_second_one() {
        let bars = |md: &str| {
            draw(md)
                .shapes
                .iter()
                .filter(|clipped| {
                    matches!(&clipped.shape, Shape::Rect(r)
                        if (r.rect.width() - QUOTE_BAR).abs() < 0.1 && r.rect.height() > 10.0)
                })
                .count()
        };
        assert_eq!(bars("> one"), 1);
        assert_eq!(bars("> one\n> > two"), 2);
        assert_eq!(bars("no quote"), 0);
    }

    #[test]
    fn a_code_block_inside_a_list_item_is_the_code_card() {
        let seen: Vec<String> = texts(&draw("- item\n\n  ```sh\n  cargo test\n  ```"))
            .into_iter()
            .map(|(text, ..)| text)
            .collect();
        assert!(seen.iter().any(|text| text == "cargo test"), "{seen:?}");
        assert!(seen.contains(&"Copy".to_owned()));
    }

    #[test]
    fn a_table_draws_its_header_in_semibold_and_every_cell() {
        let out = draw("| name | n |\n|---|--:|\n| one | 1 |");
        let seen = texts(&out);
        for cell in ["name", "n", "one", "1"] {
            assert!(seen.iter().any(|(text, ..)| text == cell), "{cell}");
        }
    }

    fn center_of(out: &FullOutput, text: &str) -> egui::Pos2 {
        let (_, pos, _) = texts(out).into_iter().find(|(t, ..)| t.starts_with(text)).expect(text);
        let galley = out
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Text(shape) if shape.galley.text().starts_with(text) => {
                    Some(shape.galley.clone())
                }
                _ => None,
            })
            .expect(text);
        pos + galley.size() / 2.0
    }

    fn button(at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn dragging_across_rendered_text_selects_it_and_copy_takes_it() {
        let md = "Hello brave new world";
        let probe = draw(md);
        let start = at(&probe, "Hello") + vec2(-1.0, 8.0);
        let middle = center_of(&probe, "Hello");
        let end = pos2(middle.x * 2.0 - at(&probe, "Hello").x - 2.0, start.y);
        let frames = vec![
            vec![Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![Event::PointerMoved(pos2(start.x + 40.0, start.y))],
            vec![Event::PointerMoved(end)],
            vec![button(end, false)],
            vec![Event::Copy],
        ];
        let outs = run_frames(md, frames);
        let copied: Vec<String> = outs
            .iter()
            .flat_map(|out| out.platform_output.commands.iter())
            .filter_map(|command| match command {
                OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(copied.len(), 1, "{copied:?}");
        assert!(copied[0].contains("Hello brave"), "{copied:?}");
    }

    #[test]
    fn a_click_on_a_web_link_still_opens_it_and_a_plain_click_opens_nothing() {
        let md = "see [docs](https://x.dev/docs) now";
        let probe = draw(md);
        let galley = probe
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Text(shape) => Some((shape.pos, shape.galley.clone())),
                _ => None,
            })
            .expect("text");
        let on_link = galley.0 + vec2(galley.1.size().x * 0.45, galley.1.size().y / 2.0);
        let off_link = galley.0 + vec2(2.0, galley.1.size().y / 2.0);
        let opened = |point: egui::Pos2| -> Vec<String> {
            run_frames(
                md,
                vec![
                    vec![Event::PointerMoved(point)],
                    vec![button(point, true)],
                    vec![button(point, false)],
                ],
            )
            .iter()
            .flat_map(|out| out.platform_output.commands.iter())
            .filter_map(|command| match command {
                OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect()
        };
        assert_eq!(opened(on_link), ["https://x.dev/docs"]);
        assert_eq!(opened(off_link).len(), 0);
    }
}
