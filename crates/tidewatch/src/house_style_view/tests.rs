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
    let mut answer = None;
    let _ = ctx
        .run_ui(input(events), |ui| answer = page(ui, "/home/u/.config/tidewatch/house-style.md"));
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
