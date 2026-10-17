//! The section: geometry, and headless frames for the clicks and for text that never overlaps.

use egui::{Event, Modifiers, PointerButton, RawInput, Shape};
use tidewatch_core::progress::Mark;

use super::*;
use crate::timeline::{Block, Card, Label, Urgency};

fn card_of(row: usize, title: &str, look: Look, button: Option<Button>, steps: &[Mark]) -> Card {
    Card {
        row,
        title: title.into(),
        tab: "webhook-switch".into(),
        steps: steps.to_vec(),
        look,
        button,
        tip: title.into(),
    }
}

fn blocks() -> Vec<Block> {
    let date = |text: &str, under: &str, urgency| Label::Date {
        text: text.into(),
        under: under.into(),
        urgency,
    };
    vec![
        Block {
            label: Label::Now,
            cards: vec![card_of(
                0,
                "Check the new route",
                Look::Now,
                None,
                &[Mark::Done, Mark::Doing],
            )],
        },
        Block {
            label: date("NOV 1", "15 d", Urgency::Ahead),
            cards: vec![card_of(
                1,
                "Switch off the old webhook and a title that is very long indeed so it must be cut",
                Look::Plain,
                Some(Button::Open),
                &[Mark::Done, Mark::Todo, Mark::Todo, Mark::Todo],
            )],
        },
        Block {
            label: Label::Later,
            cards: vec![
                card_of(2, "Theme labels", Look::On, None, &[]),
                card_of(3, "Stop collection writes", Look::Parked, None, &[]),
                card_of(4, "Review bot on the API repo", Look::You, Some(Button::Decide), &[]),
            ],
        },
    ]
}

struct Section {
    ctx: egui::Context,
    blocks: Vec<Block>,
    width: f32,
    texts: Vec<(String, Rect)>,
    clicked: Option<usize>,
}

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) if !text.galley.text().trim().is_empty() => out
            .push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

impl Section {
    fn new(width: f32) -> Self {
        let mut section = Self {
            ctx: crate::fonts::test_context(),
            blocks: blocks(),
            width,
            texts: Vec::new(),
            clicked: None,
        };
        section.frame(Vec::new());
        section.frame(Vec::new());
        section
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<usize> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 700.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut clicked = None;
        let blocks = &self.blocks;
        let output = self.ctx.run_ui(raw, |ui| clicked = show(ui, blocks));
        self.texts.clear();
        for shape in &output.shapes {
            collect(&shape.shape, &mut self.texts);
        }
        self.clicked = clicked;
        clicked
    }

    fn at(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Option<usize> {
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
fn a_block_is_as_tall_as_its_cards_and_their_gaps() {
    let all = blocks();
    let heights: Vec<f32> = all.iter().map(block_height).collect();
    assert_eq!(heights, [CARD_H, CARD_H, 3.0 * CARD_H + 2.0 * CARD_GAP]);
    assert_eq!(block_height(&Block { label: Label::Later, cards: vec![] }), 0.0);
}

#[test]
fn the_right_side_gives_way_button_last_to_go_then_name_then_steps() {
    let parts = [60.0, 80.0, 50.0];
    let cases = [(1000.0, 3), (330.0, 3), (300.0, 2), (200.0, 1), (100.0, 0)];
    for (width, want) in cases {
        assert_eq!(parts_that_fit(width, &parts), want, "{width}");
    }
    assert_eq!(parts_that_fit(500.0, &[]), 0);
}

#[test]
fn a_rounded_path_stays_inside_its_rect_and_closes() {
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 48.0));
    let path = rounded_path(rect, 12.0);
    assert_eq!(path.first(), path.last());
    for point in &path {
        assert!(rect.expand(0.01).contains(*point), "{point:?}");
    }
    assert!(path.iter().any(|p| (p.x - rect.right()).abs() < 0.01));
    assert!(path.iter().any(|p| (p.y - rect.top()).abs() < 0.01));
}

#[test]
fn steps_show_as_segments_up_to_ten() {
    assert_eq!(steps_width(&[]), 0.0);
    assert_eq!(steps_width(&[Mark::Done]), SEG_W);
    assert_eq!(steps_width(&[Mark::Done; 2]), 2.0 * SEG_W + SEG_GAP);
    assert_eq!(steps_width(&[Mark::Done; 30]), steps_width(&[Mark::Done; 10]));
}

#[test]
fn the_labels_and_titles_are_drawn() {
    let section = Section::new(900.0);
    for want in ["Coming up", "NOW", "NOV 1", "15 d", "LATER", "Theme labels", "Decide", "Open"] {
        assert!(section.texts.iter().any(|(text, _)| text == want), "{want}");
    }
}

#[test]
fn decide_and_open_ask_for_their_row_and_so_does_a_card() {
    let mut section = Section::new(900.0);
    let decide = section.at("Decide");
    assert_eq!(section.click(decide), Some(4));
    let open = section.at("Open");
    assert_eq!(section.click(open), Some(1));
    let plain = section.at("Theme labels");
    assert_eq!(section.click(plain), Some(2));
    assert_eq!(section.frame(Vec::new()), None);
}

#[test]
fn no_text_overlaps_at_any_width_the_overview_has() {
    for width in [320.0, 420.0, 600.0, 900.0, 1300.0] {
        let section = Section::new(width);
        for (index, (a, a_rect)) in section.texts.iter().enumerate() {
            assert!(a_rect.right() <= width + 0.5, "{a:?} runs out at {width}");
            for (b, b_rect) in &section.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}
