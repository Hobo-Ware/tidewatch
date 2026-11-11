//! Draws the command palette: a 52 px search header (a magnifier, the query, the result count),
//! the results under small group labels with one icon per row (a tab's own avatar for a tab row),
//! the matched letters in the accent colour and the chord as key caps at the right, and a footer strip with the key hints. Arrows
//! move, Enter runs, Esc or a click outside closes. The list is as tall as its rows, up to
//! `LIST_MAX`.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, FontId, Frame, Id, Key, Margin, Modifiers, Rect, ScrollArea, Sense,
    Stroke, TextEdit, Ui, pos2, vec2,
};
use stdusk_egui::ui::icons;

use crate::agent_avatar::{self, Face};
use crate::brand;
use crate::palette::{Hit, Item, Palette, Target, filter, group_title, grouped};
use crate::review::plural;
use crate::screen_kit::{
    FACE, TEXT_BODY, TEXT_TAG, TEXT_TITLE, body, caps_width, menu_label, paint_caps, selection_row,
};

pub(crate) const WIDTH: f32 = 600.0;
pub(crate) const HEADER_H: f32 = 52.0;
const FOOTER_H: f32 = 36.0;
pub(crate) const SIDE: f32 = 16.0;
const ROW_HEIGHT: f32 = 36.0;
const ROW_PAD: f32 = 10.0;
const LIST_PAD: f32 = 6.0;
const LIST_MAX: f32 = 400.0;
pub(crate) const TOP_GAP: f32 = 88.0;
const ICON: f32 = 16.0;
const HINTS: [(&str, &str); 3] = [("Up+Down", "pick"), ("Enter", "run"), ("Esc", "close")];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Run(Target),
    Close,
}

/// The face a row of the palette wears: the avatar of the tab a tab row switches to or opens.
/// `faces` is indexed like `Target::Tab`; a tab without one, and every other row, keeps its icon.
pub(crate) fn tab_face<'a>(item: &Item, faces: &'a [Option<Face>]) -> Option<&'a Face> {
    match item.target {
        Target::Tab(index) => faces.get(index)?.as_ref(),
        _ => None,
    }
}

pub(crate) fn show(
    ctx: &egui::Context,
    palette: &mut Palette,
    items: &[Item],
    faces: &[Option<Face>],
) -> Option<Outcome> {
    let (down, up, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    let hits = grouped(filter(items, &palette.query), items);
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
        .frame(card_frame())
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            header(ui, palette, hits.len());
            if hits.is_empty() {
                nothing_matches(ui);
            } else {
                let clicked = list(ui, palette, (items, faces), &hits, &mut moved);
                picked = picked.or(clicked);
            }
            footer(ui);
        });
    if let Some(row) = picked {
        return Some(Outcome::Run(items[hits[row].item].target.clone()));
    }
    modal.should_close().then_some(Outcome::Close)
}

/// The card the palette and the Message all agents sheet share.
pub(crate) fn card_frame() -> Frame {
    Frame::new()
        .fill(brand::card())
        .stroke(Stroke::new(1.0, brand::card_line()))
        .corner_radius(14)
        .shadow(egui::Shadow {
            offset: [0, 16],
            blur: 40,
            spread: 0,
            color: Color32::from_black_alpha(128),
        })
        .inner_margin(Margin::ZERO)
}

fn header(ui: &mut Ui, palette: &mut Palette, count: usize) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, HEADER_H), Sense::hover());
    let mid = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + SIDE + ICON / 2.0, mid),
        Align2::CENTER_CENTER,
        icons::MAGNIFYING_GLASS,
        FontId::proportional(18.0),
        brand::muted(),
    );
    let count_text = plural(count, "result");
    let count_galley = ui.painter().layout_no_wrap(count_text, body(TEXT_TAG), brand::faint());
    let count_w = count_galley.size().x;
    ui.painter().galley(
        pos2(rect.right() - SIDE - count_w, mid - count_galley.size().y / 2.0),
        count_galley,
        brand::faint(),
    );
    let field_rect = Rect::from_min_max(
        pos2(rect.left() + SIDE + ICON + 12.0, rect.top()),
        pos2(rect.right() - SIDE - count_w - 12.0, rect.bottom()),
    );
    let before = palette.query.clone();
    let field = ui.put(
        field_rect,
        TextEdit::singleline(&mut palette.query)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .font(body(TEXT_TITLE))
            .text_color(brand::text())
            .hint_text(egui::RichText::new("Type a command").color(brand::faint()))
            .vertical_align(Align::Center)
            .desired_width(field_rect.width()),
    );
    if std::mem::take(&mut palette.focus) {
        field.request_focus();
    }
    if palette.query != before {
        palette.typed();
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::card_line()));
}

