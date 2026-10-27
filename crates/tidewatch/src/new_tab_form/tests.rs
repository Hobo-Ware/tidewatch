use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, RawInput, pos2};
use tidewatch_core::prefs::NewTabPrefs;
use tidewatch_core::profiles::Profiles;

use super::*;
use crate::new_tab::{Group, MeeseeksRequest};
use tidewatch_core::meeseeks::Criteria;

struct Harness {
    ctx: egui::Context,
    state: NewTab,
    catalog: Catalog,
    facts: Option<RepoFacts>,
    keys_free: bool,
    area: Rect,
    clock: f64,
}

fn key_with(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

fn key(k: Key) -> Event {
    key_with(k, Modifiers::NONE)
}

impl Harness {
    fn new(group: Group) -> Self {
        Self::sized(group, 900.0, 800.0)
    }

    fn sized(group: Group, width: f32, height: f32) -> Self {
        let mut catalog = Catalog::new(OsString::from("/usr/bin"));
        catalog.set_profiles(Profiles::seeded(&[
            "claude".to_owned(),
            "codex".to_owned(),
            "gemini".to_owned(),
            "copilot".to_owned(),
        ]));
        let state =
            NewTab::open(group, "web".into(), PathBuf::from("/work/web"), Vec::new(), None, 3);
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height));
        let mut harness =
            Self { ctx, state, catalog, facts: None, keys_free: true, area, clock: 0.0 };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn unfocused(group: Group) -> Self {
        Self::unfocused_sized(group, 900.0, 800.0)
    }

    fn unfocused_sized(group: Group, width: f32, height: f32) -> Self {
        Self::sized(group, width, height)
    }

    fn in_the_box(group: Group) -> Self {
        let mut harness = Self::new(group);
        assert_eq!(harness.press(Key::Enter), None);
        assert!(harness.typing());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Out> {
        self.clock += 0.1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                pos2(0.0, 0.0),
                self.area.size() + vec2(100.0, 0.0),
            )),
            events,
            focused: true,
            time: Some(self.clock),
            ..RawInput::default()
        };
        let mut out = None;
        let _ = self.ctx.run_ui(raw, |ui| {
            out = show(
                ui,
                self.area,
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

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp(placed_id())).expect("the form has drawn")
    }

    fn row_center(&self, index: usize) -> egui::Pos2 {
        let entries = new_tab_lane::entries(&self.state, &self.catalog);
        let (slots, _) = layout::lane_slots(&new_tab_lane::slots(&entries));
        let at = entries
            .iter()
            .position(|entry| matches!(entry, new_tab_lane::Entry::Row(row) if row.index == index))
            .expect("a row");
        let list = layout::lane_parts(self.placed().lane.unwrap()).1;
        let (top, height) = slots[at];
        pos2(list.left() + 40.0, list.top() + top + height / 2.0)
    }

    fn click_row(&mut self, index: usize) -> Option<Out> {
        let at = self.row_center(index);
        self.click(at)
    }

    fn click_start(&mut self) -> Option<Out> {
        let at = self.placed().start.expect("the footer").center();
        self.click(at)
    }
}

fn started(out: Option<Out>) -> Request {
    match out {
        Some(Out::Start(request)) => request,
        Some(Out::Close) => panic!("closed"),
        None => panic!("nothing started"),
    }
}

#[test]
fn the_form_opens_with_no_text_field_focused() {
    let harness = Harness::new(Group::Repo);
    assert!(!harness.typing());
}

