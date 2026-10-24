use egui::{Event, PointerButton, RawInput, Rect, Shape, pos2, vec2};

use std::sync::Arc;

use super::*;

const SHELLS: u32 = 3;

fn tabs() -> Tabs<Pane<u32>> {
    let mut tabs = Tabs::new(Pane::shell(0));
    for n in 1..=SHELLS {
        tabs.apply(Action::New, || Pane::shell(n));
    }
    tabs
}

fn order_of(tab: &crate::tabs::Tab<Pane<u32>>) -> u32 {
    match tab.pane.body {
        Body::Shell(n) => n,
        _ => unreachable!("only shells here"),
    }
}

fn order(tabs: &Tabs<Pane<u32>>) -> Vec<u32> {
    tabs.items().iter().map(order_of).collect()
}

struct Strip {
    ctx: egui::Context,
    tabs: Tabs<Pane<u32>>,
    time: f64,
    rects: Vec<Rect>,
    seen: Vec<Action>,
    texts: Vec<(String, Rect)>,
}

impl Strip {
    fn new() -> Self {
        let mut strip = Self {
            ctx: crate::fonts::test_context(),
            tabs: tabs(),
            time: 0.0,
            rects: Vec::new(),
            seen: Vec::new(),
            texts: Vec::new(),
        };
        strip.frame(Vec::new());
        strip
    }

    fn rows(&self) -> Vec<pill::Row> {
        self.tabs
            .parked_in(0)
            .map(|(index, tab)| pill::Row {
                index,
                name: tab.title.clone(),
                face: Face::of("Builder", "claude", None),
                line: crate::park::Line { text: "Idle 6h".into(), tone: crate::park::Tone::Muted },
            })
            .collect()
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            events,
            time: Some(self.time),
            focused: true,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut rects = Vec::new();
        let rows = self.rows();
        let output = self.ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    rects.extend(main_icon::draw(
                        ui,
                        &self.tabs,
                        |_| (false, "main tip".to_owned()),
                        &mut actions,
                    ));
                    rects.extend(tab_row(
                        ui,
                        &self.tabs,
                        120.0,
                        0.0,
                        &mut actions,
                        (
                            |_: &Pane<u32>| (Progress::default(), CmdState::default()),
                            |_| TabExtras::default(),
                        ),
                    ));
                    if pill::wanted(ui.ctx(), rows.len()) {
                        let out = pill::show(ui, &rows, crate::remote_pill::Popover::OnClick);
                        if let Some(row) = out.picked {
                            actions.push(Action::Select(rows[row].index));
                        }
                    }
                });
            });
        });
        self.texts.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut self.texts);
        }
        self.rects = rects;
        for action in actions {
            self.tabs.apply(action, || unreachable!());
            self.seen.push(action);
        }
    }

    fn park(&mut self, pane: u32) {
        let id = self.tabs.items().iter().find(|tab| order_of(tab) == pane).unwrap().id;
        self.tabs.set_park(id, crate::park::Park { why: crate::park::Why::Idle, since_ms: 0 });
    }

    fn placed(&self) -> pill::Placed {
        self.ctx.data(|data| data.get_temp(pill::placed_id())).unwrap_or_default()
    }

    fn pill_center(&self) -> egui::Pos2 {
        self.placed().pill.expect("the pill is drawn").center()
    }

    fn click(&mut self, at: egui::Pos2) {
        self.pointer(at);
        self.button(at, true);
        self.button(at, false);
        self.frame(Vec::new());
    }

    fn parked(&self) -> Vec<u32> {
        self.tabs.parked_in(0).map(|(_, tab)| order_of(tab)).collect()
    }

    fn center(&self, position: usize) -> egui::Pos2 {
        self.rects[position].center()
    }

    fn pointer(&mut self, to: egui::Pos2) {
        self.frame(vec![Event::PointerMoved(to)]);
    }

    fn button(&mut self, at: egui::Pos2, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }]);
    }

    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        self.pointer(from);
        self.button(from, true);
        for step in 1..=8 {
            self.pointer(from + (to - from) * (step as f32 / 8.0));
        }
        self.button(to, false);
        self.frame(Vec::new());
    }

    fn moves(&self) -> Vec<Action> {
        self.seen.iter().copied().filter(|a| matches!(a, Action::Move(..))).collect()
    }
}

