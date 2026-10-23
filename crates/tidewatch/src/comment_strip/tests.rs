use egui::{Context, Event, PointerButton, Pos2, RawInput};
use tidewatch_core::comment_flags::{Origin, Record};

use super::*;

fn records(count: usize) -> Vec<Record> {
    (0..count)
        .map(|n| Record {
            at_ms: n as u64,
            origin: Origin::Edit,
            file: "src/a.rs".into(),
            line: n + 1,
            kind: "narration".into(),
            text: "loop over the items".into(),
        })
        .collect()
}

fn frames(events: Vec<Vec<Event>>, strip: &Strip<'_>) -> (Vec<Option<Action>>, Rect) {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, HEIGHT));
    let mut seen = Vec::new();
    for batch in std::iter::once(Vec::new()).chain(events) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 300.0))),
            events: batch,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let _ = ctx.run_ui(raw, |ui| action = show(ui, band, strip));
        seen.push(action);
    }
    (seen, band)
}

fn click_at(pos: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![vec![Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]]
}

#[test]
fn the_off_switch_asks_to_turn_the_tab_off_and_a_click_elsewhere_asks_nothing() {
    let rows = records(3);
    let strip = Strip { records: &rows, off: false, dev_open_popover: false };
    let (quiet, band) = frames(vec![vec![]], &strip);
    assert_eq!(quiet, [None, None]);
    let switch = pos2(band.right() - INSET - 14.0 - 20.0, band.center().y);
    let (clicked, _) = frames(click_at(switch), &strip);
    assert_eq!(clicked.last().copied().flatten(), Some(Action::SetOff(true)));
    let blank = pos2(band.left() + 520.0, band.center().y);
    let (clicked, _) = frames(click_at(blank), &strip);
    assert!(clicked.iter().all(Option::is_none));
}

#[test]
fn a_tab_that_is_off_offers_to_turn_the_guard_back_on() {
    let rows = records(2);
    let strip = Strip { records: &rows, off: true, dev_open_popover: false };
    let (_, band) = frames(vec![], &strip);
    let switch = pos2(band.right() - INSET - 14.0 - 20.0, band.center().y);
    let (clicked, _) = frames(click_at(switch), &strip);
    assert_eq!(clicked.last().copied().flatten(), Some(Action::SetOff(false)));
}

#[test]
fn the_strip_draws_with_no_flags_one_flag_many_flags_and_the_popover_open() {
    for count in [0, 1, 40] {
        let rows = records(count);
        for off in [false, true] {
            let strip = Strip { records: &rows, off, dev_open_popover: true };
            let (seen, _) = frames(vec![vec![], vec![]], &strip);
            assert!(seen.iter().all(Option::is_none), "{count} {off}");
        }
    }
}