#[test]
fn a_and_m_switch_the_kind_at_once() {
    let mut harness = Harness::new(Group::Repo);
    assert_eq!(harness.press(Key::M), None);
    assert_eq!(harness.state.kind(), Kind::Meeseeks);
    assert_eq!(harness.press(Key::A), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
}

#[test]
fn enter_and_slash_move_into_the_message_box() {
    for key_name in [Key::Enter, Key::Slash] {
        let mut harness = Harness::new(Group::Repo);
        assert_eq!(harness.press(key_name), None);
        assert!(harness.typing(), "{key_name:?}");
        assert_eq!(harness.state.kind(), Kind::Agent);
    }
}

#[test]
fn the_slash_that_moved_into_the_box_is_not_typed_into_it() {
    let mut harness = Harness::new(Group::Repo);
    harness.frame(vec![key(Key::Slash), Event::Text("/".into())]);
    harness.type_text("hi");
    assert_eq!(harness.state.text(), "hi");
}

#[test]
fn enter_is_left_to_a_focused_button() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::Tab);
    assert!(harness.ctx.memory(|memory| memory.focused().is_some()) && !harness.typing());
    harness.press(Key::Enter);
    assert!(!harness.typing());
}

#[test]
fn keys_are_never_taken_from_a_focused_text_field() {
    let mut harness = Harness::in_the_box(Group::Repo);
    for letter in [Key::A, Key::M, Key::S, Key::Num2, Key::ArrowLeft, Key::ArrowRight] {
        assert_eq!(harness.frame(vec![key(letter)]), None);
    }
    harness.frame(vec![Event::Text("m".into()), key(Key::M)]);
    harness.frame(vec![Event::Text("2".into()), key(Key::Num2)]);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert_eq!(harness.state.text(), "m2");
    assert_eq!(harness.state.selected(), 0);
    assert_eq!(harness.state.level(), Level::Edits);
}

#[test]
fn esc_leaves_the_message_box_without_closing_and_the_next_one_closes_the_draft() {
    let mut harness = Harness::in_the_box(Group::Repo);
    assert_eq!(harness.press(Key::Escape), None);
    assert!(!harness.typing());
    assert!(matches!(harness.press(Key::Escape), Some(Out::Close)));
}

#[test]
fn esc_leaves_the_name_field_too() {
    let mut harness = Harness::new(Group::Repo);
    harness.ctx.memory_mut(|memory| memory.request_focus(Id::new("new-tab-name")));
    harness.frame(Vec::new());
    assert!(harness.typing());
    assert_eq!(harness.press(Key::Escape), None);
    assert!(!harness.typing());
    assert!(matches!(harness.press(Key::Escape), Some(Out::Close)));
}

#[test]
fn esc_closes_the_branch_dropdown_and_lets_go_of_it_before_the_next_one_closes_the_draft() {
    let mut harness = Harness::new(Group::Repo);
    let at = harness.placed().base.expect("the start branch field").center();
    harness.click(at);
    assert!(egui::Popup::is_any_open(&harness.ctx));
    assert_eq!(harness.press(Key::Escape), None);
    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert!(harness.ctx.memory(|memory| memory.focused().is_none()));
    assert!(matches!(harness.press(Key::Escape), Some(Out::Close)));
}

#[test]
fn esc_lets_go_of_a_focused_button_before_it_closes_the_draft() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::Tab);
    assert!(harness.ctx.memory(|memory| memory.focused().is_some()) && !harness.typing());
    assert_eq!(harness.press(Key::Escape), None);
    assert!(matches!(harness.press(Key::Escape), Some(Out::Close)));
}

#[test]
fn m_a_and_s_pick_the_kind_or_open_a_plain_shell() {
    let mut harness = Harness::unfocused(Group::Repo);
    assert_eq!(harness.press(Key::M), None);
    assert_eq!(harness.state.kind(), Kind::Meeseeks);
    assert_eq!(harness.state.title(), "New Meeseeks");
    assert_eq!(harness.press(Key::A), None);
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
}

#[test]
fn left_and_right_change_the_level_in_an_agent_form_only() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::ArrowRight);
    assert_eq!(harness.state.level(), Level::Auto);
    harness.press(Key::ArrowLeft);
    harness.press(Key::ArrowLeft);
    assert_eq!(harness.state.level(), Level::Ask);
    harness.press(Key::M);
    harness.press(Key::ArrowRight);
    harness.press(Key::A);
    assert_eq!(harness.state.level(), Level::Ask);
}

