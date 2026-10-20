//! The window's side of Meeseeks: what `meeseeks_done` starts, each check's answer, the pastes that
//! wait for an idle agent, the poof and its fade, and Take over and Give up.

use std::time::Duration;

use tidewatch_core::meeseeks::{Command, Event};
use tidewatch_core::meeseeks_log::Poofed;

use super::WAKE_SLACK_S;
use super::{Tidewatch, wall_ms};
use crate::due::ask_blocked;
use crate::flow_glue;
use crate::meeseeks_glue::{CheckSpec, Done, Leaving};
use crate::note_send::ENTER_DELAY_SECS;
use crate::pane::Body;
use crate::progress_glue::today;

const CLOCK_TICK: Duration = Duration::from_secs(30);
const FADE_FRAME: Duration = Duration::from_millis(16);

impl Tidewatch {
    pub(super) fn drive_meeseeks(&mut self, ctx: &egui::Context, now: f64) {
        let now_ms = wall_ms() as i64;
        self.inject_fake_meeseeks(ctx, now_ms);
        for done in self.meeseeks.drain(ctx) {
            self.meeseeks_done_with(done, ctx, now);
        }
        self.paste_when_idle(ctx, now);
        self.watch_struggles(ctx, now_ms);
        for tab in self.meeseeks.faded(now) {
            self.tabs.remove_id(tab);
            self.forget_tab(tab);
        }
        if self.meeseeks.fading() {
            ctx.request_repaint_after(FADE_FRAME);
        }
    }

    fn inject_fake_meeseeks(&mut self, ctx: &egui::Context, now_ms: i64) {
        let fake = self.meeseeks.fake();
        if let Some(text) = self.meeseeks.fake_toast() {
            self.show_toast(text, ctx);
        }
        if fake.state(now_ms).is_none() {
            return;
        }
        for tab in self.tabs.items_mut() {
            if let Some(agent) = tab.pane.agent.as_mut().filter(|agent| agent.meeseeks.is_none()) {
                agent.meeseeks = fake.state(now_ms);
            }
        }
    }

    fn watch_struggles(&mut self, ctx: &egui::Context, now_ms: i64) {
        let mut soonest: Option<i64> = None;
        let mut told = Vec::new();
        for tab in self.tabs.items() {
            let Some(meeseeks) = tab.pane.agent.as_ref().and_then(|agent| agent.meeseeks.as_ref())
            else {
                continue;
            };
            if meeseeks.struggle(now_ms).is_some() {
                told.push((tab.id, tab.title.clone()));
            } else {
                let left =
                    tidewatch_core::meeseeks::MAX_MINUTES * 60_000 - meeseeks.elapsed_ms(now_ms);
                soonest = Some(soonest.map_or(left, |soonest| soonest.min(left)));
            }
        }
        for (tab, title) in told {
            if self.meeseeks.first_struggle(tab) {
                self.show_toast(format!("{title} is struggling"), ctx);
            }
        }
        if let Some(left) = soonest {
            let wait =
                Duration::from_millis(left.max(0) as u64) + Duration::from_secs_f64(WAKE_SLACK_S);
            let wait = if self.meeseeks.fake().is_on() { wait } else { wait.min(CLOCK_TICK) };
            ctx.request_repaint_after(wait);
        }
    }

    fn paste_when_idle(&mut self, ctx: &egui::Context, now: f64) {
        for tab in self.meeseeks.pending_tabs() {
            let Some(found) = self.tabs.items().iter().find(|t| t.id == tab) else {
                self.meeseeks.forget(tab);
                continue;
            };
            let status = found.pane.agent.as_ref().map(|agent| agent.run.status().clone());
            let alive = matches!(found.pane.body, Body::Shell(_));
            if !alive || ask_blocked(status.as_ref()).is_some() {
                continue;
            }
            if let Some(text) = self.meeseeks.take_paste(tab) {
                flow_glue::paste(&mut self.tabs, tab, &text);
                self.flow.queue_enter(tab, now);
                ctx.request_repaint_after(Duration::from_secs_f64(ENTER_DELAY_SECS + WAKE_SLACK_S));
            }
        }
    }

    fn meeseeks_done_with(
        &mut self,
        done: Done<stdusk_core::terminal::PtyTerm>,
        ctx: &egui::Context,
        now: f64,
    ) {
        match done {
            Done::Checked { tab, step, result } => {
                let event = match result {
                    Ok(pr_url) => Event::Passed { step, pr_url },
                    Err(detail) => Event::Failed { step, detail },
                };
                self.meeseeks_event(tab, event, ctx);
            }
            Done::Poofed { tab } => self.finish_poof(tab, ctx, now),
            Done::Refused { tab, why, shell } => {
                if let Some(pane) = self.tabs.get_mut(tab).map(|t| &mut t.pane) {
                    match shell {
                        Some(shell) => pane.restore_shell(shell),
                        None => pane.remove_failed(why.clone()),
                    }
                }
                self.meeseeks.leaving(tab);
                self.meeseeks_event(tab, Event::PoofRefused(why), ctx);
            }
            Done::Loaded(_) | Done::Logged => {}
        }
    }

