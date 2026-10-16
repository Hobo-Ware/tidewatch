use egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};

use super::*;

fn frame(ctx: &egui::Context, recent: &[Recent], events: Vec<Event>) -> Option<Action> {
    frame_with(ctx, recent, events, None)
}

fn frame_with(
    ctx: &egui::Context,
    recent: &[Recent],
    events: Vec<Event>,
    elsewhere: Option<(usize, usize)>,
) -> Option<Action> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let mut action = None;
    let _ = ctx.run_ui(raw, |ui| {
        let area = ui.max_rect();
        action = show(ui, area, recent, None, "Cmd+O", elsewhere);
    });
    action
}

fn repos() -> Vec<Recent> {
    vec![
        Recent { name: "web".into(), dir: "/code/web".into(), tabs: 3 },
        Recent { name: "api".into(), dir: "/code/api".into(), tabs: 0 },
    ]
}

fn click(at: egui::Pos2) -> Vec<Event> {
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![Event::PointerMoved(at), button(true), button(false)]
}

#[test]
fn nothing_happens_until_something_is_clicked() {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    assert!(frame(&ctx, &repos(), vec![]).is_none());
    assert!(frame(&ctx, &repos(), vec![]).is_none());
}

#[test]
fn the_open_button_is_in_the_middle_and_a_recent_row_opens_its_repo() {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    frame(&ctx, &repos(), vec![]);
    frame(&ctx, &repos(), vec![]);
    let total =
        MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + 26.0 + 22.0 + 2.0 * (ROW_H + 4.0) + 26.0 + 20.0;
    let top = 350.0 - total / 2.0;
    let button_y = top + MASCOT_H + 26.0 + 90.0 + 26.0 + 23.0;
    let first = frame(&ctx, &repos(), click(pos2(500.0, button_y)));
    assert!(matches!(first, Some(Action::Open)), "the button was not where the layout says");
    let row_y = top + MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + 26.0 + 22.0 + ROW_H / 2.0;
    let second = frame(&ctx, &repos(), click(pos2(500.0, row_y)));
    assert!(matches!(second, Some(Action::Recent(dir)) if dir == Path::new("/code/web")));
}

#[test]
fn with_nothing_recent_the_list_is_left_out_and_the_screen_still_draws() {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    assert!(frame(&ctx, &[], vec![]).is_none());
    assert!(frame(&ctx, &[], vec![]).is_none());
}

#[test]
fn sessions_found_elsewhere_get_a_line_under_the_open_button_with_its_own_button() {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    let found = Some((3, 2));
    frame_with(&ctx, &repos(), vec![], found);
    frame_with(&ctx, &repos(), vec![], found);
    let total = MASCOT_H
        + 26.0
        + 90.0
        + 26.0
        + 48.0
        + SESSIONS_H
        + SESSIONS_GAP
        + 26.0
        + 22.0
        + 2.0 * (ROW_H + 4.0)
        + 26.0
        + 20.0;
    let top = 350.0 - total / 2.0;
    let line_y = top + MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + 26.0 + SESSIONS_H / 2.0;
    let x = 1000.0 / 2.0 + 560.0 / 2.0 - 60.0;
    let hit = (-14..=14).step_by(2).any(|dy| {
        let got = frame_with(&ctx, &repos(), click(pos2(x, line_y + dy as f32)), found);
        matches!(got, Some(Action::MoveSessions))
    });
    assert!(hit, "the line's button was not near where the layout says");
}

#[test]
fn the_line_button_is_off_when_no_found_session_is_idle() {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    let found = Some((1, 0));
    frame_with(&ctx, &[], vec![], found);
    frame_with(&ctx, &[], vec![], found);
    let total = MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + SESSIONS_H + SESSIONS_GAP + 26.0 + 20.0;
    let top = 350.0 - total / 2.0;
    let line_y = top + MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + 26.0 + SESSIONS_H / 2.0;
    let x = 1000.0 / 2.0 + 560.0 / 2.0 - 60.0;
    let any = (-14..=14)
        .step_by(2)
        .any(|dy| frame_with(&ctx, &[], click(pos2(x, line_y + dy as f32)), found).is_some());
    assert!(!any);
}
