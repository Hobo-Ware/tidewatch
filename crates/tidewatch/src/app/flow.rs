//! The restore and the delivery flow, driven once per frame.

use std::path::Path;
use std::time::Duration;

use tidewatch_core::prepare::Prepared;
use tidewatch_core::restore::WindowPlan;

use super::{Tidewatch, WAKE_SLACK_S};
use crate::flow_glue;
use crate::pty::{spawn_agent, spawn_shell_noted, spawn_typed};
use crate::reader_glue;
use crate::restore_glue::{Env, Spawn};

impl Tidewatch {
    /// Carries out the restore: the plan of a dev switch, or whatever the startup and reopen jobs
    /// finished.
    pub(super) fn drive_restore(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        plan: Option<WindowPlan>,
    ) {
        let Some(paths) = &self.paths else { return };
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let typed = |dir: &Path, prepared: &Prepared| spawn_typed(ctx, dir, prepared);
        let shell = |dir: &Path, note: Option<&str>| spawn_shell_noted(ctx, dir, note);
        let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
        let report = match plan {
            Some(plan) => {
                let group = self.tabs.active_group();
                self.restore.apply(&plan, group, ctx, &mut self.tabs, &env, &spawn, now)
            }
            None => self.restore.tick(ctx, &mut self.tabs, &env, &spawn, now),
        };
        for text in report.toasts {
            self.show_toast(text, ctx);
        }
        self.notice = report.notice.or(self.notice.take());
        if let Some(leftovers) = report.leftovers {
            self.leftovers.add(leftovers);
        }
        if let Some(focus) = report.focus {
            self.focus = focus;
        }
    }

    /// Starts what the flow needs for each agent tab, applies what its jobs finished and presses the
    /// Enter that follows a paste.
    pub(super) fn drive_flow(&mut self, ctx: &egui::Context, now: f64) {
        let unix = flow_glue::unix_now();
        let outs = self.flow.drain(now);
        let toasts =
            flow_glue::apply_outs(outs, &mut self.tabs, &mut self.views, &mut self.flow, ctx, now);
        for text in toasts {
            self.show_toast(text, ctx);
        }
        for text in flow_glue::drain_outbox(&mut self.tabs, &mut self.flow, ctx, now) {
            self.show_toast(text, ctx);
        }
        for text in reader_glue::release_lines(&mut self.tabs, &mut self.flow, ctx, now) {
            self.show_toast(text, ctx);
        }
        let tabs = &self.tabs;
        for tab in self.flow.due_enters(now, |id| flow_glue::output_of(tabs, id)) {
            flow_glue::press_enter(&mut self.tabs, tab);
        }
        let active = self.tabs.items()[self.tabs.active()].id;
        let facts =
            flow_glue::facts(&self.tabs, &self.views, &self.pr, &self.done_check, active, now);
        for tab in self.flow.waiting_for_tests(&facts) {
            if self.done_check.is_idle(tab) {
                self.start_done_check(tab, ctx);
            }
        }
        if let Some(at) = self.flow.tick(&facts, ctx, now, unix) {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
        for text in self.flow.take_notices() {
            self.show_toast(text, ctx);
        }
        for text in flow_glue::ask_agents(&mut self.tabs, &mut self.flow, ctx, now) {
            self.show_toast(text, ctx);
        }
        if !self.dev_flow.is_idle() {
            let view = self.views.get(&active);
            flow_glue::dev_drive(&mut self.dev_flow, &mut self.flow, view, active, ctx);
        }
    }
}
