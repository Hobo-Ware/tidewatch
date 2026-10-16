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
use crate::commands::Command;
use crate::delivery_card::Delivery;
use crate::pane::FirstPrompt;
use crate::reader::Shown;
use crate::tab_view::TabView;
use crate::tabs::Action;
use crate::workflow_run::TabRun;
use crate::{
    agent_status_line, brand, delivery_view, end_block_view, layout, overview, overview_view,
    reader_glue, stepper_view, welcome_view, workflow_start_view,
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
        overview::build(places)
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
        self.workspace_has_panels() && !self.focus && !self.awaiting_workflow_task()
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
                self.show_welcome(ui, area, frame);
                return;
            }
            self.show_workspace(ui, area, ctx, now, frame);
        });
    }

    fn show_welcome(&mut self, ui: &mut egui::Ui, area: egui::Rect, frame: &mut FrameIntent) {
        let home = self.paths.as_ref().and_then(|paths| paths.root().parent());
        let recent = self.welcome.as_deref().unwrap_or_default();
        match welcome_view::show(ui, area, recent, home, &Command::OpenRepo.keys()) {
            Some(welcome_view::Action::Open) => frame.repo_command = Some(RepoCommand::Open),
            Some(welcome_view::Action::Recent(dir)) => frame.open_dir = Some(dir),
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
        let data = self.overview_data(now);
        if overview::is_ticking(&data) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if let Some(tab) = overview_view::show(ui, area, &data)
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
        let active_id = frame.active_id;
        let awaiting_task = self.awaiting_workflow_task();
        let panels = self.panes_showing();
        let side = self.side_layout();
        if side && panels {
            self.drag_divider(ui, area);
        }
        let mut areas = if side {
            layout::split_side(area, self.side_fraction)
        } else if self.awaiting_first_prompt() {
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
        self.show_stepper(ui, &mut areas.terminal, ctx);
        if awaiting_task {
            self.show_workflow_start(ui, areas.terminal, ctx);
            self.show_toast_pill(ui);
            return;
        }
        self.show_status_line(ui, &mut areas.terminal);
        let end = self.waiting_end_block();
        let end_band = end.as_ref().map(|(block, open)| {
            layout::band_below(&mut areas.terminal, end_block_view::height(block, *open))
        });
        let card = self.flow.tab(active_id).and_then(|state| {
            state
                .delivery
                .card_commits()
                .map(|n| (n, matches!(state.delivery, Delivery::Building { .. })))
        });
        let band = card.map(|_| layout::band_below(&mut areas.terminal, delivery_view::BAND_H));
        let grid = TerminalGrid::measure(ui, areas.terminal, self.terminal_font.in_use());

        let agent_name = self.tabs.items()[self.tabs.active()]
            .pane
            .agent
            .as_ref()
            .map(|agent| agent.profile_name.clone());
        let shown = reader_glue::shown(self.tabs.active_pane_mut());
        self.show_terminal(ui, &grid, agent_name.as_deref(), shown == Some(Shown::Reader), frame);
        self.show_reader(ui, areas.terminal, shown, ctx, now, active_id, !frame.keys.modal_open);

        if let (Some((block, open)), Some(band)) = (&end, end_band)
            && end_block_view::show_card(ui, band, block, *open)
            && let Some(agent) = self.tabs.active_pane_mut().agent.as_mut()
        {
            agent.run.end_open = !agent.run.end_open;
        }
        if let (Some((commits, building)), Some(band)) = (card, band)
            && delivery_view::show_card(ui, band, commits, building).is_some()
        {
            self.flow.open_card(active_id, ctx);
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

    pub(super) fn awaiting_workflow_task(&self) -> bool {
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        pane.agent
            .as_ref()
            .and_then(|agent| agent.workflow.as_deref())
            .is_some_and(TabRun::awaiting_task)
    }

    fn show_workflow_start(&mut self, ui: &mut egui::Ui, area: egui::Rect, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let Some(run) = self
            .tabs
            .active_pane_mut()
            .agent
            .as_mut()
            .and_then(|agent| agent.workflow.as_deref_mut())
        else {
            return;
        };
        let area = area.expand(layout::TERMINAL_PAD);
        if let Some(task) = workflow_start_view::show(ui, area, run) {
            let execs = run.start_with(&task);
            if !execs.is_empty() {
                self.run_asks.push((active_id, execs));
                ctx.request_repaint();
            }
        }
    }

    fn show_status_line(&mut self, ui: &mut egui::Ui, terminal: &mut egui::Rect) {
        let pane = self.tabs.active_pane_mut();
        let Some(shown) = reader_glue::shown(pane) else { return };
        let (cli_name, available) = pane.agent.as_ref().map_or_else(Default::default, |agent| {
            (
                self.catalog.agent(&agent.run.agent_id).map(|a| a.name.clone()).unwrap_or_default(),
                agent.reader.available(),
            )
        });
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        let Some(line) = agent_status_line::line_of(pane, shown, available, &cli_name) else {
            return;
        };
        let rect = layout::band_above(terminal, agent_status_line::HEIGHT);
        if let Some(choice) = agent_status_line::show(ui, rect, &line)
            && let Some(agent) = self.tabs.active_pane_mut().agent.as_mut()
        {
            agent.reader.set_wants_reader(choice == agent_status_line::Choice::Reader);
        }
    }

    fn show_stepper(&mut self, ui: &mut egui::Ui, terminal: &mut egui::Rect, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let header = self
            .tabs
            .active_pane_mut()
            .agent
            .as_mut()
            .and_then(|agent| agent.workflow.as_mut())
            .map(|run| stepper_view::show(ui, terminal, run));
        if let Some(execs) = header.filter(|execs| !execs.is_empty()) {
            self.run_asks.push((active_id, execs));
            ctx.request_repaint();
        }
    }

    fn waiting_end_block(&self) -> Option<(EndBlock, bool)> {
        self.tabs.items()[self.tabs.active()].pane.agent.as_ref().and_then(|agent| {
            let waiting =
                matches!(agent.run.status(), AgentStatus::Done | AgentStatus::NeedsYou(_));
            agent
                .run
                .end_block()
                .filter(|_| waiting)
                .map(|block| (block.clone(), agent.run.end_open))
        })
    }
}
