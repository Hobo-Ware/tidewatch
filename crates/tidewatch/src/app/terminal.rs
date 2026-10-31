//! The terminal area of the active tab: shell grid, status messages and the exit notice.

use egui::FontId;
use stdusk_core::config::TerminalConfig;
use stdusk_core::paste::PasteRules;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::mouse::GridGeom;
use stdusk_egui::paste_confirm::{self, Verdict};
use stdusk_egui::pointer::MousePasted;
use stdusk_egui::terminal_view::{
    Focus, Gate, TerminalOptions, TerminalOutput, TerminalView, paste_into,
};
use stdusk_egui::ui::GridStyle;

use super::frame::FrameIntent;
use super::{FONT_SIZE, Tidewatch, wall_ms};
use crate::brand;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::pane::{Body, Pane};
use crate::reader::{Note, Shown};
use crate::reader_view::Action;
use crate::screen_kit::paint_pill_card;
use crate::stopped_view;
use crate::terminal_font::InUse;
use crate::{layout, reader_glue};
use tidewatch_core::status::AgentStatus;

const CAUGHT_H: f32 = 72.0;
const CAUGHT_GAP: f32 = 6.0;

pub(super) struct TerminalGrid {
    rect: egui::Rect,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    font: FontId,
    bold: Option<FontId>,
}

pub(super) fn terminal_config(copy_on_select: bool) -> TerminalConfig {
    TerminalConfig { copy_on_select, ..TerminalConfig::default() }
}

pub(super) fn paste_rules(config: &TerminalConfig) -> PasteRules {
    TerminalOptions::from_config(config, Focus::Off).paste
}

pub(super) fn view_gate(keys_free: bool, modal_open: bool) -> Gate {
    Gate {
        keyboard: keys_free,
        paste: keys_free,
        copy: keys_free,
        wheel: !modal_open,
        reports: !modal_open,
        pointer: !modal_open,
    }
}

pub(super) fn show_shell(
    ui: &mut egui::Ui,
    term: &mut PtyTerm,
    grid: &TerminalGrid,
    config: &TerminalConfig,
    keys_free: bool,
    modal_open: bool,
    injected_wheel: &[egui::Event],
) -> TerminalOutput {
    let mods = ui.input(|i| i.modifiers);
    TerminalView {
        id_src: &[],
        grid: grid.geom(),
        font: &grid.font,
        bold_font: grid.bold.as_ref(),
        style: GridStyle::from_config(config, mods, keys_free),
        search_marks: &[],
        options: TerminalOptions::from_config(config, Focus::Manage),
        gate: view_gate(keys_free, modal_open),
        injected_wheel,
    }
    .show(ui, term)
}

impl TerminalGrid {
    pub(super) fn resize(&self, term: &mut PtyTerm) {
        term.resize(self.cols, self.rows);
    }

    fn geom(&self) -> GridGeom {
        GridGeom {
            rect: self.rect,
            cw: self.cell_w,
            ch: self.cell_h,
            cols: self.cols,
            rows: self.rows,
        }
    }

    pub(super) fn measure(ui: &egui::Ui, rect: egui::Rect, in_use: &InUse) -> Self {
        let (font, bold) = in_use.font_ids(FONT_SIZE);
        let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), brand::TEXT);
        let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
        let (cols, rows) = layout::grid_dims(rect, cell_w, cell_h);
        Self { rect, cols, rows, cell_w, cell_h, font, bold }
    }
}