#[test]
fn dragging_a_tab_past_its_neighbours_middle_moves_it_once_and_it_keeps_the_selection() {
    let mut strip = Strip::new();
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);
    let (from, past) = (strip.center(2), strip.center(3) + vec2(10.0, 0.0));
    strip.drag(from, past);
    assert_eq!(strip.moves(), [Action::Move(2, Step::Right)]);
    assert_eq!(order(&strip.tabs), [0, 1, 3, 2]);
    assert_eq!(strip.tabs.active(), 2, "the index follows the tab it swapped with");
    assert_eq!(
        order(&strip.tabs)[strip.tabs.active()],
        3,
        "the selected tab is still the one that was"
    );
}

#[test]
fn a_long_drag_walks_the_tab_across_every_tab_it_passes() {
    let mut strip = Strip::new();
    let (from, far) = (strip.center(3), strip.center(1) - vec2(10.0, 0.0));
    strip.drag(from, far);
    assert_eq!(order(&strip.tabs), [0, 3, 1, 2]);
    assert_eq!(strip.moves().len(), 2);
}

#[test]
fn a_plain_or_sloppy_click_selects_and_never_reorders() {
    let mut strip = Strip::new();
    let at = strip.center(1);
    strip.pointer(at);
    strip.button(at, true);
    strip.button(at, false);
    strip.frame(Vec::new());
    assert_eq!(strip.seen, [Action::Select(1)]);
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);

    let mut sloppy = Strip::new();
    let at = sloppy.center(2);
    sloppy.pointer(at);
    sloppy.button(at, true);
    sloppy.pointer(at + vec2(3.0, 1.0));
    sloppy.pointer(at + vec2(-3.0, 0.0));
    sloppy.button(at, false);
    sloppy.frame(Vec::new());
    assert_eq!(sloppy.moves(), Vec::<Action>::new());
    assert_eq!(order(&sloppy.tabs), [0, 1, 2, 3]);
}

#[test]
fn nothing_moves_the_pinned_tab_or_goes_past_it() {
    let mut strip = Strip::new();
    let (from, over_main) = (strip.center(1), strip.center(0) - vec2(10.0, 0.0));
    strip.drag(from, over_main);
    assert_eq!(strip.moves(), Vec::<Action>::new());
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);

    let mut strip = Strip::new();
    let (main, over) = (strip.center(0), strip.center(2));
    strip.drag(main, over);
    assert_eq!(strip.moves(), Vec::<Action>::new());
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);
}

#[test]
fn only_the_tabs_of_the_group_on_show_are_drawn_and_moved() {
    let mut strip = Strip::new();
    strip.tabs.add_group(Pane::shell(10));
    strip.tabs.push_in(1, "api".into(), Pane::shell(11));
    strip.frame(Vec::new());
    assert_eq!(strip.rects.len(), 4);
    let (from, past) = (strip.center(1), strip.center(2) + vec2(10.0, 0.0));
    strip.drag(from, past);
    assert_eq!(order(&strip.tabs), [0, 2, 1, 3, 10, 11]);
}

