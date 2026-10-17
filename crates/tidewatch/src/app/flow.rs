//! Workflow runs, the restore and the delivery flow, driven once per frame.

use std::path::Path;
use std::time::Duration;

use tidewatch_core::prepare::Prepared;
use tidewatch_core::restore::WindowPlan;
use tidewatch_core::worktree::Worktree;

use super::{Tidewatch, WAKE_SLACK_S};
use crate::flow_glue;
use crate::pty::{spawn_agent, spawn_shell, spawn_shell_noted, spawn_typed};
use crate::repo::Workspace;
use crate::restore_glue::Spawn;
use crate::workflow_glue::{Env, Spawners, Start};
use crate::workflow_run::Exec;

impl Tidewatch {
    /// A workflow needs a worktree, so only a repo can start one. The first stage starts when the
    /// worktree is ready.
    pub(super) fn open_workflow(
        &mut self,
        ctx: &egui::Context,
        start: Start,
        existing: Option<Worktree>,
        into: Option<u64>,
    ) {
        let group = self.tabs.active_group();
        let (Workspace::Repo(repo), Some(paths)) = (&self.workspaces[group], &self.paths) else {
            self.show_toast("A workflow needs a git repo", ctx);
            return;
        };
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let now = ctx.input(|i| i.time);
        self.runs.open(ctx, &mut self.tabs, group, &env, &repo.root, existing, into, start, now);
    }

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
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let typed = |dir: &Path, prepared: &Prepared| spawn_typed(ctx, dir, prepared);
        let shell = |dir: &Path, note: Option<&str>| spawn_shell_noted(ctx, dir, note);
        let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
        let runs = &mut self.runs;
        let report = match plan {
            Some(plan) => {
                let group = self.tabs.active_group();
                self.restore.apply(&plan, group, ctx, &mut self.tabs, &env, &spawn, runs, now)
            }
            None => self.restore.tick(ctx, &mut self.tabs, &env, &spawn, runs, now),
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

    /// Carries out what the workflow runs asked for (from messages, failed processes and clicks) and
    /// applies what their jobs finished.
    pub(super) fn drive_workflows(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        mut asked: Vec<(u64, Vec<Exec>)>,
    ) {
        let Some(paths) = &self.paths else { return };
        asked.append(&mut self.run_asks);
        if !self.dev_workflow.is_idle() {
            asked.extend(self.dev_workflow.drive(&mut self.tabs));
        }
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let shell = |dir: &Path| spawn_shell(ctx, dir);
        let spawn = Spawners { agent: &agent, shell: &shell };
        let toasts = self.runs.tick(ctx, &mut self.tabs, &env, &spawn, asked, now);
        for text in toasts {
            self.show_toast(text, ctx);
        }
    }

    /// Starts what the flow needs for each agent tab, applies what its jobs finished and presses the
    /// Enter that follows a paste.
    pub(super) fn drive_flow(&mut self, ctx: &egui::Context, now: f64) {
        let unix = flow_glue::unix_now();
        let outs = self.flow.drain(ctx, now);
        for text in flow_glue::apply_outs(outs, &mut self.tabs, &mut self.views) {
            self.show_toast(text, ctx);
        }
        for tab in self.flow.due_enters(now) {
            flow_glue::press_enter(&mut self.tabs, tab);
        }
        let active = self.tabs.items()[self.tabs.active()].id;
        let facts = flow_glue::facts(&self.tabs, &self.views, active);
        if let Some(at) = self.flow.tick(&facts, ctx, now, unix) {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
        if !self.dev_flow.is_idle() {
            let view = self.views.get(&active);
            flow_glue::dev_drive(&mut self.dev_flow, &mut self.flow, view, active, ctx);
        }
    }
}
