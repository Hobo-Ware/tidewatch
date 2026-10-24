//! What the wheel and the trackpad do over the terminal grid. The shell's scrollback moves on the
//! normal screen; an app on the alternate screen gets arrow keys, or SGR wheel reports once it
//! turned mouse reporting on.

use egui::{Event, Modifiers, Pos2, Rect};
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::mouse::{MouseReporting, WheelAccumulator, wheel_report_lines, wheel_sgr};

#[derive(Clone, Copy)]
pub(crate) struct Cells {
    pub(crate) rect: Rect,
    pub(crate) cell_w: f32,
    pub(crate) cell_h: f32,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
}

impl Cells {
    fn at(self, pointer: Pos2) -> (usize, usize) {
        let col = ((pointer.x - self.rect.left()) / self.cell_w).max(0.0) as usize;
        let row = ((pointer.y - self.rect.top()) / self.cell_h).max(0.0) as usize;
        (col.min(self.cols.saturating_sub(1)), row.min(self.rows.saturating_sub(1)))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Wheel {
    Scrollback(i32),
    Arrows(i32),
    Reports(i32, usize, usize),
}

fn route(lines: i32, mouse: MouseReporting, alt_screen: bool, cell: (usize, usize)) -> Wheel {
    if mouse.reports_buttons() && mouse.sgr {
        Wheel::Reports(wheel_report_lines(lines), cell.0, cell.1)
    } else if mouse.alternate_scroll && alt_screen {
        Wheel::Arrows(lines)
    } else {
        Wheel::Scrollback(lines)
    }
}

fn arrows(lines: i32, app_cursor: bool) -> Vec<u8> {
    let key: &[u8] = match (lines > 0, app_cursor) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    key.repeat(lines.unsigned_abs() as usize)
}

fn scrolls(event: &Event) -> bool {
    matches!(event, Event::MouseWheel { modifiers, .. } if !zooms(*modifiers))
}

fn zooms(modifiers: Modifiers) -> bool {
    modifiers.ctrl || modifiers.command
}

pub(crate) fn drive(
    ui: &egui::Ui,
    grid: &egui::Response,
    term: &mut PtyTerm,
    cells: Cells,
    injected: &[Event],
) {
    let Some(pointer) = grid
        .hover_pos()
        .or_else(|| if injected.is_empty() { None } else { Some(cells.rect.center()) })
    else {
        return;
    };
    let mut events = ui.input(|input| input.events.clone());
    events.extend_from_slice(injected);
    let id = grid.id.with("wheel-carry");
    let mut carry: WheelAccumulator = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    apply(term, &mut carry, &events, pointer, cells);
    ui.data_mut(|data| data.insert_temp(id, carry));
}

fn apply(
    term: &mut PtyTerm,
    carry: &mut WheelAccumulator,
    events: &[Event],
    pointer: Pos2,
    cells: Cells,
) {
    let wheel: Vec<Event> = events.iter().filter(|event| scrolls(event)).cloned().collect();
    let lines = carry.lines(&wheel, cells.cell_h, cells.rows);
    if lines == 0 {
        return;
    }
    match route(lines, term.mouse_reporting(), term.is_alt_screen(), cells.at(pointer)) {
        Wheel::Scrollback(lines) => term.scroll(lines),
        Wheel::Arrows(lines) => term.send(&arrows(lines, term.app_cursor())),
        Wheel::Reports(lines, col, row) => term.send(&wheel_sgr(lines, col, row)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: MouseReporting = MouseReporting {
        report_click: false,
        drag: false,
        motion: false,
        sgr: false,
        alternate_scroll: true,
    };
    const REPORTING: MouseReporting =
        MouseReporting { report_click: true, drag: true, sgr: true, ..OFF };

    #[test]
    fn the_wheel_goes_where_the_screen_and_the_mouse_mode_say() {
        let cases = [
            (OFF, false, Wheel::Scrollback(2)),
            (OFF, true, Wheel::Arrows(2)),
            (MouseReporting { alternate_scroll: false, ..OFF }, true, Wheel::Scrollback(2)),
            (REPORTING, true, Wheel::Reports(2, 4, 5)),
            (REPORTING, false, Wheel::Reports(2, 4, 5)),
            (MouseReporting { sgr: false, ..REPORTING }, true, Wheel::Arrows(2)),
        ];
        for (mouse, alt, want) in cases {
            assert_eq!(route(2, mouse, alt, (4, 5)), want, "{mouse:?} alt={alt}");
        }
    }

    #[test]
    fn a_flick_is_capped_for_an_app_taking_reports_but_not_for_the_scrollback() {
        assert_eq!(route(40, REPORTING, true, (0, 0)), Wheel::Reports(3, 0, 0));
        assert_eq!(route(-40, REPORTING, true, (0, 0)), Wheel::Reports(-3, 0, 0));
        assert_eq!(route(40, OFF, false, (0, 0)), Wheel::Scrollback(40));
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode() {
        let cases = [
            (2, false, &b"\x1b[A\x1b[A"[..]),
            (-1, false, b"\x1b[B"),
            (1, true, b"\x1bOA"),
            (-3, true, b"\x1bOB\x1bOB\x1bOB"),
        ];
        for (lines, app_cursor, want) in cases {
            assert_eq!(arrows(lines, app_cursor), want, "{lines} {app_cursor}");
        }
    }

    #[test]
    fn the_pointer_names_a_cell_inside_the_grid() {
        let cells = Cells {
            rect: Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(800.0, 480.0)),
            cell_w: 10.0,
            cell_h: 20.0,
            cols: 80,
            rows: 24,
        };
        assert_eq!(cells.at(Pos2::new(105.0, 55.0)), (0, 0));
        assert_eq!(cells.at(Pos2::new(155.0, 135.0)), (5, 4));
        assert_eq!(cells.at(Pos2::new(5000.0, 5000.0)), (79, 23));
    }

    #[test]
    fn ctrl_and_cmd_wheel_is_zoom_and_not_a_scroll() {
        let wheel = |modifiers| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers,
        };
        assert!(scrolls(&wheel(Modifiers::NONE)));
        assert!(scrolls(&wheel(Modifiers::SHIFT)));
        assert!(!scrolls(&wheel(Modifiers::CTRL)));
        assert!(!scrolls(&wheel(Modifiers::COMMAND)));
    }
}

#[cfg(test)]
mod frames {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use egui::{Context, Id, MouseWheelUnit, TouchPhase};
    use stdusk_core::repaint::Repaint;
    use stdusk_core::shell::ShellProfile;
    use stdusk_core::terminal::SpawnOpts;
    use stdusk_egui::ui::{CursorStyle, GridStyle, collect_input, render_grid};

    use super::*;
    use crate::terminal_focus;

    const CELL: (f32, f32) = (8.0, 16.0);

    struct Rig {
        ctx: Context,
        term: PtyTerm,
        grid: Option<Id>,
        typed: Vec<u8>,
        dir: tempfile::TempDir,
    }

    impl Rig {
        fn new(script: &str) -> Self {
            let dir = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
            let script = script
                .replace("OUT", &dir.path().join("got").display().to_string())
                .replace("READY", &dir.path().join("ready").display().to_string());
            let opts = SpawnOpts {
                detect_progress: false,
                shell_integration: false,
                autosuggestions: false,
                scrollback_lines: 2000,
                word_separators: " ".into(),
                bold_bright: false,
                agent_tracking: None,
                cwd: None,
                profile: Some(ShellProfile {
                    shell: Some("/bin/sh".into()),
                    args: vec!["-c".into(), script],
                    cwd: None,
                    env: BTreeMap::new(),
                }),
            };
            let mut term = PtyTerm::spawn(75, 25, Repaint::none(), &opts);
            term.resize(75, 25);
            Self { ctx: Context::default(), term, grid: None, typed: Vec::new(), dir }
        }

        fn area() -> Rect {
            Rect::from_min_size(Pos2::new(0.0, 100.0), egui::vec2(600.0, 400.0))
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                focused: true,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let _ = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    self.typed = collect_input(ui, false, false, self.term.app_cursor());
                    let snap = self.term.grid_snapshot();
                    let response = render_grid(
                        ui,
                        &[],
                        Self::area(),
                        &self.term,
                        &snap,
                        CELL.0,
                        CELL.1,
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
                    terminal_focus::hold(ui.ctx(), response.id, true);
                    let cells = Cells {
                        rect: Self::area(),
                        cell_w: CELL.0,
                        cell_h: CELL.1,
                        cols: 75,
                        rows: 25,
                    };
                    drive(ui, &response, &mut self.term, cells, &[]);
                });
            });
        }

        fn hover(&mut self) {
            self.frame(vec![Event::PointerMoved(Self::area().center())]);
            self.frame(vec![]);
        }

        fn wheel(&mut self, unit: MouseWheelUnit, lines: f32) {
            let event = Event::MouseWheel {
                unit,
                delta: egui::vec2(0.0, lines),
                phase: TouchPhase::Move,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![event]);
        }

        fn wait_for(&self, what: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !what(self) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(what(self), "the terminal never got there");
        }

        fn ready(&self) -> bool {
            self.dir.path().join("ready").exists()
        }

        fn received(&self, len: usize) -> Vec<u8> {
            let out = self.dir.path().join("got");
            self.wait_for(|_| std::fs::read(&out).is_ok_and(|got| got.len() >= len));
            std::fs::read(&out).unwrap()
        }

        fn focused(&self) -> Option<Id> {
            self.ctx.memory(egui::Memory::focused)
        }
    }

