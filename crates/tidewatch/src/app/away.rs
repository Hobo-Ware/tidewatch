//! The window's side of Away: the flip, its saved choice, and what the hook listener is told.

use tidewatch_core::amphetamine::State;

use super::Tidewatch;
use crate::away_model::{Presence, flip};
use crate::prefs_save::update_in_background;

impl Tidewatch {
    pub(super) fn tell_listener_away(&self) {
        if let Some(listener) = self.hooks.listener() {
            listener.set_away(self.away.is_away());
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
        let away = change.away;
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.keep_awake.away = away);
        }
        ctx.request_repaint();
    }
}
