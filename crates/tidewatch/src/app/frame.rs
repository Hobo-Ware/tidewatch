//! One frame in steps: finished jobs, background work, shortcuts, and what the drawn frame asked for.

use egui::{Key, Modifiers};

use super::Tidewatch;
use crate::commands::{self, Command, Dialog, Gate};
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
        let gate = self.key_gate(active_id);
        let mut actions = Vec::new();
        let mut toggle_overview = false;
        for command in commands::pressed(ctx) {
            if !gate.allows(command) {
                continue;
            }
            if self.status_log {
                eprintln!("tidewatch: command {} at {}", command.id(), super::wall_ms());
            }
            match command {
                Command::Overview => toggle_overview = true,
                Command::NewTab | Command::CloseTab | Command::SelectTab(_) => {
                    actions.extend(command.action());
                }
                _ => self.run_command(command, ctx),
            }
        }
        if gate.dialog != Dialog::TabChoice
            && !gate.palette
            && self.overview
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            toggle_overview = true;
        }
        FrameIntent {
            active_id,
            keys: KeyRoute { modal_open: gate.modal_open(), typing_in_a_panel },
            actions,
            toggle_overview,
            toggle_settings: false,
            settings_action: None,
        }
    }

    fn key_gate(&self, active_id: u64) -> Gate {
        let other =
            self.trust.dialog_open() || self.leftovers.is_open() || self.flow.modal_open(active_id);
        let dialog = if self.closing.is_some() || self.new_tab.is_some() {
            Dialog::TabChoice
        } else if other {
            Dialog::Other
        } else {
            Dialog::None
        };
        Gate {
            dialog,
            settings: self.settings.is_some(),
            overview: self.overview,
            palette: self.palette.is_some(),
        }
    }

    pub(super) fn run_command(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::NewTab | Command::CloseTab | Command::SelectTab(_) => {
                if let Some(action) = command.action() {
                    self.dispatch(action, ctx);
                }
            }
            Command::Overview => self.toggle_overview(),
            Command::Focus => self.toggle_focus(ctx),
            Command::Reader => self.toggle_reader(ctx),
            Command::Settings => self.toggle_settings(),
            Command::Palette => self.toggle_palette(),
            Command::ApproveDelivery => {}
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
