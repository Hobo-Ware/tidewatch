use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, RawInput, pos2};
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::new_tab::Group;

fn area() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 800.0))
}

struct Harness {
    ctx: egui::Context,
    state: NewTab,
    catalog: Catalog,
    facts: Option<RepoFacts>,
    keys_free: bool,
}

fn key_with(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

fn key(k: Key) -> Event {
    key_with(k, Modifiers::NONE)
}

impl Harness {
    fn new(group: Group) -> Self {
        let mut catalog = Catalog::new(OsString::from("/usr/bin"));
        catalog.set_profiles(Profiles::seeded(&["claude".to_owned(), "codex".to_owned()]));
        catalog.set_workflows(Workflows::defaults());
        let state =
            NewTab::open(group, "web".into(), PathBuf::from("/work/web"), Vec::new(), None, 3);
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut harness = Self { ctx, state, catalog, facts: None, keys_free: true };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    /// A form whose message box has given the keyboard up with one Esc, as a user does before
    /// pressing a letter that means something.
    fn unfocused(group: Group) -> Self {
        let mut harness = Self::new(group);
        assert_eq!(harness.press(Key::Escape), None);
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Out> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut out = None;
        let _ = self.ctx.run_ui(raw, |ui| {
            out = show(
                ui,
                area(),
                &mut self.state,
                &self.catalog,
                self.facts.as_ref(),
                self.keys_free,
            );
        });
        out
    }

    fn press(&mut self, k: Key) -> Option<Out> {
        self.frame(vec![key(k)])
    }

    fn click(&mut self, at: egui::Pos2) -> Option<Out> {
        self.frame(vec![Event::PointerMoved(at)]);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn typing(&self) -> bool {
        self.ctx.text_edit_focused()
    }
}

fn started(out: Option<Out>) -> Request {
    match out {
        Some(Out::Start(request)) => request,
        Some(Out::Close) => panic!("closed"),
        None => panic!("nothing started"),
    }
}

fn card_center(index: usize, count: usize) -> egui::Pos2 {
    let origin = pos2(SIDE_PAD, TOP_PAD + HEADER_H + GAP + SECTION_ROW_H + 8.0);
    let width = (area().width() - 2.0 * SIDE_PAD).min(COLUMN_MAX_W);
    new_tab_cards::grid(origin, width, 2, count, new_tab_cards::AGENT_H)[index].center()
}

fn start_button() -> egui::Pos2 {
    pos2(
        SIDE_PAD + COLUMN_MAX_W.min(area().width() - 2.0 * SIDE_PAD) - 30.0,
        area().bottom() - FOOTER_PAD - FOOTER_H / 2.0,
    )
}

#[test]
fn the_message_box_has_the_keyboard_when_the_form_opens() {
    let harness = Harness::new(Group::Repo);
    assert!(harness.typing());
}

#[test]
fn keys_are_never_taken_from_a_focused_text_field() {
    let mut harness = Harness::new(Group::Repo);
    for letter in [Key::A, Key::W, Key::S, Key::Num2, Key::ArrowLeft, Key::ArrowRight] {
        assert_eq!(harness.frame(vec![key(letter)]), None);
    }
    harness.frame(vec![Event::Text("w".into()), key(Key::W)]);
    harness.frame(vec![Event::Text("2".into()), key(Key::Num2)]);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert_eq!(harness.state.text(), "w2");
    assert_eq!(harness.state.selected(), 0);
    assert_eq!(harness.state.level(), Level::Edits);
}

#[test]
fn the_first_esc_gives_the_keyboard_up_and_the_second_closes_the_draft() {
    let mut harness = Harness::new(Group::Repo);
    assert_eq!(harness.press(Key::Escape), None);
    assert!(!harness.typing());
    assert!(matches!(harness.press(Key::Escape), Some(Out::Close)));
}

#[test]
fn a_w_and_s_pick_the_kind_or_open_a_plain_shell() {
    let mut harness = Harness::unfocused(Group::Repo);
    assert_eq!(harness.press(Key::W), None);
    assert_eq!(harness.state.kind(), Kind::Workflow);
    assert_eq!(harness.state.title(), "New workflow");
    assert_eq!(harness.press(Key::A), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert_eq!(started(harness.press(Key::S)), Request::Shell);
}

#[test]
fn in_other_w_does_nothing_and_a_and_s_still_work() {
    let mut harness = Harness::unfocused(Group::Other);
    assert_eq!(harness.press(Key::W), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert_eq!(started(harness.press(Key::S)), Request::Shell);
}

#[test]
fn number_keys_pick_the_nth_card_and_take_its_level() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::Num2);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("reviewer"));
    assert_eq!(harness.state.level(), Level::Plan);
    harness.press(Key::Num1);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("builder"));
    harness.press(Key::Num9);
    assert_eq!(harness.state.selected(), 0, "no ninth card");
    harness.press(Key::W);
    harness.press(Key::Num3);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("pr"));
}

