//! The repo picker: the repo's name in 15 px semibold with a small chevron, no box until it is hovered or its menu is open.
//! Its width fits the widest repo name the window holds, up to `WIDTH`, so nothing in the strip moves when the repo
//! changes; a longer name is cut with an ellipsis.
//! The menu it opens: one line per repo (its colour square, the name, an amber dot when an agent there needs you, its tab
//! count, a teal check at the right edge on the repo on show), then Open repo, Add service and Close under a divider.

use egui::epaint::PathShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, Id, Margin, Painter, Pos2, Rect, RectAlign, Response, Sense,
    Stroke, pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::ui::icons;

use super::TAB_H;
use crate::brand;
use crate::overview::repo_ink;
use crate::reader_view::ellipsized;
use crate::screen_kit::{
    MENU_ROW_H, MENU_ROW_PAD, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, body, heading, menu_divider,
    menu_label, menu_row, paint_caps, selected_ink,
};

pub(super) const WIDTH: f32 = 130.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
const CHEVRON: f32 = 14.0;
const LABEL_SIZE: f32 = 15.0;
const RADIUS: u8 = 8;
const CHEVRON_POINTS: [(f32, f32); 3] = [(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)];
const MENU_W: f32 = 380.0;
const MENU_MARGIN: i8 = 6;
const SWATCH: f32 = 20.0;
const ROW_GAP: f32 = 10.0;
const CHECK_W: f32 = 16.0;
const NEEDS_DOT: f32 = 7.0;
const ACTION_ICON: f32 = 16.0;

/// The chip's width for the widest name's text width: its padding, the gap and the chevron around it,
/// never past `WIDTH`.
fn fit(widest_text: f32) -> f32 {
    (widest_text + 2.0 * PAD + GAP + CHEVRON).min(WIDTH)
}

/// The width every repo's chip takes in this window: the same for all of them, set by the widest.
pub(super) fn width_for<'a>(ui: &egui::Ui, labels: impl Iterator<Item = &'a str>) -> f32 {
    let widest = labels
        .map(|label| {
            ui.painter()
                .layout_no_wrap(
                    label.to_owned(),
                    crate::screen_kit::heading(LABEL_SIZE),
                    brand::TEXT,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max);
    fit(widest)
}

pub(super) fn show(ui: &mut egui::Ui, label: &str, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TAB_H), Sense::click());
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let painter = ui.painter();
    let galley = painter.layout_job(cut(label, text_room(width)));
    let text_left = rect.left() + PAD;
    let chevron = Rect::from_min_size(
        pos2(text_left + galley.size().x + GAP, rect.center().y - CHEVRON / 2.0),
        vec2(CHEVRON, CHEVRON),
    );
    if resp.hovered() || menu_open {
        let tint = Rect::from_min_max(rect.min, pos2(chevron.right() + PAD, rect.max.y));
        painter.rect_filled(tint, CornerRadius::same(RADIUS), colors::hover());
    }
    painter.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    stroke_path(painter, &CHEVRON_POINTS, chevron, 24.0, Stroke::new(1.5, brand::MUTED));
    resp
}

/// One repo of the window as its menu row shows it.
pub(super) struct Entry {
    pub(super) label: String,
    pub(super) dir: String,
    pub(super) tabs: usize,
    pub(super) detail: String,
    pub(super) needs: bool,
}

pub(super) struct Menu<'a> {
    pub(super) repos: &'a [Entry],
    pub(super) here: usize,
    pub(super) several: bool,
    pub(super) open_keys: String,
    pub(super) close_note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    Show(usize),
    Open,
    AddService,
    Close,
}

/// Where the menu's rows were drawn, left in egui's memory for the headless tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Placed {
    pub(super) rows: Vec<Rect>,
    pub(super) checks: Vec<Rect>,
    pub(super) actions: Vec<(&'static str, Rect)>,
}

fn placed_id() -> Id {
    Id::new("repo-menu-placed")
}

/// `TIDEWATCH_DEV_REPO_MENU=1` keeps the menu open, for captures.
pub(super) fn dev_open() -> bool {
    static OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OPEN.get_or_init(|| std::env::var("TIDEWATCH_DEV_REPO_MENU").is_ok_and(|v| v == "1"))
}

fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(12)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
        .inner_margin(Margin::same(MENU_MARGIN))
}

