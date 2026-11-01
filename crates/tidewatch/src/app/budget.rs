//! The window's side of the token budget: save mode once per frame, its one notice, the hold on the
//! outbox, and the token facts of each agent tab for the tooltip, the Overview and the status bar.

use std::time::Duration;

use super::{Tidewatch, wall_ms};
use crate::budget_glue;
use crate::token_budget::{self, BarInput, Mark, TabTokens};

const LIMITS_LOOK: Duration = Duration::from_secs(30);

impl Tidewatch {
    pub(super) fn drive_budget(&mut self, ctx: &egui::Context) {
        if let Some(notice) = self.budget.tick(wall_ms() as i64) {
            self.show_toast(notice.text(), ctx);
        }
        self.flow.outbox.set_saving(self.budget.saving());
        if self.budget.limits().is_some() {
            ctx.request_repaint_after(LIMITS_LOOK);
        }
    }

    pub(super) fn tokens_of(&self, tab: u64) -> Option<TabTokens> {
        let agent = self.tabs.agent(tab)?;
        token_budget::tab_tokens(agent.reader.tokens(), agent.reader.token_share())
            .or_else(budget_glue::fake_tokens)
    }

    pub(super) fn budget_mark(&self, active_id: u64) -> Option<Mark> {
        let tab = self.tokens_of(active_id);
        let input = BarInput {
            limits: self.budget.limits(),
            save_mode_on: self.budget.enabled(),
            tab: tab.as_ref(),
        };
        token_budget::bar_mark(&input, wall_ms() as i64, &jiff::tz::TimeZone::system())
    }
}
