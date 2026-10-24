//! Window section: renders in every state, the switch applies, the recorder takes a chord.

use egui::{Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, pos2};

use super::*;

struct Page {
    ctx: egui::Context,
    editor: Editor,
    status: HotkeyStatus,
}

fn page(prefs: &WindowPrefs, status: HotkeyStatus) -> Page {
    Page { ctx: crate::fonts::test_context(), editor: Editor::new(prefs), status }
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(700.0, 500.0))),
        focused: true,
        events,
        ..RawInput::default()
    }
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn press(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

impl Page {
    fn section(&mut self, events: Vec<Event>, menu_bar_available: bool) -> Option<WindowPrefs> {
        let mut answer = None;
        let view = View { status: self.status.clone(), menu_bar_available };
        let editor = &mut self.editor;
        let _ = self.ctx.run_ui(input(events), |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect().shrink(8.0)), |ui| {
                answer = section(ui, editor, &view);
            });
        });
        answer
    }

    fn recorder(&mut self, events: Vec<Event>) -> Option<WindowPrefs> {
        let mut answer = None;
        let editor = &mut self.editor;
        let _ = self.ctx.run_ui(input(events), |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect().shrink(8.0)), |ui| {
                answer = recorder(ui, editor, false);
            });
        });
        answer
    }

    fn start_recording(&mut self) {
        let at = pos2(40.0, 25.0);
        self.recorder(vec![]);
        self.recorder(vec![Event::PointerMoved(at)]);
        self.recorder(vec![button(at, true)]);
        self.recorder(vec![button(at, false)]);
        assert!(self.editor.recording, "a click on the recorder starts it");
    }
}

fn registered() -> Page {
    page(&WindowPrefs::default(), HotkeyStatus::Registered)
}

#[test]
fn the_section_renders_in_every_state_and_asks_for_nothing_until_touched() {
    let statuses = [
        HotkeyStatus::Registered,
        HotkeyStatus::Off,
        HotkeyStatus::NoGlobalHotkeys,
        HotkeyStatus::Problem("already taken".to_owned()),
    ];
    for status in statuses {
        for menu_bar_available in [true, false] {
            for quake in [true, false] {
                let prefs = WindowPrefs { quake, ..WindowPrefs::default() };
                let mut page = page(&prefs, status.clone());
                for _ in 0..2 {
                    assert_eq!(page.section(vec![], menu_bar_available), None);
                }
            }
        }
    }
}

#[test]
fn the_quake_switch_in_the_section_applies_at_once_with_the_hotkey_in_use() {
    let mut page = registered();
    page.section(vec![], true);
    let mut applied = None;
    'scan: for y in (20..80).step_by(6) {
        for x in (560..690).step_by(6) {
            let at = pos2(x as f32, y as f32);
            page.section(vec![Event::PointerMoved(at)], true);
            page.section(vec![button(at, true)], true);
            applied = page.section(vec![button(at, false)], true);
            if applied.is_some() {
                break 'scan;
            }
        }
    }
    let applied = applied.expect("the first switch is the quake one");
    assert_eq!(applied, WindowPrefs { quake: false, ..WindowPrefs::default() });
    assert!(!page.editor.quake());
}

#[test]
fn a_press_while_recording_is_applied_and_ends_the_recording() {
    let mut page = registered();
    page.start_recording();
    let mods = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    let applied = page.recorder(vec![press(Key::K, mods)]).expect("a good chord applies");
    assert_eq!(applied.hotkey, "Ctrl+Shift+K");
    assert!(!page.editor.recording);
    assert_eq!(page.editor.hotkey, "Ctrl+Shift+K");
}

#[test]
fn a_chord_that_is_refused_shows_its_problem_and_is_not_applied() {
    let mut page = registered();
    page.start_recording();
    assert_eq!(page.recorder(vec![press(Key::T, Modifiers::NONE)]), None);
    assert_eq!(page.editor.hotkey, "T", "what was pressed stays visible");
    assert!(page.editor.problem().is_some());
    assert!(!page.editor.recording);
}

