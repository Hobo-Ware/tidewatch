//! The card: its room, the order of its parts, text that never overlaps, and its two clicks.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
use tidewatch_core::subagents::{Agent, Phase};

use super::*;
use crate::subagents_model::list;
use crate::test_support::painted_text_rects_nonblank;

const NOW: i64 = 2_000_000;

fn agent(index: usize, phase: Phase) -> Agent {
    Agent {
        id: format!("a{index}"),
        description: format!("Finished task {index}"),
        model: Some("sonnet".into()),
        effort: Some("high".into()),
        started_ms: NOW - 253_000 + index as i64 * 7_000,
        phase,
        now: format!(
            "Grepping sent::now_ms usages across crates for task number {index} with a long tail"
        ),
        tokens: 281_000 + index as u64,
    }
}

fn agents(running: usize, finished: usize) -> Vec<Agent> {
    (0..running)
        .map(|i| agent(i, Phase::Running))
        .chain((0..finished).map(|i| agent(running + i, Phase::Finished(NOW - 5_000))))
        .collect()
}

struct Card {
    ctx: egui::Context,
    list: crate::subagents_model::List,
    width: f32,
    folded: bool,
    opened: bool,
    room: f32,
    texts: Vec<(String, Rect)>,
    band: Rect,
    repaint: std::time::Duration,
}

impl Card {
    fn new(agents: &[Agent], width: f32) -> Self {
        let mut card = Self {
            ctx: crate::fonts::test_context(),
            list: list(agents, NOW).unwrap(),
            width,
            folded: false,
            opened: false,
            room: 600.0,
            texts: Vec::new(),
            band: Rect::NOTHING,
            repaint: std::time::Duration::MAX,
        };
        card.frame(Vec::new());
        card.frame(Vec::new());
        card
    }

    fn facts(&self) -> Facts<'_> {
        Facts { list: &self.list, folded: self.folded, opened: self.opened }
    }

    fn frame(&mut self, events: Vec<Event>) -> Vec<Out> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 700.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let band_h = height(self.facts(), self.room);
        self.band = Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, band_h));
        let (band, list, folded, opened) = (self.band, self.list.clone(), self.folded, self.opened);
        let mut out = Vec::new();
        let output = self.ctx.run_ui(raw, |ui| {
            out = show(ui, band, Facts { list: &list, folded, opened });
        });
        self.repaint = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        self.texts.clear();
        for shape in &output.shapes {
            painted_text_rects_nonblank(&shape.shape, &mut self.texts);
        }
        out
    }

    fn at(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Vec<Out> {
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

    fn drawn(&self, text: &str) -> bool {
        self.texts.iter().any(|(drawn, _)| drawn == text)
    }
}

#[test]
fn the_card_takes_a_header_and_a_line_per_row_and_one_more_line_for_the_rest() {
    let rows = |running| height(Card::new(&agents(running, 0), 700.0).facts(), 600.0);
    assert_eq!(rows(1), 2.0 * OUTER + HEAD_H + ROW_H + FOOT);
    assert_eq!(rows(4), rows(1) + 3.0 * ROW_H);
    assert_eq!(rows(6), rows(1) + 5.0 * ROW_H);
    assert_eq!(rows(9), rows(6) + ROW_H);
}

#[test]
fn a_folded_card_is_its_header_alone() {
    let mut card = Card::new(&agents(4, 1), 700.0);
    card.folded = true;
    assert_eq!(height(card.facts(), 600.0), 2.0 * OUTER + HEAD_H);
    card.frame(Vec::new());
    assert!(card.drawn("Show"));
    assert!(card.drawn("4 agents running"));
    assert!(!card.drawn("Hide"));
    assert!(!card.texts.iter().any(|(text, _)| text.contains("Grepping")));
}

#[test]
fn a_short_room_keeps_the_rows_that_fit_and_a_more_line() {
    let card = Card::new(&agents(8, 0), 700.0);
    let tight = chrome() + 3.0 * ROW_H;
    assert_eq!(height(card.facts(), tight), chrome() + 3.0 * ROW_H);
    assert_eq!(height(card.facts(), 10.0), chrome() + 2.0 * ROW_H);
}

#[test]
fn the_card_shows_four_running_agents_and_one_finished_in_the_boards_words() {
    let card = Card::new(&agents(4, 1), 700.0);
    for text in ["4 agents running", "1 finished \u{b7} ", "Hide", "Finished task 4", "done 3m 40s"]
    {
        assert!(card.drawn(text), "{text:?} in {:?}", card.texts);
    }
    assert_eq!(card.texts.iter().filter(|(text, _)| text == "sonnet \u{b7} high").count(), 5);
    assert!(card.texts.iter().any(|(text, _)| text.starts_with("4m 13s \u{b7} 281k")));
}

#[test]
fn no_text_overlaps_another_or_leaves_the_card_at_any_width() {
    for width in [420.0, 520.0, 700.0, 1200.0] {
        let card = Card::new(&agents(4, 1), width);
        let inside =
            Rect::from_min_max(pos2(INSET, OUTER), pos2(width - INSET, card.band.height() - OUTER));
        for (text, rect) in &card.texts {
            assert!(inside.contains_rect(*rect), "{text:?} {rect:?} outside {inside:?} at {width}");
        }
        for (i, (a, ra)) in card.texts.iter().enumerate() {
            for (b, rb) in &card.texts[i + 1..] {
                let rows_meet = ra.top() < rb.bottom() && rb.top() < ra.bottom();
                assert!(
                    !(rows_meet && ra.left() < rb.right() && rb.left() < ra.right()),
                    "{a:?} {ra:?} overlaps {b:?} {rb:?} at {width}"
                );
            }
        }
    }
}

#[test]
fn the_right_column_lines_up_and_the_tag_gives_way_before_the_text_is_squeezed() {
    let wide = Card::new(&agents(4, 1), 1200.0);
    let rights: Vec<f32> = wide
        .texts
        .iter()
        .filter(|(text, _)| {
            text.contains("m ") && (text.contains('\u{b7}') || text.starts_with("done"))
        })
        .map(|(_, rect)| rect.right())
        .collect();
    assert_eq!(rights.len(), 5, "{:?}", wide.texts);
    assert!(rights.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01), "{rights:?}");

    let narrow = Card::new(&agents(4, 1), 340.0);
    assert!(!narrow.drawn("sonnet \u{b7} high"));
    assert!(narrow.drawn("done 3m 40s"));
}

#[test]
fn hide_folds_and_the_more_line_opens_the_rest_and_then_folds_back() {
    let mut card = Card::new(&agents(8, 0), 700.0);
    assert!(card.drawn("+2 more"));
    let at = card.at("Hide");
    assert_eq!(card.click(at), [Out::Fold]);
    let at = card.at("+2 more");
    assert_eq!(card.click(at), [Out::Open]);
    card.opened = true;
    card.frame(Vec::new());
    assert!(card.drawn("Show fewer"));
    assert_eq!(card.texts.iter().filter(|(text, _)| text.starts_with("Grepping")).count(), 8);
}

#[test]
fn a_list_with_something_running_asks_for_a_frame_each_second_and_a_finished_one_for_none() {
    let running = Card::new(&agents(1, 0), 700.0);
    assert!(running.repaint > std::time::Duration::ZERO);
    assert!(running.repaint <= std::time::Duration::from_secs(1));
    let done = Card::new(&agents(0, 2), 700.0);
    assert!(done.drawn("2 agents finished"));
    assert_eq!(done.repaint, std::time::Duration::MAX);
}
