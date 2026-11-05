//! The window's side of Away: the flip, its saved choice, and what the hook listener is told.

use tidewatch_core::amphetamine::State;

use tidewatch_core::progress_store;
use tidewatch_core::sent::{Reason, because};

use super::Tidewatch;
use crate::away_model::{KEEP_GOING, Open, Presence, flip, hearers, keep_going, told};
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
        let (text, because) = told(self.away.is_away());
        let hearing = hearers(self.tabs.items().iter().map(|tab| {
            let agent = tab.pane.agent.as_ref().filter(|_| matches!(tab.pane.body, Body::Shell(_)));
            (tab.id, agent.map(|agent| agent.run.status()))
        }));
        for tab in hearing {
            let send = Send::by_hand(text, Reason::Away, because);
            flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now);
        }
    }

    /// A turn that ends with open steps while the owner is away gets one keep-going message, three
    /// in a row at most for each agent.
    pub(super) fn drive_away(&mut self, ctx: &egui::Context, now: f64) {
        let Some(paths) = self.paths.clone() else { return };
        let live: Vec<u64> = self.tabs.items().iter().map(|tab| tab.id).collect();
        self.away_watch.keep_only(&live);
        let mut ended = Vec::new();
        for tab in self.tabs.items() {
            let Some(agent) =
                tab.pane.agent.as_ref().filter(|_| matches!(tab.pane.body, Body::Shell(_)))
            else {
                continue;
            };
            let Some(worktree) = tab.pane.worktree.as_ref() else { continue };
            if self.away_watch.ended(tab.id, agent.run.status()) {
                ended.push((tab.id, paths.progress_file(&worktree.repo, worktree.name())));
            }
        }
        for (tab, file) in ended {
            let report = progress_store::load(&file).as_ref().map(Open::of);
            let next = keep_going(self.away.is_away(), report, self.flow.outbox.going(tab));
            let mut in_a_row = next.in_a_row;
            if next.send {
                let send = Send::auto(KEEP_GOING, Reason::KeepGoing, because::KEEP_GOING);
                let offer =
                    flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now);
                if !matches!(offer, Offer::Go | Offer::Wait) {
                    in_a_row = next.in_a_row - 1;
                }
            }
            self.flow.outbox.set_going(tab, in_a_row);
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
        self.flow.outbox.reset_going();
        self.tell_agents_away(ctx);
        let away = change.away;
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.keep_awake.away = away);
        }
        ctx.request_repaint();
    }
}
