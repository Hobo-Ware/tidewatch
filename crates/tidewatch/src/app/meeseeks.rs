//! The window's side of Meeseeks: what `meeseeks_done` starts, each check's answer, the pastes that
//! wait for an idle agent, the poof and the summary tab it leaves, and Take over and Give up.

use std::time::Duration;

use tidewatch_core::kind::Kind;
use tidewatch_core::meeseeks::{Command, Criteria, Event, Meeseeks, Phase};
use tidewatch_core::meeseeks_check::Work;
use tidewatch_core::meeseeks_log::{Poofed, StageLine};
use tidewatch_core::sent::{Reason, because};

use super::{Tidewatch, wall_ms};
use crate::due::ask_blocked;
use crate::flow_glue;
use crate::meeseeks_glue::{CheckSpec, Done, FAKE_NAMED, LOOK_EVERY_SECS, LookSpec, fake_poofed};
use crate::outbox::{Offer, Send};
use crate::pane::{Body, Pane};
use crate::progress_glue::today;
use crate::progress_model::{self, StageShape};
use crate::wake::WAKE_SLACK_S;
use crate::{meeseeks_summary, meeseeks_summary_view};

const CLOCK_TICK: Duration = Duration::from_secs(30);
const HOLD_LOOK_S: f64 = 0.5;

impl Tidewatch {
    pub(super) fn drive_meeseeks(&mut self, ctx: &egui::Context, now: f64) {
        let now_ms = wall_ms() as i64;
        self.inject_fake_meeseeks(ctx, now_ms);
        for done in self.meeseeks.drain(ctx) {
            self.meeseeks_done_with(done, ctx);
        }
        self.paste_when_idle(ctx, now);
        self.watch_worktrees(ctx, now);
        self.watch_struggles(ctx, now_ms);
        self.read_summary_branches(ctx);
    }

    pub(super) fn active_is_poofed(&self) -> bool {
        matches!(self.tabs.items()[self.tabs.active()].pane.body, Body::Poofed(_))
    }

    pub(super) fn show_poofed(&mut self, ui: &mut egui::Ui, area: egui::Rect, ctx: &egui::Context) {
        let index = self.tabs.active();
        let tab = &self.tabs.items()[index];
        let Body::Poofed(record) = &tab.pane.body else { return };
        let facts = self.meeseeks.branch(tab.id);
        let summary = meeseeks_summary::summary(record, facts, self.meeseeks.zone());
        if meeseeks_summary_view::show(ui, area, &summary).is_some() {
            self.close_tab(index, ctx);
        }
    }

    fn read_summary_branches(&mut self, ctx: &egui::Context) {
        let tools = self.flow.tools();
        for tab in self.tabs.items() {
            if let Body::Poofed(record) = &tab.pane.body
                && !record.branch.is_empty()
                && !record.repo.as_os_str().is_empty()
            {
                self.meeseeks.read_branch(ctx, tab.id, tools.clone(), record);
            }
        }
    }

    /// Opens the summary of a poofed Meeseeks: the tab that already shows it, else a new one in the
    /// group of its repo (the group on show when the repo is not in the window).
    pub(super) fn open_poofed(&mut self, record: &Poofed) -> u64 {
        let open = self
            .tabs
            .items()
            .iter()
            .find(|tab| matches!(&tab.pane.body, Body::Poofed(shown) if **shown == *record));
        if let Some(tab) = open {
            return tab.id;
        }
        let group = self
            .workspaces
            .iter()
            .position(|workspace| workspace.dir() == record.repo)
            .unwrap_or_else(|| self.tabs.active_group());
        let pane =
            Pane { body: Body::Poofed(Box::new(record.clone())), worktree: None, agent: None };
        self.tabs.push_in(group, record.name.clone(), pane)
    }

