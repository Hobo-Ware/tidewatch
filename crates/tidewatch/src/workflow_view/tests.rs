use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

use super::*;
use crate::workflow_glue::stand_in;

fn column() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput { screen_rect: Some(column()), focused: true, events, ..RawInput::default() }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn key(pressed: bool) -> Event {
    Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn frame(ctx: &Context, events: Vec<Event>) -> Option<Answer> {
    let detected = stand_in();
    let mut answer = None;
    let _ = ctx.run_ui(input(events), |ui| answer = card(ui.ctx(), "web", &detected, column()));
    answer
}

fn scan(ctx: &Context, wanted: Answer) -> bool {
    let area = ctx.memory(|memory| memory.area_rect(Id::new(CARD_ID))).unwrap();
    for y in (area.top() as i32..area.bottom() as i32).step_by(4) {
        for x in (area.left() as i32..area.right() as i32).step_by(6) {
            let at = pos2(x as f32, y as f32);
            frame(ctx, vec![Event::PointerMoved(at)]);
            frame(ctx, vec![press(at, true)]);
            if frame(ctx, vec![press(at, false)]) == Some(wanted) {
                return true;
            }
        }
    }
    false
}

#[test]
fn the_card_draws_under_the_tab_strip_and_asks_nothing_by_itself() {
    let ctx = crate::fonts::test_context();
    for _ in 0..3 {
        assert_eq!(frame(&ctx, vec![]), None);
    }
    let area = ctx.memory(|memory| memory.area_rect(Id::new(CARD_ID))).unwrap();
    assert!(area.top() >= layout::CHROME_H, "{area:?}");
    assert!(area.width() <= MAX_WIDTH + 40.0 && area.center().x > 440.0 && area.center().x < 460.0);
    assert!(area.bottom() < 400.0, "{area:?}");
}

#[test]
fn each_button_answers_with_one_click() {
    for wanted in [Answer::Accept, Answer::Change, Answer::NotNow] {
        let ctx = crate::fonts::test_context();
        frame(&ctx, vec![]);
        assert!(scan(&ctx, wanted), "{wanted:?} was never pressed");
    }
}

#[test]
fn escape_dismisses_only_while_the_pointer_is_over_the_card() {
    let ctx = crate::fonts::test_context();
    frame(&ctx, vec![]);
    frame(&ctx, vec![Event::PointerMoved(pos2(5.0, 650.0))]);
    assert_eq!(frame(&ctx, vec![key(true)]), None, "an agent's Esc is left alone");
    let inside = ctx.memory(|memory| memory.area_rect(Id::new(CARD_ID))).unwrap().center();
    frame(&ctx, vec![Event::PointerMoved(inside)]);
    assert_eq!(frame(&ctx, vec![key(true)]), Some(Answer::NotNow));
}

#[test]
fn the_settings_card_draws_for_a_repo_and_without_one_and_changes_nothing_by_itself() {
    let ctx = crate::fonts::test_context();
    let view = View {
        path: "/work/web".into(),
        repo: "web".into(),
        workflow: Workflow::default(),
        detected: Some(stand_in()),
        confirmed: false,
    };
    for shown in [Some(&view), None] {
        let mut changed = None;
        let _ = ctx.run_ui(input(vec![]), |ui| changed = settings_card(ui, shown));
        assert_eq!(changed, None);
    }
}

#[test]
fn evidence_shows_only_for_the_value_that_was_found() {
    let view = View {
        path: "/work/web".into(),
        repo: "web".into(),
        workflow: Workflow::default(),
        detected: Some(stand_in()),
        confirmed: true,
    };
    let found = view.evidence(CatchUp::Rebase, |d| (d.catch_up.value, &d.catch_up.source));
    assert_eq!(found, "found: \"rebase --autosquash origin/main\" in ~/.claude/CLAUDE.md");
    let other = view.evidence(CatchUp::Merge, |d| (d.catch_up.value, &d.catch_up.source));
    assert_eq!(other, "set by you");
    let unconfirmed = View { confirmed: false, ..view.clone() };
    assert_eq!(
        unconfirmed.evidence(CatchUp::Merge, |d| (d.catch_up.value, &d.catch_up.source)),
        ""
    );
    let none = View { detected: None, ..view };
    assert_eq!(none.evidence(CatchUp::Rebase, |d| (d.catch_up.value, &d.catch_up.source)), "");
}
