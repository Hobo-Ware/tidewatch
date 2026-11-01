//! The window's side of the token budget: the plan limits the agent tabs saved, save mode once per
//! frame, its one notice, the hold on the outbox, and the token and cost facts of each agent tab for
//! the tooltip, the Overview and the status bar.

use std::time::Duration;

use super::{Tidewatch, wall_ms};
use crate::budget_glue;
use crate::token_budget::{self, Mark, Meter, TabTokens};

impl Tidewatch {
    pub(super) fn drive_budget(&mut self, ctx: &egui::Context) {
        let now_ms = wall_ms() as i64;
        let tabs: Vec<_> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| Some((tab.id, tab.pane.agent.as_ref()?.run.notes_file.clone()?)))
            .collect();
        self.budget.look(&tabs, now_ms, ctx);
        if let Some(notice) = self.budget.tick(now_ms) {
            self.show_toast(notice.text(), ctx);
        }
        self.flow.outbox.set_saving(self.budget.saving());
        if let Some(reset_ms) = self.budget.next_reset_ms(now_ms) {
            ctx.request_repaint_after(Duration::from_millis((reset_ms - now_ms).max(1) as u64));
        }
    }

    pub(super) fn tokens_of(&self, tab: u64) -> Option<TabTokens> {
        let agent = self.tabs.agent(tab)?;
        token_budget::tab_tokens(agent.reader.tokens(), agent.reader.token_share())
            .or_else(budget_glue::fake_tokens)
    }

    pub(super) fn tab_spend_tip(&self, tab: u64) -> Option<String> {
        token_budget::tab_tip(self.tokens_of(tab).as_ref(), self.budget.cost_of(tab))
    }

    pub(super) fn budget_mark(&self, active_id: u64) -> Option<Mark> {
        token_budget::bar_mark(self.tokens_of(active_id).as_ref(), self.budget.cost_of(active_id))
    }

    pub(super) fn budget_meters(&self) -> Vec<Meter> {
        token_budget::meters(
            self.budget.limits(),
            self.budget.enabled(),
            wall_ms() as i64,
            &jiff::tz::TimeZone::system(),
        )
    }
}