    fn inject_fake_meeseeks(&mut self, ctx: &egui::Context, now_ms: i64) {
        let fake = self.meeseeks.fake();
        if let Some(text) = self.meeseeks.fake_toast() {
            self.show_toast(text, ctx);
        }
        if fake == crate::meeseeks_glue::Fake::Poofed && self.meeseeks.first_fake_summary() {
            let records = fake_poofed(today(), now_ms);
            self.open_poofed(&records[0]);
        }
        if fake == crate::meeseeks_glue::Fake::Named {
            self.inject_named_meeseeks(now_ms);
            return;
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

    fn inject_named_meeseeks(&mut self, now_ms: i64) {
        let agents = self.tabs.items_mut().iter_mut().filter_map(|tab| tab.pane.agent.as_mut());
        for (agent, fake) in agents.zip(FAKE_NAMED.iter().cycle()) {
            if agent.meeseeks.is_none() {
                agent.meeseeks = Some(Meeseeks {
                    kind: Kind::Edit,
                    name: fake.chosen.map(str::to_owned),
                    ..Meeseeks::summon(fake.task, Criteria::default(), now_ms - 6 * 60_000)
                });
                agent.reader.fake_session_title(fake.session_title);
            }
        }
    }

    fn look_spec(&self, tab: u64) -> Option<LookSpec> {
        let found = self.tabs.get(tab)?;
        let meeseeks = found.pane.agent.as_ref()?.meeseeks.as_ref()?;
        Some(LookSpec {
            tools: self.flow.tools(),
            dir: found.pane.worktree.as_ref()?.path.clone(),
            started_at: meeseeks.start_head.clone(),
        })
    }

    fn watch_worktrees(&mut self, ctx: &egui::Context, now: f64) {
        if self.meeseeks.fake().is_on() {
            return;
        }
        let waiting: Vec<u64> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| {
                tab.pane.agent.as_ref().and_then(|agent| agent.meeseeks.as_ref()).is_some_and(
                    |meeseeks| meeseeks.kind == Kind::ReadOnly && meeseeks.phase == Phase::Working,
                )
            })
            .map(|tab| tab.id)
            .collect();
        for tab in waiting {
            if self.meeseeks.look_due(tab, now)
                && let Some(spec) = self.look_spec(tab)
            {
                self.meeseeks.look(ctx, tab, spec, now);
            }
            ctx.request_repaint_after(Duration::from_secs_f64(LOOK_EVERY_SECS));
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
            let Some(found) = self.tabs.get(tab) else {
                self.meeseeks.forget(tab);
                continue;
            };
            let status = found.pane.agent.as_ref().map(|agent| agent.run.status().clone());
            let alive = matches!(found.pane.body, Body::Shell(_));
            if !alive || ask_blocked(status.as_ref()).is_some() {
                continue;
            }
            let Some(text) = self.meeseeks.take_paste(tab) else { continue };
            let send = Send::auto(&text, Reason::MeeseeksCheck, because::MEESEEKS);
            match flow_glue::offer_to_agent(&mut self.tabs, &mut self.flow, tab, &send, ctx, now) {
                Offer::Go | Offer::Refuse(_) => {}
                Offer::Wait | Offer::Paused | Offer::Saving => {
                    self.meeseeks.put_back_paste(tab, text);
                    ctx.request_repaint_after(Duration::from_secs_f64(HOLD_LOOK_S));
                }
            }
        }
    }

    fn meeseeks_done_with(
        &mut self,
        done: Done<stdusk_core::terminal::PtyTerm>,
        ctx: &egui::Context,
    ) {
        match done {
            Done::Checked { tab, step, result } => {
                let event = match result {
                    Ok(checked) => {
                        Event::Passed { step, pr_url: checked.pr_url, detail: checked.detail }
                    }
                    Err(detail) => Event::Failed { step, detail },
                };
                self.meeseeks_event(tab, event, ctx);
            }
            Done::Poofed { tab } => self.finish_poof(tab, ctx),
            Done::Looked { tab, ticket, edited } => {
                if let Ok(edited) = edited {
                    self.meeseeks_event(tab, Event::Looked { edited }, ctx);
                }
                if let Some(summary) = self.meeseeks.looked(tab, ticket) {
                    self.meeseeks_event(tab, Event::Said(summary), ctx);
                }
            }
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
            Done::Loaded(_) | Done::Branch { .. } | Done::Testing { .. } | Done::Logged => {}
        }
    }

    pub(super) fn meeseeks_said(&mut self, tab: u64, summary: String, ctx: &egui::Context) {
        if self.meeseeks.fake().is_on() {
            return;
        }
        match self.look_spec(tab) {
            Some(spec) => {
                self.meeseeks.say_after_look(tab, summary);
                self.meeseeks.look(ctx, tab, spec, ctx.input(|input| input.time));
            }
            None => self.meeseeks_event(tab, Event::Said(summary), ctx),
        }
    }

    pub(super) fn meeseeks_declared(&mut self, tab: u64, kind: Kind, ctx: &egui::Context) {
        self.meeseeks_event(tab, Event::Declared(kind), ctx);
    }

    fn meeseeks_event(&mut self, tab: u64, event: Event, ctx: &egui::Context) {
        let now_ms = wall_ms() as i64;
        let Some(meeseeks) = self.tabs.agent_mut(tab).and_then(|agent| agent.meeseeks.as_mut())
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
            Command::Poof { summary, pr_url } => {
                self.start_poof(tab, &summary, pr_url.as_deref(), ctx);
            }
        }
        ctx.request_repaint();
    }

