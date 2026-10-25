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
    Card {
        summary: format!("{} sessions \u{b7} 1 idle", rows.len()),
        can_move_all,
        rows,
        ended: Vec::new(),
        ended_open: false,
    }
}

fn with_ended(mut data: Card, ended: usize, open: bool) -> Card {
    data.ended =
        (0..ended).map(|n| row(&format!("e{n}"), Status::NotRunning, Offer::ResumeHere)).collect();
    data.ended_open = open;
    data
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

fn card_height(ctx: &egui::Context, data: &Card) -> f32 {
    let mut height = 0.0;
    let _ = ctx.run_ui(raw(vec![]), |ui| {
        let before = ui.cursor().top();
        card(ui, data, false);
        height = ui.cursor().top() - before;
    });
    height
}

#[test]
fn ended_sessions_add_only_a_bar_until_it_is_opened() {
    let ctx = ready_context();
    let running = || card_data(true, vec![row("a", Status::Idle, Offer::MoveHere)]);
    let shut = with_ended(running(), 2, false);
    let open = with_ended(running(), 2, true);
    let bare = HEADER_H + ROW_H;
    let near = |got: f32, want: f32| got >= want && got < want + 8.0;
    assert!(near(card_height(&ctx, &shut), bare + DISCLOSURE_H));
    assert!(near(card_height(&ctx, &open), bare + DISCLOSURE_H + 2.0 * ROW_H));
    assert!(near(card_height(&ctx, &running()), bare));
}

#[test]
fn the_recent_bar_toggles_and_its_rows_resume_when_open() {
    let ctx = ready_context();
    let data = with_ended(card_data(true, vec![row("a", Status::Idle, Offer::MoveHere)]), 1, true);
    warmed(&ctx, &data);
    let bar_y = HEADER_H + ROW_H + DISCLOSURE_H / 2.0;
    let hit: Vec<_> = (20..=200)
        .step_by(20)
        .filter_map(|x| card_frame(&ctx, &data, click(pos2(x as f32, bar_y))))
        .collect();
    assert_ne!(hit, Vec::new());
    assert!(hit.iter().all(|a| *a == CardAction::ToggleRecent), "{hit:?}");
    let ended_y = HEADER_H + ROW_H + DISCLOSURE_H + ROW_H / 2.0;
    let pressed = any_click(&ctx, &data, ended_y);
    assert_ne!(pressed, Vec::new());
    assert!(pressed.iter().all(|a| *a == CardAction::Press(data.ended[0].key.clone())));
}

struct Screen {
    card: Option<Card>,
    takeover: crate::takeover::Takeover,
}

impl Screen {
    fn new(card: Option<Card>) -> Self {
        Self { card, takeover: crate::takeover::Takeover::default() }
    }

    fn frame(&mut self, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        use crate::overview::{Counts, Overview};
        use crate::overview_view::{Sections, State, show};
        let data = Overview { needs: vec![], places: vec![], counts: Counts::default() };
        let sections = Sections::default();
        let mut state = State::default();
        let (open, reveal) = (self.takeover.line_open(), self.takeover.take_reveal());
        let mut clicked = None;
        let output = ctx.run_ui(raw(events), |ui| {
            let area = ui.max_rect();
            show(ui, area, &data, &mut state, &sections, |ui| {
                if let Some(card) = &self.card {
                    clicked = line(ui, card, open, reveal);
                }
            });
        });
        if clicked == Some(CardAction::ToggleLine) {
            self.takeover.toggle_line();
        }
        let mut found = Vec::new();
        for clipped in &output.shapes {
            collect_texts(&clipped.shape, &mut found);
        }
        found
    }

    fn click_on(&mut self, ctx: &egui::Context, label: &str) {
        let found = self.frame(ctx, vec![]);
        let at = found.iter().find(|(text, _)| text == label).expect(label).1.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(ctx, vec![Event::PointerMoved(at)]);
        self.frame(ctx, vec![button(true)]);
        self.frame(ctx, vec![button(false)]);
    }
}

fn collect_texts(shape: &egui::Shape, found: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => found
            .push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_texts(shape, found)),
        _ => {}
    }
}

fn top_of(found: &[(String, Rect)], text: &str) -> f32 {
    found.iter().find(|(drawn, _)| drawn == text).expect(text).1.top()
}

fn has(found: &[(String, Rect)], text: &str) -> bool {
    found.iter().any(|(drawn, _)| drawn == text)
}

#[test]
fn the_line_is_hidden_with_no_session_elsewhere() {
    let ctx = ready_context();
    let mut screen = Screen::new(None);
    let found = screen.frame(&ctx, vec![]);
    assert!(!found.iter().any(|(text, _)| text.contains("other terminal")));
    assert!(!has(&found, "Running elsewhere"));
}

#[test]
fn the_card_counts_the_live_sessions_and_offers_move_here() {
    let ctx = ready_context();
    let rows = (0..3).map(|n| row(&n.to_string(), Status::Idle, Offer::MoveHere)).collect();
    let mut screen = Screen::new(Some(with_ended(card_data(true, rows), 2, false)));
    let found = screen.frame(&ctx, vec![]);
    assert!(top_of(&found, "3 sessions in other terminals") > top_of(&found, "Running elsewhere"));
    assert!(has(&found, "Move here"));
    assert!(!has(&found, "Move all idle here"));
    let one = Screen::new(Some(card_data(true, vec![row("a", Status::Idle, Offer::MoveHere)])))
        .frame(&ctx, vec![]);
    assert!(has(&one, "1 session in another terminal"));
}

#[test]
fn a_click_on_the_line_opens_the_list_below_it_and_a_second_folds_it() {
    let ctx = ready_context();
    let rows = (0..2).map(|n| row(&n.to_string(), Status::Idle, Offer::MoveHere)).collect();
    let mut screen = Screen::new(Some(card_data(true, rows)));
    screen.click_on(&ctx, "Move here");
    assert!(screen.takeover.line_open());
    let open = screen.frame(&ctx, vec![]);
    assert!(has(&open, "Move all idle here"));
    assert_eq!(open.iter().filter(|(text, _)| text == "Running elsewhere").count(), 2);
    screen.click_on(&ctx, "Hide");
    assert!(!screen.takeover.line_open());
    let folded = screen.frame(&ctx, vec![]);
    assert!(!has(&folded, "Move all idle here"));
}

fn view(yolo_note: Option<&str>) -> DialogView {
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
        yolo_note: yolo_note.map(str::to_owned),
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
fn a_dialog_with_a_yolo_note_still_moves_on_enter() {
    let ctx = ready_context();
    let noted = view(Some("This folder is not a git repo, so it keeps running YOLO here."));
    dialog_frame_once(&ctx, &noted, vec![]);
    dialog_frame_once(&ctx, &noted, vec![]);
    assert!(matches!(
        dialog_frame_once(&ctx, &noted, vec![key(egui::Key::Enter)]),
        Some(DialogOutcome::Accept)
    ));
}

#[test]
fn statuses_have_their_own_ink() {
    let inks = [Status::Idle, Status::Working, Status::Quiet].map(status_ink);
    assert_ne!(inks[0], inks[1]);
    assert_ne!(inks[1], inks[2]);
    assert_eq!(status_ink(Status::Quiet), status_ink(Status::NotRunning));
}