impl Tidewatch {
    pub(super) fn show_terminal(
        &mut self,
        ui: &mut egui::Ui,
        grid: &TerminalGrid,
        agent_name: Option<&str>,
        reader_on: bool,
        frame: &FrameIntent,
    ) {
        let area = grid.rect;
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let now = ui.ctx().input(|i| i.time);
        let dev_wheel = if reader_on { Vec::new() } else { self.take_dev_wheel(ui.ctx(), now) };
        let mut copied = false;
        let mut mouse_pasted = None;
        let Pane { body, agent, .. } = self.tabs.active_pane_mut();
        match body {
            Body::Shell(term) => {
                term.resize(grid.cols, grid.rows);
                if !reader_on {
                    let keys_free = !frame.keys.modal_open && !frame.keys.typing_in_a_panel;
                    let out = show_shell(
                        ui,
                        term,
                        grid,
                        &self.terminal_config,
                        keys_free,
                        frame.keys.modal_open,
                        &dev_wheel,
                    );
                    for text in out.confirm_paste.iter().cloned() {
                        self.pending_pastes.push_back((active_id, text));
                    }
                    if out.bell_rang {
                        self.bell.ring(now);
                    }
                    mouse_pasted = out.mouse_pasted;
                    if out.sent_input()
                        && let Some(agent) = agent
                    {
                        agent.resume_note = None;
                        agent.base_note = None;
                    }
                    copied = out.copied.is_some();
                }
                if let (Some(name), Some(exit)) = (agent_name, term.exited()) {
                    exit_notice(ui, area, &exit_notice_text(name, exit.code));
                } else {
                    let note = agent
                        .as_ref()
                        .and_then(|agent| agent.resume_note.or(agent.base_note.as_deref()));
                    let pill = note.map(|note| pane_notice(ui, area, note));
                    let mut above = pill;
                    if let Some(agent) = agent
                        .as_mut()
                        .filter(|agent| !reader_on && *agent.run.status() == AgentStatus::Stopped)
                    {
                        let bottom = pill.map_or(area.bottom(), |pill| pill.top());
                        let band = egui::Rect::from_min_max(
                            egui::pos2(area.left(), bottom - stopped_view::HEIGHT),
                            egui::pos2(area.right(), bottom),
                        );
                        if stopped_view::show(ui, band) {
                            reader_glue::resume_in(term, agent, now);
                        }
                        above = Some(band);
                    }
                    if let Some(above) = above {
                        caught_above(ui, area, above);
                    }
                }
            }
            Body::Draft(_) | Body::Poofed(_) => {}
            Body::Creating => {
                status_message(ui, area, "Creating worktree...", brand::MUTED);
            }
            Body::Removing => status_message(ui, area, "Removing worktree...", brand::MUTED),
            Body::Failed(message) => status_message(ui, area, message, brand::RED),
        }
        self.bell.paint(ui, area, now);
        if copied {
            self.show_toast("Copied", ui.ctx());
        }
        match mouse_pasted {
            Some(MousePasted::Text) => self.show_toast("Pasted", ui.ctx()),
            Some(MousePasted::Image) => self.show_toast("Pasted image", ui.ctx()),
            None => {}
        }
    }

    pub(super) fn show_paste_confirm(&mut self, ctx: &egui::Context) {
        let Some((tab, text)) = self.pending_pastes.pop_front() else { return };
        match paste_confirm::show(ctx, &text, true) {
            Verdict::Paste => {
                if let Some(Body::Shell(term)) =
                    self.tabs.get_mut(tab).map(|tab| &mut tab.pane.body)
                {
                    paste_into(term, &text);
                }
            }
            Verdict::Cancel => {}
            Verdict::Asking => self.pending_pastes.push_front((tab, text)),
        }
    }

    fn offer_slash_commands(&mut self, ctx: &egui::Context) {
        let dir = self.workspace().dir().to_path_buf();
        let claude_dir = self.paths.as_ref().map(|paths| paths.claude_dir().to_path_buf());
        let pane = self.tabs.active_pane_mut();
        let folder = pane.worktree.as_ref().map_or(dir, |worktree| worktree.path.clone());
        let Some(agent) = pane.agent.as_mut() else { return };
        if agent.run.agent_id != "claude" || !agent.reader.view.reply.starts_with('/') {
            return;
        }
        agent.reader.view.commands = self.slash.entries(ctx, claude_dir.as_deref(), &folder);
    }

