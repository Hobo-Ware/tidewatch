//! The window's side of Away: the flip, its saved choice, and what the hook listener is told.

use tidewatch_core::amphetamine::State;

use tidewatch_core::sent::Reason;

use super::Tidewatch;
use crate::away_model::{Presence, flip, hearers, told};
use crate::flow_glue;
use crate::outbox::Send;
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

    pub(super) fn flip_away(&mut self, ctx: &egui::Context) {
        let keep_awake_on =
            matches!(self.amphetamine.state(), State::On { .. } | State::Starting { .. });
        let change = flip(self.away.is_away(), keep_awake_on);
        self.away = Presence::of(change.away);
        self.tell_listener_away();
        if change.start_keep_awake {
            self.run_amphetamine(ctx);
        }
        self.tell_agents_away(ctx);
        let away = change.away;
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.keep_awake.away = away);
        }
        ctx.request_repaint();
    }
}
