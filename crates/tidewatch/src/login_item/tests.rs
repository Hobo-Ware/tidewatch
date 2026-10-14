use egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2, pos2};

use super::*;

#[test]
fn each_status_shows_the_switch_and_help_it_needs() {
    let cases = [
        (LoginItem::Off, false, true, false),
        (LoginItem::NotFound, false, true, false),
        (LoginItem::On, true, true, false),
        (LoginItem::NeedsApproval, true, true, true),
        (LoginItem::Unavailable, false, false, false),
    ];
    for (status, on, enabled, approve) in cases {
        let got = look(status);
        assert_eq!((got.on, got.enabled, got.approve), (on, enabled, approve), "{status:?}");
    }
}

#[test]
fn only_the_waiting_and_the_unavailable_states_say_something_new() {
    assert_eq!(look(LoginItem::Off).text, look(LoginItem::On).text);
    assert_ne!(look(LoginItem::NeedsApproval).text, look(LoginItem::On).text);
    assert_ne!(look(LoginItem::Unavailable).text, look(LoginItem::On).text);
}

#[test]
fn the_status_is_read_when_the_page_shows_and_then_at_most_once_a_second() {
    let mut row = Row::default();
    assert_eq!(row.refresh(0.0, || LoginItem::NeedsApproval), LoginItem::NeedsApproval);
    let kept = row.refresh(0.5, || panic!("read again before the second was up"));
    assert_eq!(kept, LoginItem::NeedsApproval);
    assert_eq!(row.refresh(1.0, || LoginItem::On), LoginItem::On);
    assert_eq!(row.refresh(1.4, || panic!("read again too soon")), LoginItem::On);
    assert_eq!(row.refresh(0.2, || LoginItem::Off), LoginItem::Off, "the clock went back");
}

#[test]
fn a_click_makes_the_next_frame_read_the_real_status() {
    let mut row = Row::default();
    row.refresh(0.0, || LoginItem::Off);
    row.read_again();
    assert_eq!(row.refresh(0.1, || LoginItem::On), LoginItem::On);
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 300.0))),
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
        modifiers: egui::Modifiers::NONE,
    }
}

fn draw(ctx: &egui::Context, status: LoginItem, events: Vec<Event>) -> Option<Click> {
    let mut row = Row::default();
    let mut clicked = None;
    let _ = ctx.run_ui(input(events), |ui| {
        settings_ui::card(ui, |ui| clicked = show(ui, &mut row, 0.0, || status));
    });
    clicked
}

fn click_around(status: LoginItem, xs: std::ops::Range<i32>) -> Option<Click> {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::fonts::definitions());
    draw(&ctx, status, vec![]);
    for x in xs.step_by(6) {
        for y in [30.0, 44.0, 58.0] {
            let at = pos2(x as f32, y);
            draw(&ctx, status, vec![Event::PointerMoved(at)]);
            draw(&ctx, status, vec![press(at, true)]);
            if let Some(click) = draw(&ctx, status, vec![press(at, false)]) {
                return Some(click);
            }
        }
    }
    None
}

#[test]
fn a_click_on_the_switch_asks_to_register_or_unregister() {
    let cases = [
        (LoginItem::Off, Some(Click::Set(true))),
        (LoginItem::NotFound, Some(Click::Set(true))),
        (LoginItem::On, Some(Click::Set(false))),
        (LoginItem::Unavailable, None),
    ];
    for (status, want) in cases {
        assert_eq!(click_around(status, 800..890), want, "{status:?}");
    }
}

#[test]
fn a_pending_approval_offers_the_system_settings_button_beside_the_switch() {
    let got = click_around(LoginItem::NeedsApproval, 560..700);
    assert_eq!(got, Some(Click::Approve));
}
