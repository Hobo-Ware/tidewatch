//! The seams between `Flow` and the rest of the app: the facts a frame collects from the tabs and
//! views, pasting into and pressing Enter in a tab's terminal, the Diff pane's actions and the
//! flow's dialogs. They live here so `app.rs` only calls them.

use std::collections::HashMap;
use std::time::Duration;

use egui::Context;
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::paths::sent_file_beside;
use tidewatch_core::sent::{self, Record};
use tidewatch_core::transcript::Source;

use crate::delivery_card::Delivery;
use crate::delivery_view::{self, DialogAction};
use crate::dev_flow::{DevFlow, Fact, Ready, SendMode, Step};
use crate::diff_pane::PaneAction;
use crate::flow::{Flow, Gate, Out, TabFacts};
use crate::flow_jobs::Which;
use crate::note_send::ENTER_DELAY_SECS;
use crate::notes_strip::StripAction;
use crate::outbox::{NO_AGENT, Offer, Origin, Screen, Send};
use crate::pane::{Body, Pane};
use crate::post_confirm::{self, Answer, Plan};
use crate::prompt_ghost;
use crate::tab_view::TabView;
use crate::tabs::Tabs;
use tidewatch_core::status::AgentStatus;

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The agent worktree tabs whose shell runs, as the flow sees them.
pub(crate) fn facts<T>(
    tabs: &Tabs<Pane<T>>,
    views: &HashMap<u64, TabView>,
    active: u64,
) -> Vec<TabFacts> {
    tabs.items()
        .iter()
        .filter_map(|tab| {
            let agent = tab.pane.agent.as_ref()?;
            let worktree = tab.pane.worktree.as_ref()?;
            if !matches!(tab.pane.body, Body::Shell(_)) {
                return None;
            }
            let agent_id = agent.run.agent_id.clone();
            let (deliverable, stage_summary) = match &agent.workflow {
                Some(run) => (run.finished(), run.last_summary().map(str::to_owned)),
                None => {
                    (*agent.run.status() == AgentStatus::Done, agent.run.report.stage_done.clone())
                }
            };
            let pending = views.get(&tab.id).map_or(0, |view| view.notes.pending(&agent_id).len());
            Some(TabFacts {
                id: tab.id,
                active: tab.id == active,
                status: agent.run.status().clone(),
                deliverable,
                agent_id,
                dir: worktree.path.clone(),
                branch: worktree.branch.clone(),
                notes_file: agent.run.notes_file.clone(),
                pending,
                stage_summary,
                auto_deliver: agent.autonomous() && agent.meeseeks.is_none(),
                gate: Gate::Open,
            })
        })
        .collect()
}

const WAKE_SLACK_S: f64 = 0.02;

/// Whether the agent's input box is free, read off Claude Code's screen. Other CLIs, a tab that is
/// not at its prompt and a hand send are never held by the screen.
fn screen_of(tab: &crate::tabs::Tab<Pane<PtyTerm>>, status: Option<&AgentStatus>) -> Screen {
    let claude =
        tab.pane.agent.as_ref().and_then(|agent| agent.reader.source()) == Some(Source::Claude);
    match (&tab.pane.body, status) {
        (Body::Shell(term), Some(AgentStatus::Done)) if claude => {
            prompt_ghost::screen_of(&term.grid_snapshot())
        }
        _ => Screen::Free,
    }
}

/// The one place a message is typed into an agent: the record first, then the paste and its Enter.
pub(crate) fn deliver(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    send: &Send,
    ctx: &Context,
    now: f64,
) -> bool {
    let cap = flow.outbox.cap_for(tab, send);
    let Some(pane) = tabs.get_mut(tab).map(|found| &mut found.pane) else { return false };
    let Body::Shell(term) = &mut pane.body else { return false };
    term.paste(&send.text);
    if let (Some(tell), Some(agent)) = (&send.tell, pane.agent.as_mut()) {
        let record = Record::new(tell.reason, &tell.because, &send.text, cap);
        if let Some(notes) = &agent.run.notes_file {
            let _ = sent::append(&sent_file_beside(notes), &record);
        }
        agent.reader.tidewatch_said(record);
    }
    flow.outbox.sent(tab, send, now);
    flow.queue_enter(tab, now);
    ctx.request_repaint_after(Duration::from_secs_f64(ENTER_DELAY_SECS + WAKE_SLACK_S));
    true
}

fn status_of(tabs: &Tabs<Pane<PtyTerm>>, tab: u64) -> Option<AgentStatus> {
    let found = tabs.items().iter().find(|candidate| candidate.id == tab)?;
    found.pane.agent.as_ref().map(|agent| agent.run.status().clone())
}

