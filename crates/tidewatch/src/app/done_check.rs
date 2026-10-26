//! The window's side of the done check: the tabs' facts to the host each frame, a check started for
//! each turn that ended with a claim, and the card's two buttons (Ask agent to fix goes through the
//! same paste-and-Enter queue as every other message, only while the agent is idle).

use tidewatch_core::done_check::PrFact;
use tidewatch_core::progress::{State, real};
use tidewatch_core::sent::{Reason, because};

use super::Tidewatch;
use crate::done_check_glue::{Facts, Look, Request};
use crate::done_check_trigger::{Claim, claim};
use crate::done_check_view::{self, Action};
use crate::due::ask_blocked;
use crate::flow_glue;
use crate::outbox::{Offer, Origin, Send};
use crate::overview::Overview;
use crate::pane::{AgentTab, Body};
use crate::pr_state::Lookup;

impl Tidewatch {
    pub(super) fn drive_done_check(&mut self, ctx: &egui::Context, now: f64) {
        let seen: Vec<(u64, bool, Option<Claim>)> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let ordinary = agent.meeseeks.is_none()
                    && agent.workflow.is_none()
                    && tab.pane.worktree.is_some()
                    && matches!(tab.pane.body, Body::Shell(_));
                let ledger_done = self
                    .progress
                    .ledger(tab.id)
                    .is_some_and(|known| known.ledger.status.state == Some(State::Done));
                Some((tab.id, ordinary, claim(ledger_done, agent.run.end_block())))
            })
            .collect();
        let on_their_own: Vec<u64> = seen
            .iter()
            .filter(|(id, eligible, _)| *eligible && self.is_autonomous(*id))
            .map(|(id, ..)| *id)
            .collect();
        let facts: Vec<Facts<'_>> = seen
            .iter()
            .filter_map(|(id, eligible, claim)| {
                let agent = self.tabs.items().iter().find(|t| t.id == *id)?.pane.agent.as_ref()?;
                Some(Facts {
                    id: *id,
                    status: agent.run.status(),
                    eligible: *eligible,
                    claim: claim.as_ref(),
                })
            })
            .collect();
        for tab in self.done_check.tick(&facts) {
            self.start_done_check(tab, ctx);
        }
        for tab in on_their_own {
            let blocked = self.done_check_can_ask(tab).err();
            if !self.done_check.is_fake()
                && let Some(note) = self.done_check.auto_fix(tab, blocked)
            {
                self.send_fix(tab, &note, Origin::Auto, ctx, now);
            }
        }
    }

    fn done_check_request(&self, tab: u64) -> Option<Request> {
        let worktree = self.tabs.items().iter().find(|t| t.id == tab)?.pane.worktree.as_ref()?;
        let pr = match self.pr.watch(tab).and_then(|watch| watch.cards.number) {
            Some(number) => PrFact::Number(number),
            None => match self.flow.tab(tab).map(|state| &state.pr.lookup) {
                Some(Lookup::Open(pr)) => PrFact::Number(pr.number),
                Some(Lookup::NoPr) => PrFact::None,
                _ => PrFact::Unknown,
            },
        };
        let unverified = self.progress.ledger(tab).map(|known| {
            real(&known.ledger.not_verified).map(|entry| entry.text.clone()).collect()
        });
        Some(Request {
            dir: worktree.path.clone(),
            branch: worktree.branch.clone(),
            pr,
            unverified,
        })
    }

    fn start_done_check(&mut self, tab: u64, ctx: &egui::Context) {
        if let Some(request) = self.done_check_request(tab) {
            let tools = self.flow.tools();
            self.done_check.start(ctx, &tools, tab, request);
        }
    }

    pub(super) fn done_check_look(&self, tab: u64) -> Option<Look> {
        let agent = self.tabs.items().iter().find(|t| t.id == tab)?.pane.agent.as_ref()?;
        (agent.meeseeks.is_none() && agent.workflow.is_none())
            .then(|| self.done_check.look(tab))
            .flatten()
    }

    pub(super) fn done_check_can_ask(&self, tab: u64) -> Result<(), &'static str> {
        let status = self
            .tabs
            .items()
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pane.agent.as_ref())
            .map(|a| a.run.status());
        ask_blocked(status).map_or(Ok(()), Err)
    }

    pub(super) fn apply_done_check(
        &mut self,
        tab: u64,
        action: Action,
        ctx: &egui::Context,
        now: f64,
    ) {
        let Some(look) = self.done_check_look(tab) else { return };
        match action {
            Action::CheckAgain if !self.done_check.is_fake() => self.start_done_check(tab, ctx),
            Action::CheckAgain => {}
            Action::AskFix => self.ask_fix(tab, &look, ctx, now),
        }
    }

    fn ask_fix(&mut self, tab: u64, look: &Look, ctx: &egui::Context, now: f64) {
        let Look::Report(report) = look else { return };
        let Some(note) = report.fix_note() else { return };
        if self.done_check.is_fake() {
            self.show_toast("Dev run: would ask the agent to fix", ctx);
            return;
        }
        self.send_fix(tab, &note, Origin::Hand, ctx, now);
    }

    fn send_fix(&mut self, tab: u64, note: &str, origin: Origin, ctx: &egui::Context, now: f64) {
        let send = match origin {
            Origin::Auto => Send::auto(note, Reason::DoneCheck, because::DONE_CHECK_OWN),
            Origin::Hand => Send::by_hand(note, Reason::DoneCheck, because::DONE_CHECK_CLICK),
        };
        match flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now) {
            Offer::Refuse(reason) => self.show_toast(reason, ctx),
            Offer::Paused if origin == Origin::Auto => {}
            Offer::Paused | Offer::Go | Offer::Wait => self.done_check.asked(tab),
        }
    }

    pub(super) fn is_autonomous(&self, tab: u64) -> bool {
        self.tabs
            .items()
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pane.agent.as_ref())
            .is_some_and(AgentTab::autonomous)
    }

    pub(super) fn add_done_check_tags(&self, overview: &mut Overview) {
        let places = overview.places.iter_mut().flat_map(|place| place.rows.iter_mut());
        for row in overview.needs.iter_mut().chain(places) {
            row.done_check = match self.done_check_look(row.tab_id) {
                Some(Look::Report(report)) => {
                    let outcome = report.outcome();
                    Some((done_check_view::tone_of(outcome), done_check_view::tag_of(outcome)))
                }
                _ => None,
            };
        }
    }
}
