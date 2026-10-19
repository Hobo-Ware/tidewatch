use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, pos2, vec2};
use jiff::tz::TimeZone;
use tidewatch_core::job::{JobState, Report};

use super::*;
use crate::job_glue::fake_board;
use crate::job_model::cards;

const NOW: i64 = 1_760_875_200_000;

struct Frame {
    ctx: egui::Context,
    cards: Vec<Card>,
    width: f32,
    texts: Vec<(String, Rect)>,
    height: f32,
}

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) => out.push((
            text.galley.text().to_owned(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

impl Frame {
    fn new(cards: Vec<Card>, width: f32) -> Self {
        let mut frame = Self {
            ctx: crate::fonts::test_context(),
            cards,
            width,
            texts: Vec::new(),
            height: 0.0,
        };
        frame.run(Vec::new());
        frame.run(Vec::new());
        frame
    }

    fn run(&mut self, events: Vec<Event>) -> Option<(u64, String)> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width + 40.0, 900.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let (cards, width) = (&self.cards, self.width);
        let mut asked = None;
        let mut height = 0.0;
        let output = self.ctx.run_ui(raw, |ui| {
            ui.set_max_width(width);
            let top = ui.cursor().top();
            asked = show(ui, width, cards);
            height = ui.cursor().top() - top;
        });
        self.height = height;
        self.texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.texts);
        }
        asked
    }

    fn center_of(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(shown, _)| shown == text)
            .unwrap_or_else(|| panic!("{text}: {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Option<(u64, String)> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.run(vec![Event::PointerMoved(at)]);
        self.run(vec![button(true)]);
        self.run(vec![button(false)])
    }
}

#[test]
fn the_mockup_cards_say_what_the_mockup_says() {
    let frame = Frame::new(cards(&fake_board(NOW), NOW, &TimeZone::UTC), 400.0);
    for want in [
        "Backfill orders",
        "21.2%",
        "12,431 / 58,650 rows",
        "RATE",
        "ETA",
        "DONE AT",
        "PR checks",
        "3 of 5 checks",
        "Import users",
        "Stuck at 97.4% for 6m",
        "Ask agent",
        "cargo test",
        "2,246 passed \u{b7} 4m",
    ] {
        assert!(frame.texts.iter().any(|(text, _)| text == want), "{want}: {:?}", frame.texts);
    }
}

#[test]
fn ask_agent_hands_back_the_job_and_its_message() {
    let mut frame = Frame::new(cards(&fake_board(NOW), NOW, &TimeZone::UTC), 400.0);
    let at = frame.center_of("Ask agent");
    let (job, message) = frame.click(at).expect("a click on the button asks");
    assert_eq!(message, "Import users has not moved for 6m. Check on it.");
    let stalled = frame.cards.iter().find_map(|card| match card {
        Card::Stalled(card) => Some(card.id),
        _ => None,
    });
    assert_eq!(Some(job), stalled);
}

#[test]
fn a_click_anywhere_else_asks_nothing() {
    let mut frame = Frame::new(cards(&fake_board(NOW), NOW, &TimeZone::UTC), 400.0);
    let at = frame.center_of("Backfill orders");
    assert_eq!(frame.click(at), None);
}

#[test]
fn no_text_overlaps_at_any_width_from_narrow_to_wide() {
    for width in [180.0, 240.0, 320.0, 400.0, 600.0] {
        let frame = Frame::new(cards(&fake_board(NOW), NOW, &TimeZone::UTC), width);
        for (index, (a, rect_a)) in frame.texts.iter().enumerate() {
            for (b, rect_b) in &frame.texts[index + 1..] {
                let overlap = rect_a.intersect(*rect_b);
                assert!(
                    overlap.width() < 1.0 || overlap.height() < 1.0,
                    "{a:?} over {b:?} at {width}"
                );
            }
            assert!(rect_a.right() <= width + 41.0, "{a:?} runs out at {width}");
        }
    }
}

#[test]
fn a_card_for_every_kind_takes_height_and_nothing_means_none() {
    let none = Frame::new(Vec::new(), 400.0);
    assert_eq!(none.height, 0.0);
    let mut board = fake_board(NOW);
    let all = Frame::new(cards(&board, NOW, &TimeZone::UTC), 400.0);
    board.remove_finished();
    let without_finished = Frame::new(cards(&board, NOW, &TimeZone::UTC), 400.0);
    assert!(all.height > without_finished.height);
    assert!(without_finished.height > 200.0);
}

#[test]
fn a_failed_job_reads_failed_in_its_line() {
    let mut board = tidewatch_core::job::JobBoard::default();
    board.report(&Report { state: Some(JobState::Failed), ..Report::named("lint") }, NOW);
    let frame = Frame::new(cards(&board, NOW + 61_000, &TimeZone::UTC), 400.0);
    assert!(frame.texts.iter().any(|(text, _)| text == "failed \u{b7} 0s"), "{:?}", frame.texts);
}

#[test]
fn a_running_job_with_no_total_still_draws_its_count() {
    let mut board = tidewatch_core::job::JobBoard::default();
    board.report(
        &Report { done: Some(373.0), unit: Some("rows".into()), ..Report::named("scan") },
        NOW,
    );
    let frame = Frame::new(cards(&board, NOW, &TimeZone::UTC), 400.0);
    assert!(frame.texts.iter().any(|(text, _)| text == "373 rows"), "{:?}", frame.texts);
    assert!(frame.texts.iter().any(|(text, _)| text == "estimating"));
}

#[test]
fn the_card_after_the_stalled_one_starts_below_it() {
    let mut frame = Frame::new(cards(&fake_board(NOW), NOW, &TimeZone::UTC), 400.0);
    frame.run(Vec::new());
    let ask = frame.texts.iter().find(|(text, _)| text == "Ask agent").unwrap().1;
    let finished = frame.texts.iter().find(|(text, _)| text == "cargo test").unwrap().1;
    assert!(finished.top() > ask.bottom() + 8.0, "{finished:?} under {ask:?}");
}