/// Types `send` into the tab's agent if the one gate says it may go now. Anything else is the
/// caller's to keep: `Wait` means ask again later.
pub(crate) fn offer_to_agent(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    send: &Send,
    ctx: &Context,
    now: f64,
) -> Offer {
    let status = status_of(tabs, tab);
    let held = flow.outbox.held(tab, now) && status == Some(AgentStatus::Done);
    let screen = match tabs.items().iter().find(|candidate| candidate.id == tab) {
        Some(found) if send.origin == Origin::Auto && !held => screen_of(found, status.as_ref()),
        _ => Screen::Free,
    };
    let offer = flow.outbox.offer(tab, send, status.as_ref(), screen, now);
    if offer == Offer::Go && !deliver(tabs, flow, tab, send, ctx, now) {
        return Offer::Refuse(NO_AGENT);
    }
    offer
}

/// `offer_to_agent` for a message nobody can ask again for: one that has to wait goes in the tab's
/// line and is typed when the agent is next free.
pub(crate) fn send_or_queue(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    send: Send,
    ctx: &Context,
    now: f64,
) -> Offer {
    let offer = offer_to_agent(tabs, flow, tab, &send, ctx, now);
    if offer == Offer::Wait {
        flow.outbox.enqueue(tab, send);
        ctx.request_repaint_after(Duration::from_secs_f64(0.5));
    }
    offer
}

pub(crate) fn drain_outbox(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    ctx: &Context,
    now: f64,
) {
    for tab in flow.outbox.waiting_tabs() {
        let status = status_of(tabs, tab);
        let held = flow.outbox.held(tab, now) && status == Some(AgentStatus::Done);
        let screen = tabs
            .items()
            .iter()
            .find(|candidate| candidate.id == tab)
            .filter(|_| !held)
            .map_or(Screen::Free, |found| screen_of(found, status.as_ref()));
        if let Some(send) = flow.outbox.next_ready(tab, status.as_ref(), screen, now)
            && !deliver(tabs, flow, tab, &send, ctx, now)
        {
            flow.outbox.forget(tab);
        }
    }
    for tab in tabs.items_mut() {
        if let Some(agent) = tab.pane.agent.as_mut() {
            agent.reader.rows.set_auto_paused(flow.outbox.paused(tab.id));
        }
    }
    if !flow.outbox.waiting_tabs().is_empty() {
        ctx.request_repaint_after(Duration::from_secs_f64(0.5));
    }
}

pub(crate) fn output_of(tabs: &Tabs<Pane<PtyTerm>>, tab: u64) -> Option<u64> {
    match &tabs.items().iter().find(|candidate| candidate.id == tab)?.pane.body {
        Body::Shell(term) => Some(term.output_gen()),
        _ => None,
    }
}

pub(crate) fn press_enter(tabs: &mut Tabs<Pane<PtyTerm>>, tab: u64) {
    if let Some(Body::Shell(term)) = tabs.get_mut(tab).map(|t| &mut t.pane.body) {
        term.send(b"\r");
    }
}

/// Applies what finished jobs produced. Returns the toasts to show.
pub(crate) fn apply_outs(
    outs: Vec<Out>,
    tabs: &mut Tabs<Pane<PtyTerm>>,
    views: &mut HashMap<u64, TabView>,
    flow: &mut Flow,
    ctx: &Context,
    now: f64,
) -> Vec<String> {
    let mut toasts = Vec::new();
    for out in outs {
        match out {
            Out::Paste { tab, text, ids, auto } => {
                let origin = if auto { Origin::Auto } else { Origin::Hand };
                let send = Send::untold(&text, origin);
                if let Offer::Refuse(reason) = send_or_queue(tabs, flow, tab, send, ctx, now) {
                    flow.unsend(tab, ids, ctx);
                    toasts.push(format!(
                        "Could not send the review: {reason}. Its notes are back in your draft"
                    ));
                }
            }
            Out::Notes { tab, notes } => {
                if let Some(view) = views.get_mut(&tab) {
                    view.set_notes(*notes);
                }
            }
            Out::Toast(text) => toasts.push(text),
        }
    }
    toasts
}

/// Does what the Diff pane asked. Returns a toast when something cannot go.
pub(crate) fn pane_actions(
    flow: &mut Flow,
    view: &mut TabView,
    tab: u64,
    actions: Vec<PaneAction>,
    ctx: &Context,
) -> Vec<String> {
    let unix = unix_now();
    let mut toasts = Vec::new();
    for action in actions {
        match action {
            PaneAction::SendReview => {
                toasts.extend(flow.send(tab, Which::Review(String::new()), ctx, unix));
            }
            PaneAction::OpenReview => view.back_to_terminal(),
            PaneAction::PullPr => flow.pull_comments(tab, ctx),
            PaneAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            PaneAction::Strip(StripAction::Send(id)) => {
                toasts.extend(flow.send(tab, Which::One(id), ctx, unix));
            }
            PaneAction::Strip(StripAction::PostToPr(id)) => {
                let plan = view
                    .notes
                    .threads
                    .iter()
                    .find(|thread| thread.id == id)
                    .and_then(|thread| Plan::of(thread, &view.notes));
                flow.ask_post(tab, plan);
            }
            PaneAction::Strip(StripAction::Jump(path)) => view.select_file(&path),
        }
    }
    toasts
}