    fn with_history() -> Rig {
        let mut rig = Rig::new("seq 1 500; sleep 20");
        rig.wait_for(|rig| rig.term.scroll_state().1 > 400);
        rig.hover();
        rig
    }

    fn raw_app(setup: &str, len: usize) -> Rig {
        let script = format!("{setup}; stty raw -echo; : > READY; head -c {len} > OUT; sleep 20");
        let mut rig = Rig::new(&script);
        rig.wait_for(Rig::ready);
        rig.hover();
        rig
    }

    #[test]
    fn the_wheel_scrolls_the_scrollback_of_a_shell_and_the_grid_keeps_the_keyboard() {
        let mut rig = with_history();
        assert_eq!(rig.term.scroll_state().0, 0);
        assert_eq!(rig.focused(), rig.grid);
        rig.wheel(MouseWheelUnit::Line, 3.0);
        assert_eq!(rig.term.scroll_state().0, 3);
        rig.wheel(MouseWheelUnit::Line, -1.0);
        assert_eq!(rig.term.scroll_state().0, 2);
        assert_eq!(rig.focused(), rig.grid);
        assert_eq!(rig.typed, b"");
    }

    #[test]
    fn a_trackpad_swipe_in_points_adds_up_across_frames() {
        let mut rig = with_history();
        for _ in 0..4 {
            rig.wheel(MouseWheelUnit::Point, CELL.1 / 2.0);
        }
        assert_eq!(rig.term.scroll_state().0, 2);
        rig.wheel(MouseWheelUnit::Page, 1.0);
        assert_eq!(rig.term.scroll_state().0, 27);
    }

