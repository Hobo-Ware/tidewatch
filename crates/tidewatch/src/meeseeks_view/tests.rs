use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, pos2, vec2};
use tidewatch_core::meeseeks::{Criteria, Meeseeks, Phase, Step};

use super::*;
use crate::meeseeks_model;

const MIN: i64 = 60_000;

struct Harness {
    ctx: egui::Context,
    card: Card,
    width: f32,
    clock: f64,
    action: Option<Action>,
    texts: Vec<(String, Pos2)>,
}

fn card_of(meeseeks: &Meeseeks, now_ms: i64) -> Card {
    meeseeks_model::card(meeseeks, "rename", now_ms, Some("npm test"))
}

fn running() -> Meeseeks {
    let mut meeseeks = Meeseeks {
        kind: tidewatch_core::kind::Kind::Edit,
        ..Meeseeks::summon("Bump vite to 6.3", Criteria::default(), 0)
    };
    meeseeks.turns = 8;
    meeseeks.phase = Phase::Checking(Step::Tests);
    meeseeks
}

fn struggling() -> Meeseeks {
    Meeseeks { failures: 3, phase: Phase::Working, ..running() }
}

impl Harness {
    fn new(card: Card, width: f32) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut harness = Self { ctx, card, width, clock: 0.0, action: None, texts: Vec::new() };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.clock += 0.1;
        let raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 900.0))),
            events,
            focused: true,
            time: Some(self.clock),
            ..RawInput::default()
        };
        let (card, width) = (&self.card, self.width);
        let mut action = None;
        let output = self.ctx.run_ui(raw, |ui| {
            ui.set_width(width);
            action = show(ui, width, card);
        });
        self.action = action;
        self.texts = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                _ => None,
            })
            .collect();
    }

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp(placed_id())).expect("the card drew")
    }

    fn click(&mut self, at: Pos2) -> Vec<Option<Action>> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut seen = Vec::new();
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            self.frame(events);
            seen.push(self.action);
        }
        seen
    }
}

#[test]
fn the_stage_card_is_the_task_and_the_status_box_with_no_bars_of_its_own() {
    let harness = Harness::new(card_of(&running(), 12 * MIN), 420.0);
    let placed = harness.placed();
    let card = placed.card.unwrap();
    let status = placed.status.unwrap();
    assert!(card.contains_rect(status));
    assert!(placed.struggle.is_none() && placed.take_over.is_none());
    for (name, _) in &harness.card.bars {
        assert!(harness.texts.iter().all(|(text, _)| text != name), "{name} is drawn by the panel");
    }
}

#[test]
fn a_struggling_meeseeks_gets_the_orange_card_under_the_stage_card_with_both_buttons() {
    let harness = Harness::new(card_of(&struggling(), 38 * MIN), 420.0);
    let placed = harness.placed();
    let (stage, struggle) = (placed.card.unwrap(), placed.struggle.unwrap());
    assert!(struggle.top() > stage.bottom());
    let (take, give) = (placed.take_over.unwrap(), placed.give_up.unwrap());
    assert!(struggle.contains_rect(take) && struggle.contains_rect(give));
    assert!(take.right() < give.left());
}

#[test]
fn take_over_and_give_up_are_clicks_that_say_which_and_only_on_release() {
    let mut harness = Harness::new(card_of(&struggling(), 38 * MIN), 420.0);
    let at = harness.placed().take_over.unwrap().center();
    assert_eq!(harness.click(at), [None, None, Some(Action::TakeOver)]);
    let at = harness.placed().give_up.unwrap().center();
    assert_eq!(harness.click(at), [None, None, Some(Action::GiveUp)]);
    let stage = harness.placed().card.unwrap().center();
    assert_eq!(harness.click(stage), [None, None, None], "the stage card has no button");
}

#[test]
fn nothing_overlaps_or_leaves_the_card_from_a_narrow_panel_up() {
    for width in [200.0, 220.0, 260.0, 320.0, 420.0, 600.0] {
        let checking = Meeseeks { phase: Phase::Checking(Step::Tests), ..struggling() };
        let harness = Harness::new(card_of(&checking, 38 * MIN), width);
        let placed = harness.placed();
        let stage = placed.card.unwrap();
        assert!((stage.width() - width).abs() < 0.5, "{width}");
        let struggle = placed.struggle.unwrap();
        assert!(struggle.top() >= stage.bottom(), "{width}");
        assert!(struggle.contains_rect(placed.take_over.unwrap()), "{width}");
        assert!(struggle.contains_rect(placed.give_up.unwrap()), "{width}");
    }
}

#[test]
fn a_task_with_no_room_beside_its_meta_gets_the_whole_line_and_the_meta_goes_under_it() {
    let harness = Harness::new(card_of(&running(), 12 * MIN), 172.0);
    let title = harness.texts.iter().find(|(text, _)| text.starts_with("Bump")).unwrap().1;
    let meta = harness.texts.iter().find(|(text, _)| text.contains("12m")).unwrap().1;
    assert!(meta.y > title.y && meta.x == title.x, "{title:?} {meta:?}");
}