/// The flow's dialogs for the active tab: the delivery dialog and the post confirm.
pub(crate) fn dialogs(ctx: &Context, flow: &mut Flow, active: u64) {
    if let Some(confirm) = &flow.confirm {
        match post_confirm::show(ctx, confirm) {
            Some(Answer::Post) => flow.approve_post(ctx),
            Some(Answer::Cancel) => flow.cancel_post(),
            None => {}
        }
        return;
    }
    let Some(answer) = flow.tab_mut(active).and_then(|state| {
        state.delivery.is_modal().then(|| delivery_view::show_dialog(ctx, &mut state.delivery))
    }) else {
        return;
    };
    let Some(answer) = answer else { return };
    match answer {
        DialogAction::Approve => flow.approve_card(active, ctx),
        DialogAction::Cancel => {
            if let Some(state) = flow.tab_mut(active) {
                state.delivery.cancel();
            }
        }
        DialogAction::Back => {
            if let Some(state) = flow.tab_mut(active) {
                state.delivery.back();
            }
        }
        DialogAction::Close => {
            if let Some(state) = flow.tab_mut(active) {
                state.delivery.close_success();
            }
        }
        DialogAction::OpenUrl => {
            if let Some(state) = flow.tab_mut(active)
                && let Delivery::Delivered(done) = &state.delivery
            {
                ctx.open_url(egui::OpenUrl::new_tab(done.url.clone()));
            }
        }
    }
}

/// Does what the `TIDEWATCH_DEV_*` flow switches ask, as soon as the active tab is ready for it.
pub(crate) fn dev_drive(
    dev: &mut DevFlow,
    flow: &mut Flow,
    view: Option<&TabView>,
    tab: u64,
    ctx: &Context,
) {
    let Some(state) = flow.tab(tab) else { return };
    let checks = [
        (Fact::Notes, view.is_some_and(|v| v.notes_ready && !v.notes.threads.is_empty())),
        (Fact::Card, matches!(state.delivery, Delivery::Ready { .. })),
        (Fact::Form, matches!(state.delivery, Delivery::Editing(_))),
        (Fact::Pr, state.pr.pr().is_some()),
        (Fact::Lines, state.pr.lines_known()),
        (Fact::Confirm, flow.confirm.is_some()),
    ];
    let ready = Ready(checks.into_iter().filter(|(_, on)| *on).map(|(fact, _)| fact).collect());
    let unix = unix_now();
    for step in dev.steps(&ready) {
        match step {
            Step::EnableAuto => {
                flow.hold_review(tab, String::new());
            }
            Step::Send(mode) => {
                let which = match (mode, view.and_then(|v| v.notes.threads.first())) {
                    (SendMode::One, Some(thread)) => Which::One(thread.id),
                    _ => Which::Review(dev.message.clone()),
                };
                if let Some(reason) = flow.send(tab, which, ctx, unix) {
                    eprintln!("tidewatch: dev send refused: {reason}");
                }
            }
            Step::OpenCard => flow.open_card(tab, ctx),
            Step::ApproveCard => flow.approve_card(tab, ctx),
            Step::Pull => flow.pull_comments(tab, ctx),
            Step::AskPost => {
                let plan = view.and_then(|v| {
                    v.notes.threads.first().and_then(|thread| Plan::of(thread, &v.notes))
                });
                flow.ask_post(tab, plan);
            }
            Step::ApprovePost => flow.approve_post(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use tidewatch_core::tools::Tools;

    use super::*;

    fn lone_agent_tab() -> (Tabs<Pane<PtyTerm>>, u64) {
        let mut tabs = Tabs::new(Pane::creating());
        let id =
            tabs.push("agent".into(), Pane::creating_agent("Builder".into(), "claude".into(), 0.0));
        (tabs, id)
    }

    #[test]
    fn a_review_for_an_agent_that_can_no_longer_take_it_says_so_and_is_not_typed_or_queued() {
        let (mut tabs, id) = lone_agent_tab();
        let mut flow = Flow::new(Tools::default());
        let outs = vec![Out::Paste { tab: id, text: "review".into(), ids: vec![1, 2], auto: true }];
        let toasts =
            apply_outs(outs, &mut tabs, &mut HashMap::new(), &mut flow, &Context::default(), 1.0);
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].starts_with("Could not send the review: There is no running agent"));
        assert!(toasts[0].ends_with("Its notes are back in your draft"));
        assert_eq!(flow.outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn a_message_for_a_tab_with_no_agent_is_refused_and_a_closed_tab_is_too() {
        let (mut tabs, id) = lone_agent_tab();
        let mut flow = Flow::new(Tools::default());
        let ctx = Context::default();
        let send = Send::auto("x", tidewatch_core::sent::Reason::DueToday, "an item is due");
        assert_eq!(
            offer_to_agent(&mut tabs, &mut flow, id, &send, &ctx, 1.0),
            Offer::Refuse(NO_AGENT)
        );
        assert_eq!(
            offer_to_agent(&mut tabs, &mut flow, 999, &send, &ctx, 1.0),
            Offer::Refuse(NO_AGENT)
        );
    }
}