fn footer(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, FOOTER_H), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, brand::card_line()));
    let mid = rect.center().y;
    let mut x = rect.left() + SIDE;
    for (keys, what) in HINTS {
        let width = caps_width(ui.painter(), keys);
        paint_caps(ui.painter(), x + width, mid, keys);
        let label = ui.painter().layout_no_wrap(what.to_owned(), body(TEXT_TAG), brand::faint());
        let size = label.size();
        ui.painter().galley(pos2(x + width + 6.0, mid - size.y / 2.0), label, brand::faint());
        x += width + 6.0 + size.x + 16.0;
    }
}

fn nothing_matches(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, ROW_HEIGHT * 2.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "Nothing matches",
        FontId::proportional(13.0),
        brand::muted(),
    );
}

fn icon_of(kind: &str) -> &'static str {
    match kind {
        "tabs" => icons::APP_WINDOW,
        "view" => icons::PALETTE,
        "app" => icons::GEAR,
        "dialogs" => icons::KEYBOARD,
        "tab" => icons::TERMINAL_WINDOW,
        "repo" => icons::FOLDER,
        _ => icons::PLUS,
    }
}

fn list(
    ui: &mut Ui,
    palette: &mut Palette,
    (items, faces): (&[Item], &[Option<Face>]),
    hits: &[Hit],
    moved: &mut bool,
) -> Option<usize> {
    let selected = palette.selected(hits.len());
    let mut clicked = None;
    ScrollArea::vertical().max_height(LIST_MAX).auto_shrink([false, true]).show(ui, |ui| {
        Frame::new().inner_margin(Margin::same(LIST_PAD as i8)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut kind = "";
            for (row, hit) in hits.iter().enumerate() {
                let item = &items[hit.item];
                if item.kind != kind {
                    kind = item.kind;
                    menu_label(ui, group_title(kind), "");
                }
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                if response.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    palette.select(row);
                }
                if response.clicked() {
                    clicked = Some(row);
                }
                if row == selected && std::mem::take(moved) {
                    ui.scroll_to_rect(rect, None);
                }
                paint_row(ui, rect, (item, tab_face(item, faces)), hit, row == selected);
            }
        });
    });
    clicked
}

