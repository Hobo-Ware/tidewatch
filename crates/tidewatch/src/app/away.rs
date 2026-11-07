//! The window's side of Away: the flip, its saved choice, and what the hook listener is told.

use tidewatch_core::agents::wakes_itself;
use tidewatch_core::amphetamine::State;

use tidewatch_core::progress_store;
use tidewatch_core::sent::{Reason, because};

use super::Tidewatch;
use crate::away_model::{Open, Presence, Verdict, flip, hearers, keep_going, nudge_text, told};
use crate::flow_glue;
use crate::outbox::{Offer, Send};
use crate::pane::Body;
use crate::prefs_save::update_in_background;

impl Tidewatch {
    pub(super) fn tell_listener_away(&self) {
        if let Some(listener) = self.hooks.listener() {
            listener.set_away(self.away.is_away());
        }
    }

    /// The listener tells the agents that launch from now on whether the Mac is kept awake.
    pub(super) fn tell_listener_awake(&self) {
        if let Some(listener) = self.hooks.listener() {
            listener.set_kept_awake(self.amphetamine.state().is_on());
        }
    }

    fn tell_agents_away(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let hearing = hearers(self.tabs.items().iter().map(|tab| {
            let agent = tab.pane.agent.as_ref().filter(|_| matches!(tab.pane.body, Body::Shell(_)));
            (tab.id, agent.map(|agent| agent.run.status()))
        }));
        for tab in hearing {
            let wakes = self.tabs.agent(tab).is_some_and(|agent| wakes_itself(&agent.run.agent_id));
            let (text, because) = told(self.away.is_away(), wakes);
            let send = Send::by_hand(&text, Reason::Away, because);
            flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now);
        }
    }

    /// An agent that stopped with open steps and set no wake-up for itself gets one keep-going
    /// message, after a grace period. The agent is the one that schedules its own wake-ups.
    pub(super) fn drive_away(&mut self, ctx: &egui::Context, now: f64) {
        let Some(paths) = self.paths.clone() else { return };
        let live: Vec<u64> = self.tabs.items().iter().map(|tab| tab.id).collect();
        self.away_watch.keep_only(&live);
        let away = self.away.is_away();
        let mut idle = Vec::new();
        for tab in self.tabs.items() {
            let Some(agent) =
                tab.pane.agent.as_ref().filter(|_| matches!(tab.pane.body, Body::Shell(_)))
            else {
                continue;
            };
            let Some(worktree) = tab.pane.worktree.as_ref() else { continue };
            let spoke = self.flow.outbox.spoke(tab.id);
            self.away_watch.tick(tab.id, agent.run.status(), spoke, now);
            if !away {
                self.away_watch.settled(tab.id);
            }
            if let Some(secs) = self.away_watch.idle(tab.id, now) {
                let file = paths.progress_file(&worktree.repo, worktree.name());
                idle.push((tab.id, secs, file, wakes_itself(&agent.run.agent_id)));
            }
        }
        for (tab, secs, file, wakes) in idle {
            let report = if secs < crate::away_model::GRACE_SECS {
                None
            } else {
                progress_store::load(&file).as_ref().map(Open::of)
            };
            let wake = self.away_watch.wake(tab);
            let last = self.away_watch.last(tab);
            match keep_going(away, wakes, report, wake, secs, last) {
                Verdict::Wait(left) => crate::wake::request_wake(ctx, left),
                Verdict::Quiet => self.away_watch.settled(tab),
                Verdict::Send => {
                    let Some(report) = report else {
                        self.away_watch.settled(tab);
                        continue;
                    };
                    let text = nudge_text(wakes, report);
                    let send = Send::auto(text, Reason::KeepGoing, because::KEEP_GOING);
                    let offer = flow_glue::send_or_queue(
                        &mut self.tabs,
                        &mut self.flow,
                        tab,
                        send,
                        ctx,
                        now,
                    );
                    if matches!(offer, Offer::Go | Offer::Wait) {
                        self.away_watch.nudged(tab, report);
                    } else {
                        self.away_watch.settled(tab);
                    }
                }
            }
        }
    }

    pub(super) fn flip_away(&mut self, ctx: &egui::Context) {
        let keep_awake_on =
            matches!(self.amphetamine.state(), State::On { .. } | State::Starting { .. });
        let change = flip(self.away.is_away(), keep_awake_on);
        self.away = Presence::of(change.away);
        self.tell_listener_away();
        if change.start_keep_awake {
            self.run_amphetamine(ctx);
        }
        self.away_watch.reset();
        self.tell_agents_away(ctx);
        let away = change.away;
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.keep_awake.away = away);
        }
        ctx.request_repaint();
    }
}
