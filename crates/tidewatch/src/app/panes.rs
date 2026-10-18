//! The History and Diff panes: one view per tab, finished pane jobs, notes and checkout.

use std::collections::HashSet;

use kelp_core::refs::RefLabel;
use kelp_ui::graph::GraphAction;
use kelp_ui::theme as kelp_theme;

use super::{Tidewatch, wall_ms};
use crate::checkout::{self, Plan};
use crate::diff_pane::{PaneAction, PaneOut};
use crate::file_view::{self, TerminalChip};
use crate::repo::Workspace;
use crate::tab_view::ViewSpec;
use crate::{diff_pane, flow_glue, history_pane, layout, pane_jobs, progress_view};

impl Tidewatch {
    /// Keeps one view per tab that has panes: only the visible tab's view is loaded and watched.
    pub(super) fn drive_views(&mut self) {
        let open: HashSet<u64> = self.tabs.items().iter().map(|t| t.id).collect();
        self.views.retain(|id, _| open.contains(id));
        if self.focus || self.overview {
            for view in self.views.values_mut().filter(|view| view.is_active()) {
                view.stop();
            }
            return;
        }
        let active = &self.tabs.items()[self.tabs.active()];
        let active_id = active.id;
        for (id, view) in &mut self.views {
            if *id != active_id && view.is_active() {
                view.stop();
            }
        }
        let workspace = &self.workspaces[active.group];
        let Some(spec) = ViewSpec::of(workspace, self.paths.as_ref(), active) else { return };
        let dev_scope = self.dev_panes.scope;
        let side = self.side_layout();
        let view = self.views.entry(active_id).or_insert_with(|| {
            let mut view = spec.into_view();
            if let Some(scope) = dev_scope {
                view.set_scope(scope);
            }
            view
        });
        if !view.is_active() {
            view.start();
        }
        view.set_counted(side);
        view.poll();
    }

    pub(super) fn apply_pane_done(&mut self, done: pane_jobs::Done, ctx: &egui::Context) {
        let tab = done.tab();
        if let pane_jobs::Done::Checkout { outcome, .. } = &done {
            self.show_toast(checkout::message(outcome), ctx);
            if let Some(view) = self.views.get_mut(&tab) {
                view.reload_after_checkout();
            }
            return;
        }
        if let Some(view) = self.views.get_mut(&tab) {
            view.apply(done);
        }
    }

    fn double_clicked_ref(&mut self, label: &RefLabel, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let Some(view) = self.views.get(&active_id) else { return };
        match checkout::plan(view.is_main, label) {
            Plan::Ignore => {}
            Plan::Refuse(reason) => self.show_toast(reason, ctx),
            Plan::Switch(op) => {
                pane_jobs::run_checkout(&self.pane_jobs, ctx, active_id, view.dir.clone(), op);
            }
        }
    }

    pub(super) fn save_notes(&self, view_id: u64) {
        let Some(view) = self.views.get(&view_id) else { return };
        if let Some(file) = view.notes_file() {
            self.notes_writer.save(file.clone(), view.notes.clone());
        }
    }

    pub(super) fn workspace_has_panels(&self) -> bool {
        matches!(self.workspace(), Workspace::Repo(_))
    }

    pub(super) fn show_panes(
        &mut self,
        ui: &mut egui::Ui,
        areas: &layout::Layout,
        ctx: &egui::Context,
    ) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let side = self.side_layout();
        let Some(view) = self.views.get_mut(&active_id) else {
            if side {
                history_pane::paint_card(ui, areas.history);
            } else {
                for rect in [areas.history, areas.diff] {
                    ui.painter().rect_filled(rect, 0.0, kelp_theme::bg());
                }
            }
            return;
        };
        let agent = self.tabs.items()[self.tabs.active()].pane.agent.as_ref();
        let status = agent.map(|agent| agent.run.status().clone());
        let chip = TerminalChip { profile: agent.map(|a| a.profile_name.clone()), status };
        let extras = chip
            .status
            .as_ref()
            .and_then(|status| self.flow.extras(active_id, &view.notes, status));
        let agent_id = agent.map_or("", |agent| agent.run.agent_id.as_str());
        let progress = agent.and_then(|_| {
            let known = self.progress.ledger(active_id)?;
            let input = progress_view::Input {
                ledger: known,
                now_ms: wall_ms() as i64,
                time: ctx.input(|i| i.time),
                animate: ctx.input(|i| i.viewport().focused.unwrap_or(true)),
                first: self.new_tab_prefs.right_panel.into(),
            };
            Some(history_pane::Progress { input, state: self.progress_ui.tab(active_id) })
        });
        let shown = history_pane::show(
            ui,
            areas.history,
            view,
            &mut self.avatars,
            side,
            (agent_id, progress),
        );
        let graph_action = shown.graph;
        let answer = shown.progress == Some(progress_view::Action::Answer);
        let now_unix = flow_glue::unix_now();
        let pane = match (side, view.file_open()) {
            (false, _) => diff_pane::show(ui, areas.diff, view, extras.as_ref(), now_unix),
            (true, true) => file_view::show(ui, areas.diff, view, extras.as_ref(), &chip, now_unix),
            (true, false) => PaneOut::default(),
        };
        if pane.back {
            view.back_to_terminal();
        }
        let notes_changed = pane.notes_changed;
        let open_review = pane.actions.contains(&PaneAction::OpenReview);
        let toasts = flow_glue::pane_actions(&mut self.flow, view, active_id, pane.actions, ctx);
        let mut clicked_ref = None;
        match graph_action {
            Some(GraphAction::Select(selection)) => {
                if side {
                    view.click_row(selection);
                } else {
                    view.select(selection);
                }
                ctx.request_repaint();
            }
            Some(GraphAction::ActivateRef(label)) => clicked_ref = Some(label),
            Some(_) | None => {}
        }
        view.sync(&self.pane_jobs, ctx);
        for text in toasts {
            self.show_toast(text, ctx);
        }
        if open_review {
            self.open_review();
        }
        if answer {
            self.open_reply_box();
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if notes_changed || self.apply_dev_panes(active_id, now) {
            self.save_notes(active_id);
            ctx.request_repaint();
        }
        if let Some(label) = clicked_ref {
            self.double_clicked_ref(&label, ctx);
        }
    }
}