#[test]
fn left_and_right_change_the_level_in_an_agent_form_only() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::ArrowRight);
    assert_eq!(harness.state.level(), Level::Auto);
    harness.press(Key::ArrowLeft);
    harness.press(Key::ArrowLeft);
    assert_eq!(harness.state.level(), Level::Ask);
    harness.press(Key::W);
    harness.press(Key::ArrowRight);
    harness.press(Key::A);
    assert_eq!(harness.state.level(), Level::Ask);
}

#[test]
fn clicking_a_card_picks_it() {
    let mut harness = Harness::new(Group::Repo);
    let count = harness.state.visible(&harness.catalog).len();
    assert_eq!(harness.click(card_center(3, count)), None);
    assert_eq!(harness.state.selected(), 3);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("codex-reviewer"));
}

#[test]
fn cmd_enter_starts_from_the_message_box_with_the_message_and_a_name_from_it() {
    let mut harness = Harness::new(Group::Repo);
    harness.type_text("Add retry to the stage gates");
    let out = harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]);
    let Request::Agent(request) = started(out) else { panic!("agent") };
    assert_eq!(request.task, "Add retry to the stage gates");
    assert_eq!(request.placement.name.as_deref(), Some("add-retry-stage-gates"));
    assert_eq!(request.profile_id, "builder");
}

#[test]
fn the_start_button_starts_too_and_a_blocked_one_does_nothing() {
    let mut harness = Harness::new(Group::Repo);
    assert!(matches!(started(harness.click(start_button())), Request::Agent(_)));
    harness.facts = Some(RepoFacts {
        checked_out: Some("main".into()),
        branches: vec!["main".into()],
        taken: HashSet::new(),
    });
    harness.state.type_name("has space");
    assert_eq!(harness.click(start_button()), None);
}

#[test]
fn cmd_enter_on_a_blocked_form_says_why() {
    let mut harness = Harness::new(Group::Repo);
    harness.state.choose(Kind::Workflow);
    assert_eq!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]), None);
    assert_eq!(harness.state.error.as_deref(), Some("Say what the run should deliver."));
    harness.state.set_text("add saved searches");
    let Request::Workflow(request) =
        started(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]))
    else {
        panic!("workflow")
    };
    assert_eq!(request.task, "add saved searches");
}

#[test]
fn a_taken_name_blocks_cmd_enter() {
    let mut harness = Harness::new(Group::Repo);
    harness.facts = Some(RepoFacts {
        checked_out: None,
        branches: Vec::new(),
        taken: ["dup".to_owned()].into_iter().collect(),
    });
    harness.state.type_name("dup");
    assert_eq!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]), None);
    assert_eq!(harness.state.error.as_deref(), Some("dup is taken. Try dup-2."));
}

#[test]
fn nothing_is_read_while_a_dialog_is_over_the_window() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.keys_free = false;
    assert_eq!(harness.press(Key::W), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]).is_none());
    assert!(harness.press(Key::Escape).is_none());
}

#[test]
fn the_workflow_form_draws_its_stages_and_waits_for_a_task() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::W);
    harness.press(Key::Num2);
    harness.frame(Vec::new());
    assert_eq!(harness.state.selected_workflow(&harness.catalog).unwrap().id, "fix");
    assert_eq!(harness.click(start_button()), None, "Start run is off while the task is empty");
    harness.state.set_text("the login form loses focus");
    assert!(matches!(started(harness.click(start_button())), Request::Workflow(_)));
}

#[test]
fn the_start_label_names_the_profile_or_the_run() {
    let mut harness = Harness::new(Group::Repo);
    let agent = start_label(&harness.state, &harness.catalog);
    assert_eq!(agent, "Start Builder");
    harness.state.choose(Kind::Workflow);
    assert_eq!(start_label(&harness.state, &harness.catalog), "Start run");
}

#[test]
fn the_form_draws_with_no_profiles_loaded() {
    let mut harness = Harness::new(Group::Repo);
    harness.catalog = Catalog::new(OsString::from("/usr/bin"));
    harness.frame(Vec::new());
    assert_eq!(harness.press(Key::W), None);
}
