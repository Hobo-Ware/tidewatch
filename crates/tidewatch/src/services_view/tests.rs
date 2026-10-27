use egui::{Context, Event, PointerButton, Pos2, RawInput};

use super::*;

fn row(name: &str, word: Word, line: &str) -> Row {
    Row {
        name: name.to_owned(),
        command: format!(
            "run {name} --with a-very-long-command-line-that-has-to-be-cut-by-an-ellipsis --port 3309"
        ),
        port: Some(3309),
        autostart: true,
        word,
        line: line.to_owned(),
    }
}

fn rows() -> Vec<Row> {
    vec![
        row("db", Word::Running, "up 2h 14m \u{b7} :3309"),
        row("api", Word::Starting, "starting"),
        row("tunnel", Word::Crashed, "crashed, exit 1"),
    ]
}

struct Screen {
    ctx: Context,
    memory: Memory,
    rows: Vec<Row>,
    action: Option<Action>,
}

impl Screen {
    fn new(memory: Memory, rows: Vec<Row>) -> Self {
        Self { ctx: Context::default(), memory, rows, action: None }
    }

    fn frame(&mut self, events: Vec<Event>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let _ = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(560.0);
                    if let Some(action) = pill(ui, &self.rows, &mut self.memory) {
                        self.action = Some(action);
                    }
                });
            });
        });
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(vec![]);
        }
    }

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp::<Placed>(placed_id())).unwrap_or_default()
    }

    fn click(&mut self, at: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(vec![]);
    }

    fn button(&self, name: &str) -> Rect {
        self.placed()
            .buttons
            .iter()
            .find(|(label, _)| *label == name)
            .unwrap_or_else(|| panic!("no {name} button in {:?}", self.placed()))
            .1
    }
}

fn open_list() -> Memory {
    Memory { popover: Popover::Open, selected: Some("db".to_owned()), ..Memory::default() }
}

#[test]
fn the_pill_draws_at_its_width_and_the_list_draws_the_selected_cards_buttons() {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let placed = screen.placed();
    let pill = placed.pill.expect("the pill was drawn");
    assert_eq!(pill.height(), PILL_H);
    assert!(pill.width() > 120.0 && pill.width() < 220.0, "{pill:?}");
    let names: Vec<&str> = placed.buttons.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Logs", "Restart", "Stop"]);
    for (name, rect) in &placed.buttons {
        assert_eq!(rect.height(), Size::Compact.height(), "{name}");
    }
}

#[test]
fn a_click_on_restart_emits_the_restart_action_for_that_service() {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let at = screen.button("Restart").center();
    screen.click(at);
    assert_eq!(screen.action, Some(Action::Run(Command::Restart, "db".to_owned())));
}

#[test]
fn logs_and_stop_emit_their_actions_and_a_stopped_service_offers_start() {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let logs = screen.button("Logs").center();
    screen.click(logs);
    assert_eq!(screen.action.take(), Some(Action::Logs("db".to_owned())));
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let stop = screen.button("Stop").center();
    screen.click(stop);
    assert_eq!(screen.action, Some(Action::Run(Command::Stop, "db".to_owned())));
    let mut stopped = rows();
    stopped[0] = row("db", Word::Stopped, "stopped, exit 1");
    let mut screen = Screen::new(open_list(), stopped);
    screen.settle();
    let start = screen.button("Start").center();
    screen.click(start);
    assert_eq!(screen.action, Some(Action::Run(Command::Start, "db".to_owned())));
}

#[test]
fn a_click_on_a_card_selects_it_and_a_second_click_folds_it() {
    let mut screen = Screen::new(Memory { popover: Popover::Open, ..Memory::default() }, rows());
    screen.settle();
    assert_eq!(screen.placed().buttons, []);
    let list_top = screen.placed().pill.unwrap().bottom() + 6.0 + f32::from(LIST_MARGIN);
    let first_card = pos2(screen.placed().pill.unwrap().right() - 200.0, list_top + 30.0);
    screen.click(first_card);
    assert_eq!(screen.memory.selected.as_deref(), Some("db"));
    screen.click(first_card);
    assert_eq!(screen.memory.selected, None);
}

#[test]
fn no_services_draw_no_pill() {
    let mut screen = Screen::new(Memory::default(), Vec::new());
    screen.settle();
    assert_eq!(screen.placed(), Placed::default());
    assert_eq!(label(0), "0 services");
}

#[test]
fn the_label_counts_services() {
    assert_eq!(label(1), "1 service");
    assert_eq!(label(3), "3 services");
}

#[test]
fn the_pill_outline_is_teal_when_all_run_orange_when_one_gave_up_and_quiet_otherwise() {
    let cases: [(&str, Vec<Word>, Color32); 5] = [
        ("all run", vec![Word::Running, Word::Running], brand::TEAL),
        ("one crashed", vec![Word::Running, Word::Crashed], brand::ORANGE),
        ("crashed beats stopped", vec![Word::Stopped, Word::Crashed], brand::ORANGE),
        ("one starting", vec![Word::Running, Word::Starting], brand::FIELD_LINE),
        ("one stopped", vec![Word::Running, Word::Stopped], brand::FIELD_LINE),
    ];
    for (what, words, want) in cases {
        let rows: Vec<Row> = words.into_iter().map(|word| row("x", word, "")).collect();
        assert_eq!(outline(&rows), want, "{what}");
    }
}

