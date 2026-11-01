//! The app's side of the PR cards: polling GitHub for each agent tab, card actions, the PR strip and
//! the Overview and status bar pills.

use std::time::Duration;

use egui::{Rect, Ui};
use tidewatch_core::job::JobBoard;
use tidewatch_core::pr_ask;
use tidewatch_core::pr_prompt::Trigger;
use tidewatch_core::pr_status::{Body, Tone as PrTone};
use tidewatch_core::prefs::GithubPrefs;
use tidewatch_core::sent::{Cap, Reason, Switch, because};

use super::{Tidewatch, WAKE_SLACK_S, wall_ms};
use crate::merge_cleanup::Busy;
use crate::notify::{self, Notification};
use crate::outbox::{Offer, Origin, Send};
use crate::overview::Overview;
use crate::pane::Body as PaneBody;
use crate::pr_card_model::{CardAction, is_web_url};
use crate::pr_glue::Applied;
use crate::pr_watch::{Facts, Out, STAYS, Watch};
use crate::prefs_save::update_in_background;
use crate::tidewatch_view::Click;
use crate::worktree_jobs;
use crate::{flow_glue, layout, pr_strip};
use tidewatch_core::worktree::{BranchChoice, RemoveMode, Terms, Why};
use tidewatch_core::worktree_safety::Owner;

fn auto_because(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::FailedChecks => because::AUTO_CHECKS,
        Trigger::Reviews => because::AUTO_REVIEWS,
        Trigger::Conflicts => because::AUTO_CONFLICTS,
    }
}

fn now_ms() -> i64 {
    wall_ms() as i64
}

