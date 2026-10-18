use egui::{Event, Modifiers, PointerButton, RawInput};

use super::*;

const LINK: &str = "https://claude.ai/code/session_01AbCd";

fn pill(link: Option<&str>, saved_on: bool) -> Pill {
    Pill {
        name: "web \u{b7} otter".into(),
        app: "the Claude app",
        link: link.map(str::to_owned),
        saved_on,
    }
}

struct Harness {
    ctx: egui::Context,
    pill: Pill,
    clock: f64,
    popover: Popover,
}

impl Harness {
    fn new(pill: Pill) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut harness = Self { ctx, pill, clock: 0.0, popover: Popover::OnClick };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        self.clock += 0.1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 400.0))),
            events,
            focused: true,
            time: Some(self.clock),
            ..RawInput::default()
        };
        let mut action = None;
        let _ = self.ctx.run_ui(raw, |ui| {
            action = show(ui, 600.0, 20.0, &self.pill, self.popover).action;
        });
        action
    }

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp(placed_id())).expect("the pill has drawn")
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        self.frame(vec![Event::PointerMoved(at)]);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }

    fn open_popover(&mut self) {
        let at = self.placed().pill.expect("a pill").center();
        assert_eq!(self.click(at), None);
        self.frame(Vec::new());
        assert!(self.placed().copy.is_some(), "the popover is open");
    }

    fn click_copy(&mut self) -> Option<Action> {
        let at = self.placed().copy.expect("Copy").center();
        self.click(at)
    }
}

#[test]
fn the_popover_says_where_to_open_it_and_what_the_copy_button_does() {
    let cases = [
        (
            pill(Some(LINK), true),
            "Open it in the Claude app",
            "Copy link",
            "Turn off applies next time the agent starts.",
        ),
        (
            pill(None, true),
            "No link seen in the terminal yet. Look for this name in the Claude app.",
            "Copy name",
            "Turn off applies next time the agent starts.",
        ),
        (
            pill(Some(LINK), false),
            "Open it in the Claude app",
            "Copy link",
            "Off. Applies next time the agent starts.",
        ),
    ];
    for (pill, sub, copy, note_text) in cases {
        assert_eq!(
            (subline(&pill).as_str(), copy_label(&pill), note(&pill)),
            (sub, copy, note_text)
        );
    }
}

#[test]
fn the_pill_draws_and_opens_nothing_by_itself() {
    let harness = Harness::new(pill(Some(LINK), true));
    let placed = harness.placed();
    assert!(placed.pill.is_some());
    assert_eq!((placed.copy, placed.turn_off), (None, None));
    assert!(placed.pill.unwrap().right() <= 600.0);
}

#[test]
fn a_click_on_the_pill_opens_the_popover_and_copy_link_copies_the_link_seen() {
    let mut harness = Harness::new(pill(Some(LINK), true));
    harness.open_popover();
    assert_eq!(harness.click_copy(), Some(Action::CopyLink(LINK.to_owned())));
}

#[test]
fn without_a_link_the_button_copies_the_name() {
    let mut harness = Harness::new(pill(None, true));
    harness.open_popover();
    assert_eq!(harness.click_copy(), Some(Action::CopyName("web \u{b7} otter".to_owned())));
}

#[test]
fn turn_off_is_offered_while_the_saved_setting_is_on_and_flips_nothing_by_itself() {
    let mut harness = Harness::new(pill(Some(LINK), true));
    harness.open_popover();
    let at = harness.placed().turn_off.expect("Turn off").center();
    assert_eq!(harness.click(at), Some(Action::TurnOff));
}

#[test]
fn once_turned_off_the_popover_only_offers_to_copy() {
    let mut harness = Harness::new(pill(Some(LINK), false));
    harness.open_popover();
    assert_eq!(harness.placed().turn_off, None);
    assert_eq!(harness.click_copy(), Some(Action::CopyLink(LINK.to_owned())));
}

#[test]
fn a_click_outside_closes_the_popover() {
    let mut harness = Harness::new(pill(Some(LINK), true));
    harness.open_popover();
    assert_eq!(harness.click(pos2(40.0, 300.0)), None);
    harness.frame(Vec::new());
    assert_eq!(harness.placed().copy, None);
}

#[test]
fn the_dev_switch_opens_the_popover_without_a_click() {
    let mut harness = Harness::new(pill(Some(LINK), true));
    harness.popover = Popover::Open;
    harness.frame(Vec::new());
    harness.frame(Vec::new());
    assert!(harness.placed().copy.is_some());
}
