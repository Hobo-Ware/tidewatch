//! The seams between `Flow` and the rest of the app: the facts a frame collects from the tabs and
//! views, pasting into and pressing Enter in a tab's terminal, the Diff pane's actions and the
//! flow's dialogs. They live here so `app.rs` only calls them.

use std::collections::HashMap;

use egui::Context;
use stdusk_core::terminal::PtyTerm;

use crate::delivery_card::Delivery;
use crate::delivery_view::{self, DialogAction};
use crate::dev_flow::{DevFlow, Fact, Ready, SendMode, Step};
use crate::diff_pane::PaneAction;
use crate::flow::{Flow, Out, TabFacts};
use crate::flow_jobs::Which;
use crate::notes_strip::StripAction;
use crate::pane::{Body, Pane};
use crate::post_confirm::{self, Answer, Plan};
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
            })
        })
        .collect()
}

pub(crate) fn paste(tabs: &mut Tabs<Pane<PtyTerm>>, tab: u64, text: &str) {
    if let Some(Body::Shell(term)) = tabs.get_mut(tab).map(|t| &mut t.pane.body) {
        term.paste(text);
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
) -> Vec<String> {
    let mut toasts = Vec::new();
    for out in outs {
        match out {
            Out::Paste { tab, text } => paste(tabs, tab, &text),
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