#[test]
fn clicking_a_lane_row_picks_it() {
    let mut harness = Harness::new(Group::Repo);
    assert_eq!(harness.click_row(3), None);
    assert_eq!(harness.state.selected(), 3);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("codex-reviewer"));
}

#[test]
fn cmd_enter_starts_from_the_message_box_with_the_message_and_a_name_from_it() {
    let mut harness = Harness::in_the_box(Group::Repo);
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
    assert!(matches!(started(harness.click_start()), Request::Agent(_)));
    harness.facts = Some(RepoFacts {
        checked_out: Some("main".into()),
        default_branch: None,
        remote_default: None,
        branches: vec!["main".into()],
        taken: HashSet::new(),
        test_command: None,
        test_why: None,
    });
    harness.state.type_name("has space");
    assert_eq!(harness.click_start(), None);
}

#[test]
fn cmd_enter_on_a_blocked_form_says_why() {
    let mut harness = Harness::new(Group::Repo);
    harness.state.choose(Kind::Meeseeks);
    assert_eq!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]), None);
    assert_eq!(harness.state.error.as_deref(), Some("Write one task per line."));
    harness.state.set_text("add saved searches");
    let Request::Meeseeks(request) =
        started(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]))
    else {
        panic!("meeseeks")
    };
    assert_eq!(request.tabs[0].task, "add saved searches");
}

#[test]
fn a_taken_name_blocks_cmd_enter() {
    let mut harness = Harness::new(Group::Repo);
    harness.facts = Some(RepoFacts {
        checked_out: None,
        default_branch: None,
        remote_default: None,
        branches: Vec::new(),
        taken: ["dup".to_owned()].into_iter().collect(),
        test_command: None,
        test_why: None,
    });
    harness.state.type_name("dup");
    assert_eq!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]), None);
    assert_eq!(harness.state.error.as_deref(), Some("dup is taken. Try dup-2."));
}

#[test]
fn nothing_is_read_while_a_dialog_is_over_the_window() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.keys_free = false;
    assert_eq!(harness.press(Key::M), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]).is_none());
    assert!(harness.press(Key::Escape).is_none());
}

#[test]
fn the_start_label_names_the_profile_or_the_summons() {
    let mut harness = Harness::new(Group::Repo);
    let agent = start_label(&harness.state, &harness.catalog);
    assert_eq!(agent, "Start Builder");
    harness.state.choose(Kind::Meeseeks);
    assert_eq!(start_label(&harness.state, &harness.catalog), new_tab_meeseeks::summon_label(0));
}

#[test]
fn the_form_draws_with_no_profiles_loaded() {
    let mut harness = Harness::new(Group::Repo);
    harness.catalog = Catalog::new(OsString::from("/usr/bin"));
    harness.frame(Vec::new());
    assert_eq!(harness.press(Key::M), None);
}

fn parts(placed: &Placed) -> Vec<(&'static str, Rect)> {
    [
        ("lane", placed.lane),
        ("permission", placed.permission),
        ("level note", placed.level_note),
        ("name", placed.name),
        ("base", placed.base),
        ("message", placed.message),
        ("hints", placed.hints),
        ("start", placed.start),
    ]
    .into_iter()
    .filter_map(|(name, rect)| rect.map(|rect| (name, rect)))
    .collect()
}

fn nothing_meets(harness: &Harness, what: &str) {
    let parts = parts(&harness.placed());
    for (index, (a_name, a)) in parts.iter().enumerate() {
        assert!(harness.area.contains_rect(*a), "{what}: {a_name} {a:?} leaves the form");
        for (b_name, b) in &parts[index + 1..] {
            assert!(!a.intersects(*b), "{what}: {a_name} {a:?} meets {b_name} {b:?}");
        }
    }
}

fn dup_facts() -> RepoFacts {
    RepoFacts {
        checked_out: Some("main".into()),
        default_branch: None,
        remote_default: None,
        branches: vec!["main".into()],
        taken: ["dup".to_owned()].into_iter().collect(),
        test_command: None,
        test_why: None,
    }
}

