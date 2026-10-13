//! Draws the Window section of General: the quake switch, the hotkey field with its status, and
//! the menu-bar switch. It returns the new window settings when something was applied.

use egui::RichText;
use stdusk_egui::colors;
use stdusk_egui::widgets::{text_field, toggle_switch};
use tidewatch_core::prefs::WindowPrefs;

use crate::shell::HotkeyStatus;
use crate::window_settings::Editor;

const FIELD_WIDTH: f32 = 260.0;

/// What the section shows beside the editor's own state.
pub(crate) struct View {
    pub(crate) status: HotkeyStatus,
    pub(crate) menu_bar_available: bool,
}

pub(crate) fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(colors::dim()));
}

pub(crate) fn switch_row(ui: &mut egui::Ui, text: &str, on: bool) -> Option<bool> {
    let mut value = on;
    ui.horizontal(|ui| {
        let changed = toggle_switch(ui, &mut value).changed();
        ui.label(RichText::new(text).color(colors::fg()));
        changed
    })
    .inner
    .then_some(value)
}

pub(crate) fn section(ui: &mut egui::Ui, editor: &mut Editor, view: &View) -> Option<WindowPrefs> {
    let mut applied = None;
    ui.add_space(10.0);
    ui.label(RichText::new("Window").strong().color(colors::fg()));
    if let Some(on) = switch_row(ui, "Drop-down window", editor.quake()) {
        applied = Some(editor.set_quake(on));
    }
    note(
        ui,
        "Falls from the top of the screen when you press the hotkey and hides when you click \
         elsewhere. Off keeps a plain window.",
    );
    if editor.quake() {
        applied = hotkey_field(ui, editor, &view.status).or(applied);
    }
    if view.menu_bar_available {
        if let Some(on) = switch_row(ui, "Menu-bar item", editor.menu_bar()) {
            applied = Some(editor.set_menu_bar(on));
        }
        note(ui, "Show / Hide, Overview, Settings and Quit, from the menu bar.");
    }
    applied
}

fn hotkey_field(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    status: &HotkeyStatus,
) -> Option<WindowPrefs> {
    ui.add_space(4.0);
    note(ui, "Hotkey");
    let problem = editor.problem();
    let color = if problem.is_some() { colors::red() } else { colors::fg() };
    let response = ui
        .scope(|ui| text_field(ui, &mut editor.hotkey, "Ctrl+Alt+Grave", FIELD_WIDTH, color))
        .inner;
    let mut applied = None;
    if response.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            editor.revert_hotkey();
        } else {
            applied = editor.leave_hotkey();
        }
    }
    if let Some(problem) = &problem {
        ui.label(RichText::new(problem.to_string()).size(12.0).color(colors::red()));
    } else {
        match status {
            HotkeyStatus::Registered => note(ui, "Registered."),
            HotkeyStatus::Problem(reason) => {
                ui.label(
                    RichText::new(format!("Not registered: {reason}"))
                        .size(12.0)
                        .color(colors::red()),
                );
            }
            HotkeyStatus::NoGlobalHotkeys => note(
                ui,
                "This desktop has no global hotkeys. Bind a shortcut to `tidewatch --toggle`.",
            ),
            HotkeyStatus::Off => {}
        }
    }
    note(ui, "Modifiers first, then one key: Ctrl+Alt+Grave, Cmd+Shift+T, F13. Enter applies.");
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, UiBuilder, Vec2, pos2};

    struct Page {
        ctx: egui::Context,
        editor: Editor,
        status: HotkeyStatus,
    }

    fn page(prefs: &WindowPrefs, status: HotkeyStatus) -> Page {
        Page { ctx: egui::Context::default(), editor: Editor::new(prefs), status }
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

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
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

        fn field(&mut self, events: Vec<Event>) -> Option<WindowPrefs> {
            let mut answer = None;
            let status = self.status.clone();
            let editor = &mut self.editor;
            let _ = self.ctx.run_ui(input(events), |ui| {
                ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect()), |ui| {
                    answer = hotkey_field(ui, editor, &status);
                });
            });
            answer
        }

        fn focus_field(&mut self) {
            let at = pos2(60.0, 40.0);
            self.field(vec![]);
            self.field(vec![Event::PointerMoved(at)]);
            self.field(vec![button(at, true)]);
            self.field(vec![button(at, false)]);
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
    fn a_switch_row_reports_its_new_value_only_when_clicked() {
        let ctx = egui::Context::default();
        let run = |on: bool, events: Vec<Event>| {
            let mut answer = None;
            let _ = ctx.run_ui(input(events), |ui| {
                ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect()), |ui| {
                    answer = switch_row(ui, "Drop-down window", on);
                });
            });
            answer
        };
        let at = pos2(19.0, 11.0);
        assert_eq!(run(true, vec![]), None);
        assert_eq!(run(true, vec![Event::PointerMoved(at)]), None);
        assert_eq!(run(true, vec![button(at, true)]), None);
        assert_eq!(run(true, vec![button(at, false)]), Some(false));
        assert_eq!(run(false, vec![button(at, true)]), None);
        assert_eq!(run(false, vec![button(at, false)]), Some(true));
    }

    #[test]
    fn the_quake_switch_in_the_section_applies_at_once_with_the_hotkey_in_use() {
        let mut page = registered();
        page.section(vec![], true);
        let mut applied = None;
        for y in (20..60).step_by(4) {
            let at = pos2(27.0, y as f32);
            page.section(vec![Event::PointerMoved(at)], true);
            page.section(vec![button(at, true)], true);
            applied = page.section(vec![button(at, false)], true);
            if applied.is_some() {
                break;
            }
        }
        let applied = applied.expect("the first switch is the quake one");
        assert_eq!(applied, WindowPrefs { quake: false, ..WindowPrefs::default() });
        assert!(!page.editor.quake());
    }

    #[test]
    fn typing_a_hotkey_shows_a_bad_one_at_once_and_applies_a_good_one_on_enter() {
        let mut page = registered();
        page.focus_field();
        page.editor.hotkey.clear();
        page.field(vec![Event::Text("Ctrl+Banana".to_owned())]);
        assert_eq!(page.editor.hotkey, "Ctrl+Banana", "the click focused the field");
        assert!(page.editor.problem().is_some());
        assert_eq!(page.field(vec![key(Key::Enter, true)]), None, "a bad hotkey is not applied");
        assert_eq!(page.editor.hotkey, "Ctrl+Banana", "and is not thrown away");

        page.focus_field();
        page.editor.hotkey = "F13".to_owned();
        page.field(vec![]);
        let applied = page.field(vec![key(Key::Enter, true)]).expect("a good one applies on Enter");
        assert_eq!(applied.hotkey, "F13");
    }

    #[test]
    fn escape_in_the_field_puts_the_hotkey_in_use_back() {
        let mut page = registered();
        page.focus_field();
        page.field(vec![Event::Text("x".to_owned())]);
        assert_ne!(page.editor.hotkey, "Ctrl+Alt+Grave");
        assert_eq!(page.field(vec![key(Key::Escape, true)]), None);
        assert_eq!(page.editor.hotkey, "Ctrl+Alt+Grave");
    }
}
