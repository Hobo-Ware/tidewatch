use egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};

use super::*;

fn frame(ctx: &egui::Context, recent: &[Recent], events: Vec<Event>) -> Option<Action> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let mut action = None;
    let _ = ctx.run_ui(raw, |ui| {
        let area = ui.max_rect();
        action = show(ui, area, recent, None, "Cmd+O");
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
