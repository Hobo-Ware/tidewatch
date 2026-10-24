//! Draws the command palette: a search field over a list of rows, the matched letters in the accent
//! colour, the chord on the right. Arrows move, Enter runs, Esc or a click outside closes.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, FontId, Frame, Id, Key, Margin, Modifiers, Rect, ScrollArea, Sense, Stroke,
    pos2, vec2,
};

use crate::brand;
use crate::notice::key_hint;
use crate::palette::{Hit, Item, Palette, Target, filter};
use crate::screen_kit::selection_row;
use crate::settings_ui::text_input;

const WIDTH: f32 = 560.0;
const PAD: i8 = 12;
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
    let runnable = |row: usize| items[hits[row].item].disabled.is_none();
    let mut picked = (enter && !hits.is_empty())
        .then(|| palette.selected(hits.len()))
        .filter(|row| runnable(*row));
    let mut moved = down || up;

    let modal = egui::Modal::new(Id::new("command_palette"))
        .area(
            egui::Area::new(Id::new("command_palette_area"))
                .anchor(Align2::CENTER_TOP, vec2(0.0, TOP_GAP))
                .order(egui::Order::Foreground),
        )
        .frame(
            Frame::new()
                .fill(brand::CARD)
                .stroke(Stroke::new(1.0, brand::FIELD_LINE))
                .corner_radius(12)
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: Color32::from_black_alpha(100),
                })
                .inner_margin(Margin::same(PAD)),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            let before = palette.query.clone();
            let field = text_input(ui, &mut palette.query, "Type a command");
            if std::mem::take(&mut palette.focus) {
                field.request_focus();
            }
            if palette.query != before {
                palette.typed();
            }
            ui.add_space(4.0);
            let divider = field.rect.x_range().expand(f32::from(PAD));
            ui.painter().hline(divider, ui.cursor().top(), Stroke::new(1.0, brand::CARD_LINE));
            ui.add_space(4.0);
            if hits.is_empty() {
                nothing_matches(ui);
            } else {
                let clicked = list(ui, palette, items, &hits, &mut moved);
                picked = picked.or(clicked.filter(|row| runnable(*row)));
            }
            key_hint(ui, "Up and Down move  -  Enter runs  -  Esc closes");
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
        brand::MUTED,
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
            let item = &items[hit.item];
            if let Some(reason) = item.disabled {
                response.on_hover_text(reason);
            }
            paint_row(ui, rect, item, hit, row == selected);
        }
    });
    clicked
}

fn paint_row(ui: &egui::Ui, rect: Rect, item: &Item, hit: &Hit, selected: bool) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, selected, false);
    let ink = if item.disabled.is_some() { brand::MUTED } else { brand::TEXT };
    let mut job = LayoutJob::default();
    let label_marks: &[usize] = if hit.alias.is_some() { &[] } else { &hit.positions };
    for (i, c) in item.label.chars().enumerate() {
        let color = if label_marks.contains(&i) && item.disabled.is_none() {
            brand::TEAL_TEXT
        } else {
            ink
        };
        job.append(&c.to_string(), 0.0, TextFormat::simple(FontId::proportional(14.0), color));
    }
    job.append(item.kind, 12.0, TextFormat::simple(FontId::proportional(11.0), brand::MUTED));
    if let Some(alias) = hit.alias {
        let muted = TextFormat::simple(FontId::proportional(11.0), brand::MUTED);
        job.append("also called ", 8.0, muted);
        for (i, c) in alias.chars().enumerate() {
            let color = if hit.positions.contains(&i) { brand::TEAL_TEXT } else { brand::MUTED };
            job.append(&c.to_string(), 0.0, TextFormat::simple(FontId::proportional(11.0), color));
        }
    }
    let side = item.disabled.unwrap_or(&item.keys);
    let keys_width = side.chars().count() as f32 * 7.0 + 24.0;
    job.wrap = TextWrapping::truncate_at_width((rect.width() - 20.0 - keys_width).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    if !side.is_empty() {
        let font = if item.disabled.is_some() {
            FontId::proportional(11.5)
        } else {
            FontId::monospace(11.5)
        };
        painter.text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            side,
            font,
            brand::MUTED,
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
            Self::with(query, None)
        }

        fn with(query: &str, build_workflow_blocked: Option<&'static str>) -> Self {
            let sources = Sources {
                tabs: vec![
                    crate::palette::TabRow::new("main", false),
                    crate::palette::TabRow::new("calm-comet", false),
                ],
                active: 0,
                repos: vec!["web".into()],
                repo: 0,
                profiles: vec![("builder".into(), "Builder".into())],
                workflows: vec![("ship".into(), "Ship a feature".into())],
                build_workflow_blocked,
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
    fn enter_on_a_disabled_row_runs_nothing_and_a_ready_one_runs() {
        let blocked = Some("agent is working");
        let mut harness = Harness::with("build workflow", blocked);
        assert_eq!(harness.frame(vec![key(Key::Enter)]), None);
        assert_eq!(harness.frame(Vec::new()), None, "the palette stays open");
        let mut ready = Harness::with("build workflow", None);
        assert_eq!(
            ready.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Command(Command::BuildWorkflow)))
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
