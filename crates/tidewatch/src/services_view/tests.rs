use egui::{Context, Event, PointerButton, Pos2, RawInput};

use super::*;

fn row(name: &str, word: Word, line: &str) -> Row {
    Row {
        name: name.to_owned(),
        command: format!(
            "run {name} --with a-very-long-command-line-that-has-to-be-cut-by-an-ellipsis --port 3309"
        ),
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
        Self { ctx: crate::fonts::test_context(), memory, rows, action: None }
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
                    if let Some(action) = pill(ui, &self.rows, "acme-workers", &mut self.memory) {
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

    fn hover(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![]);
    }

    fn row(&self, index: usize) -> Rect {
        self.placed().rows[index]
    }

    fn names(&self) -> Vec<&'static str> {
        self.placed().buttons.iter().map(|(name, _)| *name).collect()
    }

    fn buttons_named(&self, name: &str) -> Vec<Rect> {
        self.placed()
            .buttons
            .iter()
            .filter(|(label, _)| *label == name)
            .map(|(_, rect)| *rect)
            .collect()
    }

    fn press(&mut self, key: Key) {
        self.frame(self::key(key));
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
fn the_pill_draws_at_its_width_and_the_list_draws_the_selected_rows_actions_and_the_crashed_ones_start()
 {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let placed = screen.placed();
    let pill = placed.pill.expect("the pill was drawn");
    assert_eq!(pill.height(), PILL_H);
    assert!(pill.width() > 120.0 && pill.width() < 220.0, "{pill:?}");
    let names: Vec<&str> = placed.buttons.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Restart", "...", "Start", "Add service"]);
    for (name, rect) in &placed.buttons {
        if *name != "Add service" {
            assert_eq!(rect.height(), Size::Compact.height(), "{name}");
        }
    }
}

#[test]
fn rows_are_48_tall_inside_the_list_and_never_overlap() {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let placed = screen.placed();
    let list = placed.list.expect("the list was drawn");
    assert_eq!(placed.rows.len(), 3);
    for rect in &placed.rows {
        assert_eq!(rect.height(), 48.0);
        assert!(list.contains_rect(*rect), "{rect:?} outside {list:?}");
    }
    for pair in placed.rows.windows(2) {
        assert!(pair[0].bottom() <= pair[1].top(), "{pair:?}");
    }
}

#[test]
fn restart_and_the_menu_show_on_the_hovered_row_only() {
    let mut screen = Screen::new(Memory { popover: Popover::Open, ..Memory::default() }, rows());
    screen.settle();
    assert_eq!(screen.names(), ["Start", "Add service"], "nothing hovered, nothing selected");
    let api = screen.row(1);
    screen.hover(api.left_center() + vec2(60.0, 0.0));
    assert_eq!(screen.names(), ["Restart", "...", "Start", "Add service"]);
    let restart = screen.button("Restart");
    assert!(api.contains_rect(restart), "{restart:?} is not on the hovered row {api:?}");
    assert!(restart.right() < screen.button("...").left());
    let db = screen.row(0);
    screen.hover(db.left_center() + vec2(60.0, 0.0));
    assert!(db.contains_rect(screen.button("Restart")));
    assert_eq!(screen.buttons_named("Restart").len(), 1);
}

#[test]
fn a_crashed_row_keeps_its_start_and_shows_the_menu_only_on_hover() {
    let mut screen = Screen::new(Memory { popover: Popover::Open, ..Memory::default() }, rows());
    screen.settle();
    let tunnel = screen.row(2);
    assert!(tunnel.contains_rect(screen.button("Start")));
    assert_eq!(screen.buttons_named("...").len(), 0);
    screen.hover(tunnel.left_center() + vec2(60.0, 0.0));
    assert_eq!(screen.names(), ["Start", "...", "Add service"]);
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
fn logs_and_stop_live_in_the_menu_and_a_stopped_service_offers_start_on_its_row() {
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let more = screen.button("...").center();
    screen.click(more);
    assert_eq!(screen.names().iter().filter(|name| **name == "Logs").count(), 1);
    let logs = screen.button("Logs").center();
    screen.click(logs);
    assert_eq!(screen.action.take(), Some(Action::Logs("db".to_owned())));
    let mut screen = Screen::new(open_list(), rows());
    screen.settle();
    let more = screen.button("...").center();
    screen.click(more);
    let stop = screen.button("Stop").center();
    screen.click(stop);
    assert_eq!(screen.action, Some(Action::Run(Command::Stop, "db".to_owned())));
    let mut stopped = rows();
    stopped[0] = row("db", Word::Stopped, "stopped");
    let mut screen = Screen::new(open_list(), stopped);
    screen.settle();
    assert_eq!(screen.buttons_named("Start").len(), 2, "the stopped row and the crashed one");
    let start = screen.buttons_named("Start")[0].center();
    screen.click(start);
    assert_eq!(screen.action, Some(Action::Run(Command::Start, "db".to_owned())));
}

#[test]
fn a_stopped_service_has_no_stop_in_its_menu_and_a_running_one_has_it() {
    for (word, stop) in [(Word::Stopped, false), (Word::Crashed, false), (Word::Running, true)] {
        let mut shown = rows();
        shown[0] = row("db", word, "x");
        let mut screen = Screen::new(open_list(), shown);
        screen.settle();
        let more = screen.button("...").center();
        screen.click(more);
        let names = screen.names();
        assert_eq!(names.contains(&"Stop"), stop, "{word:?} {names:?}");
        assert!(names.contains(&"Logs") && names.contains(&"Edit") && names.contains(&"Remove"));
    }
}

#[test]
fn a_service_that_gave_up_offers_start_and_a_restarting_one_restart() {
    for (word, first) in [
        (Word::Crashed, "Start"),
        (Word::Stopped, "Start"),
        (Word::Restarting, "Restart"),
        (Word::Starting, "Restart"),
    ] {
        let mut shown = rows();
        shown[0] = row("db", word, "x");
        let mut screen = Screen::new(open_list(), shown);
        screen.settle();
        assert_eq!(screen.names(), [first, "...", "Start", "Add service"], "{word:?}");
    }
}

#[test]
fn a_click_on_a_row_selects_it_and_a_second_click_folds_it() {
    let mut screen = Screen::new(Memory { popover: Popover::Open, ..Memory::default() }, rows());
    screen.settle();
    assert_eq!(screen.names(), ["Start", "Add service"]);
    let first_row = screen.row(0).left_center() + vec2(60.0, 0.0);
    screen.click(first_row);
    assert_eq!(screen.memory.selected.as_deref(), Some("db"));
    screen.click(first_row);
    assert_eq!(screen.memory.selected, None);
}

#[test]
fn the_menu_opens_beside_the_list_level_with_its_row_and_hides_no_row() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    let placed = screen.placed();
    let (list, menu) = (placed.list.expect("list"), screen.memory.menu_rect.expect("menu"));
    assert!(menu.right() <= list.left(), "menu {menu:?} overlaps the list {list:?}");
    assert!((menu.top() - placed.rows[0].top()).abs() < 3.0, "{menu:?} vs {:?}", placed.rows[0]);
    for row in &placed.rows {
        assert!(!menu.intersects(*row), "the menu hides {row:?}");
    }
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

fn form(name: &str, command: &str) -> Form {
    Form { name: name.into(), command: command.into(), autostart: true, ..Form::default() }
}

#[test]
fn the_form_becomes_a_service_or_lists_every_problem() {
    let existing = rows();
    let cases: [(Form, Result<Service, usize>); 4] = [
        (form(" web ", " npm run dev "), Ok(Service::new("web", "npm run dev", true))),
        (form("", ""), Err(2)),
        (form("DB", "x"), Err(1)),
        (form("web", ""), Err(1)),
    ];
    for (given, want) in cases {
        let got = form_service(&given, &existing).map_err(|found| found.len());
        assert_eq!(got, want, "{given:?}");
    }
}

#[test]
fn editing_may_keep_a_services_own_name() {
    let existing = rows();
    let mut keep = form("db", "x");
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
    assert_eq!(edit.name, "db");
    assert!(edit.autostart);
    let mut memory = Memory::default();
    assert!(!memory.dialog_open());
    memory.open_add();
    assert!(memory.dialog_open());
    assert_eq!(memory.dialog, Some(Dialog::Form(Form::add())));
    assert!(!Form::add().autostart, "a new service waits for Start unless its switch is turned on");
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
        Some(Action::Save { editing: None, service: Service::new("db", "sleep 1", false) })
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

fn opened_by_click() -> Screen {
    let memory = Memory { selected: Some("db".to_owned()), ..Memory::default() };
    let mut screen = Screen::new(memory, rows());
    screen.settle();
    assert_eq!(screen.names(), Vec::<&str>::new());
    let pill = screen.placed().pill.unwrap().center();
    screen.click(pill);
    screen.settle();
    assert!(screen.names().contains(&"Add service"), "the pill opened the list");
    screen
}

fn outside() -> Pos2 {
    pos2(20.0, 500.0)
}

#[test]
fn the_more_button_opens_its_menu_over_the_list_and_the_list_stays_open() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    assert_eq!(screen.memory.menu.as_deref(), Some("db"));
    let names = screen.names();
    assert!(names.contains(&"Edit") && names.contains(&"Remove"), "{names:?}");
    assert!(names.contains(&"Add service"), "the list is still open: {names:?}");
    assert_eq!(screen.memory.selected.as_deref(), Some("db"));
    assert_eq!(screen.memory.dialog, None);
}

#[test]
fn edit_opens_the_form_for_that_service_and_the_list_stays_open() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    let edit = screen.button("Edit").center();
    screen.click(edit);
    assert_eq!(screen.memory.dialog, Some(Dialog::Form(Form::edit(&rows()[0]))));
    assert_eq!(screen.memory.menu, None);
    let names = screen.names();
    assert!(names.contains(&"Add service") && !names.contains(&"Edit"), "{names:?}");
}

#[test]
fn remove_asks_first_and_the_list_stays_open() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    let remove = screen.button("Remove").center();
    screen.click(remove);
    assert_eq!(screen.memory.dialog, Some(Dialog::Remove("db".to_owned())));
    assert_eq!(screen.action, None, "nothing is removed before the dialog is confirmed");
    assert!(screen.names().contains(&"Add service"));
}

#[test]
fn a_click_outside_the_list_and_its_menu_closes_both() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    assert!(screen.names().contains(&"Edit"));
    screen.click(outside());
    screen.settle();
    assert_eq!(screen.names(), Vec::<&str>::new());
    assert_eq!(screen.memory.menu, None);
}

