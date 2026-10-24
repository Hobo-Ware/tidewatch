//! The quake drop-down: a window that falls from the top edge to the monitor's width and the chosen
//! fraction of its height, and goes away again on the hotkey or when you click elsewhere. State and geometry are
//! pure; `present` turns a show or hide into viewport commands.
//!
//! Hiding is the delicate part, and it is the sibling's hard-won rule (`.agents/rules/platform.md`):
//! on macOS a window with no pixels on screen gets occluded and App-Napped, which stops the run
//! loop that would deliver the hotkey. So a hidden window is parked with a 2 point sliver still on
//! screen and made invisible with alpha 0, never ordered out and never moved off screen.

use std::time::Duration;

use egui::{Context, Pos2, Rect, Vec2, ViewportCommand, pos2, vec2};

use crate::os::Os;

pub(crate) const HIDE_SLIVER: f32 = 2.0;
pub(crate) const HIDDEN_WAKE: Duration = Duration::from_millis(120);

/// The window's top-left y when parked: strictly on the screen, with only the sliver showing.
pub(crate) fn hidden_y(screen_h: f32) -> f32 {
    screen_h - HIDE_SLIVER
}

pub(crate) fn hidden_pos(screen: Rect) -> Pos2 {
    pos2(screen.min.x, screen.min.y + hidden_y(screen.height()))
}

pub(crate) fn shown_size(screen_w: f32, screen_h: f32, height_pct: f32) -> Vec2 {
    vec2(screen_w, (screen_h * height_pct).round())
}

/// Window alpha: opaque when shown, transparent (not removed) when hidden.
pub(crate) fn alpha(visible: bool) -> f64 {
    if visible { 1.0 } else { 0.0 }
}

/// egui-winit scales viewport commands by `zoom_factor * scale_factor` but winit divides by the
/// scale factor alone, so a zoomed UI would push the parked sliver off screen.
pub(crate) fn unzoom(v: Vec2, zoom: f32) -> Vec2 {
    if zoom > 0.0 { v / zoom } else { v }
}

pub(crate) struct Frame {
    pub(crate) monitor_known: bool,
    pub(crate) focused: bool,
    pub(crate) app_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Present(bool),
    Repaint,
    RepaintAfter(Duration),
}

pub(crate) struct Quake {
    visible: bool,
    was_focused: bool,
    sized: bool,
}

impl Quake {
    pub(crate) fn new() -> Self {
        Self { visible: true, was_focused: false, sized: false }
    }

    pub(crate) fn hidden(&self) -> bool {
        !self.visible
    }

    pub(crate) fn toggle(&mut self) -> Op {
        self.visible = !self.visible;
        if self.visible {
            self.was_focused = false;
        }
        Op::Present(self.visible)
    }

    pub(crate) fn surface(&mut self) -> Op {
        self.visible = true;
        self.was_focused = false;
        Op::Present(true)
    }

    /// The window was a plain one and became a quake window: size it again once the monitor is known.
    pub(crate) fn restart(&mut self) {
        *self = Self::new();
    }

    /// Hide only on a real deactivation: after the window held focus since it was shown, and only
    /// when the whole app is inactive, because a system panel (the emoji viewer) takes the
    /// window's focus while the app stays active.
    fn hides_on_blur(&self, frame: &Frame) -> bool {
        self.visible && self.was_focused && !frame.focused && !frame.app_active
    }

    /// Full quake sizing waits for the monitor size, which is unknown on the first frame.
    pub(crate) fn tick(&mut self, frame: &Frame) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.sized {
            if frame.monitor_known {
                self.sized = true;
                ops.push(Op::Present(self.visible));
            } else {
                ops.push(Op::Repaint);
            }
        }
        if self.visible {
            if frame.focused {
                self.was_focused = true;
            } else if self.hides_on_blur(frame) {
                self.visible = false;
                ops.push(Op::Present(false));
            }
        } else {
            ops.push(Op::RepaintAfter(HIDDEN_WAKE));
        }
        ops
    }
}

/// Gives the shown window its new height at once; the top edge stays where it is.
pub(crate) fn resize(ctx: &Context, os: &dyn Os, height_pct: f32) {
    let screen = os.window_screen().or_else(|| os.primary_screen());
    send_shown_size(ctx, screen, height_pct);
}

fn send_shown_size(ctx: &Context, screen: Option<Rect>, height_pct: f32) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    let zoom = ctx.zoom_factor();
    if let Some(size) = screen.map(|s| unzoom(s.size(), zoom)).or(monitor) {
        let size = shown_size(size.x, size.y, height_pct);
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    }
}

