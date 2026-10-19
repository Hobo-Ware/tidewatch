//! Agent profiles and probes, listener messages and per-frame agent status.

use std::time::Duration;

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::trust::TrustCli;

use super::{ANIMATION_FRAME_MS, Tidewatch, WAKE_SLACK_S, wall_ms};
use crate::agent_frame::{self, Follow, Rows, Wake};
use crate::agent_jobs;
use crate::agent_run::TermSnapshot;
use crate::notify;
use crate::pane::{Body, Pane};
use crate::pane_jobs;
use crate::tabs::Tabs;
use crate::workflow_glue;

impl Tidewatch {
    pub(super) fn apply_agent_done(&mut self, done: agent_jobs::Done, ctx: &egui::Context) {
        match done {
            agent_jobs::Done::Loaded { profiles, workflows } => {
                self.catalog.set_workflows(workflows);
                let probes = self.catalog.set_profiles(profiles);
                self.probe_agents(probes, ctx);
                self.refresh_trust(ctx);
                if let Some(paths) = &self.paths {
                    self.restore.begin(ctx, paths, &self.catalog);
                }
                self.run_dev_agent(ctx);
            }
            agent_jobs::Done::Probed { agent, availability } => {
                self.catalog.set_availability(&agent, availability);
            }
            agent_jobs::Done::Saved { result, profiles } => match result {
                Ok(()) => {
                    let probes = self.catalog.set_profiles(profiles.clone());
                    self.probe_agents(probes, ctx);
                    if let Some(settings) = &mut self.settings {
                        settings.profiles_saved(&profiles);
                    }
                    self.show_toast("Profiles saved", ctx);
                }
                Err(message) => {
                    if let Some(settings) = &mut self.settings {
                        settings.profiles.save_failed(format!("Could not save: {message}"));
                    }
                }
            },
            agent_jobs::Done::WorkflowsSaved { result, workflows } => match result {
                Ok(()) => {
                    self.catalog.set_workflows(workflows);
                    if let Some(settings) = &mut self.settings {
                        settings.workflows.saved();
                    }
                    self.show_toast("Workflows saved", ctx);
                }
                Err(message) => {
                    if let Some(settings) = &mut self.settings {
                        settings.workflows.save_failed(format!("Could not save: {message}"));
                    }
                }
            },
        }
    }

    fn refresh_trust(&self, ctx: &egui::Context) {
        let bins = self
            .catalog
            .agent_names()
            .iter()
            .filter_map(|(id, _)| self.catalog.agent(id))
            .filter_map(|a| TrustCli::from_wiring(a.wiring).map(|cli| (cli, a.bin.clone())))
            .collect();
        self.trust.refresh(ctx, bins, self.catalog.path.clone());
    }

    fn probe_agents(&self, agents: Vec<(String, String)>, ctx: &egui::Context) {
        for (agent, bin) in agents {
            agent_jobs::probe(&self.agent_jobs, ctx, agent, bin, self.catalog.path.clone());
        }
    }

    fn follow(&mut self, follow: Follow, ctx: &egui::Context) {
        match follow {
            Follow::Finding { tab, notes_file, dir, author, finding } => {
                let dir = dir.unwrap_or_else(|| self.workspace_of_tab(tab).dir().to_path_buf());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                pane_jobs::add_finding(
                    &self.pane_jobs,
                    ctx,
                    tab,
                    notes_file,
                    dir,
                    author,
                    finding,
                    now,
                );
            }
            Follow::ReloadNotes { tab } => {
                if let Some(view) = self.views.get_mut(&tab) {
                    view.reload_notes();
                    ctx.request_repaint();
                }
            }
            Follow::Proposal { tab, workflow } => self.proposal_arrived(tab, &workflow, ctx),
            Follow::Progress { tab, ledger } => self.progress_arrived(tab, *ledger, ctx),
        }
    }

    /// Takes what agents reported, feeds terminal signals into every agent tab's status and
    /// notifies for changes while the window is away.
    pub(super) fn drive_agents(&mut self, ctx: &egui::Context, now: f64) {
        let mut asked = Vec::new();
        for message in self.hooks.drain() {
            if self.status_log {
                eprintln!(
                    "tidewatch: hook tab={} gen={} {:?} at {}",
                    message.tab_id,
                    message.generation,
                    message.event,
                    wall_ms()
                );
            }
            let Some(execs) = workflow_glue::feed(&mut self.tabs, &message) else { continue };
            let tab = message.tab_id;
            if let Some(follow) = agent_frame::route(&mut self.tabs, message, now) {
                self.follow(follow, ctx);
            }
            asked.push((tab, execs));
        }
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
        let away = notify::window_is_away(focused, minimized);
        let polled =
            agent_frame::poll(&mut self.tabs, TermSnapshot::of, &mut self.notifier, away, now);
        if self.status_log {
            for (tab, change) in &polled.changes {
                eprintln!(
                    "tidewatch: status tab={tab} {:?} -> {:?} at {}",
                    change.from,
                    change.to,
                    wall_ms()
                );
            }
        }
        asked.extend(workflow_glue::failures(&mut self.tabs, &polled.changes));
        self.drive_build_workflow(ctx, now);
        self.drive_workflows(ctx, now, asked);
        self.send_dev_keys(now);
        self.send_dev_paste(ctx, now);
        release_typed_commands(ctx, &mut self.tabs, now);
        let rows = |term: &PtyTerm| Rows {
            lines: term.buffer_lines().into_iter().map(|(_, line)| line).collect(),
            cols: term.cols(),
        };
        let output = |term: &PtyTerm| term.output_gen();
        if let Some(at) = agent_frame::watch_links(&mut self.tabs, output, rows, now) {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
        schedule(ctx, polled.wake, now, minimized == Some(true));
    }
}

/// How long after a resumed tab starts the timer for a shell that marks no prompt may still fire:
/// the wait stdusk gives it, and a second.
const TYPED_FALLBACK_WINDOW_S: f64 = 6.0;

/// A shell that marks its prompts gets the typed resume command at the first one. One that does
/// not (fish) gets it from a timer that only runs when asked, so ask for a while after the start.
fn release_typed_commands(ctx: &egui::Context, tabs: &mut Tabs<Pane<PtyTerm>>, now: f64) {
    for tab in tabs.items_mut() {
        let Pane { body: Body::Shell(term), agent: Some(agent), .. } = &mut tab.pane else {
            continue;
        };
        if agent.resume_note.is_some() && agent.run.elapsed(now) < TYPED_FALLBACK_WINDOW_S {
            term.release_pending_after_wait();
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

/// Asks for the next frame agent status needs: a short tick while a tab works (its bar moves), and
/// one wake-up at the moment a status decays. A window that is minimized does not animate.
fn schedule(ctx: &egui::Context, wake: Wake, now: f64, minimized: bool) {
    if wake.animate && !minimized {
        ctx.request_repaint_after(Duration::from_millis(ANIMATION_FRAME_MS));
    }
    if let Some(at) = wake.next_change {
        ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
    }
}