pub(super) fn menu(response: &Response, menu: &Menu, keep_open: bool) -> Option<Pick> {
    if keep_open {
        egui::Popup::open_id(&response.ctx, egui::Popup::default_response_id(response));
    }
    let mut pick = None;
    let mut placed = Placed::default();
    egui::Popup::menu(response).align(RectAlign::BOTTOM_START).gap(4.0).frame(menu_frame()).show(
        |ui| {
            ui.set_width(MENU_W - f32::from(MENU_MARGIN) * 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            if menu.several {
                menu_label(ui, "Repos", "");
                for (index, entry) in menu.repos.iter().enumerate() {
                    let here = index == menu.here;
                    let (rect, row) = menu_row(ui, MENU_ROW_H, here, true);
                    placed.rows.push(rect);
                    if let Some(check) = paint_repo(ui.painter(), rect, entry, here) {
                        placed.checks.push(check);
                    }
                    if row.on_hover_text(&entry.dir).clicked() {
                        pick = Some(Pick::Show(index));
                    }
                }
                menu_divider(ui);
            }
            let actions = [
                ("Open repo", icons::FOLDER, "Open repo...".to_owned(), true, Pick::Open),
                ("Add service", icons::PLUS, "Add service...".to_owned(), true, Pick::AddService),
                (
                    "Close",
                    icons::X,
                    format!("Close {}", menu.repos[menu.here].label),
                    menu.several,
                    Pick::Close,
                ),
            ];
            for (name, icon, label, enabled, target) in actions {
                let (rect, row) = menu_row(ui, MENU_ROW_H, false, enabled);
                placed.actions.push((name, rect));
                let note = match target {
                    Pick::Open => menu.open_keys.as_str(),
                    Pick::Close if enabled => menu.close_note.as_str(),
                    _ => "",
                };
                paint_action(ui.painter(), rect, (icon, &label), (note, target), enabled);
                if row.clicked() {
                    pick = Some(target);
                }
            }
            if pick.is_some() {
                ui.close();
            }
        },
    );
    response.ctx.data_mut(|data| data.insert_temp(placed_id(), placed));
    pick
}

fn paint_repo(painter: &Painter, rect: Rect, entry: &Entry, here: bool) -> Option<Rect> {
    let mid = rect.center().y;
    let swatch = Rect::from_center_size(
        pos2(rect.left() + MENU_ROW_PAD + SWATCH / 2.0, mid),
        vec2(SWATCH, SWATCH),
    );
    painter.rect_filled(swatch, CornerRadius::same(6), repo_ink(&entry.label));
    let letter =
        entry.label.chars().next().map_or_else(String::new, |c| c.to_uppercase().to_string());
    painter.text(swatch.center(), Align2::CENTER_CENTER, letter, heading(TEXT_TAG), brand::GROUND);
    let check = Rect::from_center_size(
        pos2(rect.right() - MENU_ROW_PAD - CHECK_W / 2.0, mid),
        vec2(CHECK_W, CHECK_W),
    );
    let mut right = check.left() - ROW_GAP;
    let detail_ink = match (here, entry.tabs) {
        (true, _) => brand::SOFT,
        (false, 0) => brand::FAINT,
        (false, _) => brand::MUTED,
    };
    let detail = painter.layout_no_wrap(entry.detail.clone(), body(TEXT_SECONDARY), detail_ink);
    painter.galley(
        pos2(right - detail.size().x, mid - detail.size().y / 2.0),
        detail.clone(),
        detail_ink,
    );
    right -= detail.size().x;
    if entry.needs {
        right -= ROW_GAP;
        painter.circle_filled(pos2(right - NEEDS_DOT / 2.0, mid), NEEDS_DOT / 2.0, brand::AMBER);
        right -= NEEDS_DOT;
    }
    let left = swatch.right() + ROW_GAP;
    let font = if here { heading(TEXT_BODY) } else { body(TEXT_BODY) };
    let ink = selected_ink(here, brand::TEXT);
    let name = ellipsized(painter, &entry.label, font, ink, right - ROW_GAP - left);
    painter.galley(pos2(left, mid - name.size().y / 2.0), name, ink);
    here.then(|| {
        let ink = Stroke::new(2.0, brand::TEAL);
        let at = |x: f32, y: f32| check.min + vec2(x, y);
        painter.add(PathShape::line(vec![at(3.5, 8.5), at(6.5, 11.5), at(12.5, 4.5)], ink));
        check
    })
}

fn paint_action(
    painter: &Painter,
    rect: Rect,
    (icon, label): (&str, &str),
    (note, target): (&str, Pick),
    enabled: bool,
) {
    let mid = rect.center().y;
    let fade = if enabled { 1.0 } else { 0.4 };
    let slot = pos2(rect.left() + MENU_ROW_PAD + SWATCH / 2.0, mid);
    painter.text(
        slot,
        Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(ACTION_ICON),
        brand::SOFT.gamma_multiply(fade),
    );
    let mut right = rect.right() - MENU_ROW_PAD;
    if target == Pick::Open && !note.is_empty() {
        right = paint_caps(painter, right, mid, note) - ROW_GAP;
    } else if !note.is_empty() {
        let galley = painter.layout_no_wrap(note.to_owned(), body(TEXT_TAG), brand::FAINT);
        painter.galley(
            pos2(right - galley.size().x, mid - galley.size().y / 2.0),
            galley.clone(),
            brand::FAINT,
        );
        right -= galley.size().x + ROW_GAP;
    }
    let left = slot.x + SWATCH / 2.0 + ROW_GAP;
    let text =
        ellipsized(painter, label, body(TEXT_BODY), brand::TEXT.gamma_multiply(fade), right - left);
    painter.galley(pos2(left, mid - text.size().y / 2.0), text, brand::TEXT);
}

fn text_room(width: f32) -> f32 {
    width - 2.0 * PAD - GAP - CHEVRON
}

fn cut(label: &str, room: f32) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
        crate::screen_kit::heading(LABEL_SIZE),
        brand::TEXT,
    );
    job.wrap = TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    job
}

