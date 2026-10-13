//! The central panel: the overview, the settings page or the workspace, and the view toggles.

use std::time::Duration;

use egui::Frame;
use stdusk_egui::colors;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::status::AgentStatus;

use super::Tidewatch;
use super::frame::FrameIntent;
use super::terminal::TerminalGrid;
use crate::commands::Command;
use crate::delivery_card::Delivery;
use crate::reader::Shown;
use crate::tabs::Action;
use crate::{
    delivery_view, end_block_view, layout, overview, overview_view, reader_glue, stepper_view,
};

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
        egui::CentralPanel::default().frame(Frame::new().fill(colors::bg())).show(ui, |ui| {
            let area = ui.max_rect();
            if self.overview && self.settings.is_none() {
                self.show_overview(ui, area, ctx, now, frame);
                return;
            }
            if self.settings.is_some() {
                frame.settings_action = self.show_settings_page(ui, area);
                return;
            }
            self.show_workspace(ui, area, ctx, now, frame);
        });
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
        let panels = self.workspace_has_panels() && !self.focus;
        let mut areas = layout::split(area);
        if !panels {
            areas.terminal = layout::terminal_only(area);
        }
        self.show_stepper(ui, &mut areas.terminal, ctx);
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
        let grid = TerminalGrid::measure(ui, areas.terminal);

        let agent_name = self.tabs.items()[self.tabs.active()]
            .pane
            .agent
            .as_ref()
            .map(|agent| agent.profile_name.clone());
        let shown = reader_glue::shown(self.tabs.active_pane_mut());
        self.show_terminal(ui, &grid, agent_name.as_deref(), shown == Some(Shown::Reader), frame);
        self.show_reader(ui, areas.terminal, shown, ctx, now, active_id);

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
