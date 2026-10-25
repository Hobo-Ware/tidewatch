//! The app's side of the PR cards: polling GitHub for each agent tab, card actions, the PR strip and
//! the Overview and status bar pills.

use std::time::Duration;

use egui::{Rect, Ui};
use tidewatch_core::pr_status::{Body, Tone as PrTone};
use tidewatch_core::prefs::{GithubPrefs, Prefs};

use super::{Tidewatch, WAKE_SLACK_S, wall_ms};
use crate::delivery_card::Delivery;
use crate::note_send::ENTER_DELAY_SECS;
use crate::notify::{self, Notification};
use crate::overview::Overview;
use crate::pane::Body as PaneBody;
use crate::pr_card_model::{CardAction, is_web_url};
use crate::pr_glue::Applied;
use crate::pr_watch::{Facts, Out, Watch};
use crate::worktree_jobs;
use crate::{flow_glue, layout, pr_strip};
use tidewatch_core::worktree::{BranchChoice, RemoveMode};

fn now_ms() -> i64 {
    wall_ms() as i64
}

impl Tidewatch {
    pub(super) fn drive_pr(&mut self, ctx: &egui::Context, now: f64) {
        let Some(paths) = &self.paths else { return };
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
                    pr,
                    autonomous: agent.autonomous(),
                })
            })
            .collect();
        let on_their_own: Vec<u64> = facts.iter().filter(|f| f.autonomous).map(|f| f.id).collect();
        let ms = now_ms();
        let (mut applied, wake) = self.pr.tick(&facts, paths, ctx, now, ms);
        applied.extend(self.pr.drain(now, ms));
        self.apply_pr(applied, ctx, now);
        for tab in on_their_own {
            self.push_left_over_commits(tab, ctx);
        }
        if let Some(at) = wake {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
    }

    fn apply_pr(&mut self, applied: Vec<Applied>, ctx: &egui::Context, now: f64) {
        for Applied { tab, out } in applied {
            match out {
                Out::Paste(text) => {
                    flow_glue::paste(&mut self.tabs, tab, &text);
                    self.flow.queue_enter(tab, now);
                    ctx.request_repaint_after(Duration::from_secs_f64(
                        ENTER_DELAY_SECS + WAKE_SLACK_S,
                    ));
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
                CardAction::Cleanup => self.begin_cleanup(tab, ctx),
                CardAction::KeepIt(card) => self.pr.dismiss(tab, card, false),
                CardAction::NotNow(card) => self.pr.dismiss(tab, card, true),
                CardAction::PushFix(card) => self.push_fix(tab, card, ctx),
                CardAction::SendNow(id) => applied.extend(self.pr.send_now(tab, id, now, ms)),
                CardAction::Cancel(id) => self.pr.cancel(tab, id),
            }
        }
        self.apply_pr(applied, ctx, now);
    }

    fn begin_cleanup(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone() else { return };
        let Some(worktree) = self.tabs.get_mut(tab).and_then(|t| t.pane.worktree.clone()) else {
            return;
        };
        self.pr.check_before_removal(tab, worktree, &paths, ctx);
    }

    fn remove_merged(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone() else { return };
        let Some((shell, worktree)) = self.tabs.get_mut(tab).and_then(|t| t.pane.begin_remove())
        else {
            return;
        };
        let trust = self.trust.grant();
        worktree_jobs::remove(
            &self.jobs,
            ctx,
            paths,
            tab,
            shell,
            worktree,
            BranchChoice::Delete,
            RemoveMode::Safe,
            trust,
        );
    }

    fn open_pr_page(&mut self, url: &str, ctx: &egui::Context) {
        if !self.pr.outside_reachable() {
            self.show_toast(format!("Dev run: would open {url}"), ctx);
        } else if is_web_url(url) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// The commits the agent made after a message and left unpushed, for a tab that runs on its own:
    /// the delivery card takes them from here without a click.
    fn push_left_over_commits(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(card) = self.pr.push_fix_due(tab) else { return };
        let free = self.flow.tab(tab).is_some_and(|state| {
            matches!(state.delivery, Delivery::Hidden | Delivery::Ready { .. })
        });
        if free {
            self.push_fix(tab, card, ctx);
            self.pr.dismiss(tab, card, false);
        }
    }

    fn push_fix(&mut self, tab: u64, card: u64, ctx: &egui::Context) {
        let Some(Body::PushFix { commits, .. }) = self.pr.body(tab, card) else { return };
        if let Some(state) = self.flow.tab_mut(tab) {
            state.delivery.probe_started();
            state.delivery.probe_done(Some(commits));
        }
        self.flow.open_card(tab, ctx);
    }

    pub(super) fn show_pr_strip(
        &mut self,
        ui: &mut Ui,
        terminal: &mut Rect,
        ctx: &egui::Context,
        active_id: u64,
    ) {
        let Some(summary) = self.pr.watch(active_id).and_then(Watch::strip_summary) else {
            return;
        };
        let strip = pr_strip::Strip {
            summary,
            policy: self.pr.policy(active_id),
            dev_open_popover: self.pr.open_popover(),
        };
        let band = layout::band_above(terminal, pr_strip::HEIGHT);
        let now = ctx.input(|i| i.time);
        match pr_strip::show(ui, band, &strip) {
            Some(pr_strip::Action::Open(url)) => self.open_pr_page(&url, ctx),
            Some(pr_strip::Action::Policy(policy)) => self.pr.set_policy(active_id, policy, now),
            None => {}
        }
    }

    pub(super) fn set_github_default(&mut self, prefs: GithubPrefs) {
        self.pr.set_default(prefs);
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.github = prefs) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
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