impl Tidewatch {
    pub(super) fn drive_pr(&mut self, ctx: &egui::Context, now: f64) {
        let Some(paths) = &self.paths else { return };
        let live = self.github_events.live_repos();
        let facts: Vec<Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let worktree = tab.pane.worktree.as_ref()?;
                if !matches!(tab.pane.body, PaneBody::Shell(_)) {
                    return None;
                }
                let pr = self
                    .flow
                    .tab(tab.id)
                    .and_then(|state| state.pr.pr())
                    .map(|pr| (pr.number, pr.head_sha.as_str()));
                Some(Facts {
                    id: tab.id,
                    status: agent.run.status(),
                    title: &tab.title,
                    dir: &worktree.path,
                    repo: &worktree.repo,
                    name: worktree.name(),
                    branch: &worktree.branch,
                    live: live.contains(&worktree.repo),
                    workflow: self.workflow.profile(&worktree.repo),
                    pr,
                    autonomous: agent.autonomous(),
                    busy: Busy::of(
                        &agent.run.at_rest_status(now),
                        self.progress.ledger(tab.id).map(|known| &known.ledger),
                        self.job_live.board(tab.id).is_some_and(JobBoard::has_running),
                    ),
                })
            })
            .collect();
        let on_their_own: Vec<u64> = facts.iter().filter(|f| f.autonomous).map(|f| f.id).collect();
        let ms = now_ms();
        let (mut applied, wake) = self.pr.tick(&facts, paths, ctx, now, ms);
        applied.extend(self.pr.drain(now, ms));
        self.apply_pr(applied, ctx, now);
        for tab in on_their_own {
            self.push_left_over_commits(tab, ctx, now);
        }
        if let Some(at) = wake {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
    }

    fn apply_pr(&mut self, applied: Vec<Applied>, ctx: &egui::Context, now: f64) {
        let mut told: Option<(Cap, Trigger)> = None;
        for Applied { tab, out } in applied {
            match out {
                Out::Told { n, max, trigger } => told = Some((Cap { n, max }, trigger)),
                Out::Paste(text) => {
                    let send = match told.take() {
                        Some((cap, trigger)) => {
                            Send::auto(&text, Reason::AutoFix, auto_because(trigger)).with_cap(cap)
                        }
                        None => Send::untold(&text, Origin::Hand),
                    };
                    if let Offer::Refuse(reason) = flow_glue::send_or_queue(
                        &mut self.tabs,
                        &mut self.flow,
                        tab,
                        send,
                        ctx,
                        now,
                    ) {
                        self.show_toast(reason, ctx);
                    }
                }
                Out::Toast(text) => self.show_toast(text, ctx),
                Out::Notify(notice) => {
                    let (focused, minimized) =
                        ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
                    let away = notify::window_is_away(focused, minimized);
                    let notification = Notification { title: notice.what, body: notice.detail };
                    self.notifier.offer_pr(tab, notice.tone, notification, away, now);
                }
                Out::Save => {}
                Out::Remove => self.remove_merged(tab, ctx),
            }
        }
    }

    pub(super) fn apply_pr_actions(
        &mut self,
        tab: u64,
        actions: Vec<CardAction>,
        ctx: &egui::Context,
        now: f64,
    ) {
        let ms = now_ms();
        let mut applied = Vec::new();
        for action in actions {
            match action {
                CardAction::Ask(card) => applied.extend(self.pr.ask(tab, card, now, ms)),
                CardAction::Open(url) => self.open_pr_page(&url, ctx),
                CardAction::Rerun(run) => applied.extend(self.pr.rerun(tab, run, ctx)),
                CardAction::OpenNotes => self.flow.pull_comments(tab, ctx),
                CardAction::CloseTab => {
                    if let Some(index) = self.tabs.index_of(tab) {
                        self.close_tab(index, ctx);
                    }
                }
                CardAction::Cleanup => self.begin_cleanup(tab, ctx, now),
                CardAction::KeepIt(card) => self.pr.dismiss(tab, card, false),
                CardAction::NotNow(card) => self.pr.dismiss(tab, card, true),
                CardAction::PushFix(card) => self.ask_to_push(tab, card, false, ctx, now),
                CardAction::SendNow(id) => applied.extend(self.pr.send_now(tab, id, now, ms)),
                CardAction::Cancel(id) => self.pr.cancel(tab, id),
                CardAction::ChangeWording => self.open_updates_settings(),
            }
        }
        self.apply_pr(applied, ctx, now);
    }

    /// A button under one of Tidewatch's rows: the workflow its words came from, or the setting that
    /// stops its kind. A pull request kind is turned off for this tab, as the strip's popover does.
    pub(super) fn apply_tidewatch_clicks(
        &mut self,
        tab: u64,
        clicks: Vec<Click>,
        ctx: &egui::Context,
        now: f64,
    ) {
        for click in clicks {
            match click {
                Click::ChangeWorkflow => self.open_updates_settings(),
                Click::TurnOff(Switch::Auto(job)) => {
                    let policy = self.pr.policy(tab).with(job, false);
                    self.pr.set_policy(tab, policy, now);
                    self.show_toast("Turned off for this tab. The PR strip turns it back on", ctx);
                }
                Click::TurnOff(Switch::DoneCheck) => {
                    self.set_done_check(false);
                    self.show_toast("Done check is off. Settings, General turns it back on", ctx);
                }
                Click::TurnOff(Switch::Comments) => {
                    self.comments.set_off(tab, true);
                    self.show_toast("Comment guard is off for this tab", ctx);
                }
            }
        }
    }

    fn busy_now(&self, tab: u64, now: f64) -> Busy {
        let Some(agent) = self.tabs.agent(tab) else {
            return Busy::default();
        };
        Busy::of(
            &agent.run.at_rest_status(now),
            self.progress.ledger(tab).map(|known| &known.ledger),
            self.job_live.board(tab).is_some_and(JobBoard::has_running),
        )
    }

    fn begin_cleanup(&mut self, tab: u64, ctx: &egui::Context, now: f64) {
        let Some(paths) = self.paths.clone() else { return };
        let Some(worktree) = self.tabs.get_mut(tab).and_then(|t| t.pane.worktree.clone()) else {
            return;
        };
        let busy = self.busy_now(tab, now);
        let refused = self.pr.check_before_removal(tab, worktree, busy, &paths, ctx);
        self.apply_pr(refused, ctx, now);
    }

    fn remove_merged(&mut self, tab: u64, ctx: &egui::Context) {
        if self.busy_now(tab, ctx.input(|i| i.time)).any() {
            self.show_toast(STAYS, ctx);
            return;
        }
        let Some(paths) = self.paths.clone() else { return };
        let Some((shell, worktree)) = self.tabs.get_mut(tab).and_then(|t| t.pane.begin_remove())
        else {
            return;
        };
        let trust = self.trust.grant();
        let terms = Terms::new(BranchChoice::Delete, RemoveMode::Safe)
            .why(Why::MergedTab)
            .owner(Owner::ThisWindow)
            .delivered(self.pr.watch(tab).and_then(crate::pr_watch::Watch::pr_head));
        worktree_jobs::remove(&self.jobs, ctx, paths, tab, shell, worktree, terms, trust);
    }

    fn open_pr_page(&mut self, url: &str, ctx: &egui::Context) {
        if !self.pr.outside_reachable() {
            self.show_toast(format!("Dev run: would open {url}"), ctx);
        } else if is_web_url(url) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// The commits the agent made after a message and left unpushed, for a tab that runs on its own:
    /// the agent is asked to push them, once.
    fn push_left_over_commits(&mut self, tab: u64, ctx: &egui::Context, now: f64) {
        let Some(card) = self.pr.push_fix_due(tab) else { return };
        self.ask_to_push(tab, card, true, ctx, now);
        self.pr.dismiss(tab, card, false);
    }

    fn ask_to_push(
        &mut self,
        tab: u64,
        card: u64,
        on_its_own: bool,
        ctx: &egui::Context,
        now: f64,
    ) {
        let Some(Body::PushFix { commits, .. }) = self.pr.body(tab, card) else { return };
        let Some(repo) = self
            .tabs
            .get(tab)
            .and_then(|candidate| candidate.pane.worktree.as_ref())
            .map(|worktree| worktree.repo.clone())
        else {
            return;
        };
        let number = self.pr.watch(tab).and_then(|watch| watch.cards.number);
        let text = pr_ask::push_commits(number, commits, self.workflow.profile(&repo));
        let send = if on_its_own {
            Send::auto(&text, Reason::AutoPush, because::ASK_PUSH_OWN)
        } else {
            Send::by_hand(&text, Reason::PullRequest, because::ASK_PUSH_CLICK)
        };
        if let Offer::Refuse(reason) =
            flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now)
        {
            self.show_toast(reason, ctx);
        }
    }

    pub(super) fn show_pr_strip(
        &mut self,
        ui: &mut Ui,
        terminal: &mut Rect,
        ctx: &egui::Context,
        active_id: u64,
    ) {
        let Some(summary) = self.pr.watch(active_id).and_then(Watch::summary) else {
            return;
        };
        let strip = pr_strip::Strip {
            summary,
            policy: self.pr.policy(active_id),
            dev_open_popover: self.pr.open_popover(),
            updates: self.updates_line_for(active_id),
        };
        let band = layout::band_above(terminal, pr_strip::HEIGHT);
        let now = ctx.input(|i| i.time);
        match pr_strip::show(ui, band, &strip) {
            Some(pr_strip::Action::Open(url)) => self.open_pr_page(&url, ctx),
            Some(pr_strip::Action::Policy(policy)) => self.pr.set_policy(active_id, policy, now),
            None => {}
        }
    }

    fn updates_line_for(&self, tab: u64) -> String {
        let live = self.pr.watch(tab).is_some_and(Watch::live);
        let why = self.pr.watch(tab).and_then(|watch| self.github_events.why_polled(&watch.repo));
        pr_strip::updates_line(live, why)
    }

    pub(super) fn set_github_default(&mut self, prefs: GithubPrefs) {
        self.pr.set_default(prefs);
        if let Some(paths) = self.paths.clone() {
            update_in_background(paths, move |p| p.github = prefs);
        }
    }

    pub(super) fn add_pr_pills(&self, overview: &mut Overview) {
        let places = overview.places.iter_mut().flat_map(|place| place.rows.iter_mut());
        for row in overview.needs.iter_mut().chain(places) {
            row.pr = self.pr.watch(row.tab_id).and_then(Watch::pill);
        }
    }

    pub(super) fn pr_bar_item(&self, active_id: u64) -> Option<(PrTone, String)> {
        let summary = self.pr.watch(active_id)?.summary()?;
        let (tone, word) = summary.headline()?;
        Some((tone, format!("PR #{} {word}", summary.number)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::sent::Record;

    #[test]
    fn every_kind_of_auto_fix_is_told_with_the_switch_of_its_own_job() {
        for trigger in [Trigger::FailedChecks, Trigger::Reviews, Trigger::Conflicts] {
            let record = Record::new(Reason::AutoFix, auto_because(trigger), "x", None);
            assert_eq!(record.switch(), Some(Switch::Auto(trigger.job())), "{trigger:?}");
        }
    }
}
