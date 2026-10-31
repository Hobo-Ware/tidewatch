//! The central panel: the overview, the settings page or the workspace, and the view toggles.

use std::time::Duration;

use egui::Frame;
use tidewatch_core::prefs::Panels;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::status::AgentStatus;

use super::Tidewatch;
use super::frame::FrameIntent;
use super::repos::RepoCommand;
use super::terminal::TerminalGrid;
use crate::answer_chips::Pick;
use crate::coming_up::{Row as Scheduled, coming_up, with_extra};
use crate::commands::Command;
use crate::pane::FirstPrompt;
use crate::progress_glue::today;
use crate::reader::Shown;
use crate::repo::Workspace;
use crate::tab_view::TabView;
use crate::tabs::Action;
use crate::{
    agenda, agent_status_line, brand, closed_view, delivery_view, layout, overview, overview_fake,
    overview_view, reader_glue, takeover_view, welcome_view,
};

const EMPTY_PANELS_H: f32 = 200.0;

impl Tidewatch {
    fn overview_data(&self, now: f64) -> overview::Overview {
        let places = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(group, workspace)| {
                overview::collect(&workspace.chip_label(), workspace.dir(), group, &self.tabs, now)
            })
            .collect();
        let mut overview = overview::build(places);
        self.add_pr_pills(&mut overview);
        self.add_done_check_tags(&mut overview);
        self.add_looks(&mut overview);
        overview
    }

    pub(super) fn place_label(&self, repo: &std::path::Path) -> String {
        self.workspaces.iter().find(|workspace| workspace.dir() == repo).map_or_else(
            || repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            Workspace::chip_label,
        )
    }

    fn add_next(&self, overview: &mut overview::Overview, rows: &[Scheduled]) {
        for row in overview.places.iter_mut().flat_map(|place| place.rows.iter_mut()) {
            let id = row.tab_id;
            row.next = overview::next_of(
                rows.iter().filter(|due| self.open_tab_of(&due.repo, &due.tab) == Some(id)),
            );
        }
    }

    pub(super) fn toggle_overview(&mut self) {
        self.overview = !self.overview;
        self.settings = None;
    }

    pub(super) fn toggle_reader(&mut self, ctx: &egui::Context) {
        if let Some(shown) = reader_glue::toggle(self.tabs.active_pane_mut()) {
            self.show_toast(reader_glue::toggle_text(shown), ctx);
        }
    }

    pub(super) fn side_layout(&self) -> bool {
        self.shell.prefs().panels == Panels::Side
    }

    pub(super) fn panes_showing(&self) -> bool {
        self.workspace_has_panels() && !self.focus && !self.active_is_poofed()
    }

    /// Whether the active tab draws a file's diff where its terminal would be.
    pub(super) fn file_view_showing(&self, active_id: u64) -> bool {
        self.side_layout()
            && self.panes_showing()
            && self.views.get(&active_id).is_some_and(TabView::file_open)
    }

    pub(super) fn with_active_view(&mut self, change: impl FnOnce(&mut TabView)) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        if let Some(view) = self.views.get_mut(&active_id) {
            change(view);
        }
    }

    pub(super) fn toggle_file_view(&mut self, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let shown = self.side_layout()
            && self.views.get_mut(&active_id).is_some_and(TabView::toggle_file_view);
        if !shown {
            self.show_toast("Open a file from the History tree first", ctx);
        }
    }

    pub(super) fn toggle_focus(&mut self, ctx: &egui::Context) {
        self.focus = !self.focus;
        let state = if self.focus { "on" } else { "off" };
        self.show_toast(format!("Focus mode {state} ({})", Command::Focus.keys()), ctx);
    }

    pub(super) fn show_central_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        now: f64,
        frame: &mut FrameIntent,
    ) {
        egui::CentralPanel::default().frame(Frame::new().fill(brand::GROUND)).show(ui, |ui| {
            let area = ui.max_rect();
            if self.overview && self.settings.is_none() && self.welcome.is_none() {
                self.show_overview(ui, area, ctx, now, frame);
                return;
            }
            if self.settings.is_some() {
                frame.settings_action = self.show_settings_page(ui, area);
                return;
            }
            if self.welcome.is_some() {
                self.show_welcome(ui, ctx, area, frame);
                return;
            }
            self.show_workspace(ui, area, ctx, now, frame);
        });
    }

    fn show_welcome(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        area: egui::Rect,
        frame: &mut FrameIntent,
    ) {
        self.open_pasted_link(ctx);
        let home = self.paths.as_ref().and_then(|paths| paths.root().parent());
        let recent = self.welcome.as_deref().unwrap_or_default();
        let elsewhere = self.takeover_welcome_line();
        match welcome_view::show(ui, area, recent, home, elsewhere) {
            Some(welcome_view::Action::Open) => frame.repo_command = Some(RepoCommand::Open),
            Some(welcome_view::Action::Recent(dir)) => frame.open_dir = Some(dir),
            Some(welcome_view::Action::MoveSessions) => self.open_batch(ctx),
            None => {}
        }
        self.show_toast_pill(ui);
    }

    fn show_overview(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        ctx: &egui::Context,
        now: f64,
        frame: &mut FrameIntent,
    ) {
        let fake = overview_fake::requested().map(overview_fake::build);
        let rows = if fake.is_some() {
            Vec::new()
        } else {
            with_extra(coming_up(today(), self.progress.index()), self.progress.next_rows())
        };
        let mut data =
            fake.as_ref().map_or_else(|| self.overview_data(now), |f| f.overview.clone());
        if fake.is_none() {
            self.add_next(&mut data, &rows);
        }
        if overview::is_ticking(&data) {
            ctx.request_repaint_after(Duration::from_secs(1));
        } else {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
        let card = match &fake {
            Some(fake) => Some(fake.takeover.clone()),
            None => self.takeover_card(),
        };
        let closed = self.closed_card();
        let reveal = card.is_some() && self.takeover.take_reveal();
        let line_open = self.takeover.line_open();
        let mut clicked = None;
        let mut closed_clicked = None;
        let days = match &fake {
            Some(fake) => fake.agenda.clone(),
            None => agenda::days(&rows, today(), |repo| self.place_label(repo)),
        };
        let entries = if fake.is_some() { Vec::new() } else { self.meeseeks.poofed_on(today()) };
        let poofed = match &fake {
            Some(fake) => fake.poofed.clone(),
            None => overview::poofed_rows(&entries, |repo| self.place_label(repo)),
        };
        let sections = overview_view::Sections { agenda: &days, poofed: &poofed };
        let mut state = std::mem::take(&mut self.overview_state);
        let out = overview_view::show(ui, area, &data, &mut state, &sections, |ui| {
            if let Some(card) = &card {
                clicked = takeover_view::line(ui, card, line_open, reveal);
            }
            if let Some(closed) = &closed {
                if card.is_some() {
                    ui.add_space(8.0);
                }
                closed_clicked = closed_view::section(ui, closed);
            }
        });
        self.overview_state = state;
        if let Some(row) = out.agenda.and_then(|index| rows.get(index)) {
            self.open_scheduled(row, ctx);
        }
        if let Some(record) = out.poofed.and_then(|index| entries.get(index)) {
            let tab = self.open_poofed(record);
            if let Some(index) = self.tabs.index_of(tab) {
                frame.actions.push(Action::Select(index));
            }
        }
        let mut jump = out.jump;
        match out.answer {
            Some((tab, Pick::Answer(text))) => {
                self.send_answer(tab, text, ctx, ctx.input(|i| i.time));
            }
            Some((tab, Pick::Other)) => {
                self.open_reply_box_of(tab);
                jump = Some(tab);
            }
            None => {}
        }
        if let Some(action) = clicked {
            self.takeover_click(action, ctx);
        }
        if let Some(action) = closed_clicked {
            self.closed_click(action, ctx);
        }
        if let Some(tab) = jump
            && let Some(index) = self.tabs.index_of(tab)
        {
            frame.actions.push(Action::Select(index));
        }
    }

    fn show_workspace(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        ctx: &egui::Context,
        now: f64,
        frame: &FrameIntent,
    ) {
        if self.active_is_poofed() {
            self.show_poofed(ui, area, ctx);
            self.show_toast_pill(ui);
            return;
        }
        let active_id = frame.active_id;
        let panels = self.panes_showing();
        let side = self.side_layout();
        if side && panels {
            self.drag_divider(ui, area);
        }
        let mut areas = if side {
            layout::split_side(area, self.side_fraction)
        } else if self.awaiting_first_prompt() || self.active_is_draft() {
            layout::split_panels(area, EMPTY_PANELS_H)
        } else {
            layout::split(area)
        };
        if !panels {
            areas.terminal = layout::terminal_only(area);
        }
        if self.file_view_showing(active_id) {
            self.show_panes(ui, &areas, ctx);
            self.show_toast_pill(ui);
            return;
        }
        if !self.active_is_draft()
            && let Some(name) = self.service_logs_showing()
        {
            let keys_free = !frame.keys.modal_open && !frame.keys.typing_in_a_panel;
            self.show_service_logs(ui, areas.terminal, &name, keys_free, frame.keys.modal_open);
            if panels {
                self.show_panes(ui, &areas, ctx);
            }
            self.show_toast_pill(ui);
            return;
        }
        if self.active_is_draft() {
            self.show_draft(ui, areas.terminal, ctx, frame);
            if panels {
                self.show_panes(ui, &areas, ctx);
            }
            self.show_toast_pill(ui);
            return;
        }
        self.show_status_line(ui, &mut areas.terminal);
        self.show_pr_strip(ui, &mut areas.terminal, ctx, active_id);
        self.show_comment_strip(ui, &mut areas.terminal, active_id);
        let agent_name = self.tabs.items()[self.tabs.active()]
            .pane
            .agent
            .as_ref()
            .map(|agent| agent.profile_name.clone());
        let card = agent_name.as_deref().and_then(|name| {
            let state = self.flow.tab(active_id)?;
            state.delivery.card(state.after_merge(), name)
        });
        let band =
            card.as_ref().map(|_| layout::band_below(&mut areas.terminal, delivery_view::BAND_H));
        let grid_area =
            if agent_name.is_some() { layout::agent_grid(areas.terminal) } else { areas.terminal };
        let grid = TerminalGrid::measure(ui, grid_area, self.terminal_font.in_use());
        let shown = reader_glue::shown(self.tabs.active_pane_mut());
        self.show_terminal(ui, &grid, agent_name.as_deref(), shown == Some(Shown::Reader), frame);
        self.show_reader(ui, areas.terminal, shown, ctx, now, active_id, !frame.keys.modal_open);

        if let (Some(view), Some(band)) = (&card, band)
            && delivery_view::show_card(ui, band, view).is_some()
        {
            self.flow.ask_to_open(active_id);
        }
        if panels {
            self.show_panes(ui, &areas, ctx);
        }
        self.show_toast_pill(ui);
    }

    fn drag_divider(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let grab = layout::divider_grab(area, self.side_fraction);
        let response = ui.interact(grab, egui::Id::new("side-divider"), egui::Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if let Some(pointer) = response.interact_pointer_pos().filter(|_| response.dragged()) {
            self.side_fraction = layout::fraction_at(area, pointer.x);
        }
    }

    pub(super) fn awaiting_first_prompt(&self) -> bool {
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        pane.agent.as_ref().is_some_and(|agent| agent.first_prompt == FirstPrompt::Awaiting)
    }

    fn show_status_line(&mut self, ui: &mut egui::Ui, terminal: &mut egui::Rect) {
        let pane = self.tabs.active_pane_mut();
        let Some(shown) = reader_glue::shown(pane) else { return };
        let (cli_name, available, remote_app) =
            pane.agent.as_ref().map_or_else(Default::default, |agent| {
                let cli = self.catalog.agent(&agent.run.agent_id);
                (
                    cli.map(|a| a.name.clone()).unwrap_or_default(),
                    agent.reader.available(),
                    cli.and_then(|a| a.remote.app()),
                )
            });
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        let Some(line) =
            agent_status_line::line_of(pane, shown, available, (&cli_name, remote_app))
        else {
            return;
        };
        let rect = layout::band_above(terminal, agent_status_line::HEIGHT);
        let out = agent_status_line::show(ui, rect, &line, self.dev_remote_popover);
        if let Some(choice) = out.choice
            && let Some(agent) = self.tabs.active_pane_mut().agent.as_mut()
        {
            agent.reader.set_wants_reader(choice == agent_status_line::Choice::Reader);
        }
        if let Some(action) = out.remote {
            self.apply_remote_action(action, ui.ctx());
        }
        if out.change {
            self.open_change_active(ui.ctx());
        }
    }

    pub(super) fn waiting_end_block(&self) -> Option<EndBlock> {
        let tab = &self.tabs.items()[self.tabs.active()];
        let agent = tab.pane.agent.as_ref()?;
        let waiting = matches!(agent.run.status(), AgentStatus::Done | AgentStatus::NeedsYou(_));
        if !waiting {
            return None;
        }
        match self.panel_view_of(tab.id) {
            Some(view) => view.end_block,
            None => agent.run.end_block().cloned(),
        }
    }
}
