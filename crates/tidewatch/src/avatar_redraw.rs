//! Redraw: the avatar ask goes to the tab's agent through the one send gate. An agent that is busy
//! is not refused: the ask is held and typed once the agent is idle, and only then does the owner
//! hear that it went.

use std::time::Duration;

use egui::Context;
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::avatar_ask::ASK;
use tidewatch_core::sent::{Reason, because};
use tidewatch_core::status::AgentStatus;

use crate::flow::Flow;
use crate::flow_glue::offer_to_agent;
use crate::note_send::{ReviewGate, review_gate};
use crate::outbox::{Offer, Send};
use crate::pane::Pane;
use crate::tab_title;
use crate::tabs::Tabs;

const RECHECK_S: f64 = 0.5;

#[derive(Debug, Default)]
pub(crate) struct Waiting {
    tabs: Vec<u64>,
}

fn status_of(tabs: &Tabs<Pane<PtyTerm>>, tab: u64) -> Option<AgentStatus> {
    let found = tabs.items().iter().find(|candidate| candidate.id == tab)?;
    found.pane.agent.as_ref().map(|agent| agent.run.status().clone())
}

fn name_of(tabs: &Tabs<Pane<PtyTerm>>, tab: u64) -> Option<String> {
    let found = tabs.items().iter().find(|candidate| candidate.id == tab)?;
    Some(
        found.pane.agent.as_ref().map_or_else(
            || found.title.clone(),
            |agent| tab_title::of_agent(agent, &found.title).1,
        ),
    )
}

impl Waiting {
    pub(crate) fn add(&mut self, tab: u64) {
        if !self.tabs.contains(&tab) {
            self.tabs.push(tab);
        }
    }

    pub(crate) fn holds(&self, tab: u64) -> bool {
        self.tabs.contains(&tab)
    }

    /// Offers each held ask to its agent that is idle. Returns the toasts to show: one per ask that went or
    /// that cannot go at all. An ask for a tab that closed is dropped without a word.
    pub(crate) fn drive(
        &mut self,
        tabs: &mut Tabs<Pane<PtyTerm>>,
        flow: &mut Flow,
        ctx: &Context,
        now: f64,
    ) -> Vec<String> {
        let mut toasts = Vec::new();
        let held = std::mem::take(&mut self.tabs);
        for tab in held {
            let Some(name) = name_of(tabs, tab) else { continue };
            match review_gate(status_of(tabs, tab).as_ref()) {
                ReviewGate::WhenIdle => {
                    self.tabs.push(tab);
                    continue;
                }
                ReviewGate::Refused(reason) => {
                    toasts.push(reason.to_owned());
                    continue;
                }
                ReviewGate::Now => {}
            }
            let send = Send::by_hand(ASK, Reason::Avatar, because::AVATAR_REDRAW);
            match offer_to_agent(tabs, flow, tab, &send, ctx, now) {
                Offer::Go => toasts.push(format!("Asked {name} to redraw its icon")),
                Offer::Wait => self.tabs.push(tab),
                Offer::Paused => {}
                Offer::Refuse(reason) => toasts.push(reason.to_owned()),
            }
        }
        if !self.tabs.is_empty() {
            ctx.request_repaint_after(Duration::from_secs_f64(RECHECK_S));
        }
        toasts
    }
}

#[cfg(test)]
mod tests;