#[test]
fn the_dots_are_green_teal_orange_and_grey() {
    let cases = [
        (Word::Running, brand::GREEN),
        (Word::Starting, brand::TEAL),
        (Word::Restarting, brand::TEAL),
        (Word::Crashed, brand::ORANGE),
        (Word::Stopped, brand::FAINT),
    ];
    for (word, want) in cases {
        assert_eq!(dot_color(word), want, "{word:?}");
    }
}

fn form(name: &str, command: &str, port: &str) -> Form {
    Form {
        name: name.into(),
        command: command.into(),
        port: port.into(),
        autostart: true,
        ..Form::default()
    }
}

#[test]
fn the_form_becomes_a_service_or_lists_every_problem() {
    let existing = rows();
    let cases: [(Form, Result<Service, usize>); 7] = [
        (
            form(" web ", " npm run dev ", " 3000 "),
            Ok(Service::new("web", "npm run dev", Some(3000), true)),
        ),
        (form("web", "npm run dev", ""), Ok(Service::new("web", "npm run dev", None, true))),
        (form("", "", ""), Err(2)),
        (form("DB", "x", ""), Err(1)),
        (form("web", "x", "abc"), Err(1)),
        (form("web", "x", "70000"), Err(1)),
        (form("web", "x", "0"), Err(1)),
    ];
    for (given, want) in cases {
        let got = form_service(&given, &existing).map_err(|found| found.len());
        assert_eq!(got, want, "{given:?}");
    }
}

#[test]
fn editing_may_keep_a_services_own_name() {
    let existing = rows();
    let mut keep = form("db", "x", "");
    keep.editing = Some("db".to_owned());
    assert!(form_service(&keep, &existing).is_ok());
    keep.name = "api".to_owned();
    assert!(form_service(&keep, &existing).is_err());
}

#[test]
fn the_card_menu_opens_the_edit_and_the_remove_dialogs_with_the_services_own_values() {
    let existing = rows();
    let edit = Form::edit(&existing[0]);
    assert_eq!(edit.editing.as_deref(), Some("db"));
    assert_eq!((edit.name.as_str(), edit.port.as_str()), ("db", "3309"));
    assert!(edit.autostart);
    let mut memory = Memory::default();
    assert!(!memory.dialog_open());
    memory.open_add();
    assert!(memory.dialog_open());
    assert_eq!(memory.dialog, Some(Dialog::Form(Form::add())));
    assert!(Form::add().autostart, "a new service starts with the repo unless turned off");
}

fn dialog_frame_of(
    ctx: &Context,
    memory: &mut Memory,
    rows: &[Row],
    events: Vec<Event>,
) -> Option<Action> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
        events,
        focused: true,
        ..Default::default()
    };
    let mut action = None;
    let _ = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().show(ui, |_| {});
        action = dialog(ui.ctx(), memory, rows);
    });
    action
}

fn key(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn filled(memory: &mut Memory, name: &str, command: &str) {
    let Some(Dialog::Form(form)) = &mut memory.dialog else { panic!("a form is open") };
    form.name = name.to_owned();
    form.command = command.to_owned();
    form.focus = true;
}

#[test]
fn enter_in_the_add_dialog_saves_a_good_form_and_keeps_a_bad_one_open_with_its_problems() {
    let ctx = crate::fonts::test_context();
    let mut memory = Memory::default();
    memory.open_add();
    for _ in 0..3 {
        assert_eq!(dialog_frame_of(&ctx, &mut memory, &[], vec![]), None);
    }
    assert!(memory.dialog_open());
    assert_eq!(dialog_frame_of(&ctx, &mut memory, &[], key(Key::Enter)), None);
    let Some(Dialog::Form(form)) = &memory.dialog else { panic!("still open") };
    assert!(form.tried, "an empty form is refused and says why");

    filled(&mut memory, "db", "sleep 1");
    for _ in 0..2 {
        dialog_frame_of(&ctx, &mut memory, &[], vec![]);
    }
    let saved = dialog_frame_of(&ctx, &mut memory, &[], key(Key::Enter));
    assert_eq!(
        saved,
        Some(Action::Save { editing: None, service: Service::new("db", "sleep 1", None, true) })
    );
    assert!(!memory.dialog_open());
}

#[test]
fn the_remove_dialog_asks_once_and_cancel_forgets_it() {
    let ctx = crate::fonts::test_context();
    let mut memory = Memory { dialog: Some(Dialog::Remove("tunnel".into())), ..Memory::default() };
    for _ in 0..3 {
        assert_eq!(dialog_frame_of(&ctx, &mut memory, &rows(), vec![]), None);
    }
    assert_eq!(memory.dialog, Some(Dialog::Remove("tunnel".into())));
    let esc = Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    dialog_frame_of(&ctx, &mut memory, &rows(), vec![esc]);
    dialog_frame_of(&ctx, &mut memory, &rows(), vec![]);
    assert_eq!(memory.dialog, None);
}

#[test]
fn the_logs_view_belongs_to_the_tab_it_was_opened_from() {
    let mut memory = Memory::default();
    memory.show_logs("db", 7);
    assert_eq!(memory.logs_over(7), Some("db"));
    assert_eq!(memory.logs_over(8), None);
    assert_eq!(memory.logs_over(7), None, "another tab closed it for good");
    memory.logs = Some(Logs { name: "db".into(), tab: None });
    assert_eq!(memory.logs_over(3), Some("db"), "a dev switch adopts the tab in front");
    assert_eq!(memory.logs_over(4), None);
}