#[test]
fn no_part_of_the_form_meets_another_in_a_short_or_a_narrow_window() {
    let sizes = [(980.0, 560.0), (742.0, 642.0), (835.0, 822.0), (638.0, 562.0), (900.0, 800.0)];
    for (width, height) in sizes {
        for group in [Group::Repo, Group::Other] {
            let mut harness = Harness::sized(group, width, height);
            nothing_meets(&harness, &format!("{width}x{height} {group:?}"));
            harness.press(Key::M);
            harness.frame(Vec::new());
            nothing_meets(&harness, &format!("{width}x{height} {group:?} meeseeks"));
        }
    }
}

#[test]
fn nothing_meets_with_a_taken_name_a_warning_and_a_typed_message() {
    for (width, height) in [(980.0, 560.0), (638.0, 660.0)] {
        let mut harness = Harness::sized(Group::Repo, width, height);
        harness.facts = Some(dup_facts());
        harness.state.select(6, &harness.catalog);
        harness.state.type_name("dup");
        harness
            .type_text("A first message that is long enough to matter for the hint under the box");
        harness.frame(Vec::new());
        assert_ne!(harness.state.warnings(&harness.catalog), Vec::<String>::new());
        nothing_meets(&harness, &format!("{width}x{height} with problems"));
    }
}

#[test]
fn the_lane_is_as_tall_as_the_form_and_the_footer_sits_at_the_bottom_of_the_pane() {
    let harness = Harness::sized(Group::Repo, 980.0, 560.0);
    let placed = harness.placed();
    let lane = placed.lane.unwrap();
    assert_eq!(lane.bottom(), harness.area.bottom());
    assert!(lane.height() > 400.0, "{lane:?}");
    let start = placed.start.unwrap();
    assert_eq!(start.height(), 32.0);
    assert_eq!(start.right(), harness.area.right() - layout::SIDE_PAD);
    assert_eq!(start.bottom(), harness.area.bottom() - 14.0);
}

#[test]
fn a_pane_too_short_for_the_message_scrolls_and_the_footer_stays_pinned() {
    let tall = Harness::sized(Group::Repo, 980.0, 560.0).placed().start.unwrap();
    let short = Harness::sized(Group::Repo, 980.0, 330.0);
    let start = short.placed().start.unwrap();
    assert_eq!(start.bottom(), short.area.bottom() - 14.0);
    assert_eq!(start.size(), tall.size());
    assert!(
        short.placed().message.unwrap().bottom() > start.top(),
        "the body scrolls under the footer"
    );
}

#[test]
fn up_and_down_move_the_pick_and_stop_at_the_ends() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::ArrowUp);
    assert_eq!(harness.state.selected(), 0);
    harness.press(Key::ArrowDown);
    harness.press(Key::ArrowDown);
    assert_eq!(harness.state.selected(), 2);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("codex-builder"));
    harness.press(Key::ArrowUp);
    assert_eq!(harness.state.selected(), 1);
    for _ in 0..12 {
        harness.press(Key::ArrowDown);
    }
    assert_eq!(harness.state.selected(), harness.state.visible(&harness.catalog).len() - 1);
    harness.press(Key::M);
    harness.press(Key::ArrowDown);
    assert_eq!(harness.state.selected(), 1);
}

#[test]
fn up_and_down_are_left_to_a_focused_text_field() {
    let mut harness = Harness::in_the_box(Group::Repo);
    harness.press(Key::ArrowDown);
    assert_eq!(harness.state.selected(), 0);
}

#[test]
fn the_picked_row_stays_in_view_when_the_keys_move_it() {
    let mut harness = Harness::unfocused_sized(Group::Repo, 980.0, 420.0);
    let last = harness.state.visible(&harness.catalog).len() - 1;
    for _ in 0..last {
        harness.press(Key::ArrowDown);
        harness.frame(Vec::new());
    }
    harness.frame(Vec::new());
    let lane = harness.placed().lane.unwrap();
    let row = harness.placed().selected_row.unwrap();
    assert!(row.top() >= lane.top() && row.bottom() <= lane.bottom() + 0.5, "{row:?} in {lane:?}");
    for _ in 0..last {
        harness.press(Key::ArrowUp);
        harness.frame(Vec::new());
    }
    harness.frame(Vec::new());
    let row = harness.placed().selected_row.unwrap();
    assert!(row.top() >= lane.top() && row.bottom() <= lane.bottom() + 0.5, "{row:?} in {lane:?}");
}