fn filled_by(
    look: impl FnOnce(&egui::Painter, ShapeIdx, Rect),
    tab: Rect,
) -> Vec<Option<egui::Color32>> {
    let ctx = egui::Context::default();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
        ..RawInput::default()
    };
    let mut fills = Vec::new();
    let mut look = Some(look);
    let _ = ctx.run_ui(raw, |ui| {
        let painter = ui.painter().clone();
        let under = painter.add(Shape::Noop);
        painter.rect_filled(tab, 6.0, egui::Color32::GRAY);
        if let Some(look) = look.take() {
            look(&painter, under, tab);
        }
        fills = ctx.graphics(|graphics| {
            graphics
                .get(painter.layer_id())
                .map(|list| {
                    list.all_entries()
                        .map(|clipped| match &clipped.shape {
                            Shape::Rect(rect) => Some(rect.fill),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
    });
    fills
}

#[test]
fn the_active_tab_swaps_the_fill_draw_tab_painted_for_the_nav_tint_and_adds_a_bar() {
    let tab = Rect::from_min_size(pos2(10.0, 0.0), vec2(120.0, 34.0));
    let fills = filled_by(|painter, under, tab| nav_look(painter, under, tab, true, false), tab);
    assert_eq!(fills, [None, Some(brand::TEAL_NAV), Some(brand::TEAL)]);
}

#[test]
fn a_hovered_inactive_tab_gets_a_fainter_tint_under_its_text_and_a_plain_one_nothing() {
    let tab = Rect::from_min_size(pos2(10.0, 0.0), vec2(120.0, 34.0));
    let hovered = filled_by(|painter, under, tab| nav_look(painter, under, tab, false, true), tab);
    assert_eq!(hovered[0], Some(brand::TEAL_NAV.gamma_multiply(0.6)));
    let plain = filled_by(|painter, under, tab| nav_look(painter, under, tab, false, false), tab);
    assert_eq!(plain[0], None);
}

#[test]
fn the_room_for_a_title_leaves_the_ring_and_the_trailing_slot_free() {
    let cases = [
        (200.0, 7.2, true, 18),
        (200.0, 7.2, false, 21),
        (140.0, 7.2, true, 10),
        (140.0, 7.2, false, 13),
        (60.0, 7.2, true, 1),
    ];
    for (width, char_w, ring, want) in cases {
        assert_eq!(title_budget(width, char_w, ring), want, "{width} {ring}");
    }
}

fn collect_text(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) => out
            .push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
        _ => {}
    }
}

fn agent_strip(
    title: &str,
    width: f32,
    (with_ring, with_job, with_avatar): (bool, bool, bool),
) -> (Vec<Rect>, Vec<(String, Rect)>) {
    use crate::agent_run::AgentRun;
    use crate::pane::AgentTab;
    let ctx = crate::fonts::test_context();
    let mut pane = Pane::shell(1_u32);
    pane.agent = Some(AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0)));
    let mut tabs = Tabs::new(Pane::shell(0));
    tabs.push_in(0, title.into(), pane);
    let ledger = tidewatch_core::progress::Ledger::parse(
        "# Goal\nx\n\n## Status\nstate: blocked\n\n## Checklist\n### 1. A\n- [x] a\n- [!] b\n",
    );
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    for _ in 0..2 {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    rects = tab_row(
                        ui,
                        &tabs,
                        width,
                        0.0,
                        &mut actions,
                        (
                            |_: &Pane<u32>| (Progress::default(), CmdState::default()),
                            |_| TabExtras {
                                ring: with_ring.then(|| tab_ring(&ledger)),
                                avatar: with_avatar.then(|| {
                                    let (svg, reason) = crate::avatar_glue::FAKE[0];
                                    Arc::new(tidewatch_core::avatar::clean(svg, reason).unwrap())
                                }),
                                job: with_job.then(|| TabJob {
                                    fraction: 0.21,
                                    label: "21%".into(),
                                    stalled: false,
                                    tip: "orders".into(),
                                }),
                                can_draw: false,
                                mood: Mood::default(),
                            },
                        ),
                    );
                });
            });
        });
        texts.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
    }
    (rects, texts)
}

const QUARTZ: &str = "quartz-worktree-name-that-is-far-too-long";

fn title_of(texts: &[(String, Rect)]) -> &(String, Rect) {
    texts.iter().find(|(text, _)| text.contains("qua")).expect("the title is drawn")
}

#[test]
fn an_agent_tab_shows_its_face_in_place_of_the_number_and_the_cli_logo() {
    for width in [140.0, 200.0, 260.0] {
        let (rects, texts) = agent_strip(QUARTZ, width, (false, false, true));
        let title = title_of(&texts);
        assert!(title.0.starts_with("    qua"), "four spaces of room for the face: {:?}", title.0);
        assert!(texts.iter().all(|(text, _)| !text.starts_with("2 ")), "no number in {texts:?}");
        let face = face_rect(rects[0]);
        assert_eq!((face.width(), face.height()), (AVATAR_SIZE, AVATAR_SIZE));
        assert_eq!(face.left(), rects[0].left() + TAB_PAD_X);
        let char_w = title.1.width() / title.0.chars().count() as f32;
        assert!(face.right() <= title.1.left() + AVATAR_PAD_CHARS as f32 * char_w + 0.5);
    }
}

