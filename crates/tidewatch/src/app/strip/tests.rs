use egui::{Event, PointerButton, RawInput, Rect, Shape, pos2, vec2};

use super::*;

const SHELLS: u32 = 3;

fn tabs() -> Tabs<Pane<u32>> {
    let mut tabs = Tabs::new(Pane::shell(0));
    for n in 1..=SHELLS {
        tabs.apply(Action::New, || Pane::shell(n));
    }
    tabs
}

fn order(tabs: &Tabs<Pane<u32>>) -> Vec<u32> {
    tabs.items()
        .iter()
        .map(|tab| match tab.pane.body {
            Body::Shell(n) => n,
            _ => unreachable!("only shells here"),
        })
        .collect()
}

struct Strip {
    ctx: egui::Context,
    tabs: Tabs<Pane<u32>>,
    time: f64,
    rects: Vec<Rect>,
    seen: Vec<Action>,
}

impl Strip {
    fn new() -> Self {
        let mut strip = Self {
            ctx: egui::Context::default(),
            tabs: tabs(),
            time: 0.0,
            rects: Vec::new(),
            seen: Vec::new(),
        };
        strip.frame(Vec::new());
        strip
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
        let _ = self.ctx.run_ui(raw, |ui| {
            egui::Panel::top("tabbar").exact_size(TAB_H + 6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    rects = tab_row(
                        ui,
                        &self.tabs,
                        120.0,
                        0.0,
                        &mut actions,
                        (|_: &Pane<u32>| (Progress::default(), CmdState::default()), |_| None),
                    );
                });
            });
        });
        self.rects = rects;
        for action in actions {
            self.tabs.apply(action, || unreachable!());
            self.seen.push(action);
        }
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
    let cases = [(200.0, 7.2, 2, 15), (120.0, 7.2, 2, 4), (60.0, 7.2, 2, 1), (200.0, 7.2, 12, 14)];
    for (width, char_w, number, want) in cases {
        assert_eq!(title_budget(width, char_w, number), want, "{width} {number}");
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

fn agent_strip(title: &str, width: f32, with_ring: bool) -> (Vec<Rect>, Vec<(String, Rect)>) {
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
                            |_| with_ring.then(|| tab_ring(&ledger)),
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

#[test]
fn a_tab_with_a_ledger_cuts_its_title_before_the_ring() {
    for width in [100.0, 120.0, 200.0] {
        let (rects, texts) = agent_strip("a-very-long-worktree-name-indeed", width, true);
        let tab = rects[1];
        let title = texts.iter().find(|(text, _)| text.starts_with("2  ")).unwrap();
        assert!(
            title.1.right() <= ring_rect(tab).left(),
            "{:?} reaches the ring at {width}",
            title.1
        );
        assert!(title.0.ends_with('\u{2026}'), "{}", title.0);
    }
}

#[test]
fn a_tab_without_a_ledger_keeps_the_whole_title_room() {
    let (rects, with) = agent_strip("a-very-long-worktree-name-indeed", 200.0, true);
    let (_, without) = agent_strip("a-very-long-worktree-name-indeed", 200.0, false);
    let width = |texts: &[(String, Rect)]| {
        texts.iter().find(|(text, _)| text.starts_with("2  ")).unwrap().1.width()
    };
    assert!(width(&without) > width(&with));
    assert_eq!(rects[1].width(), 200.0);
}