fn pick_profile(harness: &mut Harness, id: &str) {
    harness.state.select_id(id, &harness.catalog);
    harness.frame(Vec::new());
    harness.frame(Vec::new());
}

fn click_remote_switch(harness: &mut Harness) -> Option<Out> {
    let at = harness.placed().remote_switch.expect("the Remote control row").center();
    harness.click(at)
}

fn started_remote(harness: &mut Harness) -> bool {
    match started(harness.click_start()) {
        Request::Agent(request) => request.remote,
        other => panic!("not an agent: {other:?}"),
    }
}

#[test]
fn the_remote_row_sits_under_permission_and_its_switch_turns_remote_control_on_and_off() {
    for id in ["builder", "copilot-builder"] {
        let mut harness = Harness::new(Group::Repo);
        pick_profile(&mut harness, id);
        let placed = harness.placed();
        let (permission, remote, name) =
            (placed.permission.unwrap(), placed.remote.unwrap(), placed.name.unwrap());
        assert!(permission.bottom() < remote.top() && remote.bottom() < name.top(), "{id}");
        assert!(remote.contains_rect(placed.remote_switch.unwrap()), "{id}");
        assert!(!harness.state.remote());
        assert_eq!(click_remote_switch(&mut harness), None);
        assert!(harness.state.remote(), "{id}: on");
        assert!(started_remote(&mut harness), "{id}: the request asks for it");
        click_remote_switch(&mut harness);
        assert!(!harness.state.remote(), "{id}: off again");
        assert!(!started_remote(&mut harness), "{id}");
    }
}

#[test]
fn for_a_cli_without_it_the_row_is_there_but_its_switch_does_nothing() {
    for id in ["gemini-builder", "codex-builder"] {
        let mut harness = Harness::new(Group::Repo);
        pick_profile(&mut harness, id);
        assert!(harness.placed().remote.is_some(), "{id}: the row stays so the reason shows");
        click_remote_switch(&mut harness);
        assert!(!harness.state.remote(), "{id}: a click changes nothing");
        assert!(!started_remote(&mut harness), "{id}");
    }
}

#[test]
fn a_remote_default_that_a_cli_cannot_honour_is_not_sent_to_it() {
    let mut harness = Harness::new(Group::Repo);
    harness.state.set_remote(true);
    pick_profile(&mut harness, "gemini-builder");
    assert!(harness.state.remote(), "the form remembers the wish");
    assert!(!started_remote(&mut harness));
    pick_profile(&mut harness, "builder");
    assert!(started_remote(&mut harness));
}

#[test]
fn a_meeseeks_form_has_no_remote_row() {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.press(Key::M);
    harness.frame(Vec::new());
    assert_eq!(harness.placed().remote, None);
}

#[test]
fn in_a_plain_folder_the_row_is_there_too() {
    let mut harness = Harness::new(Group::Other);
    assert!(harness.placed().remote.is_some());
    click_remote_switch(&mut harness);
    assert!(started_remote(&mut harness));
}

fn tab_stops(harness: &mut Harness) -> Vec<egui::Id> {
    let mut stops = Vec::new();
    for _ in 0..80 {
        harness.frame(vec![key(Key::Tab)]);
        match harness.ctx.memory(egui::Memory::focused) {
            Some(id) if !stops.contains(&id) => stops.push(id),
            _ => {}
        }
    }
    stops
}

