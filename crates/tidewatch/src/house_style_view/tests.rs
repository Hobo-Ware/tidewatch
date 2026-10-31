//! House style page: renders, one click opens the file, reset needs a second click.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, pos2};

use super::*;

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1000.0, 700.0))),
        focused: true,
        events,
        ..RawInput::default()
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn frame(ctx: &egui::Context, events: Vec<Event>) -> Option<Action> {
    frame_with(ctx, events, &[])
}

fn frame_with(ctx: &egui::Context, events: Vec<Event>, dead: &[DeadLine]) -> Option<Action> {
    let mut answer = None;
    let _ = ctx.run_ui(input(events), |ui| {
        answer = page(ui, "/home/u/.config/tidewatch/house-style.md", dead);
    });
    answer
}

fn click(ctx: &egui::Context, at: Pos2) -> Option<Action> {
    frame(ctx, vec![Event::PointerMoved(at)]);
    frame(ctx, vec![press(at, true)]);
    frame(ctx, vec![press(at, false)])
}

fn scan(ctx: &egui::Context, ys: std::ops::Range<i32>) -> Option<Action> {
    for y in ys.step_by(4) {
        for x in (700..990).step_by(6) {
            if let Some(action) = click(ctx, pos2(x as f32, y as f32)) {
                return Some(action);
            }
        }
    }
    None
}

#[test]
fn the_page_renders_and_asks_for_nothing_until_clicked() {
    let ctx = crate::fonts::test_context();
    for _ in 0..3 {
        assert!(frame(&ctx, vec![]).is_none());
    }
}

#[test]
fn open_in_editor_is_one_click() {
    let ctx = crate::fonts::test_context();
    frame(&ctx, vec![]);
    let action = scan(&ctx, 90..200);
    assert!(matches!(action, Some(Action::OpenHouseStyle)));
}

fn reset(ctx: &egui::Context, events: Vec<Event>) -> bool {
    let mut answer = false;
    let _ = ctx.run_ui(input(events), |ui| answer = reset_button(ui));
    answer
}

fn tap(ctx: &egui::Context, at: Pos2) -> bool {
    reset(ctx, vec![Event::PointerMoved(at)]);
    reset(ctx, vec![press(at, true)]);
    reset(ctx, vec![press(at, false)])
}

#[test]
fn reset_needs_a_second_click_and_a_click_elsewhere_disarms_it() {
    let ctx = crate::fonts::test_context();
    reset(&ctx, vec![]);
    let at = pos2(40.0, 15.0);
    assert!(!tap(&ctx, at), "the first click only arms the button");
    assert!(tap(&ctx, at), "the second one resets");
    assert!(!tap(&ctx, at), "and it is armed again from scratch");
    tap(&ctx, pos2(600.0, 400.0));
    assert!(!tap(&ctx, at), "a click elsewhere disarmed it, so this click arms it again");
    assert!(tap(&ctx, at));
}

fn dead() -> Vec<DeadLine> {
    tidewatch_core::dead_tools::scan("Be brief.\nCall `stage_done` at the end.\n")
}

fn click_with(ctx: &egui::Context, at: Pos2, dead: &[DeadLine]) -> Option<Action> {
    frame_with(ctx, vec![Event::PointerMoved(at)], dead);
    frame_with(ctx, vec![press(at, true)], dead);
    frame_with(ctx, vec![press(at, false)], dead)
}

fn is_removal(action: Option<&Action>) -> bool {
    matches!(action, Some(Action::RemoveHouseStyleLine { .. }))
}

fn second_click_removes(ctx: &egui::Context, dead: &[DeadLine]) -> Option<Action> {
    for y in (90..400).step_by(4) {
        for x in (700..990).step_by(6) {
            let at = pos2(x as f32, y as f32);
            let first = click_with(ctx, at, dead);
            assert!(!is_removal(first.as_ref()), "a first click asked for the removal at {at:?}");
            let second = click_with(ctx, at, dead);
            if is_removal(second.as_ref()) {
                return second;
            }
        }
    }
    None
}

#[test]
fn a_page_with_a_dead_line_renders_and_asks_for_nothing_until_clicked() {
    let ctx = crate::fonts::test_context();
    let lines = dead();
    for _ in 0..3 {
        assert!(frame_with(&ctx, vec![], &lines).is_none());
    }
}

#[test]
fn a_dead_line_goes_only_on_a_second_click_and_the_action_names_its_number_and_text() {
    let ctx = crate::fonts::test_context();
    let lines = dead();
    frame_with(&ctx, vec![], &lines);
    let Some(Action::RemoveHouseStyleLine { number, text }) = second_click_removes(&ctx, &lines)
    else {
        panic!("no removal asked")
    };
    assert_eq!((number, text.as_str()), (2, "Call `stage_done` at the end."));
}

#[test]
fn a_clean_file_offers_no_removal() {
    let ctx = crate::fonts::test_context();
    frame(&ctx, vec![]);
    assert!(second_click_removes(&ctx, &[]).is_none());
}