    pub(super) fn meeseeks_said(&mut self, tab: u64, summary: String, ctx: &egui::Context) {
        if !self.meeseeks.fake().is_on() {
            self.meeseeks_event(tab, Event::Said(summary), ctx);
        }
    }

    fn meeseeks_event(&mut self, tab: u64, event: Event, ctx: &egui::Context) {
        let now_ms = wall_ms() as i64;
        let Some(meeseeks) = self
            .tabs
            .get_mut(tab)
            .and_then(|t| t.pane.agent.as_mut())
            .and_then(|agent| agent.meeseeks.as_mut())
        else {
            return;
        };
        let shown = self.status_log.then(|| format!("{event:?}"));
        let command = meeseeks.apply(event, now_ms);
        if let Some(event) = shown {
            eprintln!("tidewatch: meeseeks tab={tab} {event} -> {command:?} at {}", wall_ms());
        }
        match command {
            Command::Nothing => {}
            Command::Check(step) => self.start_check(tab, step, ctx),
            Command::Paste(text) => self.meeseeks.queue_paste(tab, text),
            Command::Poof { summary, pr_url } => self.start_poof(tab, summary, pr_url, ctx),
        }
        ctx.request_repaint();
    }

    fn start_check(&mut self, tab: u64, step: tidewatch_core::meeseeks::Step, ctx: &egui::Context) {
        let found = self.tabs.get_mut(tab);
        let Some((worktree, summary, pr)) = found.and_then(|t| {
            let agent = t.pane.agent.as_ref()?;
            let meeseeks = agent.meeseeks.as_ref()?;
            Some((t.pane.worktree.clone()?, meeseeks.summary.clone()?, meeseeks.criteria.pr))
        }) else {
            self.meeseeks_event(
                tab,
                Event::Failed { step, detail: "this tab has no worktree to check".to_owned() },
                ctx,
            );
            return;
        };
        let spec = CheckSpec { tools: self.flow.tools(), worktree, summary };
        self.meeseeks.check(ctx, tab, step, spec, pr);
    }

    fn start_poof(
        &mut self,
        tab: u64,
        summary: String,
        pr_url: Option<String>,
        ctx: &egui::Context,
    ) {
        let (Some(paths), now_ms) = (self.paths.clone(), wall_ms() as i64) else {
            self.meeseeks_event(
                tab,
                Event::PoofRefused("Tidewatch has no home folder".into()),
                ctx,
            );
            return;
        };
        let started = self.tabs.get_mut(tab).and_then(|t| {
            let started = t.pane.agent.as_ref()?.meeseeks.as_ref()?.started_ms;
            let (shell, worktree) = t.pane.begin_remove()?;
            Some((started, shell?, worktree))
        });
        let Some((started_ms, shell, worktree)) = started else {
            self.meeseeks_event(tab, Event::PoofRefused("the tab is busy".into()), ctx);
            return;
        };
        let leaving = Leaving {
            name: worktree.name().to_owned(),
            summary,
            pr_url,
            duration_secs: ((now_ms - started_ms).max(0) / 1000) as u64,
        };
        let trust = self.trust.grant();
        self.meeseeks.poof(ctx, paths, (tab, shell, worktree, trust), leaving);
    }

    fn finish_poof(&mut self, tab: u64, ctx: &egui::Context, now: f64) {
        let Some(leaving) = self.meeseeks.leaving(tab) else { return };
        let name = leaving.name.clone();
        self.meeseeks.record(
            ctx,
            Poofed {
                name: leaving.name,
                summary: leaving.summary,
                pr_url: leaving.pr_url,
                duration_secs: leaving.duration_secs,
                date: today(),
            },
        );
        self.show_toast(format!("{name} poofed"), ctx);
        self.meeseeks.start_fade(tab, now);
        ctx.request_repaint();
    }

    pub(super) fn take_over_meeseeks(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(agent) = self.tabs.get_mut(tab).and_then(|t| t.pane.agent.as_mut()) else {
            return;
        };
        if agent.meeseeks.take().is_none() {
            return;
        }
        self.meeseeks.forget(tab);
        self.show_toast("Taken over. It is an ordinary agent now", ctx);
    }

    pub(super) fn give_up_meeseeks(&mut self, tab: u64, ctx: &egui::Context) {
        if let Some(index) = self.tabs.index_of(tab) {
            self.close_tab(index, ctx);
        }
    }
}