#[test]
fn the_switch_takes_tab_focus_when_it_works_and_space_flips_it() {
    let stops_of = |id: &str| {
        let mut harness = Harness::unfocused(Group::Repo);
        pick_profile(&mut harness, id);
        tab_stops(&mut harness)
    };
    let (claude, gemini) = (stops_of("builder"), stops_of("gemini-builder"));
    let only_claude: Vec<_> = claude.iter().filter(|id| !gemini.contains(id)).collect();
    assert_eq!(only_claude.len(), 1, "an enabled switch is one more stop than a disabled one");
    let switch = *only_claude[0];
    let mut harness = Harness::unfocused(Group::Repo);
    for _ in 0..80 {
        harness.frame(vec![key(Key::Tab)]);
        if harness.ctx.memory(egui::Memory::focused) == Some(switch) {
            break;
        }
    }
    assert!(!harness.state.remote());
    harness.frame(vec![key(Key::Space)]);
    assert!(harness.state.remote(), "Space on the focused switch turns remote control on");
}

fn command_facts() -> RepoFacts {
    RepoFacts {
        checked_out: Some("main".into()),
        branches: vec!["main".into()],
        test_command: Some("npm test".into()),
        ..RepoFacts::default()
    }
}

fn meeseeks_form(text: &str) -> Harness {
    let mut harness = Harness::unfocused(Group::Repo);
    harness.facts = Some(command_facts());
    assert_eq!(harness.press(Key::M), None);
    harness.state.set_text(text);
    harness.frame(Vec::new());
    harness
}

fn meeseeks_of(out: Option<Out>) -> MeeseeksRequest {
    match started(out) {
        Request::Meeseeks(request) => *request,
        other => panic!("a Meeseeks request, got {other:?}"),
    }
}

fn center(rect: Option<Rect>) -> egui::Pos2 {
    rect.expect("drawn").center()
}

#[test]
fn m_picks_the_meeseeks_kind_and_the_control_lists_it_between_agent_and_shell() {
    let mut harness = Harness::unfocused(Group::Repo);
    assert_eq!(harness.press(Key::M), None);
    assert_eq!(harness.state.kind(), Kind::Meeseeks);
    assert_eq!(harness.state.title(), "New Meeseeks");
    assert!(harness.placed().kinds.iter().all(Option::is_some));
    assert_eq!(harness.press(Key::A), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
}

#[test]
fn clicking_the_meeseeks_segment_switches_the_form_and_the_agent_segment_switches_back() {
    let mut harness = Harness::unfocused(Group::Repo);
    let at = center(harness.placed().kinds[1]);
    assert_eq!(harness.click(at), None);
    assert_eq!(harness.state.kind(), Kind::Meeseeks);
    let at = center(harness.placed().kinds[0]);
    harness.click(at);
    assert_eq!(harness.state.kind(), Kind::Agent);
}

#[test]
fn in_other_there_is_no_meeseeks_segment_and_m_does_nothing() {
    let mut harness = Harness::unfocused(Group::Other);
    assert_eq!(harness.press(Key::M), None);
    assert_eq!(harness.state.kind(), Kind::Agent);
    assert_eq!(harness.placed().kinds.iter().flatten().count(), 2);
}

#[test]
fn the_meeseeks_form_has_no_name_fields_permission_or_remote_row_and_shows_three_ticks_on() {
    let harness = meeseeks_form("one task");
    let placed = harness.placed();
    assert!(placed.name.is_none() && placed.base.is_none() && placed.remote.is_none());
    assert!(placed.permission.is_none(), "the level is not a choice");
    assert!(placed.yolo_line.is_some() && placed.message.is_some());
    assert!(placed.ticks.iter().all(Option::is_some));
    assert_eq!(harness.state.ticks(), Criteria::default());
}

#[test]
fn summon_says_how_many_and_starts_one_request_per_line() {
    let mut harness = meeseeks_form("Bump vite\nAdd a test\n\nRename a helper");
    assert_eq!(start_label(&harness.state, &harness.catalog), "Summon 3");
    let request = meeseeks_of(harness.click_start());
    assert_eq!(request.tabs.len(), 3);
    assert_eq!(request.ticks, Criteria::default());
    assert!(request.tabs.iter().all(|tab| tab.meeseeks == Some(Criteria::default())));
}

#[test]
fn cmd_enter_summons_from_the_task_box_too() {
    let mut harness = Harness::new(Group::Repo);
    harness.facts = Some(command_facts());
    harness.state.choose(Kind::Meeseeks);
    harness.press(Key::Enter);
    harness.type_text("Fix the flaky retry test");
    let request = meeseeks_of(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]));
    assert_eq!(request.tabs[0].task, "Fix the flaky retry test");
}

