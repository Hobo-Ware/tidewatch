//! The main tab's split terminal in the app: the panes and their dividers drawn, the keys that
//! split, move focus and close a pane, and a saved layout brought back with fresh shells.

use std::path::Path;

use egui::{CursorIcon, Sense, Stroke, StrokeKind, vec2};
use stdusk_core::config::TerminalConfig;
use stdusk_core::pane::{self, Dir, SplitDir};
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::terminal_view::{Focus, TerminalOutput};
use tidewatch_core::session::SavedSplit;

use super::Tidewatch;
use super::terminal::{TerminalGrid, show_shell_in};
use crate::brand;
use crate::handoff::GroupSplit;
use crate::layout::TERMINAL_PAD;
use crate::pane::Body;
use crate::pty::spawn_shell;
use crate::split::{Divider, Splits};

const GRAB: f32 = 2.0;

/// The area the panes tile: the grid's rect and the margin a lone terminal keeps around it.
fn split_area(grid: egui::Rect) -> egui::Rect {
    grid.expand(TERMINAL_PAD)
}

/// Where a pane's text goes: the same margin a lone terminal has, so the outline never touches it.
fn pane_text_rect(pane: egui::Rect) -> egui::Rect {
    pane.shrink(TERMINAL_PAD)
}

/// Draws every pane of `splits` over the grid's area. Only the focused pane takes keys and pastes;
/// a click in another pane focuses it. Returns what the panes did, the focused pane's input first.
pub(super) fn show(
    ui: &mut egui::Ui,
    splits: &mut Splits<PtyTerm>,
    main: &mut PtyTerm,
    grid: &TerminalGrid,
    config: &TerminalConfig,
    (keys_free, modal_open): (bool, bool),
    injected_wheel: &[egui::Event],
) -> Option<TerminalOutput> {
    let area = split_area(grid.rect());
    splits.set_area(area);
    let refocus = splits.take_refocus();
    let mut drawn = Vec::new();
    for place in splits.places() {
        let Some(term) = splits.term_mut(main, place.id) else { continue };
        let pane_grid = grid.within(pane_text_rect(place.rect));
        pane_grid.resize(term);
        let (wheel, focus) =
            if place.focused { (injected_wheel, Focus::Manage) } else { (&[][..], Focus::Off) };
        let out = show_shell_in(
            ui,
            term,
            &pane_grid,
            config,
            (place.focused && keys_free, modal_open),
            (&place.path, focus),
            wheel,
        );
        if place.focused && refocus {
            ui.ctx().memory_mut(|memory| memory.request_focus(out.response.id));
        }
        drawn.push((place.id, place.focused, out));
    }
    let clicked = drawn.iter().find_map(|(id, _, out)| {
        let pressed = out.response.clicked() || out.response.drag_started();
        (!modal_open && (pressed || out.mouse_pasted.is_some())).then_some(*id)
    });
    if let Some(id) = clicked {
        splits.focus(id);
        ui.ctx().request_repaint();
    }
    for divider in splits.layout().dividers(area) {
        if let Some(ratio) = show_divider(ui, &divider) {
            splits.layout_mut().drag(&divider.path, ratio);
        }
    }
    outline_focused(ui, splits);
    if let Some(id) = splits.find(main, |term| term.exited().is_some()) {
        splits.close(id, main);
        ui.ctx().request_repaint();
    }
    drawn.sort_by_key(|(_, focused, _)| !*focused);
    let mut outs = drawn.into_iter().map(|(_, _, out)| out);
    let mut merged = outs.next()?;
    for out in outs {
        merged.confirm_paste.extend(out.confirm_paste);
        merged.bell_rang |= out.bell_rang;
        merged.copied = merged.copied.or(out.copied);
        merged.mouse_pasted = merged.mouse_pasted.or(out.mouse_pasted);
    }
    Some(merged)
}

fn outline_focused(ui: &egui::Ui, splits: &Splits<PtyTerm>) {
    if let Some(place) = splits.places().into_iter().find(|place| place.focused) {
        let stroke = Stroke::new(1.0, brand::TEAL);
        ui.painter().rect_stroke(place.rect, 0.0, stroke, StrokeKind::Inside);
    }
}

