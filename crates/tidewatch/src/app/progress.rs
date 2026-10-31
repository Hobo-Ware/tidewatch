//! The app's side of the progress ledgers: the agent tabs' facts go to the host each frame, and a
//! ledger an agent sends is stored and logged.

use tidewatch_core::background;
use tidewatch_core::civil_date::Date;
use tidewatch_core::job::{Agent, JobBoard};
use tidewatch_core::progress::{Ledger, State};

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::WAKE_SLACK_S;
use super::settings::open_in_editor;
use super::{Tidewatch, wall_ms};
use crate::agent_avatar;
use crate::ask_model::{self, Ask};
use crate::coming_up::coming_up;
use crate::due::{
    Due, Key, ask_blocked, ask_text, due_items, next_to_start, secs_to_midnight, tell_all,
};
use crate::job_glue;
use crate::job_model::{agent_of, lane_line};
use crate::link_model;
use crate::notify;
use crate::outbox::{Offer, Origin, Send};
use crate::overview::Overview;
use crate::overview_checklist::Source;
use crate::pane::{AgentTab, Body};
use crate::progress_glue::today;
use crate::progress_model::{look, look_as, questions};
use crate::progress_nudge as nudge;
use crate::progress_state::Facts;
use crate::status_view::state_of;
use crate::{due_view, flow_glue, reader_glue, reader_view};
use tidewatch_core::panel_rules::{Live, answered_since_save, is_stale, standing};
use tidewatch_core::sent::{Cap, Reason as Said, because};
use tidewatch_core::status::{AgentStatus, Reason};

const DRAW_ASK: &str = tidewatch_core::avatar_ask::ASK;