#[test]
fn a_shell_tab_keeps_its_number() {
    let ctx = crate::fonts::test_context();
    let mut texts = Vec::new();
    for _ in 0..2 {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        let tabs = tabs();
        let output = ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    tab_row(
                        ui,
                        &tabs,
                        160.0,
                        0.0,
                        &mut actions,
                        (
                            |_: &Pane<u32>| (Progress::default(), CmdState::default()),
                            |_| TabExtras::default(),
                        ),
                    );
                });
            });
        });
        texts.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
    }
    assert!(texts.iter().any(|(text, _)| text.starts_with("2  shell 1")), "{texts:?}");
}

#[test]
fn a_tab_with_a_ledger_cuts_its_title_before_the_ring_at_every_width_from_the_floor_up() {
    for width in [crate::layout::MIN_TAB_W, 180.0, 260.0] {
        let (rects, texts) = agent_strip(QUARTZ, width, (true, false, true));
        let title = title_of(&texts);
        assert!(
            title.1.right() <= ring_rect(rects[0]).left(),
            "{:?} reaches the ring at {width}",
            title.1
        );
        assert!(title.0.ends_with('\u{2026}'), "{}", title.0);
    }
}

#[test]
fn a_tab_without_a_ledger_keeps_the_whole_title_room() {
    let (rects, with) = agent_strip(QUARTZ, 200.0, (true, false, true));
    let (_, without) = agent_strip(QUARTZ, 200.0, (false, false, true));
    assert!(title_of(&without).1.width() > title_of(&with).1.width());
    assert_eq!(rects[0].width(), 200.0);
}

#[test]
fn the_title_gets_what_the_face_and_the_slot_leave_at_the_least_tab_width() {
    let (rects, texts) =
        agent_strip("backfill-orders", crate::layout::MIN_TAB_W, (false, false, true));
    let title = texts.iter().find(|(text, _)| text.contains("backf")).unwrap();
    assert!(title.0.ends_with('\u{2026}'), "cut with an ellipsis: {:?}", title.0);
    assert!(title.1.right() <= rects[0].right() - TAB_PAD_X - TAB_SLOT_W);
}

#[test]
fn the_job_percent_is_not_in_the_title_row_but_the_bar_along_the_floor_stays() {
    let (_, texts) = agent_strip("orders", 200.0, (true, true, true));
    assert!(texts.iter().all(|(text, _)| !text.contains('%')), "{texts:?}");
    let tab = Rect::from_min_size(pos2(10.0, 0.0), vec2(200.0, TAB_H));
    let half = job_bar_rect(tab, 0.5);
    assert_eq!(half.left(), 16.0);
    assert_eq!(half.width(), 94.0);
    assert_eq!(half.height(), 2.0);
    assert_eq!(half.bottom(), TAB_H - 3.0);
    assert_eq!(job_bar_rect(tab, 2.0).width(), 188.0);
    assert_eq!(job_bar_rect(tab, -1.0).width(), 0.0);
}

#[test]
fn a_stalled_job_paints_its_bar_orange_and_a_running_one_teal() {
    for (stalled, want) in [(false, brand::TEAL), (true, brand::ORANGE)] {
        let ctx = egui::Context::default();
        let tab = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, TAB_H));
        let job = TabJob { fraction: 0.5, label: "50%".into(), stalled, tip: String::new() };
        let output = ctx.run_ui(RawInput::default(), |ui| paint_job(ui, tab, &job));
        let mut fills = Vec::new();
        for clipped in &output.shapes {
            if let Shape::Rect(rect) = &clipped.shape {
                fills.push(rect.fill);
            }
        }
        assert_eq!(fills, [want]);
    }
}

#[test]
fn the_tooltip_names_the_number_the_chord_and_the_job() {
    let job =
        TabJob { fraction: 0.2, label: "21%".into(), stalled: false, tip: "orders: 21%".into() };
    let cases = [
        ("backfill", 3, None, "backfill\nTab 3 (Cmd+3)"),
        ("backfill", 3, Some(&job), "backfill\nTab 3 (Cmd+3)\norders: 21%"),
        ("backfill", 10, None, "backfill\nTab 10"),
    ];
    for (title, number, job, want) in cases {
        let got = tab_tip(title, title, number, job);
        if cfg!(target_os = "macos") {
            assert_eq!(got, want);
        } else {
            assert!(got.starts_with(&format!("{title}\nTab {number}")), "{got}");
        }
    }
}

