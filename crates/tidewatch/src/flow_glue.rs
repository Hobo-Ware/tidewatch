//! The seams between `Flow` and the rest of the app: the facts a frame collects from the tabs and
//! views, pasting into and pressing Enter in a tab's terminal, the Diff pane's actions and the
//! flow's dialogs. They live here so `app.rs` only calls them.

use std::collections::HashMap;
use std::time::Duration;

use egui::Context;
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::meeseeks::Meeseeks;
use tidewatch_core::meeseeks_target::Target;
use tidewatch_core::paths::sent_file_beside;
use tidewatch_core::sent::{self, Reason, Record, because};
use tidewatch_core::transcript::Source;

use crate::delivery_card::Delivery;
use crate::dev_flow::{DevFlow, Fact, Ready, SendMode, Step};
use crate::diff_pane::PaneAction;
use crate::done_check_glue::{self, TestsGate};
use crate::flow::{Asker, Flow, Gate, Out, TabFacts};
use crate::flow_jobs::Which;
use crate::note_send::ENTER_DELAY_SECS;
use crate::notes_strip::StripAction;
use crate::outbox::{NO_AGENT, Offer, Origin, Screen, Send};
use crate::pane::{Body, Pane};
use crate::post_confirm::{self, Answer, Plan};
use crate::pr_glue;
use crate::prompt_ghost;
use crate::tab_view::TabView;
use crate::tabs::Tabs;
use tidewatch_core::status::AgentStatus;

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A Meeseeks whose task names a pull request updates that one, so the card that asks to open a new
/// pull request stays away from it, merged or not.
fn delivery_gate(shut: bool, meeseeks: Option<&Meeseeks>) -> Gate {
    let updates = meeseeks.is_some_and(|found| found.target() != Target::New);
    if shut || updates { Gate::Shut } else { Gate::Open }
}

/// The agent worktree tabs whose shell runs, as the flow sees them.
pub(crate) fn facts<T>(
    tabs: &Tabs<Pane<T>>,
    views: &HashMap<u64, TabView>,
    pr: &pr_glue::Host,
    done: &done_check_glue::Host,
    active: u64,
    now: f64,
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
            let deliverable = *agent.run.status() == AgentStatus::Done;
            let pending = views.get(&tab.id).map_or(0, |view| view.notes.pending(&agent_id).len());
            let meeseeks = agent.meeseeks.is_some();
            let push = pr.push(tab.id, agent.autonomous(), meeseeks);
            let workflow = pr.workflow(tab.id);
            let tests =
                if meeseeks { TestsGate::Clear } else { done.tests_gate(tab.id, workflow.checks) };
            Some(TabFacts {
                id: tab.id,
                active: tab.id == active,
                status: agent.run.at_rest_status(now),
                deliverable,
                agent_id,
                dir: worktree.path.clone(),
                branch: worktree.branch.clone(),
                notes_file: agent.run.notes_file.clone(),
                pending,
                auto_push: push.auto,
                merged_head: push.merged_head,
                gate: delivery_gate(push.shut, agent.meeseeks.as_ref()),
                profile: agent.profile_name.clone(),
                workflow,
                tests,
            })
        })
        .collect()
}

const WAKE_SLACK_S: f64 = 0.02;