/// Shows (drops to the top edge of the screen under the cursor, focused) or hides the window.
pub(crate) fn present(ctx: &Context, os: &mut dyn Os, visible: bool, height_pct: f32) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    os.set_alpha(alpha(visible));
    let screen = if visible {
        os.cursor_screen().or_else(|| os.window_screen())
    } else {
        os.window_screen()
    }
    .or_else(|| os.primary_screen());
    let zoom = ctx.zoom_factor();
    let command_pos = |p: Pos2| unzoom(p.to_vec2(), zoom).to_pos2();
    if visible {
        if !os.parks_when_hidden() {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }
        send_shown_size(ctx, screen, height_pct);
        let origin = screen.map_or(Pos2::ZERO, |s| command_pos(s.min));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    } else if os.parks_when_hidden() {
        let parked = screen.map_or_else(
            || pos2(0.0, monitor.map_or(2000.0, |m| hidden_y(m.y))),
            |s| command_pos(hidden_pos(s)),
        );
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(parked));
    } else {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fake::{Call, Recorder};
    use egui::{RawInput, ViewportId};

    fn commands(ctx: &Context, work: impl FnOnce(&Context)) -> Vec<ViewportCommand> {
        let mut work = Some(work);
        let output = ctx.run_ui(RawInput::default(), |ui| {
            if let Some(work) = work.take() {
                work(ui.ctx());
            }
        });
        output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default()
    }

    fn screen(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn mac(window: Rect) -> Recorder {
        Recorder { parks: true, window: Some(window), cursor: Some(window), ..Recorder::default() }
    }

    #[test]
    fn the_shown_window_is_the_monitor_wide_and_the_chosen_fraction_tall_rounded() {
        assert_eq!(shown_size(1440.0, 900.0, 0.5), vec2(1440.0, 450.0));
        assert_eq!(shown_size(1920.0, 1080.0, 0.5), vec2(1920.0, 540.0));
        assert_eq!(shown_size(2560.0, 1111.0, 0.5), vec2(2560.0, 556.0));
        assert_eq!(shown_size(1440.0, 900.0, 0.505), vec2(1440.0, 455.0));
        assert_eq!(shown_size(1440.0, 900.0, 0.9), vec2(1440.0, 810.0));
        assert_eq!(shown_size(1440.0, 900.0, 0.2), vec2(1440.0, 180.0));
    }

    #[test]
    fn a_hidden_window_keeps_exactly_the_sliver_on_its_own_screen() {
        assert_eq!(hidden_y(900.0), 898.0);
        let second = screen(1440.0, 100.0, 1920.0, 1080.0);
        assert_eq!(hidden_pos(second), pos2(1440.0, 100.0 + 1080.0 - HIDE_SLIVER));
        assert!(hidden_pos(second).y < second.max.y, "the sliver stays on screen");
    }

    #[test]
    fn a_zoomed_ui_still_parks_the_sliver_on_screen() {
        let screen = screen(1728.0, -323.0, 2560.0, 1440.0);
        for zoom in [0.5_f32, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0] {
            let sent = unzoom(hidden_pos(screen).to_vec2(), zoom);
            let landed = sent * zoom;
            assert!((landed.y - hidden_pos(screen).y).abs() < 0.01, "zoom {zoom}");
        }
        assert_eq!(unzoom(vec2(10.0, 10.0), 0.0), vec2(10.0, 10.0));
    }

    #[test]
    fn hidden_means_transparent_and_shown_means_opaque() {
        assert_eq!(alpha(true), 1.0);
        assert_eq!(alpha(false), 0.0);
    }

    #[test]
    fn blur_hides_only_on_a_real_deactivation_after_the_window_held_focus() {
        let cases = [
            ((true, true, false, false), true),
            ((true, true, false, true), false),
            ((true, false, false, false), false),
            ((true, true, true, false), false),
            ((false, true, false, false), false),
        ];
        for ((visible, was_focused, focused, app_active), want) in cases {
            let quake = Quake { visible, was_focused, sized: true };
            let frame = Frame { monitor_known: true, focused, app_active };
            assert_eq!(
                quake.hides_on_blur(&frame),
                want,
                "visible={visible} was_focused={was_focused} focused={focused} app_active={app_active}"
            );
        }
    }

    fn frame(monitor_known: bool, focused: bool, app_active: bool) -> Frame {
        Frame { monitor_known, focused, app_active }
    }

    #[test]
    fn the_first_frames_wait_for_the_monitor_then_size_the_window_once() {
        let mut quake = Quake::new();
        assert_eq!(quake.tick(&frame(false, true, true)), vec![Op::Repaint]);
        assert_eq!(quake.tick(&frame(true, true, true)), vec![Op::Present(true)]);
        assert_eq!(quake.tick(&frame(true, true, true)), vec![]);
    }

    #[test]
    fn a_window_that_starts_unfocused_is_not_hidden_before_it_ever_held_focus() {
        let mut quake = Quake::new();
        quake.tick(&frame(true, false, false));
        for _ in 0..3 {
            assert_eq!(quake.tick(&frame(true, false, false)), vec![]);
        }
        assert_eq!(quake.tick(&frame(true, true, true)), vec![]);
        assert_eq!(quake.tick(&frame(true, false, false)), vec![Op::Present(false)]);
    }

    #[test]
    fn a_system_panel_taking_focus_does_not_hide_the_window() {
        let mut quake = Quake::new();
        quake.tick(&frame(true, true, true));
        assert_eq!(quake.tick(&frame(true, false, true)), vec![]);
    }

    #[test]
    fn a_hidden_window_keeps_asking_for_frames_so_the_hotkey_can_wake_it() {
        let mut quake = Quake::new();
        quake.tick(&frame(true, true, true));
        assert_eq!(quake.toggle(), Op::Present(false));
        assert_eq!(quake.tick(&frame(true, false, false)), vec![Op::RepaintAfter(HIDDEN_WAKE)]);
        assert_eq!(quake.toggle(), Op::Present(true));
        assert_eq!(quake.tick(&frame(true, false, false)), vec![], "focus is not armed yet");
    }

    #[test]
    fn surfacing_shows_a_hidden_window_and_re_arms_the_focus_rule() {
        let mut quake = Quake::new();
        quake.tick(&frame(true, true, true));
        quake.toggle();
        assert_eq!(quake.surface(), Op::Present(true));
        assert_eq!(quake.surface(), Op::Present(true), "already shown stays shown");
        assert_eq!(quake.tick(&frame(true, false, false)), vec![]);
    }

    #[test]
    fn becoming_a_quake_window_again_sizes_it_again() {
        let mut quake = Quake::new();
        quake.tick(&frame(true, true, true));
        quake.restart();
        assert_eq!(quake.tick(&frame(true, true, true)), vec![Op::Present(true)]);
    }

    #[test]
    fn showing_on_macos_drops_to_the_top_of_the_cursor_screen_and_takes_focus() {
        let ctx = Context::default();
        let mut os = mac(screen(0.0, 0.0, 1440.0, 900.0));
        os.cursor = Some(screen(1440.0, 0.0, 1920.0, 1080.0));
        let sent = commands(&ctx, |ctx| present(ctx, &mut os, true, 0.5));
        assert_eq!(
            sent,
            vec![
                ViewportCommand::InnerSize(vec2(1920.0, 540.0)),
                ViewportCommand::OuterPosition(pos2(1440.0, 0.0)),
                ViewportCommand::Focus,
            ]
        );
        assert_eq!(os.calls, vec![Call::Alpha(1.0)]);
    }

    #[test]
    fn showing_uses_the_chosen_height() {
        let ctx = Context::default();
        let mut os = mac(screen(0.0, 0.0, 1440.0, 900.0));
        let sent = commands(&ctx, |ctx| present(ctx, &mut os, true, 0.8));
        assert_eq!(sent.first(), Some(&ViewportCommand::InnerSize(vec2(1440.0, 720.0))));
    }

    #[test]
    fn resizing_changes_only_the_height_of_the_screen_the_window_is_on() {
        let ctx = Context::default();
        let mut os = mac(screen(0.0, 0.0, 1440.0, 900.0));
        os.window = Some(screen(1440.0, 0.0, 1920.0, 1080.0));
        let sent = commands(&ctx, |ctx| resize(ctx, &os, 0.75));
        assert_eq!(sent, vec![ViewportCommand::InnerSize(vec2(1920.0, 810.0))]);
        assert_eq!(os.calls, vec![]);
    }

    #[test]
    fn hiding_on_macos_parks_the_sliver_on_screen_and_goes_transparent_never_unmapped() {
        let ctx = Context::default();
        let mut os = mac(screen(0.0, 0.0, 1440.0, 900.0));
        let sent = commands(&ctx, |ctx| present(ctx, &mut os, false, 0.5));
        assert_eq!(sent, vec![ViewportCommand::OuterPosition(pos2(0.0, 898.0))]);
        assert_eq!(os.calls, vec![Call::Alpha(0.0)]);
        assert!(!sent.iter().any(|c| matches!(c, ViewportCommand::Visible(_))));
    }

    #[test]
    fn elsewhere_hiding_unmaps_and_showing_maps_again() {
        let ctx = Context::default();
        let mut os = Recorder::default();
        let hide = commands(&ctx, |ctx| present(ctx, &mut os, false, 0.5));
        assert_eq!(hide, vec![ViewportCommand::Visible(false)]);
        let show = commands(&ctx, |ctx| present(ctx, &mut os, true, 0.5));
        assert_eq!(show.first(), Some(&ViewportCommand::Visible(true)));
        assert_eq!(show.last(), Some(&ViewportCommand::Focus));
        assert!(show.contains(&ViewportCommand::OuterPosition(Pos2::ZERO)));
    }

    #[test]
    fn with_no_screen_known_the_primary_screen_is_the_last_resort() {
        let ctx = Context::default();
        let mut os = Recorder {
            parks: true,
            primary: Some(screen(0.0, 0.0, 1000.0, 800.0)),
            ..Recorder::default()
        };
        let sent = commands(&ctx, |ctx| present(ctx, &mut os, false, 0.5));
        assert_eq!(sent, vec![ViewportCommand::OuterPosition(pos2(0.0, 798.0))]);
    }
}