#[test]
fn a_click_outside_a_list_with_no_menu_still_closes_it() {
    let mut screen = opened_by_click();
    assert!(screen.names().contains(&"Add service"));
    screen.click(outside());
    screen.settle();
    assert_eq!(screen.names(), Vec::<&str>::new());
}

#[test]
fn a_click_inside_the_list_off_the_menu_closes_only_the_menu() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    let second_card = screen.row(1).left_center() + vec2(60.0, 0.0);
    screen.click(second_card);
    assert_eq!(screen.memory.menu, None);
    assert_eq!(screen.memory.selected.as_deref(), Some("api"));
    assert!(screen.names().contains(&"...") && !screen.names().contains(&"Edit"));
}

#[test]
fn the_more_button_toggles_its_menu_and_another_cards_button_moves_it() {
    let mut screen = opened_by_click();
    let first = screen.button("...").center();
    screen.click(first);
    assert_eq!(screen.memory.menu.as_deref(), Some("db"));
    let api = screen.row(1);
    screen.hover(api.left_center() + vec2(60.0, 0.0));
    let second = screen.buttons_named("...")[1].center();
    screen.click(second);
    assert_eq!(screen.memory.menu.as_deref(), Some("api"));
    let again = screen.buttons_named("...")[1].center();
    screen.click(again);
    assert_eq!(screen.memory.menu, None);
    assert!(screen.names().contains(&"Add service"), "the list stays open");
}