fn parked_strip() -> Strip {
    let mut strip = Strip::new();
    strip.tabs.apply(Action::Select(1), || unreachable!());
    strip.park(2);
    strip.park(3);
    strip.frame(Vec::new());
    strip.frame(Vec::new());
    strip
}

#[test]
fn the_strip_draws_the_tabs_that_are_not_parked_and_a_pill_to_their_right_that_counts_the_rest() {
    let strip = parked_strip();
    assert_eq!(strip.rects.len(), 2);
    let placed = strip.placed();
    let pill = placed.pill.expect("a pill");
    assert!(pill.left() > strip.rects[1].right());
    assert!(strip.texts.iter().any(|(text, _)| text == "Parked 2"), "{:?}", strip.texts);
    assert_eq!(placed.rows, Vec::<Rect>::new(), "the list is closed until it is clicked");
}

#[test]
fn nothing_parked_and_nothing_dragged_draws_no_pill() {
    let strip = Strip::new();
    assert_eq!(strip.placed().pill, None);
    assert!(strip.texts.iter().all(|(text, _)| !text.starts_with("Parked")));
}

#[test]
fn a_click_on_the_pill_opens_the_list_and_a_click_on_a_row_brings_that_tab_back_selected() {
    let mut strip = parked_strip();
    let at = strip.pill_center();
    strip.click(at);
    strip.frame(Vec::new());
    let rows = strip.placed().rows;
    assert_eq!(rows.len(), 2, "one row per parked tab, in the order of the tabs");
    assert!(rows[0].top() > strip.placed().pill.unwrap().bottom());
    assert!(strip.texts.iter().any(|(text, _)| text == "Idle 6h"));
    let second = rows[1].center();
    strip.seen.clear();
    strip.click(second);
    assert_eq!(strip.seen, [Action::Select(3)]);
    assert_eq!(strip.parked(), [2], "the other one is still parked");
    assert_eq!(order_of(&strip.tabs.items()[strip.tabs.active()]), 3);
    strip.frame(Vec::new());
    assert_eq!(strip.rects.len(), 3, "the tab is back in the strip");
}

#[test]
fn a_tab_dragged_onto_the_pill_is_parked_and_the_tabs_it_passed_stay_in_order() {
    let mut strip = parked_strip();
    strip.tabs.apply(Action::Select(1), || unreachable!());
    let (from, onto) = (strip.center(1), strip.pill_center());
    strip.seen.clear();
    strip.drag(from, onto);
    assert_eq!(strip.seen.iter().filter(|a| matches!(a, Action::Park(_))).count(), 1);
    assert_eq!(strip.moves(), Vec::<Action>::new(), "nothing swapped on the way");
    assert_eq!(strip.parked(), [1, 2, 3]);
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);
    assert_eq!(strip.tabs.active(), 0, "the selection went to the main tab, the only one left");
}

#[test]
fn with_nothing_parked_a_drag_shows_a_place_to_drop_and_dropping_there_parks_the_tab() {
    let mut strip = Strip::new();
    strip.tabs.apply(Action::Select(1), || unreachable!());
    strip.frame(Vec::new());
    let from = strip.center(3);
    strip.pointer(from);
    strip.button(from, true);
    for step in 1..=4 {
        strip.pointer(from + vec2(0.0, 1.0) + vec2(step as f32 * 3.0, 0.0));
    }
    strip.frame(Vec::new());
    assert!(strip.texts.iter().any(|(text, _)| text == "Drop to park"), "{:?}", strip.texts);
    let onto = strip.pill_center();
    strip.pointer(onto);
    strip.frame(Vec::new());
    strip.button(onto, false);
    strip.frame(Vec::new());
    strip.frame(Vec::new());
    assert_eq!(strip.parked(), [3]);
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);
    assert!(strip.texts.iter().any(|(text, _)| text == "Parked 1"), "{:?}", strip.texts);
}