/// Draws a divider and returns the ratio a drag of it asks for.
fn show_divider(ui: &egui::Ui, divider: &Divider) -> Option<f32> {
    let (grab, icon, line) = match divider.dir {
        SplitDir::Row => {
            (vec2(GRAB, 0.0), CursorIcon::ResizeHorizontal, vec2(1.0, divider.handle.height()))
        }
        SplitDir::Column => {
            (vec2(0.0, GRAB), CursorIcon::ResizeVertical, vec2(divider.handle.width(), 1.0))
        }
    };
    let id = egui::Id::new(("main_split", &divider.path));
    let response = ui.interact(divider.handle.expand2(grab), id, Sense::click_and_drag());
    ui.painter().rect_filled(
        egui::Rect::from_center_size(divider.handle.center(), line),
        0.0,
        brand::ANSWER_LINE,
    );
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(icon);
    }
    let pointer = response.interact_pointer_pos().filter(|_| response.dragged())?;
    Some(pane::ratio_from_pointer(divider.parent, divider.dir, pointer))
}

impl Tidewatch {
    /// The active tab's id when it is a main tab showing its shell, the only tab that splits.
    fn splittable(&self) -> Option<u64> {
        let tab = &self.tabs.items()[self.tabs.active()];
        let shell = matches!(tab.pane.body, Body::Shell(_));
        (tab.pinned && shell && self.welcome.is_none()).then_some(tab.id)
    }

    pub(super) fn split_main(&mut self, dir: SplitDir, ctx: &egui::Context) {
        let Some(id) = self.splittable() else { return };
        let folder = self.workspace().dir().to_path_buf();
        let splits = self.main_splits.entry(id).or_insert_with(Splits::single);
        if splits.can_split() {
            splits.split(dir, spawn_shell(ctx, &folder));
        }
        ctx.request_repaint();
    }

    pub(super) fn focus_main_pane(&mut self, dir: Dir) {
        let Some(splits) = self.splittable().and_then(|id| self.main_splits.get_mut(&id)) else {
            return;
        };
        splits.focus_toward(dir);
    }

    /// Cmd+W on a split main tab closes the focused pane. False when there is only one pane, and the
    /// key does what it does on a main tab.
    pub(super) fn close_main_pane(&mut self) -> bool {
        let Some(id) = self.splittable() else { return false };
        let Some(Body::Shell(main)) = self.tabs.get_mut(id).map(|tab| &mut tab.pane.body) else {
            return false;
        };
        let Some(splits) = self.main_splits.get_mut(&id) else { return false };
        splits.close_focused(main);
        self.drop_if_single(id);
        true
    }

    pub(super) fn drop_if_single(&mut self, tab: u64) {
        if self.main_splits.get(&tab).is_some_and(Splits::is_single) {
            self.main_splits.remove(&tab);
        }
    }

    pub(super) fn split_entries(&self) -> Vec<GroupSplit> {
        self.main_splits
            .iter()
            .filter(|(_, splits)| !splits.is_single())
            .filter_map(|(tab, splits)| {
                let group = self.tabs.group_of(*tab)?;
                Some(GroupSplit { group, shape: splits.layout().to_saved() })
            })
            .collect()
    }