#[test]
fn summon_with_an_empty_box_does_nothing_and_says_why_in_the_footer() {
    let mut harness = meeseeks_form("");
    assert_eq!(start_label(&harness.state, &harness.catalog), "Summon");
    assert_eq!(harness.click_start(), None);
    assert_eq!(harness.frame(vec![key_with(Key::Enter, Modifiers::COMMAND)]), None);
    assert_eq!(harness.state.error.as_deref(), Some("Write one task per line."));
}

#[test]
fn a_click_on_a_tick_flips_it_and_the_request_carries_the_new_ticks() {
    let mut harness = meeseeks_form("one task");
    let at = center(harness.placed().ticks[2]);
    assert_eq!(harness.click(at), None);
    let at = center(harness.placed().ticks[1]);
    assert_eq!(harness.click(at), None);
    let want = Criteria { tests: true, committed: false, pr: false };
    assert_eq!(harness.state.ticks(), want);
    assert_eq!(meeseeks_of(harness.click_start()).ticks, want);
}

#[test]
fn the_tests_tick_cannot_be_clicked_without_a_test_command_and_the_tabs_drop_it() {
    let mut harness = meeseeks_form("one task");
    harness.facts = Some(RepoFacts { test_command: None, ..command_facts() });
    harness.frame(Vec::new());
    let at = center(harness.placed().ticks[0]);
    assert_eq!(harness.click(at), None);
    assert!(harness.state.ticks().tests, "the stored tick is left alone");
    let request = meeseeks_of(harness.click_start());
    assert_eq!(request.tabs[0].meeseeks, Some(Criteria { tests: false, ..Criteria::default() }));
}

#[test]
fn the_number_keys_pick_the_cli_and_left_and_right_leave_the_yolo_level_alone() {
    let mut harness = meeseeks_form("one task");
    harness.press(Key::ArrowRight);
    harness.press(Key::ArrowLeft);
    assert_eq!(harness.state.level(), Level::Yolo);
    harness.press(Key::Num3);
    assert_eq!(harness.state.selected_id(&harness.catalog).as_deref(), Some("gemini"));
    harness.state.set_text("one task");
    let request = meeseeks_of(harness.click_start());
    let tab = &request.tabs[0];
    assert_eq!((tab.agent.id.as_str(), tab.profile.level), ("gemini", Level::Yolo));
    assert_eq!(tab.profile_id, "gemini-builder");
}

#[test]
fn the_meeseeks_lane_has_one_row_per_cli_the_yolo_line_replaces_the_permission_row_and_the_footer_counts_to_four()
 {
    let mut harness = meeseeks_form("one task");
    let rows = new_tab_lane::entries(&harness.state, &harness.catalog);
    assert_eq!(rows.len(), 4);
    assert_eq!(footer_hints(Kind::Meeseeks)[0][0].0, "1-4 Up Down");
    let yolo = harness.placed().yolo_line.expect("drawn");
    assert!(yolo.height() > 0.0 && harness.placed().message.unwrap().top() > yolo.bottom());
    harness.press(Key::A);
    assert!(harness.placed().yolo_line.is_none() && harness.placed().permission.is_some());
}

fn level_center(harness: &Harness, level: Level) -> egui::Pos2 {
    let control = harness.placed().permission.expect("the permission row");
    let labels = Level::ALL.map(Level::label);
    let mut centers = Vec::new();
    let _ = harness.ctx.run_ui(RawInput::default(), |ui| {
        let right = control.left() + segmented::inline_width(ui, &labels);
        centers = segmented::centers(ui, &labels, right, control.center().y);
    });
    centers[Level::ALL.iter().position(|l| *l == level).unwrap()]
}

