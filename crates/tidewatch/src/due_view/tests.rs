//! The card: its room, and headless frames for Ask, Snooze, a busy agent and text that never overlaps.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Shape};
use tidewatch_core::progress::Mark;

use super::*;

fn due(steps: usize) -> Due {
    Due {
        what: "Switch off the old webhook (#42) with a title long enough to need cutting at narrow widths".into(),
        days: 0,
        steps: (0..steps)
            .map(|i| {
                let mark = [Mark::Done, Mark::Todo, Mark::Doing, Mark::Skipped][i % 4];
                (mark, format!("Step number {i} with some words so it can be cut short too"))
            })
            .collect(),
    }
}

struct Card {
    ctx: egui::Context,
    due: Due,
    width: f32,
    blocked: Option<&'static str>,
    texts: Vec<(String, Rect)>,
    action: Option<Action>,
}

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) if !text.galley.text().trim().is_empty() => out
            .push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

impl Card {
    fn new(steps: usize, width: f32, blocked: Option<&'static str>) -> Self {
        let mut card = Self {
            ctx: crate::fonts::test_context(),
            due: due(steps),
            width,
            blocked,
            texts: Vec::new(),
            action: None,
        };
        card.frame(Vec::new());
        card.frame(Vec::new());
        card
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 500.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let facts = Facts { due: &self.due, more: 1, profile: "Builder", blocked: self.blocked };
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, height(&self.due, 500.0)));
        let output = self.ctx.run_ui(raw, |ui| action = show(ui, rect, facts));
        self.texts.clear();
        for shape in &output.shapes {
            collect(&shape.shape, &mut self.texts);
        }
        self.action = action.clone();
        action
    }

    fn at(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }
}

#[test]
fn the_card_takes_what_its_steps_need_and_nothing_when_the_room_is_short() {
    let no_steps = height(&due(0), 600.0);
    let two = height(&due(2), 600.0);
    let many = height(&due(9), 600.0);
    assert!(no_steps > 0.0 && two > no_steps && many > two);
    assert_eq!(two - no_steps, GAP + 2.0 * STEP_H);
    assert_eq!(many - height(&due(MAX_STEPS), 600.0), MORE_H);
    assert_eq!(height(&due(9), 200.0), 0.0, "does not fit, so it is left out");
    assert_eq!(height(&due(2), 50.0), 0.0);
}

#[test]
fn the_ask_button_names_the_profile_and_cuts_a_long_name() {
    assert_eq!(ask_label("Builder"), "Ask Builder to start");
    let long = ask_label("A profile with a very long name indeed");
    assert!(long.ends_with("\u{2026} to start"), "{long}");
}

#[test]
fn the_card_says_what_is_due_with_its_steps_and_buttons() {
    let card = Card::new(3, 700.0, None);
    for want in ["Due today", "+1 more due", "Ask Builder to start", "Snooze"] {
        assert!(card.texts.iter().any(|(text, _)| text == want), "{want}");
    }
    assert!(card.texts.iter().any(|(text, _)| text.starts_with("Step number 2")));
}

#[test]
fn ask_hands_back_the_message_and_snooze_the_item() {
    let mut card = Card::new(3, 700.0, None);
    let ask = card.at("Ask Builder to start");
    let Some(Action::Ask(text)) = card.click(ask) else { panic!("{:?}", card.action) };
    assert!(text.starts_with("Due today: Switch off the old webhook (#42)"), "{text}");
    assert!(text.contains("Steps:") && text.ends_with("update the progress report."));
    let snooze = card.at("Snooze");
    assert_eq!(card.click(snooze), Some(Action::Snooze(card.due.what.clone())));
}

#[test]
fn a_busy_agent_leaves_ask_off_and_snooze_on() {
    let mut card = Card::new(2, 700.0, Some("The agent is busy. Ask when it is idle"));
    let ask = card.at("Ask Builder to start");
    assert_eq!(card.click(ask), None);
    let snooze = card.at("Snooze");
    assert!(matches!(card.click(snooze), Some(Action::Snooze(_))));
}

#[test]
fn no_text_overlaps_or_leaves_the_card_at_any_reader_width() {
    for width in [320.0, 420.0, 600.0, 900.0] {
        let card = Card::new(9, width, None);
        for (index, (a, a_rect)) in card.texts.iter().enumerate() {
            assert!(a_rect.right() <= width - INSET + 0.5, "{a:?} runs out at {width}");
            for (b, b_rect) in &card.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}