    pub(super) fn restore_main_splits(
        &mut self,
        tab: u64,
        shape: &SavedSplit,
        dir: &Path,
        ctx: &egui::Context,
    ) {
        if let Some(splits) = Splits::from_saved(shape, || spawn_shell(ctx, dir)) {
            self.main_splits.insert(tab, splits);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use egui::{Context, Event, Modifiers, Pos2, Rect, vec2};
    use stdusk_core::repaint::Repaint;

    use super::*;
    use crate::app::terminal::terminal_config;
    use crate::pty::spawn_service;
    use crate::split::MAIN;

    const AREA_MIN: Pos2 = Pos2::new(0.0, 0.0);

    struct Window {
        ctx: Context,
        splits: Splits<PtyTerm>,
        main: PtyTerm,
        config: TerminalConfig,
        dir: PathBuf,
    }

    fn area() -> Rect {
        Rect::from_min_size(AREA_MIN, vec2(800.0, 400.0))
    }

    fn wait_for(what: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !what() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(what(), "the terminal never got there");
    }

    fn reads_into(dir: &Path, name: &str, len: usize) -> PtyTerm {
        let (ready, got) = (dir.join(format!("{name}.ready")), dir.join(format!("{name}.got")));
        let script = format!(
            "stty raw -echo; : > {}; head -c {len} > {}; sleep 20",
            ready.display(),
            got.display()
        );
        let term = spawn_service(Repaint::none(), dir, &script);
        wait_for(|| ready.exists());
        term
    }

    impl Window {
        fn two_panes(tag: &str, len: usize) -> Self {
            let dir = std::env::temp_dir().join(format!("tw-split-{}-{tag}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let main = reads_into(&dir, "main", len);
            let mut splits = Splits::single();
            assert!(splits.split(SplitDir::Row, reads_into(&dir, "right", len)));
            let mut window =
                Self { ctx: Context::default(), splits, main, config: terminal_config(false), dir };
            for _ in 0..3 {
                window.frame(Vec::new());
            }
            window
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(AREA_MIN, vec2(800.0, 600.0))),
                events,
                focused: true,
                ..Default::default()
            };
            let area = area();
            let ctx = self.ctx.clone();
            let _ = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let grid = TerminalGrid::fixed(area);
                    let gates = (true, false);
                    show(ui, &mut self.splits, &mut self.main, &grid, &self.config, gates, &[]);
                });
            });
        }

        fn click_in(&mut self, id: u32) {
            let rect = self.splits.places().into_iter().find(|place| place.id == id).unwrap().rect;
            let pos = rect.center();
            let button = |pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![Event::Text(text.to_owned())]);
        }

        fn got(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.dir.join(format!("{name}.got"))).unwrap_or_default()
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_click_in_the_right_pane_focuses_it_and_typing_reaches_only_the_focused_pane() {
        let mut window = Window::two_panes("focus", 1);
        window.splits.focus(MAIN);
        window.frame(Vec::new());
        window.type_text("a");
        wait_for(|| window.got("main") == b"a");
        assert_eq!(window.got("right"), b"");

        window.click_in(1);
        assert_eq!(window.splits.layout().focused(), 1);
        window.type_text("b");
        wait_for(|| window.got("right") == b"b");
        assert_eq!(window.got("main"), b"a");
    }

    #[test]
    fn every_pane_keeps_the_lone_terminal_margin_inside_its_outline() {
        let mut window = Window::two_panes("pad", 1);
        assert!(window.splits.split(SplitDir::Column, reads_into(&window.dir, "low", 1)));
        window.frame(Vec::new());
        let places = window.splits.places();
        assert_eq!(places.len(), 3);
        for place in places {
            let text = pane_text_rect(place.rect);
            assert_eq!(text.left() - place.rect.left(), TERMINAL_PAD);
            assert_eq!(text.top() - place.rect.top(), TERMINAL_PAD);
            assert_eq!(place.rect.right() - text.right(), TERMINAL_PAD);
            assert_eq!(place.rect.bottom() - text.bottom(), TERMINAL_PAD);
        }
        let outer = split_area(area());
        assert_eq!(pane_text_rect(outer), area());
    }

    #[test]
    fn a_pasted_text_goes_to_the_focused_pane_only() {
        let mut window = Window::two_panes("paste", 8);
        window.splits.focus(MAIN);
        window.frame(vec![Event::Paste("one line".into())]);
        wait_for(|| window.got("main") == b"one line");
        assert_eq!(window.got("right"), b"");
    }

    #[test]
    fn dragging_the_divider_resizes_the_panes() {
        let mut window = Window::two_panes("drag", 1);
        let before = window.splits.places()[0].rect.width();
        let divider = window.splits.layout().dividers(split_area(area()));
        let grab = divider[0].handle.center();
        let to = grab + vec2(100.0, 0.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        window.frame(vec![Event::PointerMoved(grab)]);
        window.frame(vec![button(grab, true)]);
        window.frame(vec![Event::PointerMoved(grab + vec2(40.0, 0.0))]);
        window.frame(vec![Event::PointerMoved(to)]);
        window.frame(vec![button(to, false)]);
        window.frame(Vec::new());
        let after = window.splits.places()[0].rect.width();
        assert!((after - before - 100.0).abs() < 8.0, "{before} -> {after}");
    }

    #[test]
    fn a_pane_whose_shell_exits_closes_and_the_main_pane_takes_the_survivors_shell() {
        let mut window = Window::two_panes("exit", 1);
        window.main.kill();
        wait_for(|| window.main.exited().is_some());
        window.frame(Vec::new());
        assert!(window.splits.is_single());
        assert!(window.main.exited().is_none());
    }
}