fn paint_row(ui: &Ui, rect: Rect, (item, face): (&Item, Option<&Face>), hit: &Hit, selected: bool) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, selected, false);
    let mid = rect.center().y;
    let icon_ink = if selected { brand::teal_text() } else { brand::soft() };
    let icon_at = pos2(rect.left() + ROW_PAD + ICON / 2.0, mid);
    if let Some(face) = face {
        let plate = Rect::from_center_size(icon_at, vec2(FACE, FACE));
        agent_avatar::paint_face(ui.ctx(), &painter, plate, face);
    } else {
        painter.text(
            icon_at,
            Align2::CENTER_CENTER,
            icon_of(item.kind),
            FontId::proportional(ICON),
            icon_ink,
        );
    }
    let ink = crate::screen_kit::selected_ink(selected, brand::text());
    let mut job = LayoutJob::default();
    let label_marks: &[usize] = if hit.alias.is_some() { &[] } else { &hit.positions };
    for (i, c) in item.label.chars().enumerate() {
        let marked = label_marks.contains(&i);
        let color = if marked { brand::teal_text() } else { ink };
        job.append(&c.to_string(), 0.0, TextFormat::simple(body(TEXT_BODY), color));
    }
    if let Some(alias) = hit.alias {
        let muted = TextFormat::simple(FontId::proportional(TEXT_TAG), brand::muted());
        job.append("also called ", 8.0, muted);
        for (i, c) in alias.chars().enumerate() {
            let color =
                if hit.positions.contains(&i) { brand::teal_text() } else { brand::muted() };
            job.append(
                &c.to_string(),
                0.0,
                TextFormat::simple(FontId::proportional(TEXT_TAG), color),
            );
        }
    }
    let right = rect.right() - ROW_PAD;
    let caps_left = if item.keys.is_empty() {
        right
    } else {
        let width = caps_width(&painter, &item.keys);
        paint_caps(&painter, right, mid, &item.keys);
        right - width - ROW_PAD
    };
    let left = rect.left() + ROW_PAD + ICON + 10.0;
    job.wrap = TextWrapping::truncate_at_width((caps_left - left).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left, mid - galley.size().y / 2.0), galley, ink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Command, Platform};
    use crate::palette::{Sources, items};
    use crate::test_support::painted_text_rects;
    use egui::{Event, RawInput, pos2};

    struct Harness {
        ctx: egui::Context,
        palette: Palette,
        items: Vec<Item>,
        faces: Vec<Option<Face>>,
        texts: Vec<(String, Rect)>,
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
                tabs: vec![
                    crate::palette::TabRow::new("main", false),
                    crate::palette::TabRow::new("calm-comet", false),
                ],
                active: 0,
                repos: vec!["web".into()],
                repo: 0,
                profiles: vec![("builder".into(), "Builder".into())],
            };
            let mut harness = Self {
                ctx: crate::fonts::test_context(),
                palette: Palette::open(query),
                items: items(&sources, Platform::CURRENT),
                faces: vec![None, Some(Face::of("Zed", "claude", None))],
                texts: Vec::new(),
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
            let (palette, items, faces) = (&mut self.palette, &self.items, &self.faces);
            let mut outcome = None;
            let output = self.ctx.run_ui(raw, |ui| outcome = show(ui.ctx(), palette, items, faces));
            self.texts.clear();
            for clipped in &output.shapes {
                painted_text_rects(&clipped.shape, &mut self.texts);
            }
            outcome
        }

        fn text(&self, wanted: &str) -> Rect {
            self.texts
                .iter()
                .find(|(text, _)| text == wanted)
                .map_or_else(|| panic!("{wanted:?} in {:?}", self.texts), |(_, rect)| *rect)
        }
    }

    #[test]
    fn a_tab_row_wears_its_tab_face_and_every_other_row_keeps_its_icon() {
        let harness = Harness::new("");
        let faces = &harness.faces;
        let worn: Vec<(String, bool)> = harness
            .items
            .iter()
            .filter(|item| item.kind == "tab" || item.target == Target::OpenRepo)
            .map(|item| (item.label.clone(), tab_face(item, faces).is_some()))
            .collect();
        assert_eq!(
            worn,
            [("Switch to tab 2: calm-comet".to_owned(), true), ("Open repo...".to_owned(), false)]
        );
        let item = harness.items.iter().find(|item| item.target == Target::Tab(1)).unwrap();
        assert!(tab_face(item, &faces[..1]).is_none(), "no entry for that tab");
    }

    #[test]
    fn the_face_of_a_tab_is_drawn_in_its_row_instead_of_the_icon() {
        let mut harness = Harness::new("");
        harness.faces = vec![None, Some(Face::of("Zed", "claude", None))];
        harness.frame(Vec::new());
        let letter = harness.text("Z");
        let label = harness.text("Switch to tab 2: calm-comet");
        assert!(letter.right() < label.left(), "{letter:?} left of {label:?}");
        harness.faces = vec![None, None];
        harness.frame(Vec::new());
        assert!(harness.texts.iter().all(|(text, _)| text != "Z"));
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
            Some(Outcome::Run(Target::Profile("builder".into())))
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
        harness.frame(vec![Event::Text("builder".into())]);
        assert_eq!(harness.palette.query, "builder");
        assert_eq!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(Target::Profile("builder".into())))
        );
    }

    #[test]
    fn message_all_agents_shows_in_the_list_and_enter_on_it_runs_it() {
        let harness = Harness::new("");
        harness.text("Message all agents");
        let mut query = Harness::new("message all");
        assert_eq!(query.frame(vec![key(Key::Enter)]), Some(Outcome::Run(Target::MessageAll)));
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

    #[test]
    fn results_sit_under_small_group_labels_in_the_order_their_first_row_shows() {
        let harness = Harness::new("");
        let labels = ["TABS", "VIEW", "APP", "OPEN TABS", "REPOS", "NEW TAB"];
        let tops: Vec<f32> = labels.iter().map(|label| harness.text(label).top()).collect();
        let mut sorted = tops.clone();
        sorted.sort_by(f32::total_cmp);
        assert_eq!(tops, sorted, "{labels:?} {tops:?}");
        let first_tab_row = harness.text("New tab");
        assert!(first_tab_row.top() > harness.text("TABS").bottom());
        assert!(first_tab_row.bottom() < harness.text("VIEW").top());
    }

    #[test]
    fn a_query_that_scores_kinds_apart_still_shows_each_group_once_and_enter_runs_the_first_shown()
    {
        let probe = Harness::new("");
        let interleaved = |query: &str| {
            let kinds: Vec<&str> =
                filter(&probe.items, query).iter().map(|hit| probe.items[hit.item].kind).collect();
            kinds
                .iter()
                .enumerate()
                .any(|(at, kind)| at > 0 && kinds[..at].contains(kind) && kinds[at - 1] != *kind)
        };
        let query = ["tab", "e", "o", "t", "s", "n"]
            .into_iter()
            .find(|query| interleaved(query))
            .expect("a query whose best matches alternate between kinds");
        let mut harness = Harness::new(query);
        let titles: Vec<String> =
            harness.items.iter().map(|item| group_title(item.kind).to_uppercase()).collect();
        for title in titles {
            let shown = harness.texts.iter().filter(|(text, _)| *text == title).count();
            assert!(shown <= 1, "{title} is drawn {shown} times for {query:?}");
        }
        let first = grouped(filter(&harness.items, query), &harness.items)[0].item;
        assert_eq!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Run(harness.items[first].target.clone()))
        );
    }

    #[test]
    fn a_label_shows_only_over_a_group_that_has_a_result() {
        let harness = Harness::new("builder");
        harness.text("NEW TAB");
        assert!(harness.texts.iter().all(|(text, _)| text != "TABS" && text != "OPEN TABS"));
    }

    #[test]
    fn the_header_counts_the_results_and_the_footer_holds_the_key_hints() {
        let harness = Harness::new("move");
        let hits = harness
            .texts
            .iter()
            .find(|(text, _)| text.ends_with(" results") || text.ends_with(" result"))
            .map(|(text, rect)| (text.clone(), *rect))
            .expect("the count");
        let shown = filter(&harness.items, "move").len();
        assert_eq!(hits.0, plural(shown, "result"));
        let field = harness.text("move");
        assert!(hits.1.left() > field.right(), "the count sits at the right of the query");
        let last_row = harness.text("Move sessions to Tidewatch");
        for hint in ["Up", "Down", "pick", "Enter", "run", "Esc", "close"] {
            assert!(harness.text(hint).top() > last_row.bottom(), "{hint} is in the footer");
        }
        assert!(harness.text("pick").left() < harness.text("run").left());
        assert!(harness.text("run").left() < harness.text("close").left());
    }

    #[test]
    fn a_shortcut_is_one_key_cap_per_key_and_never_the_old_mono_text() {
        let harness = Harness::new("");
        let new_tab = harness
            .items
            .iter()
            .find(|item| item.target == Target::Command(Command::NewTab))
            .expect("the new tab command");
        let row = harness.text("New tab");
        let caps: Vec<&str> = crate::screen_kit::chord_keys(&new_tab.keys);
        assert!(caps.len() > 1, "{caps:?}");
        let mut left = f32::MIN;
        for key in &caps {
            let rect = harness
                .texts
                .iter()
                .filter(|(text, rect)| {
                    text == key && (rect.center().y - row.center().y).abs() < 4.0
                })
                .map(|(_, rect)| *rect)
                .next()
                .unwrap_or_else(|| panic!("no cap {key:?} on the row of {row:?}"));
            assert!(rect.left() > row.right(), "{key} sits at the right");
            assert!(rect.left() > left, "caps run left to right");
            left = rect.left();
        }
        assert!(harness.texts.iter().all(|(text, _)| text != &new_tab.keys));
    }

    #[test]
    fn nothing_matching_says_so_and_counts_no_results() {
        let harness = Harness::new("zzzzzz");
        harness.text("Nothing matches");
        harness.text("0 results");
    }

    #[test]
    fn the_list_is_as_tall_as_its_rows() {
        let few = Harness::new("builder");
        let many = Harness::new("");
        let gap = |harness: &Harness, last: &str| {
            harness.text("close").top() - harness.text(last).bottom()
        };
        assert!(gap(&few, "New tab: Builder") < 30.0, "{}", gap(&few, "New tab: Builder"));
        assert!(many.text("close").top() > few.text("close").top());
    }
}