fn stroke_path(
    painter: &Painter,
    points: &[(f32, f32)],
    into: Rect,
    view_box: f32,
    stroke: Stroke,
) {
    let scale = into.width() / view_box;
    let mapped: Vec<Pos2> =
        points.iter().map(|(x, y)| into.min + vec2(x * scale, y * scale)).collect();
    painter.add(PathShape::line(mapped, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    const LONG: &str = "a-repository-with-a-very-long-name-indeed";

    fn widths(labels: &[&str], shown: &str) -> (f32, f32) {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 100.0))),
            ..Default::default()
        };
        let (mut fit, mut width) = (0.0, 0.0);
        let _ = ctx.run_ui(raw, |ui| {
            fit = width_for(ui, labels.iter().copied());
            ui.horizontal(|ui| width = show(ui, shown, fit).rect.width());
        });
        (fit, width)
    }

    #[test]
    fn the_chip_is_as_wide_as_its_widest_name_and_never_past_the_cap() {
        assert_eq!(LONG.len(), 41);
        let (short, _) = widths(&["web"], "web");
        let (mid, _) = widths(&["web", "tidewatch"], "web");
        let (long, _) = widths(&["web", LONG], "web");
        assert!(short < mid && mid < WIDTH, "{short} {mid}");
        assert_eq!(long, WIDTH);
    }

    #[test]
    fn every_repo_of_the_window_gets_the_same_width_so_switching_moves_nothing() {
        let labels = ["web", "tidewatch", "api"];
        let sizes: Vec<f32> = labels.iter().map(|shown| widths(&labels, shown).1).collect();
        assert_eq!(sizes[0], sizes[1]);
        assert_eq!(sizes[1], sizes[2]);
        assert!(sizes[0] < WIDTH);
    }

    #[test]
    fn the_fit_adds_padding_gap_and_chevron_and_caps_at_the_width() {
        let around = 2.0 * PAD + GAP + CHEVRON;
        let cases = [(0.0, around), (40.0, 40.0 + around), (500.0, WIDTH)];
        for (text, want) in cases {
            assert_eq!(fit(text), want, "{text}");
        }
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis_inside_the_room() {
        let ctx = crate::fonts::test_context();
        let room = text_room(WIDTH);
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            let long =
                ui.painter().layout_job(cut("a-repository-with-a-very-long-name-indeed", room));
            assert!(long.size().x <= room, "{} > {room}", long.size().x);
            assert!(long.elided);
            let short = ui.painter().layout_job(cut("web", room));
            assert!(!short.elided);
        });
    }

    fn entries() -> Vec<Entry> {
        let entry = |label: &str, tabs: usize, needs: bool| Entry {
            label: label.to_owned(),
            dir: format!("/work/{label}"),
            tabs,
            detail: crate::review::plural(tabs, "tab"),
            needs,
        };
        vec![
            entry("web", 0, false),
            entry("tidewatch", 1, false),
            entry("workers", 7, true),
            entry("soundtrack", 1, false),
        ]
    }

    struct Screen {
        ctx: egui::Context,
        repos: Vec<Entry>,
        several: bool,
        pick: Option<Pick>,
        texts: Vec<String>,
    }

    impl Screen {
        fn new(several: bool) -> Self {
            let repos = if several { entries() } else { entries().into_iter().take(1).collect() };
            let mut screen = Self {
                ctx: crate::fonts::test_context(),
                repos,
                several,
                pick: None,
                texts: Vec::new(),
            };
            for _ in 0..4 {
                screen.frame(vec![]);
            }
            screen
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                events,
                focused: true,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let here = if self.several { 2 } else { 0 };
            let output = ctx.run_ui(raw, |ui| {
                ui.horizontal(|ui| {
                    let resp = show(ui, "workers", 130.0);
                    let menu = Menu {
                        repos: &self.repos,
                        here,
                        several: self.several,
                        open_keys: "Cmd+O".to_owned(),
                        close_note: "closes 7 tabs".to_owned(),
                    };
                    if let Some(pick) = super::menu(&resp, &menu, true) {
                        self.pick = Some(pick);
                    }
                });
            });
            self.texts.clear();
            for clipped in &output.shapes {
                collect_texts(&clipped.shape, &mut self.texts);
            }
        }

        fn placed(&self) -> Placed {
            self.ctx.data(|data| data.get_temp::<Placed>(placed_id())).unwrap_or_default()
        }

        fn click(&mut self, at: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(vec![]);
        }

        fn action(&self, name: &str) -> Rect {
            self.placed().actions.iter().find(|(label, _)| *label == name).expect(name).1
        }
    }

    fn collect_texts(shape: &egui::Shape, found: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => found.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_texts(shape, found)),
            _ => {}
        }
    }

    #[test]
    fn repo_rows_are_one_line_each_and_never_overlap() {
        let screen = Screen::new(true);
        let placed = screen.placed();
        assert_eq!(placed.rows.len(), 4);
        for rect in &placed.rows {
            assert_eq!(rect.height(), MENU_ROW_H);
        }
        for pair in placed.rows.windows(2) {
            assert_eq!(pair[0].bottom(), pair[1].top(), "{pair:?}");
        }
    }

    #[test]
    fn the_check_sits_at_the_right_edge_of_the_repo_on_show_and_nowhere_else() {
        let screen = Screen::new(true);
        let placed = screen.placed();
        assert_eq!(placed.checks.len(), 1);
        let (check, row) = (placed.checks[0], placed.rows[2]);
        assert!(row.contains_rect(check), "{check:?} {row:?}");
        assert_eq!(check.right(), row.right() - MENU_ROW_PAD);
        assert!(!placed.rows[0].intersects(check) && !placed.rows[3].intersects(check));
    }

    #[test]
    fn open_add_service_and_close_sit_in_order_under_the_repos_and_close_names_the_repo() {
        let screen = Screen::new(true);
        let placed = screen.placed();
        let names: Vec<&str> = placed.actions.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["Open repo", "Add service", "Close"]);
        let last_repo = placed.rows.last().unwrap();
        let open = screen.action("Open repo");
        assert!(open.top() > last_repo.bottom(), "a divider separates them");
        assert_eq!(screen.action("Add service").top(), open.bottom());
        assert_eq!(screen.action("Close").top(), screen.action("Add service").bottom());
        for text in ["Close workers", "closes 7 tabs", "Open repo...", "Add service...", "REPOS"] {
            assert!(screen.texts.iter().any(|shown| shown == text), "{text} in {:?}", screen.texts);
        }
        assert!(screen.texts.iter().any(|shown| shown == "O"), "the key cap: {:?}", screen.texts);
    }

    #[test]
    fn a_click_on_a_row_or_an_action_picks_it() {
        for (index, want) in [(0, Pick::Show(0)), (3, Pick::Show(3))] {
            let mut screen = Screen::new(true);
            let at = screen.placed().rows[index].center();
            screen.click(at);
            assert_eq!(screen.pick, Some(want));
        }
        for (name, want) in
            [("Open repo", Pick::Open), ("Add service", Pick::AddService), ("Close", Pick::Close)]
        {
            let mut screen = Screen::new(true);
            let at = screen.action(name).center();
            screen.click(at);
            assert_eq!(screen.pick, Some(want), "{name}");
        }
    }

    #[test]
    fn with_one_repo_there_is_no_list_and_close_takes_no_click() {
        let mut screen = Screen::new(false);
        let placed = screen.placed();
        assert_eq!((placed.rows.len(), placed.checks.len()), (0, 0));
        let close = screen.action("Close").center();
        screen.click(close);
        assert_eq!(screen.pick, None);
        let open = screen.action("Open repo").center();
        screen.click(open);
        assert_eq!(screen.pick, Some(Pick::Open));
    }

    #[test]
    fn an_agent_that_needs_you_puts_an_amber_dot_on_its_repo_row_only() {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            ..Default::default()
        };
        let mut amber = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(380.0, MENU_ROW_H));
            for entry in &entries() {
                let _ = paint_repo(ui.painter(), rect, entry, false);
            }
        });
        for clipped in &output.shapes {
            collect_amber(&clipped.shape, &mut amber);
        }
        assert_eq!(amber.len(), 1, "only the workers entry needs you: {amber:?}");
    }

    fn collect_amber(shape: &egui::Shape, found: &mut Vec<Pos2>) {
        match shape {
            egui::Shape::Circle(circle) if circle.fill == brand::AMBER => found.push(circle.center),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_amber(shape, found)),
            _ => {}
        }
    }
}
