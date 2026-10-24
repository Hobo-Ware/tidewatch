//! The terminal grid as the keyboard target. While nothing else wants the keys it holds egui's focus
//! with Tab, Shift+Tab, the arrows and Esc filtered to it, so they reach the shell as bytes and never
//! walk keyboard focus across the buttons around it.

use egui::{Context, EventFilter, Id};

const KEYS_TO_THE_SHELL: EventFilter =
    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };

fn grid_id_key() -> Id {
    Id::new("terminal-grid-widget")
}

pub(crate) fn hold(ctx: &Context, grid: Id, keys_free: bool) {
    ctx.data_mut(|data| data.insert_temp(grid_id_key(), grid));
    let keys_free = keys_free && !egui::Popup::is_any_open(ctx);
    ctx.memory_mut(|memory| {
        match (keys_free, memory.focused()) {
            (true, None) => memory.request_focus(grid),
            (false, Some(held)) if held == grid => memory.surrender_focus(grid),
            _ => {}
        }
        if keys_free {
            memory.set_focus_lock_filter(grid, KEYS_TO_THE_SHELL);
        }
    });
}

pub(crate) fn is_grid(ctx: &Context, id: Id) -> bool {
    ctx.data(|data| data.get_temp::<Id>(grid_id_key())) == Some(id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use egui::{Event, Key, Modifiers};
    use stdusk_core::repaint::Repaint;
    use stdusk_core::shell::ShellProfile;
    use stdusk_core::terminal::{PtyTerm, SpawnOpts};
    use stdusk_egui::ui::{CursorStyle, GridStyle, collect_input, render_grid};

    use super::*;
    use crate::screen_kit::{esc_left_a_widget, remember_focus};

    fn spawn(script: &str) -> PtyTerm {
        let opts = SpawnOpts {
            detect_progress: false,
            shell_integration: false,
            autosuggestions: false,
            scrollback_lines: 200,
            word_separators: " ".into(),
            bold_bright: false,
            agent_tracking: None,
            cwd: None,
            profile: Some(ShellProfile {
                shell: Some("/bin/sh".into()),
                args: vec!["-c".into(), script.into()],
                cwd: None,
                env: BTreeMap::new(),
            }),
        };
        PtyTerm::spawn(80, 24, Repaint::none(), &opts)
    }

    struct Screen {
        ctx: Context,
        term: PtyTerm,
        text: String,
        button: Option<egui::Response>,
        field: Option<egui::Response>,
        grid: Option<Id>,
        typed: Vec<u8>,
    }

    impl Screen {
        fn new(script: &str) -> Self {
            Self {
                ctx: Context::default(),
                term: spawn(script),
                text: String::new(),
                button: None,
                field: None,
                grid: None,
                typed: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                modifiers,
                focused: true,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let _ = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    self.field = Some(ui.text_edit_singleline(&mut self.text));
                    self.button = Some(ui.button("Close"));
                    let area =
                        egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(600.0, 400.0));
                    let typing = ui.ctx().text_edit_focused();
                    self.typed = collect_input(ui, false, false, self.term.app_cursor());
                    let snap = self.term.grid_snapshot();
                    let response = render_grid(
                        ui,
                        &[],
                        area,
                        &self.term,
                        &snap,
                        8.0,
                        16.0,
                        &egui::FontId::monospace(14.0),
                        None,
                        GridStyle {
                            cursor: CursorStyle::Block,
                            dimmed: false,
                            link_active: false,
                            blink: false,
                            ligatures: false,
                            min_contrast: 1.0,
                            app_mouse: false,
                        },
                        &[],
                    );
                    self.grid = Some(response.id);
                    hold(ui.ctx(), response.id, !typing);
                });
                remember_focus(ui.ctx());
            });
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame(vec![], Modifiers::NONE);
            }
        }

        fn press(&mut self, key: Key, modifiers: Modifiers) {
            let event =
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            self.frame(vec![event], modifiers);
        }

        fn focused(&self) -> Option<Id> {
            self.ctx.memory(egui::Memory::focused)
        }
    }

    #[test]
    fn tab_in_the_terminal_reaches_the_shell_and_leaves_focus_on_the_grid() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        assert_eq!(screen.focused(), screen.grid);
        screen.press(Key::Tab, Modifiers::NONE);
        assert_eq!(screen.typed, b"\t");
        assert_eq!(screen.focused(), screen.grid);
        screen.settle();
        assert_eq!(screen.focused(), screen.grid);
        assert!(!screen.button.as_ref().unwrap().has_focus());
    }

    #[test]
    fn shift_tab_in_the_terminal_is_back_tab_and_leaves_focus_on_the_grid() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        screen.press(Key::Tab, Modifiers::SHIFT);
        assert_eq!(screen.typed, b"\x1b[Z");
        screen.settle();
        assert_eq!(screen.focused(), screen.grid);
    }

    #[test]
    fn arrows_and_esc_in_the_terminal_do_not_move_or_drop_focus() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        let cases = [
            (Key::ArrowUp, &b"\x1b[A"[..]),
            (Key::ArrowDown, b"\x1b[B"),
            (Key::ArrowRight, b"\x1b[C"),
            (Key::ArrowLeft, b"\x1b[D"),
            (Key::Escape, b"\x1b"),
        ];
        for (key, bytes) in cases {
            screen.press(key, Modifiers::NONE);
            assert_eq!(screen.typed, bytes, "{key:?}");
            assert_eq!(screen.focused(), screen.grid, "{key:?}");
        }
    }

    #[test]
    fn esc_is_not_used_up_by_the_grid_holding_the_keyboard() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        let ctx = screen.ctx.clone();
        let event = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let raw = egui::RawInput { events: vec![event], ..Default::default() };
        let mut taken = true;
        let _ = ctx.run_ui(raw, |_| taken = esc_left_a_widget(&ctx, false));
        assert!(!taken);
    }

    #[test]
    fn tab_in_a_text_field_still_moves_focus_and_the_grid_does_not_take_it_back() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        screen.field.as_ref().unwrap().request_focus();
        screen.settle();
        assert_eq!(screen.focused(), screen.field.as_ref().map(|field| field.id));
        screen.press(Key::Tab, Modifiers::NONE);
        screen.settle();
        assert_eq!(screen.focused(), screen.button.as_ref().map(|button| button.id));
    }

    #[test]
    fn the_grid_lets_go_of_the_keyboard_when_the_keys_are_not_free() {
        let ctx = Context::default();
        let grid = Id::new("grid");
        let run = |free: bool| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.interact(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(50.0, 50.0)),
                        grid,
                        egui::Sense::click_and_drag(),
                    );
                    hold(ui.ctx(), grid, free);
                });
            });
        };
        run(true);
        run(true);
        assert_eq!(ctx.memory(egui::Memory::focused), Some(grid));
        run(false);
        assert_eq!(ctx.memory(egui::Memory::focused), None);
    }

    fn delivered(bytes: &[u8]) -> Vec<u8> {
        let dir = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
        let (out, ready) = (dir.path().join("got"), dir.path().join("ready"));
        let script = format!(
            "stty raw -echo; : > '{}'; head -c {} > '{}'",
            ready.display(),
            bytes.len(),
            out.display()
        );
        let mut term = spawn(&script);
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
        term.send(bytes);
        while Instant::now() < deadline {
            if std::fs::read(&out).is_ok_and(|got| got.len() >= bytes.len()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read(&out).unwrap_or_default()
    }

    #[test]
    fn the_bytes_of_tab_and_shift_tab_arrive_at_a_program_on_the_pty() {
        let mut screen = Screen::new("sleep 5");
        screen.settle();
        screen.press(Key::Tab, Modifiers::NONE);
        assert_eq!(delivered(&screen.typed), b"\t");
        screen.press(Key::Tab, Modifiers::SHIFT);
        assert_eq!(delivered(&screen.typed), b"\x1b[Z");
    }
}