impl Tidewatch {
    pub(super) fn drive_progress(&mut self, ctx: &egui::Context) {
        let facts: Vec<Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| !matches!(tab.pane.body, Body::Draft(_)))
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                Some(Facts {
                    id: tab.id,
                    notes_file: agent.run.notes_file.as_deref()?,
                    worktree: tab
                        .pane
                        .worktree
                        .as_ref()
                        .map(|worktree| (worktree.repo.as_path(), worktree.name())),
                    profile_id: agent
                        .launched
                        .as_ref()
                        .map(|launched| launched.profile_id.as_str()),
                    session_id: agent
                        .launched
                        .as_ref()
                        .and_then(|launched| launched.session_id.as_deref()),
                    fallback: agent
                        .launched
                        .as_ref()
                        .and_then(|launched| launched.dir.as_deref())
                        .map(|dir| (dir, tab.title.as_str())),
                })
            })
            .collect();
        self.progress.tick(&facts, ctx);
        for tab in self.tabs.items_mut() {
            if let Some(agent) = tab.pane.agent.as_mut() {
                let goal = self.progress.ledger(tab.id).map(|known| known.ledger.goal.as_str());
                agent.run.set_goal(goal);
            }
        }
        let tabs: std::collections::HashSet<u64> =
            self.tabs.items().iter().map(|tab| tab.id).collect();
        self.progress_ui.forget_except(|id| tabs.contains(&id));
        self.report_nudges.forget_except(|id| tabs.contains(&id));
        self.drive_reported_states(ctx.input(|input| input.time));
    }

    pub(super) fn drive_avatar_choice(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let choice = agent_avatar::take_choice(ctx);
        if let Some((tab, agent_avatar::Choice::Redraw)) = choice {
            self.avatar_redraws.add(tab);
        }
        let mut toasts = self.avatar_redraws.drive(&mut self.tabs, &mut self.flow, ctx, now);
        if let Some((tab, agent_avatar::Choice::Redraw)) = choice
            && self.avatar_redraws.holds(tab)
        {
            toasts.push("The agent is busy. It will be asked to redraw when it is idle".to_owned());
        }
        for text in toasts {
            self.show_toast(text, ctx);
        }
        let Some((tab, choice)) = choice else { return };
        match choice {
            agent_avatar::Choice::Redraw => {}
            agent_avatar::Choice::Draw => {
                let now = ctx.input(|i| i.time);
                let send = Send::by_hand(DRAW_ASK, Said::Avatar, because::AVATAR_DRAW);
                if self.ask_agent(tab, &send, ctx, now) {
                    self.remember_avatar_ask(tab, ctx);
                }
            }
            agent_avatar::Choice::UseLetter => {
                let notes = self
                    .tabs
                    .items()
                    .iter()
                    .find(|t| t.id == tab)
                    .and_then(|t| t.pane.agent.as_ref()?.run.notes_file.clone());
                if let Some(notes) = notes {
                    self.avatar_live.clear(tab, &notes, ctx);
                }
            }
        }
    }

    fn notes_of(&self, tab: u64) -> Option<std::path::PathBuf> {
        self.tabs.items().iter().find(|t| t.id == tab)?.pane.agent.as_ref()?.run.notes_file.clone()
    }

    fn remember_avatar_ask(&mut self, tab: u64, ctx: &egui::Context) {
        if let Some(notes) = self.notes_of(tab) {
            self.avatar_live.mark_asked(&notes, ctx);
        }
    }

    pub(super) fn job_agent(&self, tab: u64) -> Agent {
        if let Some(fake) = self.job_live.fake_agent() {
            return fake;
        }
        self.tabs
            .items()
            .iter()
            .find(|candidate| candidate.id == tab)
            .and_then(|candidate| candidate.pane.agent.as_ref())
            .map_or(Agent::Working, |agent| agent_of(agent.run.shown_tone()))
    }

    pub(super) fn drive_jobs(&mut self, ctx: &egui::Context) {
        let now_ms = wall_ms() as i64;
        let facts: Vec<job_glue::Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let notes_file = tab.pane.agent.as_ref()?.run.notes_file.as_deref()?;
                Some(job_glue::Facts { id: tab.id, notes_file })
            })
            .collect();
        let meeseeks: Vec<u64> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| tab.pane.agent.as_ref().is_some_and(|agent| agent.meeseeks.is_some()))
            .map(|tab| tab.id)
            .collect();
        self.avatar_live.fixed_faces(&meeseeks, ctx);
        self.avatar_live.tick(&facts, ctx);
        self.avatar_redraws.queue_unreadable(&mut self.avatar_live);
        if let Some(listener) = self.hooks.listener() {
            listener.exempt_from_avatar_ask(&self.avatar_live.exempt_from_ask(&facts));
        }
        let agents: Vec<(u64, Agent)> =
            self.tabs.items().iter().map(|tab| (tab.id, self.job_agent(tab.id))).collect();
        let running: Vec<u64> = agents
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| self.job_live.board(*id).is_some_and(JobBoard::has_running))
            .collect();
        let mut wake = self.job_live.tick(&facts, &agents, ctx, now_ms);
        let ended =
            running.iter().any(|id| !self.job_live.board(*id).is_some_and(JobBoard::has_running));
        if ended {
            self.parking.look_again();
            ctx.request_repaint();
        }
        for tab in self.tabs.items() {
            let (Body::Shell(term), Some(_)) = (&tab.pane.body, &tab.pane.agent) else { continue };
            let later = self.job_live.scan(tab.id, term.output_gen(), now_ms, || {
                job_glue::screen_rows(&term.grid_snapshot())
            });
            wake = wake.into_iter().chain(later).min();
        }
        if let Some(at) = wake {
            let wait = u64::try_from((at - now_ms).max(0)).unwrap_or(0);
            ctx.request_repaint_after(
                Duration::from_millis(wait) + Duration::from_secs_f64(WAKE_SLACK_S),
            );
        }
    }

    pub(super) fn drive_links(&mut self, ctx: &egui::Context) {
        let now_ms = wall_ms() as i64;
        let facts: Vec<job_glue::Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let notes_file = tab.pane.agent.as_ref()?.run.notes_file.as_deref()?;
                Some(job_glue::Facts { id: tab.id, notes_file })
            })
            .collect();
        self.link_live.tick(&facts, ctx, now_ms);
        for tab in self.tabs.items_mut() {
            let id = tab.id;
            if let Some(agent) = tab.pane.agent.as_mut() {
                self.link_live.found(id, agent.reader.take_links(), ctx);
            }
        }
        for tab in self.tabs.items() {
            if tab.pane.agent.is_none() {
                continue;
            }
            let summary = self.pr.watch(tab.id).and_then(crate::pr_watch::Watch::summary);
            let lookup = self.flow.tab(tab.id).and_then(|state| state.pr.pr());
            if let Some(pr) = crate::link_model::own_pr(summary, lookup) {
                self.link_live.own_pr(tab.id, &pr, now_ms, ctx);
            }
        }
    }

    pub(super) fn drive_background(&mut self, ctx: &egui::Context, now: f64) {
        let now_ms = wall_ms() as i64;
        let idle: Vec<u64> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| {
                tab.pane
                    .agent
                    .as_ref()
                    .is_some_and(|agent| *agent.run.status() == AgentStatus::Done)
            })
            .map(|tab| tab.id)
            .collect();
        for tab in idle {
            let Some(launch) = self.job_live.background_due(tab, true, now_ms) else { continue };
            let text = background::nudge_text(&launch.command);
            let send = Send::auto(&text, Said::BackgroundJob, because::BACKGROUND_JOB);
            match flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now) {
                Offer::Go | Offer::Wait => {
                    self.job_live.background_nudged(tab, &launch.tool_use_id);
                }
                Offer::Paused | Offer::Refuse(_) => {}
            }
        }
    }

    /// Tells an agent whose report went stale, or an idle one whose last message holds a question
    /// the report lacks, through the one send gate. An idle agent hears each once per saved
    /// report. A working one hears it mid-turn when its input box is free, again on a backoff, up
    /// to a cap per saved report.
    pub(super) fn drive_report_nudges(&mut self, ctx: &egui::Context, now: f64) {
        if self.screenshot.is_some() {
            return;
        }
        let now_ms = wall_ms() as i64;
        let mut due = Vec::new();
        let mut wake: Option<i64> = None;
        for tab in self.tabs.items() {
            let Some(agent) =
                tab.pane.agent.as_ref().filter(|_| matches!(tab.pane.body, Body::Shell(_)))
            else {
                continue;
            };
            let known = self.progress.ledger(tab.id);
            let facts = nudge::Facts {
                idle: *agent.run.status() == AgentStatus::Done,
                busy: *agent.run.status() == AgentStatus::Working,
                ledger: known.map(|known| &known.ledger),
                saved_ms: known.and_then(|known| known.updated_ms),
                turn_started_ms: agent
                    .run
                    .turn_started_at()
                    .map(|at| now_ms - ((now - at) * 1000.0) as i64),
                block: agent.run.end_block(),
                now_ms,
            };
            let told = self.report_nudges.told(tab.id);
            if let Some(found) = nudge::due(&facts, &told) {
                let cap = Cap { n: told.quiet_sent(facts.saved_ms) + 1, max: nudge::QUIET_MAX };
                due.push((tab.id, found, facts.saved_ms, nudge::text(found, &facts), cap));
            }
            wake = wake.into_iter().chain(nudge::wait_ms(&facts, &told)).min();
        }
        for (tab, found, saved, text, cap) in due {
            let send = Send::auto(&text, Said::LedgerGuard, found.because());
            if found == nudge::Nudge::Quiet {
                let send = send.with_cap(cap).mid_turn();
                match flow_glue::offer_to_agent(
                    &mut self.tabs,
                    &mut self.flow,
                    tab,
                    &send,
                    ctx,
                    now,
                ) {
                    Offer::Go => self.report_nudges.mark(tab, found, saved, now_ms),
                    Offer::Wait => {
                        self.report_nudges.retry_later(tab, saved, now_ms);
                        wake = wake.into_iter().chain(Some(nudge::QUIET_RETRY_MS)).min();
                    }
                    Offer::Paused | Offer::Refuse(_) => {}
                }
                continue;
            }
            match flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, tab, send, ctx, now) {
                Offer::Go | Offer::Wait => self.report_nudges.mark(tab, found, saved, now_ms),
                Offer::Paused | Offer::Refuse(_) => {}
            }
        }
        if let Some(wait) = wake {
            let wait = u64::try_from(wait).unwrap_or(0);
            ctx.request_repaint_after(
                Duration::from_millis(wait) + Duration::from_secs_f64(WAKE_SLACK_S),
            );
        }
    }

    /// The items due in tab `id` that the Reader offers: open, dated today or before, not snoozed. A
    /// tab that runs on its own offers none: it is asked to start them (`drive_due_start`).
    pub(super) fn due_of(&self, id: u64) -> Vec<Due> {
        if self.is_autonomous(id) { Vec::new() } else { self.due_items_of(id) }
    }

    fn due_items_of(&self, id: u64) -> Vec<Due> {
        let tab = self.tabs.items().iter().find(|tab| tab.id == id);
        let (Some(worktree), Some(known)) =
            (tab.and_then(|tab| tab.pane.worktree.as_ref()), self.progress.ledger(id))
        else {
            return Vec::new();
        };
        let place = (worktree.repo.as_path(), worktree.name());
        due_items(today(), &known.ledger, place, &self.snoozes)
    }

    pub(super) fn apply_due_clicks(
        &mut self,
        tab: u64,
        clicks: Vec<due_view::Action>,
        ctx: &egui::Context,
        now: f64,
    ) {
        for click in clicks {
            match click {
                due_view::Action::Ask(text) => {
                    let send = Send::by_hand(&text, Said::DueToday, because::DUE_CLICK);
                    self.ask_agent(tab, &send, ctx, now);
                }
                due_view::Action::Snooze(what) => {
                    let key = self.due_key(tab, &what);
                    if let Some(key) = key {
                        self.snoozes.snooze(key, today());
                        self.show_toast("Snoozed until tomorrow", ctx);
                    }
                }
            }
        }
    }

    fn due_key(&self, tab: u64, what: &str) -> Option<Key> {
        let worktree = self.tabs.items().iter().find(|t| t.id == tab)?.pane.worktree.as_ref()?;
        Some(Key::new(&worktree.repo, worktree.name(), what))
    }

    /// Types the message and presses Enter, only while the agent is idle and no other message
    /// just went in. False when it did not go.
    pub(super) fn ask_agent(
        &mut self,
        tab: u64,
        send: &Send,
        ctx: &egui::Context,
        now: f64,
    ) -> bool {
        let status = self
            .tabs
            .items()
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pane.agent.as_ref())
            .map(|agent| agent.run.status().clone());
        if let Some(reason) = ask_blocked(status.as_ref()) {
            self.show_toast(reason, ctx);
            return false;
        }
        match flow_glue::offer_to_agent(&mut self.tabs, &mut self.flow, tab, send, ctx, now) {
            Offer::Go => true,
            Offer::Wait => {
                if send.origin == Origin::Hand {
                    self.show_toast("A message just went to the agent. Ask again in a moment", ctx);
                }
                false
            }
            Offer::Paused => false,
            Offer::Refuse(reason) => {
                self.show_toast(reason, ctx);
                false
            }
        }
    }

    /// Asks each tab that runs on its own to start its due items, one at a time, only while the agent
    /// is idle and once per item per day.
    pub(super) fn drive_due_start(&mut self, ctx: &egui::Context, now: f64) {
        if self.screenshot.is_some() {
            return;
        }
        let today = today();
        let ids: Vec<u64> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| matches!(tab.pane.body, Body::Shell(_)))
            .filter(|tab| tab.pane.agent.as_ref().is_some_and(AgentTab::autonomous))
            .map(|tab| tab.id)
            .collect();
        for id in ids {
            let status = self
                .tabs
                .items()
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.pane.agent.as_ref())
                .map(|agent| agent.run.status().clone());
            if ask_blocked(status.as_ref()).is_some() {
                continue;
            }
            self.load_asked(id, today);
            let dues = self.due_items_of(id);
            let Some((key, text)) =
                next_to_start(today, &dues, |due| self.key_of(id, due), &self.started)
                    .map(|(key, due)| (key, ask_text(due)))
            else {
                continue;
            };
            let send = Send::auto(&text, Said::DueToday, because::DUE_OWN);
            if self.ask_agent(id, &send, ctx, now) {
                self.started.mark(key, today);
                self.save_asked(id, today);
            }
        }
    }

    fn asked_file(&self, tab: u64) -> Option<(PathBuf, Key)> {
        let key = self.due_key(tab, "")?;
        let file = self.paths.as_ref()?.due_asked_file(&key.repo, &key.tab);
        Some((file, key))
    }

    fn load_asked(&mut self, tab: u64, today: Date) {
        if !self.started_loaded.insert(tab) {
            return;
        }
        let Some((file, key)) = self.asked_file(tab) else { return };
        self.started.restore(&file, (&key.repo, &key.tab), today);
    }

    fn save_asked(&self, tab: u64, today: Date) {
        let Some((file, key)) = self.asked_file(tab) else { return };
        if let Err(e) = self.started.persist(&file, (&key.repo, &key.tab), today) {
            eprintln!("tidewatch: could not save {}: {e}", file.display());
        }
    }

    fn key_of(&self, tab: u64, due: &Due) -> Key {
        self.due_key(tab, &due.what).unwrap_or_else(|| Key::new(Path::new(""), "", &due.what))
    }

    /// Tells the desktop about each item that turned due, once a day, while the window is away.
    pub(super) fn drive_due(&mut self, ctx: &egui::Context) {
        if self.progress.index().tabs.is_empty() || self.screenshot.is_some() {
            return;
        }
        let today = today();
        let rows = coming_up(today, self.progress.index());
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
        let away = notify::window_is_away(focused, minimized);
        let notifier = &mut self.notifier;
        let sent = tell_all(today, &rows, &self.snoozes, &mut self.told, |note| {
            notifier.offer_due(note, away)
        });
        if sent > 0 && self.status_log {
            eprintln!("tidewatch: due told {sent} at {}", wall_ms());
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        ctx.request_repaint_after(Duration::from_secs(secs_to_midnight(secs) + 1));
    }

    fn live_of(agent: &AgentTab, now: f64) -> Live {
        let now_ms = wall_ms() as i64;
        let wall = |at: f64| now_ms - ((now - at) * 1000.0) as i64;
        Live {
            asking: matches!(
                agent.run.status(),
                AgentStatus::NeedsYou(Reason::Permission | Reason::Question)
            ),
            turn_started_ms: agent.run.turn_started_at().map(wall),
            rested_at_ms: agent.run.rested_at().map(wall),
        }
    }

    /// Whether the tab's ledger still asks the owner something the agent has moved on from.
    pub(super) fn ledger_stale(&self, tab: u64, now: f64) -> bool {
        let Some(known) = self.progress.ledger(tab) else { return false };
        if answered_since_save(self.answered_asks.get(&tab), known.updated_ms) {
            return true;
        }
        let Some(agent) = self
            .tabs
            .items()
            .iter()
            .find(|candidate| candidate.id == tab)
            .and_then(|candidate| candidate.pane.agent.as_ref())
        else {
            return false;
        };
        is_stale(&known.ledger, known.updated_ms, Self::live_of(agent, now))
    }

    /// Hands each agent tab the state its report stands for, so every screen shows one word.
    fn drive_reported_states(&mut self, now: f64) {
        let states: Vec<(u64, Option<State>)> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let state = self.progress.ledger(tab.id).map(|known| {
                    let stale = self.ledger_stale(tab.id, now);
                    let working = *agent.run.status() == AgentStatus::Working;
                    standing(
                        &known.ledger,
                        known.updated_ms,
                        Self::live_of(agent, now),
                        (stale, working),
                    )
                });
                Some((tab.id, state))
            })
            .collect();
        for (id, state) in states {
            if let Some(agent) = self.tabs.get_mut(id).and_then(|tab| tab.pane.agent.as_mut()) {
                agent.run.set_reported(state);
            }
        }
    }

    pub(super) fn add_looks(&self, overview: &mut Overview, now: f64) {
        let now_ms = wall_ms() as i64;
        let places = overview.places.iter_mut().flat_map(|place| place.rows.iter_mut());
        for row in overview.needs.iter_mut().chain(places) {
            let stale = self.ledger_stale(row.tab_id, now);
            row.look = self.progress.ledger(row.tab_id).map(|known| match state_of(row.tone) {
                Some(state) => look_as(&known.ledger, stale, state),
                None => look(&known.ledger, stale),
            });
            row.avatar = self.avatar_live.get(row.tab_id).cloned();
            row.job_line = self
                .job_live
                .board(row.tab_id)
                .and_then(|board| lane_line(board, now_ms, self.job_agent(row.tab_id)));
        }
    }

    pub(super) fn drive_checklist(&mut self, ctx: &egui::Context, now: f64) {
        let shown = self.settings.is_some();
        let repo = self.workspace().dir().to_path_buf();
        self.checklist.tick(self.paths.as_ref(), &repo, shown, now, ctx);
    }

    pub(super) fn edit_checklist(&mut self, ctx: &egui::Context) {
        let repo = self.workspace().dir().to_path_buf();
        let (Some(paths), Some(list)) = (&self.paths, self.checklist.of(&repo)) else { return };
        let file = match list.source {
            Source::Repo => Ok(list.file.clone()),
            Source::Default | Source::Yours => tidewatch_core::checklist::ensure(paths),
        };
        match file {
            Ok(file) => open_in_editor(file),
            Err(e) => self.show_toast(format!("Could not create the checklist: {e}"), ctx),
        }
    }

    /// The Answer button: the Reader in front, its reply box asking for the keyboard.
    pub(super) fn open_reply_box_of(&mut self, tab: u64) {
        if let Some(agent) = self.tabs.get_mut(tab).and_then(|found| found.pane.agent.as_mut()) {
            agent.reader.set_wants_reader(true);
            agent.reader.view.request_reply_focus();
        }
        if let Some(view) = self.views.get_mut(&tab) {
            view.back_to_terminal();
        }
    }

    /// A ready answer chip: the exact text goes to the agent as the reply box would send it, unless
    /// the reply box is shut. The ledger's question clears until the agent saves a new ledger.
    pub(super) fn send_answer(&mut self, tab: u64, text: String, ctx: &egui::Context, now: f64) {
        if self.send_ready_answer(tab, text, ctx, now) {
            self.mark_answered(tab);
        }
    }

    /// An answer picked on the Reader's Needs you card: the card moves on to the next question.
    pub(super) fn answer_ask(&mut self, tab: u64, text: String, ctx: &egui::Context, now: f64) {
        let Some(known) = self.progress.ledger(tab) else {
            self.send_answer(tab, text, ctx, now);
            return;
        };
        let asked = questions(&known.ledger);
        let (saved, total) = (known.updated_ms, asked.len());
        let step =
            ask_model::shown(total, false, self.ask_given.get(&tab).copied(), saved).unwrap_or(0);
        let question = asked.get(step).map_or("", |entry| entry.text.as_str());
        let text = ask_model::reply_text(question, total, &text);
        if !self.send_ready_answer(tab, text, ctx, now) {
            return;
        }
        match ask_model::after_answer(total, step, saved) {
            Some(given) => {
                self.ask_given.insert(tab, given);
            }
            None => self.mark_answered(tab),
        }
    }

    fn mark_answered(&mut self, tab: u64) {
        let saved = self.progress.ledger(tab).map(|known| known.updated_ms);
        if let Some(saved) = saved {
            self.answered_asks.insert(tab, saved);
        }
    }

    fn send_ready_answer(&mut self, tab: u64, text: String, ctx: &egui::Context, now: f64) -> bool {
        let shut = self
            .tabs
            .items()
            .iter()
            .find(|found| found.id == tab)
            .and_then(|found| found.pane.agent.as_ref())
            .and_then(reader_glue::answer_blocked);
        if let Some(reason) = shut {
            self.show_toast(reason, ctx);
            return false;
        }
        let sent = vec![reader_view::Action::Reply(text)];
        if let Some(toast) = reader_glue::apply(sent, &mut self.tabs, tab, &mut self.flow, ctx, now)
        {
            self.show_toast(toast, ctx);
        }
        ctx.request_repaint();
        true
    }

    /// The question the Reader's card shows for tab `tab`, if any.
    pub(super) fn ask_of(&self, tab: u64, now: f64, keys: bool) -> Option<Ask> {
        let known = self.progress.ledger(tab)?;
        let asked = questions(&known.ledger);
        let given = self.ask_given.get(&tab).copied();
        let step =
            ask_model::shown(asked.len(), self.ledger_stale(tab, now), given, known.updated_ms)?;
        let entry = (*asked.get(step)?).clone();
        let read = self.link_live.board(tab).map_or_else(Vec::new, |board| {
            link_model::read_first(board, &entry.text).into_iter().cloned().collect()
        });
        Some(Ask { entry, index: step, total: asked.len(), read, keys })
    }

    pub(super) fn progress_arrived(&mut self, tab: u64, ledger: Ledger, ctx: &egui::Context) {
        if let Some(kind) = ledger.status.kind {
            self.meeseeks_declared(tab, kind, ctx);
        }
        self.progress.arrived(tab, ledger, wall_ms() as i64);
        if self.status_log
            && let Some(known) = self.progress.ledger(tab)
        {
            let ledger = &known.ledger;
            let counts = ledger.counts();
            eprintln!(
                "tidewatch: progress tab={tab} state={} phase={:?} done={}/{} scheduled={} coming_up={} at {}",
                ledger.status.state.map_or("none", |state| state.word()),
                ledger.current_phase().map(|phase| phase.title.as_str()).unwrap_or_default(),
                counts.done,
                counts.total,
                ledger.scheduled.len(),
                coming_up(today(), self.progress.index()).len(),
                wall_ms()
            );
        }
        ctx.request_repaint();
    }
}