#[test]
fn a_tab_let_go_beside_the_pill_is_not_parked() {
    let mut strip = parked_strip();
    let (from, beside) = (strip.center(1), strip.pill_center() + vec2(0.0, 60.0));
    strip.drag(from, beside);
    assert_eq!(strip.parked(), [2, 3]);
}

#[test]
fn the_pinned_tab_cannot_be_dropped_on_the_pill() {
    let mut strip = parked_strip();
    let (main, onto) = (strip.center(0), strip.pill_center());
    strip.drag(main, onto);
    assert_eq!(strip.seen.iter().filter(|action| matches!(action, Action::Park(_))).count(), 0);
    assert_eq!(strip.parked(), [2, 3]);
}

#[test]
fn a_right_click_on_a_tab_offers_park_and_never_park_and_the_pinned_tab_offers_neither() {
    let mut strip = Strip::new();
    let at = strip.center(1);
    strip.pointer(at);
    strip.frame(vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    }]);
    strip.frame(vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    }]);
    strip.frame(Vec::new());
    let find = |strip: &Strip, label: &str| {
        strip.texts.iter().find(|(text, _)| text == label).map(|(_, rect)| rect.center())
    };
    assert!(find(&strip, "Park").is_some(), "{:?}", strip.texts);
    let keep = find(&strip, "Never park this tab").expect("never park");
    strip.seen.clear();
    strip.click(keep);
    assert_eq!(strip.seen, [Action::ToggleKeep(1)]);
    assert!(strip.tabs.items()[1].keep);

    let mut main = Strip::new();
    let at = main.center(0);
    main.pointer(at);
    for pressed in [true, false] {
        main.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }]);
    }
    main.frame(Vec::new());
    assert!(main.texts.iter().all(|(text, _)| text != "Park"), "{:?}", main.texts);
}

#[test]
fn the_pill_list_names_each_tab_and_says_why_it_left() {
    let mut strip = parked_strip();
    let at = strip.pill_center();
    strip.click(at);
    strip.frame(Vec::new());
    let names: Vec<_> = strip.texts.iter().map(|(text, _)| text.as_str()).collect();
    assert!(names.contains(&"shell 2") && names.contains(&"shell 3"), "{names:?}");
}

fn fills(shape: &Shape, out: &mut Vec<egui::Color32>) {
    match shape {
        Shape::Rect(rect) => out.push(rect.fill),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| fills(shape, out)),
        _ => {}
    }
}

fn strip_fills(mood: Mood, locked: bool) -> (Vec<egui::Color32>, bool) {
    use crate::agent_run::AgentRun;
    use crate::pane::AgentTab;
    let ctx = crate::fonts::test_context();
    let mut pane = Pane::shell(1_u32);
    pane.agent = Some(AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0)));
    let mut tabs = Tabs::new(Pane::shell(0));
    let id = tabs.push_in(0, "rename".into(), pane);
    crate::agent_avatar::lock(&ctx, if locked { vec![id] } else { Vec::new() });
    let mut all = Vec::new();
    for _ in 0..2 {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    tab_row(
                        ui,
                        &tabs,
                        200.0,
                        0.0,
                        &mut actions,
                        (
                            |_: &Pane<u32>| (Progress::default(), CmdState::default()),
                            |_| TabExtras {
                                avatar: Some(crate::avatar_glue::meeseeks_avatar()),
                                mood,
                                ..TabExtras::default()
                            },
                        ),
                    );
                });
            });
        });
        all.clear();
        for clipped in &output.shapes {
            fills(&clipped.shape, &mut all);
        }
    }
    (all, locked)
}

#[test]
fn a_struggling_tab_is_tinted_orange_and_a_calm_one_is_not() {
    let orange = brand::ORANGE.gamma_multiply(0.16);
    let (calm, _) = strip_fills(Mood::default(), true);
    assert!(!calm.contains(&orange));
    let (struggling, _) = strip_fills(Mood { struggling: true, fade: None }, true);
    assert!(struggling.contains(&orange), "the tint over the tab");
    assert!(struggling.contains(&brand::ORANGE), "the bar at its left");
}