    pub(super) fn show_reader(
        &mut self,
        ui: &mut egui::Ui,
        terminal: egui::Rect,
        shown: Option<Shown>,
        ctx: &egui::Context,
        now: f64,
        active_id: u64,
        keys_free: bool,
    ) {
        match shown {
            Some(Shown::Reader) => {
                self.offer_slash_commands(ctx);
                let parts = self.review_parts(active_id);
                let notes = self.views.get(&active_id).map(|view| &view.notes);
                let review =
                    parts.as_ref().zip(notes).map(|(parts, notes)| parts.facts(notes, keys_free));
                let profile =
                    self.tabs.active_pane_mut().agent.as_ref().map(|a| a.profile_name.clone());
                let data = self.pr.data(
                    active_id,
                    profile.as_deref().unwrap_or_default(),
                    wall_ms() as i64,
                );
                let ask = self.ask_of(active_id, keys_free);
                let due = self.due_of(active_id);
                let first = due.first().map(|item| (item, due.len() - 1));
                let avatar = self.avatar_live.get(active_id).cloned();
                let pane = self.tabs.active_pane_mut();
                let mut asked = reader_glue::draw(
                    ui,
                    terminal,
                    active_id,
                    pane,
                    &mut self.dev_reader,
                    now,
                    review,
                    (data, first),
                    ask.as_ref(),
                    avatar,
                );
                let clicks = reader_glue::take_pr_clicks(&mut asked);
                let due_clicks = reader_glue::take_due_clicks(&mut asked);
                let picks = reader_glue::take_picks(&mut asked);
                let rows_clicked = reader_glue::take_tidewatch_clicks(&mut asked);
                let (reviews, asked): (Vec<_>, Vec<_>) =
                    asked.into_iter().partition(|action| matches!(action, Action::Review(_)));
                let toast =
                    reader_glue::apply(asked, &mut self.tabs, active_id, &mut self.flow, ctx, now);
                if let Some(text) = toast {
                    self.show_toast(text, ctx);
                }
                let reviews = reviews
                    .into_iter()
                    .filter_map(|action| match action {
                        Action::Review(review) => Some(review),
                        _ => None,
                    })
                    .collect();
                self.apply_review(reviews, active_id, ctx);
                self.apply_pr_actions(active_id, clicks, ctx, now);
                self.apply_tidewatch_clicks(active_id, rows_clicked, ctx, now);
                self.apply_due_clicks(active_id, due_clicks, ctx, now);
                for text in picks {
                    self.answer_ask(active_id, text, ctx, now);
                }
            }
            Some(Shown::Terminal(Some(note))) => {
                let opened = self
                    .tabs
                    .active_pane_mut()
                    .agent
                    .as_ref()
                    .and_then(|agent| agent.reader.opened().map(str::to_owned));
                match opened {
                    Some(command) if note == Note::ChosenRaw => {
                        if reader_glue::opened_note(ui, terminal, &command)
                            && let Some(agent) = self.tabs.active_pane_mut().agent.as_mut()
                        {
                            agent.reader.set_wants_reader(true);
                            agent.reader.view.request_reply_focus();
                        }
                    }
                    _ => reader_glue::note(ui, terminal, note),
                }
            }
            Some(Shown::Terminal(None)) | None => {}
        }
    }
}

fn caught_above(ui: &egui::Ui, area: egui::Rect, pill: egui::Rect) {
    let room = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, pill.top() - CAUGHT_GAP));
    if room.height() < CAUGHT_H * 2.0 {
        return;
    }
    let size = egui::vec2(mascot::width_at(Pose::Caught, CAUGHT_H), CAUGHT_H);
    let at = egui::Rect::from_center_size(
        egui::pos2(pill.center().x, room.bottom() - size.y / 2.0),
        size,
    );
    mascot::show(ui, at, Pose::Caught);
}

