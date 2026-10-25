//! One frame in steps: finished jobs, background work, shortcuts, and what the drawn frame asked for.

use egui::{Key, Modifiers};

use tidewatch_core::ui_zoom::Direction;

use super::Tidewatch;
use super::repos::RepoCommand;
use crate::commands::{self, Card, Command, Dialog, Focus, Gate};
use crate::progress_view;
use crate::reader_glue;
use crate::settings_view;
use crate::tab_view::TabView;
use crate::tabs::Action;
use crate::updates::Wish;

pub(super) struct KeyRoute {
    pub(super) modal_open: bool,
    pub(super) typing_in_a_panel: bool,
}

pub(super) struct FrameIntent {
    pub(super) active_id: u64,
    pub(super) keys: KeyRoute,
    pub(super) actions: Vec<Action>,
    pub(super) repo_command: Option<RepoCommand>,
    pub(super) open_dir: Option<std::path::PathBuf>,
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
        for workspace in self.workspace_jobs.drain() {
            self.adopt_workspace(workspace, ctx);
        }
        for done in self.stdusk_jobs.drain() {
            self.apply_stdusk_done(done, ctx);
        }
        self.terminal_font.drain(ctx);
        self.drain_picked(ctx);
        self.drain_link_jobs(ctx);
        self.bases.drain();
        for event in self.shell.tick(ctx) {
            self.shell_event(&event, ctx);
        }
        self.updates.tick();
        if self.updates.take_restart() {
            self.restart_to_update(ctx);
        }
        match self.updates.take_wish() {
            Some(Wish::OpenSettings) => self.open_updates_settings(),
            Some(Wish::Quit) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(Wish::Toast(text)) => self.show_toast(text, ctx),
            None => {}
        }
        let toasts = [self.trust.drain(ctx), self.leftovers.tick()];
        for text in toasts.into_iter().flatten() {
            self.show_toast(text, ctx);
        }
    }

    pub(super) fn drive_background_work(&mut self, ctx: &egui::Context, now: f64) {
        self.dev_reader.press_toggle(ctx, now);
        if let Some(command) = self.dev_panes.press(ctx, now) {
            self.run_command(command, ctx);
        }
        reader_glue::start(&mut self.tabs, ctx, &self.dev_reader);
        reader_glue::drain(&mut self.tabs, ctx, now);
        self.drive_agents(ctx, now);
        self.drive_flow(ctx, now);
        self.drive_pr(ctx, now);
        self.drive_comments(ctx, now);
        self.drive_progress(ctx);
        self.drive_parking(ctx);
        self.drive_group_merge(ctx);
        self.drive_avatar_choice(ctx);
        self.drive_jobs(ctx);
        self.drive_meeseeks(ctx, now);
        self.drive_done_check(ctx, now);
        self.drive_keep_awake(ctx, now);
        self.drive_autofetch(ctx);
        self.drive_amphetamine(ctx, now);
        self.drive_checklist(ctx, now);
        self.drive_settings_sync(ctx, now);
        self.drive_due(ctx);
        self.drive_due_start(ctx, now);
        self.drive_dev_scheduled(ctx);
        self.drive_dev_draft(ctx, now);
        self.drive_dev_form_start(ctx);
        self.drive_restore(ctx, now, None);
        self.drive_takeover(ctx, now);
        self.drive_repo_close(ctx);
        self.drive_dev_leftovers(ctx);
        self.drive_dev_link(ctx);
        self.drive_views();
    }

    pub(super) fn read_shortcuts(&mut self, ctx: &egui::Context) -> FrameIntent {
        let typing_in_a_panel = ctx.text_edit_focused();
        let active_id = self.tabs.items()[self.tabs.active()].id;
        self.clear_search_on_escape(ctx, active_id);
        let gate = self.key_gate(active_id, typing_in_a_panel);
        if gate.focus == Focus::Terminal && !gate.modal_open() && !self.overview {
            crate::screen_kit::esc_left_a_widget(ctx, false);
        }
        let mut actions = Vec::new();
        let mut toggle_overview = false;
        for command in commands::pressed(ctx, gate) {
            if !gate.allows(command) {
                continue;
            }
            if self.status_log {
                eprintln!("tidewatch: command {} at {}", command.id(), super::wall_ms());
            }
            match command {
                Command::Overview => toggle_overview = true,
                Command::NewTab
                | Command::CloseTab
                | Command::SelectTab(_)
                | Command::MoveTabLeft
                | Command::MoveTabRight
                | Command::ParkTab
                | Command::NextRepo
                | Command::PrevRepo => actions.extend(command.action()),
                _ => self.run_command(command, ctx),
            }
        }
        if gate.dialog != Dialog::TabChoice
            && !gate.palette
            && self.overview
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            if self.takeover.line_open() {
                self.takeover.fold_line();
            } else {
                toggle_overview = true;
            }
        }
        FrameIntent {
            active_id,
            keys: KeyRoute { modal_open: gate.modal_open(), typing_in_a_panel },
            actions,
            repo_command: None,
            open_dir: None,
            toggle_overview,
            toggle_settings: false,
            settings_action: None,
        }
    }

    /// Esc in the commit search field empties it. egui has already taken the keyboard from the
    /// field by now, so the field's own state from the last frame says it was the one typed in.
    fn clear_search_on_escape(&mut self, ctx: &egui::Context, active_id: u64) {
        let Some(view) = self.views.get_mut(&active_id) else { return };
        if view.search.was_focused(ctx.cumulative_frame_nr())
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            view.search.clear();
            ctx.memory_mut(|memory| memory.surrender_focus(crate::search_bar::field_id(active_id)));
        }
    }

    fn find_commit(&mut self, ctx: &egui::Context) {
        if !self.panes_showing() {
            return;
        }
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let Some(view) = self.views.get_mut(&active_id) else { return };
        view.search.want_focus = Some(crate::commit_search::Focus::SelectAll);
        self.progress_ui.tab(active_id).chosen = Some(progress_view::Side::History);
        ctx.request_repaint();
    }

    fn key_gate(&self, active_id: u64, typing: bool) -> Gate {
        let other = self.trust.dialog_open()
            || self.leftovers.is_open()
            || self.open_link.is_some()
            || !self.pending_pastes.is_empty()
            || self.flow.modal_open(active_id);
        let dialog = if self.closing.is_some() || self.takeover.dialog_open() {
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
            focus: if typing {
                Focus::TextField
            } else if self.file_view_showing(active_id) {
                Focus::FileView
            } else {
                Focus::Terminal
            },
            card: if self.panes_showing()
                && self.progress_ui.shown(active_id) == Some(progress_view::Side::History)
            {
                Card::History
            } else {
                Card::Other
            },
        }
    }

    pub(super) fn run_command(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::NewTab
            | Command::CloseTab
            | Command::SelectTab(_)
            | Command::MoveTabLeft
            | Command::MoveTabRight
            | Command::ParkTab
            | Command::NextRepo
            | Command::PrevRepo => {
                if let Some(action) = command.action() {
                    self.dispatch(action, ctx);
                }
            }
            Command::Overview => self.toggle_overview(),
            Command::Focus => self.toggle_focus(ctx),
            Command::ZoomIn => self.step_zoom(Direction::Bigger, ctx),
            Command::ZoomOut => self.step_zoom(Direction::Smaller, ctx),
            Command::ZoomReset => self.step_zoom(Direction::Reset, ctx),
            Command::Reader => self.toggle_reader(ctx),
            Command::FileView => self.toggle_file_view(ctx),
            Command::BackToTerminal => self.with_active_view(TabView::back_to_terminal),
            Command::PrevFile => self.with_active_view(|view| view.step_file(-1)),
            Command::NextFile => self.with_active_view(|view| view.step_file(1)),
            Command::PrevCommit => self.with_active_view(|view| view.step_commit(-1)),
            Command::NextCommit => self.with_active_view(|view| view.step_commit(1)),
            Command::FindCommit => self.find_commit(ctx),
            Command::Settings => self.toggle_settings(),
            Command::Shortcuts => self.toggle_shortcuts_settings(),
            Command::OpenRepo => self.run_repo_command(RepoCommand::Open, ctx),
            Command::Palette => self.toggle_palette(),
            Command::BuildWorkflow => self.run_build_workflow(ctx),
            Command::MoveSessions => self.run_move_sessions(ctx),
            Command::Amphetamine => self.run_amphetamine(ctx),
            Command::ApproveDelivery => {}
        }
    }

    pub(super) fn apply_frame_intent(&mut self, ctx: &egui::Context, intent: FrameIntent) {
        for action in intent.actions {
            self.dispatch(action, ctx);
        }
        if let Some(command) = intent.repo_command {
            self.run_repo_command(command, ctx);
        }
        if let Some(dir) = intent.open_dir {
            self.open_workspace(dir, ctx);
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
        crate::screen_kit::remember_focus(ctx);
        self.dev_session.tick(ctx, &mut self.tabs, now);
        if self.dev_session.restart_due(now) {
            self.restart_to_update(ctx);
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if self.last_used.gained_focus(focused) && self.welcome.is_none() {
            self.last_used.record(&super::workspaces::remembered(&self.workspaces, &self.tabs));
        }
        self.saver.tick(
            self.restore.settled(),
            super::workspaces::window_views(&self.workspaces, self.focus),
            &self.tabs,
        );
        if let Some(shot) = &mut self.screenshot {
            shot.tick(ctx);
        }
    }
}
