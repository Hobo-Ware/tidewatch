//! One frame in steps: finished jobs, background work, shortcuts, and what the drawn frame asked for.

use egui::{Key, Modifiers};

use super::Tidewatch;
use crate::layout;
use crate::reader_glue;
use crate::session_save::WindowView;
use crate::settings_view;
use crate::tabs::Action;

pub(super) struct KeyRoute {
    pub(super) modal_open: bool,
    pub(super) typing_in_a_panel: bool,
}

pub(super) struct FrameIntent {
    pub(super) active_id: u64,
    pub(super) keys: KeyRoute,
    pub(super) actions: Vec<Action>,
    pub(super) toggle_overview: bool,
    pub(super) toggle_settings: bool,
    pub(super) settings_action: Option<settings_view::Action>,
}

impl Tidewatch {
    pub(super) fn drain_events_and_jobs(&mut self, ctx: &egui::Context) {
        for done in self.jobs.drain() {
            self.apply_done(done, ctx);
        }
        for done in self.pane_jobs.drain() {
            self.apply_pane_done(done, ctx);
        }
        for done in self.agent_jobs.drain() {
            self.apply_agent_done(done, ctx);
        }
        for event in self.shell.tick(ctx) {
            self.shell_event(&event, ctx);
        }
        self.updates.tick();
        let toasts = [self.trust.drain(ctx), self.leftovers.tick()];
        for text in toasts.into_iter().flatten() {
            self.show_toast(text, ctx);
        }
    }

    pub(super) fn drive_background_work(&mut self, ctx: &egui::Context, now: f64) {
        self.dev_reader.press_toggle(ctx, now);
        reader_glue::start(&mut self.tabs, ctx, &self.dev_reader);
        reader_glue::drain(&mut self.tabs);
        self.drive_agents(ctx, now);
        self.drive_flow(ctx, now);
        self.drive_restore(ctx, now, None);
        self.drive_dev_leftovers(ctx);
        self.drive_views();
    }

    pub(super) fn read_shortcuts(&mut self, ctx: &egui::Context) -> FrameIntent {
        let typing_in_a_panel = ctx.text_edit_focused();
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let modal_open = self.closing.is_some()
            || self.new_tab.is_some()
            || self.settings.is_some()
            || self.trust.dialog_open()
            || self.leftovers.is_open()
            || self.flow.modal_open(active_id);
        if self.closing.is_none()
            && self.new_tab.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Comma))
        {
            self.toggle_settings();
        }
        let reader_key = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        if !modal_open
            && !self.overview
            && self.settings.is_none()
            && ctx.input_mut(|i| i.consume_key(reader_key, Key::T))
        {
            self.toggle_reader(ctx);
        }
        let mut actions = if modal_open { Vec::new() } else { shortcut_actions(ctx) };
        if self.overview {
            actions.retain(|action| *action != Action::CloseActive);
        }
        let mut toggle_overview = false;
        if self.closing.is_none() && self.new_tab.is_none() {
            toggle_overview = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num0));
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Period)) {
                self.toggle_focus(ctx);
            }
            if self.overview && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                toggle_overview = true;
            }
        }
        FrameIntent {
            active_id,
            keys: KeyRoute { modal_open, typing_in_a_panel },
            actions,
            toggle_overview,
            toggle_settings: false,
            settings_action: None,
        }
    }

    pub(super) fn apply_frame_intent(&mut self, ctx: &egui::Context, intent: FrameIntent) {
        for action in intent.actions {
            self.dispatch(action, ctx);
        }
        if intent.toggle_overview {
            self.toggle_overview();
        }
        self.apply_settings_action(intent.settings_action, ctx);
        if intent.toggle_settings {
            self.toggle_settings();
        }
    }

    pub(super) fn finish_frame(&mut self, ctx: &egui::Context, now: f64) {
        self.dev_session.tick(ctx, &mut self.tabs, now);
        self.saver.tick(
            self.restore.settled(),
            &WindowView::of(&self.workspace, self.focus),
            &self.tabs,
        );
        if let Some(shot) = &mut self.screenshot {
            shot.tick(ctx);
        }
    }
}

fn shortcut_actions(ctx: &egui::Context) -> Vec<Action> {
    ctx.input_mut(|i| {
        let mut actions = Vec::new();
        if i.consume_key(Modifiers::COMMAND, Key::T) {
            actions.push(Action::New);
        }
        if i.consume_key(Modifiers::COMMAND, Key::W) {
            actions.push(Action::CloseActive);
        }
        for key in [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ] {
            if let Some(idx) = layout::digit_index(key)
                && i.consume_key(Modifiers::COMMAND, key)
            {
                actions.push(Action::Select(idx));
            }
        }
        actions
    })
}