#[test]
fn escape_while_recording_puts_the_hotkey_in_use_back() {
    let mut page = registered();
    page.start_recording();
    assert_eq!(page.recorder(vec![press(Key::Escape, Modifiers::NONE)]), None);
    assert!(!page.editor.recording);
    assert_eq!(page.editor.hotkey, "Ctrl+Alt+Grave");
}

#[test]
fn a_click_elsewhere_stops_recording_and_a_key_press_without_recording_does_nothing() {
    let mut page = registered();
    assert_eq!(page.recorder(vec![press(Key::K, Modifiers::CTRL)]), None);
    assert_eq!(page.editor.hotkey, "Ctrl+Alt+Grave");
    page.start_recording();
    let away = pos2(400.0, 300.0);
    page.recorder(vec![Event::PointerMoved(away)]);
    page.recorder(vec![button(away, true)]);
    page.recorder(vec![button(away, false)]);
    assert!(!page.editor.recording);
    assert_eq!(page.editor.hotkey, "Ctrl+Alt+Grave");
}

#[test]
fn the_panels_choice_applies_at_once_and_keeps_the_rest() {
    let mut page = registered();
    page.section(vec![], true);
    let mut applied = None;
    'scan: for y in (20..420).step_by(8) {
        for x in (400..690).step_by(8) {
            let at = pos2(x as f32, y as f32);
            page.section(vec![Event::PointerMoved(at)], true);
            page.section(vec![button(at, true)], true);
            let answer = page.section(vec![button(at, false)], true);
            if answer.as_ref().is_some_and(|prefs| prefs.panels == Panels::Below) {
                applied = answer;
                break 'scan;
            }
            if answer.is_some() {
                page = registered();
            }
        }
    }
    let applied = applied.expect("a click on Below applies it");
    assert_eq!(applied, WindowPrefs { panels: Panels::Below, ..WindowPrefs::default() });
    assert_eq!(page.editor.panels(), Panels::Below);
}

fn height_thumb(page: &mut Page) -> Pos2 {
    page.section(vec![], true);
    page.section(vec![], true);
    let mut best = None;
    for y in (20..300).step_by(4) {
        for x in (380..690).step_by(4) {
            let at = pos2(x as f32, y as f32);
            page.section(vec![Event::PointerMoved(at)], true);
            page.section(vec![button(at, true)], true);
            let answer = page.section(vec![button(at, false)], true);
            if answer.as_ref().is_some_and(|prefs| prefs.height_pct != 0.5) {
                best = Some(at);
                break;
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.expect("a click on the slider track applies a new height")
}

#[test]
fn the_height_slider_applies_when_it_is_let_go_and_keeps_the_rest() {
    let mut page = registered();
    let at = height_thumb(&mut page);
    assert!(at.x > 380.0);
    assert!((0.2..=0.9).contains(&page.editor.height));
    assert_eq!(page.editor.settle_height(), None, "the release already applied it");
}

#[test]
fn the_height_row_is_only_there_while_the_drop_down_is_on() {
    let on = WindowPrefs::default();
    let off = WindowPrefs { quake: false, ..WindowPrefs::default() };
    let mut with = page(&on, HotkeyStatus::Registered);
    let mut without = page(&off, HotkeyStatus::Registered);
    let mut heights = Vec::new();
    for page in [&mut with, &mut without] {
        let mut height = 0.0;
        let view = View { status: HotkeyStatus::Registered, menu_bar_available: true };
        let editor = &mut page.editor;
        let _ = page.ctx.run_ui(input(vec![]), |ui| {
            section(ui, editor, &view);
            height = ui.min_rect().height();
        });
        heights.push(height);
    }
    assert!(heights[0] > heights[1] + 40.0, "{heights:?}");
}