#[test]
fn clicking_yolo_in_other_keeps_the_level_and_shows_the_reason_under_the_control() {
    let mut harness = Harness::new(Group::Other);
    assert_eq!(harness.placed().level_note, None);
    let at = level_center(&harness, Level::Yolo);
    let level = harness.state.level();
    harness.click(at);
    harness.frame(Vec::new());
    assert_eq!(harness.state.level(), level);
    let note = harness.placed().level_note.expect("the reason shows");
    let control = harness.placed().permission.unwrap();
    assert!(note.top() > control.bottom());
    assert!(harness.state.level_note().is_some_and(|text| text.contains("not in Other")));
    let at = level_center(&harness, Level::Ask);
    harness.click(at);
    harness.frame(Vec::new());
    assert_eq!(harness.state.level(), Level::Ask);
    assert_eq!(harness.placed().level_note, None);
}

#[test]
fn right_onto_a_refused_yolo_shows_the_reason_and_stays_put() {
    let mut harness = Harness::unfocused(Group::Other);
    harness.state.set_level(Level::Auto, &harness.catalog);
    harness.press(Key::ArrowRight);
    harness.frame(Vec::new());
    assert_eq!(harness.state.level(), Level::Auto);
    assert!(harness.placed().level_note.is_some());
    harness.press(Key::ArrowLeft);
    harness.frame(Vec::new());
    assert_eq!(harness.placed().level_note, None);
}

#[test]
fn a_yolo_default_in_other_opens_at_auto_with_the_reason_already_showing() {
    let mut harness = Harness::new(Group::Other);
    let defaults = NewTabPrefs { permission: Some(Level::Yolo), ..NewTabPrefs::default() };
    harness.state =
        NewTab::open(Group::Other, "web".into(), PathBuf::from("/work/web"), Vec::new(), None, 3)
            .with_defaults(defaults);
    harness.state.settle(&harness.catalog);
    harness.frame(Vec::new());
    harness.frame(Vec::new());
    assert_eq!(harness.state.level(), Level::Auto);
    assert!(harness.placed().level_note.is_some());
}

#[test]
fn a_repo_form_has_nothing_refused_so_no_note_and_no_room_for_one() {
    let repo = Harness::new(Group::Repo);
    assert_eq!(repo.placed().level_note, None);
    let other = Harness::new(Group::Other);
    assert!(
        other.placed().remote.unwrap().top() > repo.placed().remote.unwrap().top(),
        "Other keeps one line free for the reason"
    );
}

#[test]
fn the_reason_appearing_moves_nothing_below_it() {
    let mut harness = Harness::new(Group::Other);
    let before = harness.placed();
    let at = level_center(&harness, Level::Yolo);
    harness.click(at);
    harness.frame(Vec::new());
    let after = harness.placed();
    assert!(after.level_note.is_some());
    assert_eq!(
        (before.remote, before.name, before.message),
        (after.remote, after.name, after.message)
    );
}

fn tooltip_shows(harness: &mut Harness, over: Level) -> bool {
    let at = level_center(harness, over);
    harness.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..12 {
        harness.frame(Vec::new());
    }
    harness.ctx.memory(|memory| {
        memory.areas().visible_layer_ids().into_iter().any(|id| id.order == egui::Order::Tooltip)
    })
}

#[test]
fn hovering_the_greyed_yolo_segment_shows_its_reason_as_a_tooltip_and_an_open_one_shows_none() {
    assert!(tooltip_shows(&mut Harness::new(Group::Other), Level::Yolo));
    assert!(!tooltip_shows(&mut Harness::new(Group::Other), Level::Auto));
    assert!(!tooltip_shows(&mut Harness::new(Group::Repo), Level::Yolo));
}

#[test]
fn an_empty_list_says_why() {
    assert_eq!(empty_text("zz", true, "profile"), "No profile matches \"zz\".");
    assert!(empty_text("", true, "agent").starts_with("No agents yet. Add one in Settings ("));
    assert_eq!(empty_text("", false, "profile"), "The profiles are still loading...");
}