#[test]
fn a_fading_tab_is_veiled_in_the_strips_own_color_more_as_the_fade_goes_on() {
    let veil = |fade: f32| brand::SIDEBAR.gamma_multiply(fade);
    let (plain, _) = strip_fills(Mood::default(), true);
    assert!(!plain.contains(&veil(0.5)));
    for fade in [0.25, 0.5, 1.0] {
        let (shown, _) = strip_fills(Mood { struggling: false, fade: Some(fade) }, true);
        assert!(shown.contains(&veil(fade)), "{fade}");
    }
}

fn main_icon_shapes(
    select_main: bool,
    running: bool,
    hover: bool,
) -> (Vec<egui::Color32>, Vec<egui::Color32>, Rect) {
    let ctx = crate::fonts::test_context();
    let mut tabs = Tabs::new(Pane::shell(0_u32));
    tabs.push_in(0, "agent".into(), Pane::shell(1));
    if !select_main {
        tabs.apply(Action::Select(1), || unreachable!());
    }
    let (mut rects, mut circles, mut rect) = (Vec::new(), Vec::new(), Rect::NOTHING);
    for _ in 0..2 {
        let events = if hover { vec![Event::PointerMoved(pos2(20.0, 20.0))] } else { Vec::new() };
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| {
                rect =
                    main_icon::draw(ui, &tabs, |_| (running, String::new()), &mut actions).unwrap();
            });
        });
        (rects, circles) = (Vec::new(), Vec::new());
        for clipped in &output.shapes {
            fills(&clipped.shape, &mut rects);
            circle_fills(&clipped.shape, &mut circles);
        }
    }
    (rects, circles, rect)
}

fn circle_fills(shape: &Shape, out: &mut Vec<egui::Color32>) {
    match shape {
        Shape::Circle(circle) => out.push(circle.fill),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| circle_fills(shape, out)),
        _ => {}
    }
}

#[test]
fn the_main_icon_is_a_square_far_narrower_than_a_tab_and_sits_before_the_agent_tabs() {
    let strip = Strip::new();
    assert_eq!(strip.rects[0].width(), main_icon::WIDTH);
    assert_eq!(strip.rects[0].height(), TAB_H);
    assert!(strip.rects[0].width() * 3.0 < strip.rects[1].width());
    assert!(strip.rects[0].right() <= strip.rects[1].left());
    assert_eq!(strip.rects.len(), 4, "the icon and the three shells");
}

#[test]
fn a_click_on_the_main_icon_selects_main_and_never_reorders_or_closes() {
    let mut strip = Strip::new();
    strip.tabs.apply(Action::Select(2), || unreachable!());
    strip.frame(Vec::new());
    strip.seen.clear();
    let at = strip.center(0);
    strip.click(at);
    assert_eq!(strip.seen, [Action::Select(0)]);
    assert_eq!(strip.tabs.active(), 0);
    assert_eq!(order(&strip.tabs), [0, 1, 2, 3]);
}

#[test]
fn cmd_1_selects_main_and_cmd_2_to_9_select_the_agent_tabs_in_order() {
    let mut strip = Strip::new();
    strip.tabs.apply(Action::Select(3), || unreachable!());
    let nth = |n: usize| Command::SelectTab(n).action();
    assert_eq!(nth(0), Some(Action::SelectNth(0)));
    for (key, want) in [(1, 1), (2, 2), (0, 0)] {
        strip.tabs.apply(nth(key).unwrap(), || unreachable!());
        assert_eq!(strip.tabs.active(), want, "SelectTab({key})");
    }
}

#[test]
fn the_main_icon_has_the_active_tab_look_only_while_main_is_selected() {
    let (on, _, _) = main_icon_shapes(true, false, false);
    assert!(on.contains(&brand::TEAL_NAV) && on.contains(&brand::TEAL), "{on:?}");
    let (off, _, _) = main_icon_shapes(false, false, false);
    assert!(!off.contains(&brand::TEAL_NAV) && !off.contains(&brand::TEAL), "{off:?}");
    let (hovered, _, _) = main_icon_shapes(false, false, true);
    assert!(hovered.contains(&brand::TEAL_NAV.gamma_multiply(0.6)), "{hovered:?}");
}