fn status_message(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout(
        text.to_owned(),
        FontId::monospace(FONT_SIZE),
        color,
        (rect.width() * 0.8).max(1.0),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

fn exit_notice_text(profile_name: &str, code: i32) -> String {
    format!("{profile_name} exited with code {code}. {} closes the tab.", Command::CloseTab.keys())
}

fn exit_notice(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), brand::TEXT);
    let pill = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - 28.0),
        galley.size() + egui::vec2(24.0, 14.0),
    );
    paint_pill_card(&painter, pill);
    painter.galley(pill.center() - galley.size() / 2.0, galley, brand::TEXT);
}

/// A small card along the bottom edge of the pane, drawn over the grid and never sent to the pty.
/// Long text wraps. Returns the card's rectangle.
fn pane_notice(ui: &egui::Ui, pane: egui::Rect, text: &str) -> egui::Rect {
    let (pad, margin) = (egui::vec2(10.0, 5.0), 8.0);
    let wrap_width = (pane.width() - 2.0 * (margin + pad.x)).max(60.0);
    let galley =
        ui.painter().layout(text.to_owned(), FontId::proportional(12.0), brand::MUTED, wrap_width);
    let size = galley.size() + pad * 2.0;
    let pill = egui::Rect::from_min_size(
        egui::pos2(pane.center().x - size.x / 2.0, pane.bottom() - margin - size.y),
        size,
    );
    let painter = ui.painter_at(pane);
    paint_pill_card(&painter, pill);
    painter.galley(pill.min + pad, galley, brand::MUTED);
    pill
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use egui::{Context, Event, Key, Modifiers, OutputCommand, Pos2, Rect};
    use stdusk_core::paste::prepare;
    use stdusk_core::repaint::Repaint;
    use stdusk_core::shell::ShellProfile;
    use stdusk_core::terminal::{PtyTerm, SpawnOpts};
    use stdusk_egui::terminal_view::{paste_into, send_typed};

    use super::*;
    use crate::screen_kit::{esc_left_a_widget, remember_focus};

    fn spawn(script: &str) -> PtyTerm {
        let opts = SpawnOpts {
            detect_progress: false,
            shell_integration: false,
            autosuggestions: false,
            scrollback_lines: 1000,
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
        let mut term = PtyTerm::spawn(75, 25, Repaint::none(), &opts);
        term.resize(75, 25);
        term
    }

    fn wait_for(what: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !what() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(what(), "the terminal never got there");
    }

    #[test]
    fn typing_or_pasting_jumps_back_to_the_bottom_of_the_scrollback() {
        let mut term = spawn("seq 1 300; sleep 20");
        wait_for(|| term.scroll_state().1 > 200);
        for typing in [true, false] {
            term.scroll(40);
            assert_eq!(term.scroll_state().0, 40);
            if typing {
                send_typed(&mut term, b"a");
            } else {
                paste_into(&mut term, "text");
            }
            assert_eq!(term.scroll_state().0, 0);
        }
    }

    #[test]
    fn a_paste_is_prepared_with_line_breaks_as_cr_and_no_control_bytes() {
        let cases = [
            ("one\r\ntwo\nthree", Some("one\rtwo\rthree")),
            ("a\x1b[201~b\x03c", Some("a[201~bc")),
            ("  padded  \n", Some("padded")),
            ("  a\n  b\n\n", Some("  a\r  b")),
            ("tab\tstays", Some("tab\tstays")),
            ("naive \u{e9} text \u{1f600}", Some("naive \u{e9} text \u{1f600}")),
            ("", None),
            (" \r\n\t ", None),
        ];
        for (text, want) in cases {
            assert_eq!(
                prepare(text, paste_rules(&terminal_config(false))).as_deref(),
                want,
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_modal_takes_the_keys_and_the_pointer_but_a_text_panel_only_the_keys() {
        let free = view_gate(true, false);
        assert!(
            free.keyboard && free.paste && free.copy && free.wheel && free.reports && free.pointer
        );
        let typing = view_gate(false, false);
        assert!(
            !typing.keyboard
                && !typing.paste
                && !typing.copy
                && typing.wheel
                && typing.reports
                && typing.pointer
        );
        let modal = view_gate(false, true);
        assert!(
            !modal.keyboard
                && !modal.paste
                && !modal.copy
                && !modal.wheel
                && !modal.reports
                && !modal.pointer
        );
    }

    struct Screen {
        ctx: Context,
        term: PtyTerm,
        typed: Vec<u8>,
        pasted: Vec<String>,
        confirm: Vec<String>,
        copied: Option<String>,
        clipboard: Vec<String>,
        grid: Option<egui::Id>,
        config: TerminalConfig,
        cursor: egui::CursorIcon,
        commands: Vec<Command>,
    }

    impl Screen {
        fn new(script: &str) -> Self {
            Self {
                ctx: Context::default(),
                term: spawn(script),
                typed: Vec::new(),
                pasted: Vec::new(),
                confirm: Vec::new(),
                copied: None,
                clipboard: Vec::new(),
                grid: None,
                config: TerminalConfig { clickable_links: false, ..terminal_config(false) },
                cursor: egui::CursorIcon::Default,
                commands: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                modifiers,
                focused: true,
                ..Default::default()
            };
            let ctx = self.ctx.clone();
            let font = FontId::monospace(14.0);
            let full = ctx.run_ui(raw, |ui| {
                self.commands =
                    crate::commands::pressed(ui.ctx(), crate::commands::Gate::default());
                egui::CentralPanel::default().show(ui, |ui| {
                    let grid = TerminalGrid {
                        rect: Rect::from_min_size(Pos2::new(0.0, 100.0), egui::vec2(600.0, 400.0)),
                        cols: 75,
                        rows: 25,
                        cell_w: 8.0,
                        cell_h: 16.0,
                        font: font.clone(),
                        bold: None,
                    };
                    let out = show_shell(ui, &mut self.term, &grid, &self.config, true, false, &[]);
                    self.grid = Some(out.response.id);
                    self.typed = out.typed;
                    self.pasted = out.pasted;
                    self.confirm = out.confirm_paste;
                    self.copied = out.copied;
                });
                remember_focus(ui.ctx());
            });
            self.cursor = full.platform_output.cursor_icon;
            for command in full.platform_output.commands {
                if let OutputCommand::CopyText(text) = command {
                    self.clipboard.push(text);
                }
            }
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame(vec![], Modifiers::NONE);
            }
        }

        fn click(&mut self, pos: Pos2, pressed: bool) {
            let button = Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![button], Modifiers::NONE);
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)], Modifiers::NONE);
            self.click(from, true);
            let steps = ((to.x - from.x).abs() / 3.0).ceil().max(1.0) as usize;
            for step in 1..=steps {
                let pos = from + (to - from) * (step as f32 / steps as f32);
                self.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
            }
            self.click(to, false);
            self.frame(vec![], Modifiers::NONE);
        }

        fn with_text() -> Self {
            let mut screen = Self::new("printf 'hello world\\n'; sleep 20");
            wait_for(|| {
                let snap = screen.term.grid_snapshot();
                snap.cells.iter().take(11).map(|cell| cell.c).collect::<String>() == "hello world"
            });
            screen.settle();
            screen.term.start_selection(screen.term.grid_snapshot().top_line, 0, false);
            screen.term.update_selection(screen.term.grid_snapshot().top_line, 10, true);
            screen
        }
    }

    fn copy_chord() -> (Event, Modifiers) {
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers::COMMAND
        } else {
            Modifiers::CTRL | Modifiers::SHIFT
        };
        (Event::Copy, modifiers)
    }

    fn ctrl_c() -> (Event, Modifiers) {
        if cfg!(target_os = "macos") {
            let key = Event::Key {
                key: Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL,
            };
            (key, Modifiers::CTRL)
        } else {
            (Event::Copy, Modifiers::CTRL)
        }
    }

    #[test]
    fn the_copy_chord_puts_the_selected_text_on_the_clipboard() {
        let mut screen = Screen::with_text();
        let (event, modifiers) = copy_chord();
        screen.frame(vec![event], modifiers);
        assert_eq!(screen.copied.as_deref(), Some("hello world"));
        assert_eq!(screen.clipboard, ["hello world"]);
        assert_eq!(screen.typed, b"");
    }

    fn cell(col: f32) -> Pos2 {
        Pos2::new(col * 8.0 + 4.0, 108.0)
    }

    #[test]
    fn a_finished_drag_is_copied_when_copy_on_select_is_on() {
        let mut screen = Screen::with_text();
        screen.term.clear_selection();
        screen.config.copy_on_select = true;
        screen.drag(cell(0.0), cell(10.0));
        assert_eq!(screen.clipboard.len(), 1);
        assert!(screen.clipboard[0].contains("llo wor"), "{:?}", screen.clipboard);
    }

    #[test]
    fn a_finished_drag_is_not_copied_when_copy_on_select_is_off() {
        let mut screen = Screen::with_text();
        screen.term.clear_selection();
        screen.drag(cell(0.0), cell(10.0));
        assert!(screen.term.selection_text().is_some());
        assert_eq!(screen.clipboard.len(), 0);
    }

    #[test]
    fn a_double_click_on_a_word_is_copied_when_copy_on_select_is_on() {
        let mut screen = Screen::with_text();
        screen.term.clear_selection();
        screen.config.copy_on_select = true;
        screen.frame(vec![Event::PointerMoved(cell(7.0))], Modifiers::NONE);
        for _ in 0..2 {
            screen.click(cell(7.0), true);
            screen.click(cell(7.0), false);
        }
        assert_eq!(screen.clipboard.last().map(String::as_str), Some("world"));
    }

    #[test]
    fn ctrl_c_copies_and_clears_a_selection_and_is_ctrl_c_without_one() {
        let mut screen = Screen::with_text();
        screen.term.clear_selection();
        let (event, modifiers) = ctrl_c();
        screen.frame(vec![event], modifiers);
        assert_eq!(screen.typed, b"\x03");
        assert_eq!(screen.copied, None);
        assert_eq!(screen.clipboard.len(), 0);

        let mut screen = Screen::with_text();
        let (event, modifiers) = ctrl_c();
        screen.frame(vec![event], modifiers);
        assert_eq!(screen.typed, b"");
        assert_eq!(screen.copied.as_deref(), Some("hello world"));
        assert_eq!(screen.clipboard, ["hello world"]);
        assert_eq!(screen.term.selection_text(), None);
    }

    #[test]
    fn tab_reaches_the_shell_and_esc_is_left_for_the_app() {
        let mut screen = Screen::new("sleep 20");
        screen.settle();
        assert_eq!(screen.ctx.memory(egui::Memory::focused), screen.grid);
        for (key, modifiers, bytes) in [
            (Key::Tab, Modifiers::NONE, &b"\t"[..]),
            (Key::Tab, Modifiers::SHIFT, b"\x1b[Z"),
            (Key::ArrowUp, Modifiers::NONE, b"\x1b[A"),
            (Key::Escape, Modifiers::NONE, b"\x1b"),
        ] {
            let event =
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            screen.frame(vec![event], modifiers);
            assert_eq!(screen.typed, bytes, "{key:?}");
            assert_eq!(screen.ctx.memory(egui::Memory::focused), screen.grid, "{key:?}");
        }
        let ctx = screen.ctx.clone();
        let escape = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let raw = egui::RawInput { events: vec![escape], ..Default::default() };
        let mut taken = true;
        let _ = ctx.run_ui(raw, |_| taken = esc_left_a_widget(&ctx, false));
        assert!(!taken);
    }

    #[test]
    fn a_multiline_paste_waits_for_the_paste_question_and_a_single_line_goes_through() {
        let mut screen = Screen::new("sleep 20");
        screen.settle();
        screen.frame(vec![Event::Paste("one\ntwo".into())], Modifiers::NONE);
        assert_eq!(screen.confirm, ["one\rtwo"]);
        assert_eq!(screen.pasted.len(), 0);
        screen.frame(vec![Event::Paste("one line".into())], Modifiers::NONE);
        assert_eq!(screen.confirm.len(), 0);
        assert_eq!(screen.pasted, ["one line"]);
    }

    fn with_link() -> Screen {
        let mut screen = Screen::new("printf 'see https://example.com/page now\\n'; sleep 20");
        wait_for(|| {
            let snap = screen.term.grid_snapshot();
            snap.cells.iter().take(32).map(|cell| cell.c).collect::<String>()
                == "see https://example.com/page now"
        });
        screen.config.clickable_links = true;
        screen.settle();
        screen
    }

    fn hovering_a_link(screen: &mut Screen, col: f32, modifiers: Modifiers) -> bool {
        screen.frame(vec![Event::PointerMoved(cell(col))], modifiers);
        screen.frame(vec![], modifiers);
        screen.cursor == egui::CursorIcon::PointingHand
    }

    #[test]
    fn the_shell_terminal_marks_a_link_under_cmd_hover_and_not_other_text() {
        let mut screen = with_link();
        assert!(hovering_a_link(&mut screen, 12.0, Modifiers::COMMAND));
        assert!(!hovering_a_link(&mut screen, 1.0, Modifiers::COMMAND));
    }

    #[test]
    fn the_shell_terminal_follows_stdusks_link_settings() {
        let mut screen = with_link();
        assert!(hovering_a_link(&mut screen, 12.0, Modifiers::NONE));
        screen.config.link_modifier = "cmd".into();
        assert!(!hovering_a_link(&mut screen, 12.0, Modifiers::NONE));
        assert!(hovering_a_link(&mut screen, 12.0, Modifiers::COMMAND));
        screen.config.clickable_links = false;
        assert!(!hovering_a_link(&mut screen, 12.0, Modifiers::COMMAND));
    }

    #[test]
    fn the_config_tidewatch_hands_the_view_is_stdusks_default_but_for_copy_on_select() {
        let stock = TerminalConfig::default();
        assert_eq!(
            serde_json::to_string(&terminal_config(stock.copy_on_select)).unwrap(),
            serde_json::to_string(&stock).unwrap(),
        );
        assert!(terminal_config(true).copy_on_select);
        assert!(!terminal_config(false).copy_on_select);
    }

    fn raw_screen(setup: &str, len: usize) -> (Screen, std::path::PathBuf) {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("tidewatch-keys-{}-{call}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (ready, out) = (dir.join("ready"), dir.join("got"));
        let script = format!(
            "{setup}; stty raw -echo; : > {}; head -c {len} > {}; sleep 20",
            ready.display(),
            out.display()
        );
        let mut screen = Screen::new(&script);
        wait_for(|| ready.exists());
        screen.settle();
        (screen, out)
    }

    fn received(out: &std::path::Path, len: usize) -> Vec<u8> {
        wait_for(|| std::fs::read(out).is_ok_and(|got| got.len() >= len));
        std::fs::read(out).unwrap()
    }

    fn click(screen: &mut Screen, pos: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        screen.frame(vec![Event::PointerMoved(pos)], Modifiers::NONE);
        screen.frame(vec![button(true)], Modifiers::NONE);
        screen.frame(vec![button(false)], Modifiers::NONE);
    }

    fn at_cell(col: usize, row: usize) -> Pos2 {
        Pos2::new(col as f32 * 8.0 + 4.0, 100.0 + row as f32 * 16.0 + 8.0)
    }

    #[test]
    fn a_click_reaches_a_program_with_mouse_reporting_in_every_tracking_mode() {
        for mode in ["1000", "1002", "1003"] {
            let setup = format!("printf '\\033[?{mode}h\\033[?1006h'");
            let (mut screen, out) = raw_screen(&setup, if mode == "1003" { 28 } else { 18 });
            wait_for(|| screen.term.mouse_reporting().sgr);
            screen.frame(vec![Event::PointerMoved(at_cell(3, 2))], Modifiers::NONE);
            click(&mut screen, at_cell(3, 2));
            let want: &[u8] = b"\x1b[<0;4;3M\x1b[<0;4;3m";
            let got = if mode == "1003" { 28 } else { 18 };
            let all = received(&out, got);
            assert!(all.ends_with(want), "{mode} {:?}", String::from_utf8_lossy(&all));
        }
    }

    #[test]
    fn a_click_still_reaches_the_program_after_the_wheel_scrolled_it() {
        let (mut screen, out) = raw_screen("printf '\\033[?1049h\\033[?1000h\\033[?1006h'", 28);
        wait_for(|| screen.term.mouse_reporting().sgr);
        screen.frame(vec![Event::PointerMoved(at_cell(3, 2))], Modifiers::NONE);
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        screen.frame(vec![wheel], Modifiers::NONE);
        click(&mut screen, at_cell(3, 2));
        assert_eq!(received(&out, 28), b"\x1b[<64;4;3M\x1b[<0;4;3M\x1b[<0;4;3m");
    }

    #[test]
    fn a_click_inside_one_frame_still_reports_press_then_release() {
        let (mut screen, out) = raw_screen("printf '\\033[?1000h\\033[?1006h'", 18);
        wait_for(|| screen.term.mouse_reporting().sgr);
        let button = |pressed| Event::PointerButton {
            pos: at_cell(0, 0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        screen.frame(vec![button(true), button(false)], Modifiers::NONE);
        assert_eq!(received(&out, 18), b"\x1b[<0;1;1M\x1b[<0;1;1m");
    }

    fn line_editing_bytes(title: &str, platform: crate::commands::Platform) -> &'static [u8] {
        use crate::commands::Platform::{Linux, Mac};
        match (title, platform) {
            ("Back one word", _) => b"\x1bb",
            ("Forward one word", _) => b"\x1bf",
            ("Start of the line", Mac) => b"\x01",
            ("Start of the line", Linux) => b"\x1b[H",
            ("End of the line", Mac) => b"\x05",
            ("End of the line", Linux) => b"\x1b[F",
            ("Delete the previous word", _) => b"\x1b\x7f",
            ("Delete to the start of the line", _) => b"\x15",
            _ => b"unlisted",
        }
    }

    fn press_in_the_terminal(event: Event) -> Screen {
        let Event::Key { modifiers, .. } = event else { unreachable!("a key press") };
        let mut screen = Screen::new("sleep 20");
        screen.settle();
        screen.frame(vec![event], modifiers);
        screen
    }

    #[test]
    fn every_chord_of_the_table_reaches_its_command_and_not_the_shell() {
        let gate = crate::commands::Gate::default();
        for entry in crate::commands::TABLE {
            let Some(event) = crate::commands::press_event(entry.command) else { continue };
            let screen = press_in_the_terminal(event);
            let want: &[Command] = if gate.allows(entry.command) { &[entry.command] } else { &[] };
            assert_eq!(screen.commands, want, "{}", entry.id);
            if !want.is_empty() {
                assert_eq!(screen.typed, b"", "{} leaked to the shell", entry.id);
            }
        }
    }

    #[test]
    fn the_line_editing_keys_the_page_lists_reach_the_shell_as_their_bytes() {
        let platform = crate::commands::Platform::CURRENT;
        for key in crate::commands::terminal_keys() {
            let screen = press_in_the_terminal(key.chord_on(platform).press_event(platform));
            assert_eq!(screen.commands, [], "{} was taken by the app", key.title);
            assert_eq!(screen.typed, line_editing_bytes(key.title, platform), "{}", key.title);
        }
    }

    #[test]
    fn the_exit_notice_names_the_profile_and_the_code() {
        assert_eq!(
            exit_notice_text("Builder", 130),
            format!("Builder exited with code 130. {} closes the tab.", Command::CloseTab.keys())
        );
    }
}
