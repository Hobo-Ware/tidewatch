use egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};
use tidewatch_core::elsewhere::Cli;

use super::*;

fn row(id: &str, status: Status, offer: Offer) -> Row {
    Row {
        key: SessionKey::Stored { cli: Cli::Claude, id: id.to_owned() },
        cli_id: "claude".to_owned(),
        name: format!("session {id}"),
        folder: "~/code/acme".to_owned(),
        host: "Terminal".to_owned(),
        level: "Auto edit".to_owned(),
        yolo: false,
        status,
        offer,
    }
}

fn card_data(can_move_all: bool, rows: Vec<Row>) -> Card {
    Card { summary: format!("{} sessions \u{b7} 1 idle", rows.len()), can_move_all, rows }
}

fn raw(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
        events,
        focused: true,
        ..RawInput::default()
    }
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

fn ready_context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    ctx
}

fn card_frame(ctx: &egui::Context, data: &Card, events: Vec<Event>) -> Option<CardAction> {
    let mut action = None;
    let _ = ctx.run_ui(raw(events), |ui| action = card(ui, data, false));
    action
}

fn warmed(ctx: &egui::Context, data: &Card) {
    card_frame(ctx, data, vec![]);
    card_frame(ctx, data, vec![]);
}

fn row_y(index: usize) -> f32 {
    HEADER_H + index as f32 * ROW_H + ROW_H / 2.0
}

fn any_click(ctx: &egui::Context, data: &Card, y: f32) -> Vec<CardAction> {
    (880..=980)
        .step_by(10)
        .filter_map(|x| card_frame(ctx, data, click(pos2(x as f32, y))))
        .collect()
}

#[test]
fn a_click_on_the_button_of_a_row_presses_that_row() {
    let ctx = ready_context();
    let data = card_data(
        true,
        vec![row("a", Status::Idle, Offer::MoveHere), row("b", Status::Idle, Offer::MoveHere)],
    );
    warmed(&ctx, &data);
    let first = any_click(&ctx, &data, row_y(0));
    assert_ne!(first, Vec::new());
    assert!(first.iter().all(|a| *a == CardAction::Press(data.rows[0].key.clone())), "{first:?}");
    let second = any_click(&ctx, &data, row_y(1));
    assert_ne!(second, Vec::new());
    assert!(second.iter().all(|a| *a == CardAction::Press(data.rows[1].key.clone())), "{second:?}");
}

#[test]
fn a_blocked_or_busy_row_cannot_be_pressed() {
    let ctx = ready_context();
    let data = card_data(
        false,
        vec![
            row("a", Status::Quiet, Offer::Blocked("its session id could not be found".into())),
            row("b", Status::Idle, Offer::Moving),
        ],
    );
    warmed(&ctx, &data);
    assert_eq!(any_click(&ctx, &data, row_y(0)), []);
    assert_eq!(any_click(&ctx, &data, row_y(1)), []);
}

#[test]
fn move_all_idle_is_a_button_only_while_something_can_move() {
    let ctx = ready_context();
    let on = card_data(true, vec![row("a", Status::Idle, Offer::MoveHere)]);
    let off = card_data(false, vec![row("a", Status::Idle, Offer::MoveLive)]);
    let at = |x: f32| click(pos2(x, HEADER_H / 2.0));
    warmed(&ctx, &on);
    let hit: Vec<_> =
        (800..=980).step_by(10).filter_map(|x| card_frame(&ctx, &on, at(x as f32))).collect();
    assert_ne!(hit, Vec::new());
    assert!(hit.iter().all(|a| *a == CardAction::MoveAllIdle), "{hit:?}");
    warmed(&ctx, &off);
    let none: Vec<_> =
        (800..=980).step_by(10).filter_map(|x| card_frame(&ctx, &off, at(x as f32))).collect();
    assert_eq!(none, []);
}

#[test]
fn the_card_is_as_tall_as_its_header_and_rows() {
    let ctx = ready_context();
    let data = card_data(true, vec![row("a", Status::Idle, Offer::MoveHere); 3]);
    let mut height = 0.0;
    let _ = ctx.run_ui(raw(vec![]), |ui| {
        let before = ui.cursor().top();
        card(ui, &data, false);
        height = ui.cursor().top() - before;
    });
    let wanted = HEADER_H + 3.0 * ROW_H;
    assert!(height >= wanted && height < wanted + 8.0, "{height}");
}

fn view(blocked: Option<&'static str>) -> DialogView {
    DialogView {
        title: "Move retry-stage-gates to Tidewatch?".to_owned(),
        lines: ["one".to_owned(), "two".to_owned(), "three".to_owned()],
        items: vec![crate::takeover::DialogItem {
            name: "retry-stage-gates".to_owned(),
            folder: "~/code/acme".to_owned(),
            command: Some("claude --resume abc --permission-mode acceptEdits".to_owned()),
        }],
        notes: vec![],
        offer_worktree: true,
        worktree: false,
        blocked,
        confirm: "Move".to_owned(),
    }
}

fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn dialog_frame_once(
    ctx: &egui::Context,
    view: &DialogView,
    events: Vec<Event>,
) -> Option<DialogOutcome> {
    let mut outcome = None;
    let mut worktree = view.worktree;
    let _ = ctx.run_ui(raw(events), |_| outcome = dialog(ctx, view, &mut worktree));
    outcome
}

#[test]
fn enter_moves_and_escape_cancels() {
    let ctx = ready_context();
    let shown = view(None);
    dialog_frame_once(&ctx, &shown, vec![]);
    dialog_frame_once(&ctx, &shown, vec![]);
    assert!(matches!(
        dialog_frame_once(&ctx, &shown, vec![key(egui::Key::Enter)]),
        Some(DialogOutcome::Accept)
    ));
    assert!(matches!(
        dialog_frame_once(&ctx, &shown, vec![key(egui::Key::Escape)]),
        Some(DialogOutcome::Cancel)
    ));
    assert!(dialog_frame_once(&ctx, &shown, vec![]).is_none());
}

#[test]
fn a_blocked_dialog_does_not_move_on_enter_but_still_cancels() {
    let ctx = ready_context();
    let blocked = view(Some("YOLO only runs in a worktree."));
    dialog_frame_once(&ctx, &blocked, vec![]);
    dialog_frame_once(&ctx, &blocked, vec![]);
    assert!(dialog_frame_once(&ctx, &blocked, vec![key(egui::Key::Enter)]).is_none());
    assert!(matches!(
        dialog_frame_once(&ctx, &blocked, vec![key(egui::Key::Escape)]),
        Some(DialogOutcome::Cancel)
    ));
}

#[test]
fn statuses_have_their_own_ink() {
    let inks = [Status::Idle, Status::Working, Status::Quiet].map(status_ink);
    assert_ne!(inks[0], inks[1]);
    assert_ne!(inks[1], inks[2]);
    assert_eq!(status_ink(Status::Quiet), status_ink(Status::NotRunning));
}