#[test]
fn a_dot_shows_on_the_main_icon_only_while_its_shell_runs_a_command() {
    let (_, running, rect) = main_icon_shapes(false, true, false);
    assert_eq!(running, [brand::SIDEBAR, brand::TEAL]);
    let body = rect.shrink2(vec2(0.0, TAB_INSET_Y));
    let dot = main_icon::dot_center(body);
    assert!(body.contains(dot), "the dot sits on the icon's corner");
    let (_, idle, _) = main_icon_shapes(false, false, false);
    assert_eq!(idle, Vec::<egui::Color32>::new());
}

#[test]
fn the_main_icon_tip_names_the_repo_the_branch_the_chord_and_a_running_command() {
    let keys = Command::SelectTab(0).keys();
    assert_eq!(
        main_icon::tip("acme-web", Some("feat/auth"), false),
        format!("main \u{b7} acme-web \u{b7} feat/auth \u{b7} {keys}")
    );
    assert_eq!(
        main_icon::tip("Other: notes", None, true),
        format!("main \u{b7} Other: notes \u{b7} {keys}\nRunning a command")
    );
}

fn titled_strip(prepare: impl FnOnce(&mut crate::pane::AgentTab)) -> (Vec<(String, Rect)>, String) {
    use crate::agent_run::AgentRun;
    use crate::pane::AgentTab;
    let ctx = crate::fonts::test_context();
    let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
    prepare(&mut agent);
    let mut pane = Pane::shell(1_u32);
    pane.agent = Some(agent);
    let mut tabs = Tabs::new(Pane::shell(0));
    tabs.push_in(0, "brave-otter".into(), pane);
    let mut texts = Vec::new();
    let mut tip = String::new();
    for _ in 0..2 {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    tab_row(
                        ui,
                        &tabs,
                        260.0,
                        0.0,
                        &mut actions,
                        (
                            |_: &Pane<u32>| (Progress::default(), CmdState::default()),
                            |_| TabExtras::default(),
                        ),
                    );
                });
            });
        });
        texts.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        tip = tabs.items()[1]
            .pane
            .agent
            .as_ref()
            .map(|agent| crate::tab_title::label(Some(agent), &tabs.items()[1].title))
            .unwrap_or_default();
    }
    (texts, tip)
}

#[test]
fn an_agent_tab_is_labelled_by_its_work_and_not_by_the_worktree_name() {
    use crate::agent_run::TermSnapshot;
    let (texts, _) = titled_strip(|_| {});
    assert!(texts.iter().any(|(text, _)| text.contains("brave-otter")), "{texts:?}");
    let (texts, label) = titled_strip(|agent| agent.run.set_goal(Some("Fix the login redirect")));
    assert!(texts.iter().any(|(text, _)| text.contains("Fix the login redirect")), "{texts:?}");
    assert!(texts.iter().all(|(text, _)| !text.contains("brave-otter")), "{texts:?}");
    assert_eq!(label, "Fix the login redirect");
    let (texts, _) = titled_strip(|agent| {
        agent.run.set_goal(Some("Ledger goal"));
        let title = Some("\u{2733} Add retry to sync".to_owned());
        agent.run.feed(TermSnapshot { title, ..TermSnapshot::default() }, 1.0);
    });
    assert!(texts.iter().any(|(text, _)| text.contains("Add retry to sync")), "{texts:?}");
    assert!(texts.iter().all(|(text, _)| !text.contains("Ledger goal")), "{texts:?}");
}

#[test]
fn the_tooltip_names_the_worktree_under_the_work() {
    let got = tab_tip("Fix login", "brave-otter", 12, None);
    assert_eq!(got, "Fix login\nbrave-otter\nTab 12");
}

#[test]
fn the_avatar_menu_is_a_table_of_picture_tool_and_lock() {
    let cases = [
        (true, true, false, AvatarMenu::Picture),
        (true, false, false, AvatarMenu::Picture),
        (false, true, false, AvatarMenu::Letter),
        (false, false, false, AvatarMenu::None),
        (true, true, true, AvatarMenu::None),
        (false, true, true, AvatarMenu::None),
    ];
    for (picture, can_draw, locked, want) in cases {
        assert_eq!(
            avatar_menu(picture, can_draw, locked),
            want,
            "picture {picture} can_draw {can_draw} locked {locked}"
        );
    }
}