    fn start_check(&mut self, tab: u64, step: tidewatch_core::meeseeks::Step, ctx: &egui::Context) {
        let found = self.tabs.get_mut(tab);
        let Some((worktree, pr, work)) = found.and_then(|t| {
            let agent = t.pane.agent.as_ref()?;
            let meeseeks = agent.meeseeks.as_ref()?;
            Some((t.pane.worktree.clone()?, meeseeks.criteria.pr, Work::of(meeseeks)))
        }) else {
            self.meeseeks_event(
                tab,
                Event::Failed { step, detail: "this tab has no worktree to check".to_owned() },
                ctx,
            );
            return;
        };
        let workflow = self.workflow.profile(&worktree.repo);
        let spec = CheckSpec { tools: self.flow.tools(), worktree, work, workflow };
        self.meeseeks.check(ctx, tab, step, spec, pr);
    }

    fn start_poof(&mut self, tab: u64, summary: &str, pr_url: Option<&str>, ctx: &egui::Context) {
        let (Some(paths), now_ms) = (self.paths.clone(), wall_ms() as i64) else {
            self.meeseeks_event(
                tab,
                Event::PoofRefused("Tidewatch has no home folder".into()),
                ctx,
            );
            return;
        };
        let started = self.tabs.get_mut(tab).and_then(|t| {
            let agent = t.pane.agent.as_ref()?;
            let meeseeks = agent.meeseeks.as_ref()?;
            let end = agent.run.end_block().cloned().unwrap_or_default();
            let worktree = t.pane.worktree.as_ref()?;
            let name = meeseeks.shown_name().unwrap_or_else(|| worktree.name().to_owned());
            let leaving = Poofed {
                task: meeseeks.task.clone(),
                branch: worktree.branch.clone(),
                repo: worktree.repo.clone(),
                ended_ms: now_ms,
                checks: meeseeks.check_lines(),
                kind: meeseeks.kind,
                done: end.done,
                next: end.next,
                ..Poofed::bare(
                    &name,
                    summary,
                    pr_url,
                    ((now_ms - meeseeks.started_ms).max(0) / 1000) as u64,
                    today(),
                )
            };
            let (shell, worktree) = t.pane.begin_remove()?;
            Some((shell?, worktree, leaving))
        });
        let Some((shell, worktree, mut leaving)) = started else {
            self.meeseeks_event(tab, Event::PoofRefused("the tab is busy".into()), ctx);
            return;
        };
        let kind = leaving.kind;
        leaving.stages = self.progress.ledger(tab).map_or_else(Vec::new, |known| {
            progress_model::bars(&known.ledger, StageShape { kind, no_diff: false })
                .into_iter()
                .map(|bar| StageLine {
                    name: bar.label,
                    done: u32::try_from(bar.done).unwrap_or(u32::MAX),
                    total: u32::try_from(bar.total).unwrap_or(u32::MAX),
                })
                .collect()
        });
        let trust = self.trust.grant();
        self.meeseeks.poof(ctx, paths, (tab, shell, worktree, trust), leaving);
    }

    fn finish_poof(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(poofed) = self.meeseeks.leaving(tab) else { return };
        self.meeseeks.record(ctx, poofed.clone());
        self.show_toast(format!("{} poofed", poofed.name), ctx);
        self.forget_tab(tab);
        self.views.remove(&tab);
        if let Some(tab) = self.tabs.get_mut(tab) {
            tab.pane.poofed(poofed);
        }
        ctx.request_repaint();
    }

    pub(super) fn take_over_meeseeks(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(agent) = self.tabs.agent_mut(tab) else {
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