    #[test]
    fn the_wheel_does_nothing_while_the_pointer_is_elsewhere() {
        let mut rig = with_history();
        rig.frame(vec![Event::PointerMoved(Pos2::new(700.0, 20.0))]);
        rig.frame(vec![]);
        rig.wheel(MouseWheelUnit::Line, 3.0);
        assert_eq!(rig.term.scroll_state().0, 0);
    }

    #[test]
    fn an_alternate_screen_app_gets_arrow_keys() {
        let mut rig = raw_app("printf '\\033[?1049h'", 9);
        rig.wait_for(|rig| rig.term.is_alt_screen());
        rig.wheel(MouseWheelUnit::Line, 2.0);
        rig.wheel(MouseWheelUnit::Line, -1.0);
        assert_eq!(rig.received(9), b"\x1b[A\x1b[A\x1b[B");
        assert_eq!(rig.term.scroll_state().0, 0);
        assert_eq!(rig.focused(), rig.grid);
    }

    #[test]
    fn an_app_that_turned_on_mouse_reporting_gets_sgr_wheel_events_at_the_pointer() {
        let mut rig = raw_app("printf '\\033[?1049h\\033[?1000h\\033[?1006h'", 24);
        rig.wait_for(|rig| rig.term.mouse_reporting().sgr);
        rig.wheel(MouseWheelUnit::Line, 1.0);
        rig.wheel(MouseWheelUnit::Line, -1.0);
        assert_eq!(rig.received(24), b"\x1b[<64;38;13M\x1b[<65;38;13M");
        assert_eq!(rig.focused(), rig.grid);
    }

    #[test]
    fn mouse_reporting_in_the_normal_screen_still_gets_the_wheel() {
        let mut rig = raw_app("printf '\\033[?1002h\\033[?1006h'", 12);
        rig.wait_for(|rig| rig.term.mouse_reporting().sgr);
        rig.wheel(MouseWheelUnit::Line, 1.0);
        assert_eq!(rig.received(12), b"\x1b[<64;38;13M");
    }
}