#[test]
fn escape_closes_the_menu_first_and_the_list_next() {
    let mut screen = opened_by_click();
    let more = screen.button("...").center();
    screen.click(more);
    screen.press(Key::Escape);
    assert_eq!(screen.memory.menu, None);
    assert!(screen.names().contains(&"Add service"), "{:?}", screen.names());
    screen.press(Key::Escape);
    screen.settle();
    assert_eq!(screen.names(), Vec::<&str>::new());
}

#[test]
fn the_form_footer_names_only_keys_and_actions_that_exist() {
    let ctx = crate::fonts::test_context();
    let raw = || RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
        focused: true,
        ..Default::default()
    };
    let mut out = None;
    for _ in 0..2 {
        out = Some(ctx.run_ui(raw(), |ui| {
            let _ = form_dialog(ui.ctx(), Form::add(), &[]);
        }));
    }
    let texts: Vec<String> =
        crate::screen_kit::painted_texts(&out.unwrap()).into_iter().map(|(text, _)| text).collect();
    assert!(texts.contains(&"saves".to_owned()), "{texts:?}");
    assert!(texts.contains(&"cancels".to_owned()), "{texts:?}");
    assert_eq!(texts.iter().filter(|t| t.contains("Start runs") || t.contains(" - ")).count(), 0);
}
