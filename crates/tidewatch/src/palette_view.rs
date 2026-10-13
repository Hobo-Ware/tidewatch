//! Draws the command palette: a search field over a list of rows, the matched letters in the accent
//! colour, the chord on the right. Arrows move, Enter runs, Esc or a click outside closes.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, CornerRadius, FontId, Id, Key, Modifiers, Rect, ScrollArea, Sense, Stroke, StrokeKind,
    pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::{overlay_frame, text_field};

use crate::palette::{Hit, Item, Palette, Target, filter};

const WIDTH: f32 = 560.0;
const ROW_HEIGHT: f32 = 30.0;
const LIST_HEIGHT: f32 = ROW_HEIGHT * 9.0;
const TOP_GAP: f32 = 88.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Run(Target),
    Close,
}

pub(crate) fn show(ctx: &egui::Context, palette: &mut Palette, items: &[Item]) -> Option<Outcome> {
    let (down, up, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    let hits = filter(items, &palette.query);
    if down || up {
        palette.step(down, hits.len());
    }
    let mut picked = (enter && !hits.is_empty()).then(|| palette.selected(hits.len()));
    let mut moved = down || up;

    let modal = egui::Modal::new(Id::new("command_palette"))
        .area(
            egui::Area::new(Id::new("command_palette_area"))
                .anchor(Align2::CENTER_TOP, vec2(0.0, TOP_GAP))
                .order(egui::Order::Foreground),
        )
        .frame(overlay_frame())
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            let before = palette.query.clone();
            let field = ui
                .scope(|ui| {
                    text_field(ui, &mut palette.query, "Type a command", WIDTH, colors::fg())
                })
                .inner;
            if std::mem::take(&mut palette.focus) {
                field.request_focus();
            }
            if palette.query != before {
                palette.typed();
            }
            if hits.is_empty() {
                nothing_matches(ui);
            } else {
                let clicked = list(ui, palette, items, &hits, &mut moved);
                picked = picked.or(clicked);
            }
            ui.label(
                egui::RichText::new("Up / Down move   Enter runs   Esc closes")
                    .size(11.0)
                    .color(colors::dim()),
            );
        });
    if let Some(row) = picked {
        return Some(Outcome::Run(items[hits[row].item].target.clone()));
    }
    modal.should_close().then_some(Outcome::Close)
}

fn nothing_matches(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, ROW_HEIGHT * 2.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "Nothing matches",
        FontId::proportional(13.0),
        colors::dim(),
    );
}

fn list(
    ui: &mut egui::Ui,
    palette: &mut Palette,
    items: &[Item],
    hits: &[Hit],
    moved: &mut bool,
) -> Option<usize> {
    let selected = palette.selected(hits.len());
    let mut clicked = None;
    ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (row, hit) in hits.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(WIDTH, ROW_HEIGHT), Sense::click());
            if response.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                palette.select(row);
            }
            if response.clicked() {
                clicked = Some(row);
            }
            if row == selected && std::mem::take(moved) {
                ui.scroll_to_rect(rect, None);
            }
            paint_row(ui, rect, &items[hit.item], &hit.positions, row == selected);
        }
    });
    clicked
}

fn paint_row(ui: &egui::Ui, rect: Rect, item: &Item, positions: &[usize], selected: bool) {
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, CornerRadius::same(7), colors::selection());
        painter.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(1.0, colors::accent()),
            StrokeKind::Inside,
        );
    }
    let ink = colors::fg();
    let mut job = LayoutJob::default();
    for (i, c) in item.label.chars().enumerate() {
        let color = if positions.contains(&i) { colors::accent() } else { ink };
        job.append(&c.to_string(), 0.0, TextFormat::simple(FontId::proportional(14.0), color));
    }
    job.append(item.kind, 12.0, TextFormat::simple(FontId::proportional(11.0), colors::dim()));
    let keys_width = item.keys.chars().count() as f32 * 7.0 + 24.0;
    job.wrap = TextWrapping::truncate_at_width((rect.width() - 20.0 - keys_width).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    if !item.keys.is_empty() {
        painter.text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            &item.keys,
            FontId::monospace(11.5),
            colors::dim(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Command, Platform};
    use crate::palette::{Sources, items};
    use egui::{Event, RawInput, pos2};

    struct Harness {
        ctx: egui::Context,
        palette: Palette,
        items: Vec<Item>,
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    impl Harness {
        fn new(query: &str) -> Self {
            let sources = Sources {
                tabs: vec!["main".into(), "calm-comet".into()],
                active: 0,
                repos: vec!["web".into()],
                repo: 0,
                profiles: vec![("builder".into(), "Builder".into())],
                workflows: vec![("ship".into(), "Ship a feature".into())],
            };
            let mut harness = Self {
                ctx: egui::Context::default(),
                palette: Palette::open(query),
                items: items(&sources, Platform::CURRENT),
            };
            harness.frame(Vec::new());
            harness.frame(Vec::new());
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<Outcome> {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (palette, items) = (&mut self.palette, &self.items);
            let mut outcome = None;
            let _ = self.ctx.run_ui(raw, |ui| outcome = show(ui.ctx(), palette, items));
            outcome
        }
    }

    #[test]
    fn enter_runs_the_first_row() {
        let mut harness = Harness::new("");
        assert_eq!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Command(Command::NewTab)))
        );
    }

    #[test]
    fn arrows_move_the_selection_and_enter_runs_that_row() {
        let mut harness = Harness::new("");
        assert_eq!(harness.frame(vec![key(Key::ArrowDown)]), None);
        assert_eq!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Command(Command::CloseTab)))
        );
        let mut up = Harness::new("");
        up.frame(vec![key(Key::ArrowUp)]);
        assert_eq!(
            up.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Workflow("ship".into())))
        );
    }

    #[test]
    fn a_prefilled_query_filters_the_rows() {
        let mut harness = Harness::new("comet");
        assert_eq!(harness.frame(vec![key(Key::Enter)]), Some(Outcome::Run(Target::Tab(1))));
    }

    #[test]
    fn typing_narrows_the_list_and_resets_the_selection() {
        let mut harness = Harness::new("");
        harness.frame(vec![key(Key::ArrowDown)]);
        harness.frame(vec![Event::Text("ship".into())]);
        assert_eq!(harness.palette.query, "ship");
        assert_eq!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Workflow("ship".into())))
        );
    }

    #[test]
    fn escape_closes() {
        let mut harness = Harness::new("");
        assert_eq!(harness.frame(vec![key(Key::Escape)]), Some(Outcome::Close));
    }

    #[test]
    fn enter_with_nothing_matching_does_nothing() {
        let mut harness = Harness::new("zzzzzz");
        assert_eq!(harness.frame(vec![key(Key::Enter)]), None);
        assert_eq!(harness.frame(Vec::new()), None);
    }

    #[test]
    fn an_idle_palette_stays_open() {
        let mut harness = Harness::new("");
        for _ in 0..3 {
            assert_eq!(harness.frame(Vec::new()), None);
        }
    }
}