/// Whether the agent's input box is free, read off Claude Code's screen. Other CLIs, a tab that is
/// not at its prompt (a working one too, unless the send goes mid-turn) and a hand send are never
/// held by the screen.
fn screen_of(
    tab: &crate::tabs::Tab<Pane<PtyTerm>>,
    status: Option<&AgentStatus>,
    mid_turn: bool,
) -> Screen {
    let claude =
        tab.pane.agent.as_ref().and_then(|agent| agent.reader.source()) == Some(Source::Claude);
    let read = match status {
        Some(AgentStatus::Done) => true,
        Some(AgentStatus::Working) => mid_turn,
        _ => false,
    };
    match &tab.pane.body {
        Body::Shell(term) if claude && read => prompt_ghost::screen_of(&term.grid_snapshot()),
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
        let record = Record {
            typed: true,
            hand: send.origin == Origin::Hand,
            ..Record::new(tell.reason, &tell.because, &send.text, cap)
        };
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
        Some(found) if send.origin == Origin::Auto && !held => {
            screen_of(found, status.as_ref(), send.mid_turn)
        }
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

/// Each agent that is to open a pull request is asked through the one send gate. Only once the
/// message went does the card say the agent was asked. An agent that cannot take it leaves the
/// card's button for the owner. Returns the toasts for the owner's refused asks.
pub(crate) fn ask_agents(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    ctx: &Context,
    now: f64,
) -> Vec<String> {
    let mut toasts = Vec::new();
    for ask in flow.asking() {
        let send = match ask.by {
            Asker::Auto => Send::auto(&ask.text, Reason::AutoPush, because::AUTO_PUSH),
            Asker::Owner => Send::by_hand(&ask.text, Reason::PullRequest, because::ASK_PR_CLICK),
        };
        match offer_to_agent(tabs, flow, ask.tab, &send, ctx, now) {
            Offer::Go => flow.asked(ask.tab, now),
            Offer::Wait => ctx.request_repaint_after(Duration::from_secs_f64(0.5)),
            Offer::Paused => flow.not_asked(ask.tab),
            Offer::Refuse(reason) => {
                flow.not_asked(ask.tab);
                if ask.by == Asker::Owner {
                    toasts.push(reason.to_owned());
                }
            }
        }
    }
    toasts
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
            .map_or(Screen::Free, |found| screen_of(found, status.as_ref(), false));
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

/// The flow's dialog for the active tab: the post confirm.
pub(crate) fn dialogs(ctx: &Context, flow: &mut Flow) {
    let Some(confirm) = &flow.confirm else { return };
    match post_confirm::show(ctx, confirm) {
        Some(Answer::Post) => flow.approve_post(ctx),
        Some(Answer::Cancel) => flow.cancel_post(),
        None => {}
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
            Step::AskCard => flow.ask_to_open(tab),
            Step::ShowAsked => flow.show_asked(tab, ctx.input(|i| i.time)),
            Step::ShowMissing => flow.show_missing(tab),
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
    use tidewatch_core::workflow_profile::Workflow;

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
    fn the_delivery_stays_shut_for_a_meeseeks_whose_task_names_a_pull_request_and_open_otherwise() {
        use tidewatch_core::meeseeks::Criteria;
        let meeseeks = |task: &str| Meeseeks::summon(task, Criteria::default(), 0);
        let sensei = meeseeks("Sensei https://github.com/trakt/trakt-workers/pull/2437");
        let plain = meeseeks("Add a retry to the sync job");
        let cases = [
            (false, None, Gate::Open),
            (true, None, Gate::Shut),
            (false, Some(&plain), Gate::Open),
            (true, Some(&plain), Gate::Shut),
            (false, Some(&sensei), Gate::Shut),
            (true, Some(&sensei), Gate::Shut),
        ];
        for (shut, found, want) in cases {
            assert_eq!(delivery_gate(shut, found), want, "shut {shut}");
        }
    }

    fn done_facts(tab: u64) -> TabFacts {
        TabFacts {
            id: tab,
            active: true,
            status: AgentStatus::Done,
            deliverable: true,
            agent_id: "claude".into(),
            dir: std::path::PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            notes_file: None,
            pending: 0,
            auto_push: true,
            merged_head: None,
            gate: Gate::Open,
            profile: "Builder".into(),
            workflow: Workflow::default(),
            tests: crate::done_check_glue::TestsGate::Clear,
        }
    }

    #[test]
    fn an_agent_that_cannot_be_asked_is_not_asked_and_its_button_stays() {
        let (mut tabs, id) = lone_agent_tab();
        let mut flow = Flow::new(Tools::default());
        flow.ready_for_ask(&done_facts(id), 2);
        flow.ask_on_its_own(&done_facts(id));
        let toasts = ask_agents(&mut tabs, &mut flow, &Context::default(), 1.0);
        assert_eq!(toasts, Vec::<String>::new(), "Auto says nothing");
        let (push, delivery) = flow.push_state(id).unwrap();
        assert_eq!(push, crate::flow::AutoPush::Spent);
        assert_eq!(delivery, Delivery::Ready { commits: 2 });
        assert_eq!(flow.outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn a_refused_click_says_why_and_the_button_stays() {
        let (mut tabs, id) = lone_agent_tab();
        let mut flow = Flow::new(Tools::default());
        flow.ready_for_ask(&done_facts(id), 2);
        flow.ask_to_open(id);
        let toasts = ask_agents(&mut tabs, &mut flow, &Context::default(), 1.0);
        assert_eq!(toasts, [NO_AGENT.to_owned()]);
        assert_eq!(flow.push_state(id).unwrap().1, Delivery::Ready { commits: 2 });
        assert_eq!(flow.asking(), []);
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
