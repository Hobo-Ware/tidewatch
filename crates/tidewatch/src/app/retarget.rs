//! Changing what a tab runs, the window's side: look inside the shell tabs for an agent session
//! (on a job, a few seconds apart, only while the window shows), build what the popover needs from
//! the tab, start a change on a job and swap the tab's pane when it lands. The tab keeps its id, its
//! place, its worktree and its folder.

use std::path::PathBuf;
use std::time::Duration;

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::agents::Level;
use tidewatch_core::elsewhere::Cli;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::status::AgentStatus;

use super::Tidewatch;
use crate::change_tab::{self, Card, Change, Facts, Pick, Running, TabKind};
use crate::change_tab_view::{self, Outcome};
use crate::new_tab_meeseeks::builder_entry;
use crate::pane::{Body, Pane};
use crate::pty::{spawn_agent, spawn_shell_noted, spawn_typed};
use crate::restore_glue::{Spawn, reopened};
use crate::retarget::Subject;
use crate::retarget_glue::{DevOpen, Open};
use crate::retarget_jobs::{self, Done, Request, Retargeted};
use crate::shell_agents::{EVERY_S, Probe, Remembered};
use crate::tabs::Tab;
use crate::takeover::END_WAIT_S;

const NOT_READY: &str = "Profiles are still loading.";

fn is_shell(pane: &Pane<PtyTerm>) -> bool {
    matches!(pane.body, Body::Shell(_)) && pane.agent.is_none()
}

fn live_pid(pane: &Pane<PtyTerm>) -> Option<u32> {
    let Body::Shell(term) = &pane.body else { return None };
    term.exited().is_none().then(|| term.shell_pid()).flatten()
}

fn foreground_busy(pane: &Pane<PtyTerm>) -> bool {
    let Body::Shell(term) = &pane.body else { return false };
    term.foreground_pgid().zip(term.shell_pid()).is_some_and(|(fg, sh)| fg != sh)
}

pub(crate) fn kind_of(tab: &Tab<Pane<PtyTerm>>) -> TabKind {
    change_tab::tab_kind(&tab.pane, tab.pinned)
}

impl Tidewatch {
    fn shell_probes(&self) -> Vec<Probe> {
        self.tabs
            .items()
            .iter()
            .filter(|tab| !tab.pinned && is_shell(&tab.pane))
            .filter_map(|tab| Some(Probe { tab: tab.id, shell_pid: live_pid(&tab.pane)? }))
            .collect()
    }

    pub(super) fn drive_retarget(&mut self, ctx: &egui::Context, now: f64) {
        for done in self.retarget.jobs.drain() {
            self.apply_retarget_done(done, now, ctx);
        }
        let probes = self.shell_probes();
        if self.retarget.agents.fake() != crate::shell_agents::Fake::Off {
            self.retarget.agents.note_fake(self.first_shell_tab());
        }
        let visible = !self.shell.window_hidden();
        let ready = self.paths.is_some() && self.catalog.is_loaded();
        if ready && self.retarget.agents.begin_scan(now, visible, probes.len()) {
            if let Some(profiles) = self.catalog.profiles().cloned() {
                retarget_jobs::detect(&self.retarget.jobs, ctx, profiles, probes);
            } else {
                self.retarget.agents.failed();
            }
        }
        if let Some(after) = self.retarget.agents.next_in(now, visible, self.shell_probes().len())
            && ready
        {
            ctx.request_repaint_after(Duration::from_secs_f64(after.max(EVERY_S / 4.0)));
        }
        self.retry_waiting(now, ctx);
        self.drive_dev_change_tab();
    }

    fn first_shell_tab(&self) -> Option<u64> {
        self.tabs.items().iter().find(|tab| !tab.pinned && is_shell(&tab.pane)).map(|tab| tab.id)
    }

    fn retry_waiting(&mut self, now: f64, ctx: &egui::Context) {
        let due: Vec<Change> = self
            .retarget
            .waiting
            .values()
            .filter(|(change, at)| *at <= now && !self.retarget.busy.contains(&change.tab))
            .map(|(change, _)| change.clone())
            .collect();
        for change in due {
            if self.tabs.index_of(change.tab).is_none() {
                self.retarget.waiting.remove(&change.tab);
            } else {
                self.start_retarget(change, now, ctx);
            }
        }
        if let Some(next) = self.retarget.waiting.values().map(|(_, at)| *at).reduce(f64::min) {
            ctx.request_repaint_after(Duration::from_secs_f64((next - now).max(0.25)));
        }
    }

    fn tab_dir(&self, tab: &Tab<Pane<PtyTerm>>) -> PathBuf {
        let launched = tab.pane.agent.as_ref().and_then(|agent| agent.launched.as_ref());
        match (&tab.pane.worktree, launched.and_then(|launched| launched.dir.clone())) {
            (Some(worktree), _) => worktree.path.clone(),
            (None, Some(dir)) => dir,
            (None, None) => self.workspaces[tab.group].dir().to_path_buf(),
        }
    }

    fn subject_of(&self, id: u64) -> Option<Subject> {
        let tab = self.tabs.items().iter().find(|tab| tab.id == id)?;
        let shell_pid = live_pid(&tab.pane).or_else(|| {
            let Body::Shell(term) = &tab.pane.body else { return None };
            term.shell_pid()
        })?;
        let agent = tab.pane.agent.as_ref();
        let launched = agent.and_then(|agent| agent.launched.as_ref());
        Some(Subject {
            tab: id,
            shell_pid,
            repo: self.workspaces[tab.group].dir().to_path_buf(),
            worktree: tab.pane.worktree.clone(),
            dir: self.tab_dir(tab),
            cli: agent.and_then(|agent| Cli::from_id(&agent.run.agent_id)),
            known_session_id: launched.and_then(|launched| launched.session_id.clone()),
            foreground_busy: agent.is_none() && foreground_busy(&tab.pane),
            remote: launched.is_some_and(|launched| launched.remote),
        })
    }

    fn cards(&self) -> Vec<Card> {
        self.catalog
            .entries()
            .into_iter()
            .map(|(id, profile, agent)| Card {
                id: id.to_owned(),
                name: profile.name.clone(),
                detail: detail_of(profile.model.as_deref(), profile.effective_effort()),
                agent_id: agent.id.clone(),
                cli_name: agent.name.clone(),
                unsupported: Level::ALL
                    .into_iter()
                    .filter(|level| agent.level_args(*level).is_none())
                    .collect(),
                below_yolo: agent.highest_level_below_yolo(),
                level: profile.level,
            })
            .collect()
    }

    pub(super) fn change_facts(&self, id: u64) -> Option<Facts> {
        let tab = self.tabs.items().iter().find(|tab| tab.id == id)?;
        change_tab::availability(kind_of(tab)).ok()?;
        let agent = tab.pane.agent.as_ref();
        let name_of = |cli: &str| {
            self.catalog.agent(cli).map_or_else(|| cli.to_owned(), |agent| agent.name.clone())
        };
        let found = self.retarget.agents.found(id);
        let alive = live_pid(&tab.pane).is_some();
        let running = match (agent, found) {
            (Some(agent), _) if alive => Running::Agent {
                agent_id: agent.run.agent_id.clone(),
                cli_name: name_of(&agent.run.agent_id),
                working: matches!(agent.run.status(), AgentStatus::Working),
            },
            (None, Some(found)) => Running::Agent {
                agent_id: found.cli.id().to_owned(),
                cli_name: name_of(found.cli.id()),
                working: found.working,
            },
            _ => Running::No,
        };
        let launched = agent.and_then(|agent| agent.launched.as_ref());
        let stored = match (agent, launched.and_then(|launched| launched.session_id.as_ref())) {
            (Some(agent), Some(_)) if !alive => {
                Some((agent.run.agent_id.clone(), name_of(&agent.run.agent_id)))
            }
            _ => self
                .retarget
                .agents
                .remembered(id)
                .map(|kept| (kept.cli.id().to_owned(), name_of(kept.cli.id()))),
        };
        let own_cli = agent
            .map(|agent| agent.run.agent_id.clone())
            .or_else(|| found.map(|found| found.cli.id().to_owned()))
            .or_else(|| stored.as_ref().map(|(cli, _)| cli.clone()));
        Some(Facts {
            tab: id,
            in_worktree: tab.pane.worktree.is_some(),
            running,
            stored,
            current: launched.map(|launched| (launched.profile_id.clone(), launched.level)),
            seen_level: found.and_then(|found| found.level),
            cards: change_tab::ordered_cards(&self.cards(), own_cli.as_deref()),
        })
    }

    pub(super) fn open_change_tab(&mut self, id: u64, ctx: &egui::Context) {
        if !self.catalog.is_loaded() {
            self.show_toast(NOT_READY, ctx);
            return;
        }
        let Some(facts) = self.change_facts(id) else { return };
        let entries = self.catalog.entries();
        let builder = |cli: &str| builder_entry(cli, &entries).map(|(key, _)| key);
        let pick = Pick::start(&facts, &builder);
        self.retarget.open = Some(Open { tab: id, pick });
        ctx.request_repaint();
    }

    pub(super) fn show_change_tab(&mut self, ctx: &egui::Context) {
        let Some(open) = self.retarget.open.take() else { return };
        let Some(facts) = self.change_facts(open.tab) else { return };
        let Open { tab, mut pick } = open;
        match change_tab_view::show(ctx, &facts, &mut pick) {
            Some(Outcome::Close) => {}
            Some(Outcome::Change { interrupt }) => {
                if let Some(change) = change_tab::change(&facts, &pick, interrupt) {
                    let now = ctx.input(|input| input.time);
                    if change_tab::is_working(&facts) && !interrupt {
                        self.show_toast("Switching when the turn ends", ctx);
                    }
                    self.start_retarget(change, now, ctx);
                }
            }
            None => self.retarget.open = Some(Open { tab, pick }),
        }
    }

    pub(super) fn can_change(&self) -> bool {
        let tab = &self.tabs.items()[self.tabs.active()];
        change_tab::availability(kind_of(tab)).is_ok()
    }

    pub(super) fn open_change_active(&mut self, ctx: &egui::Context) {
        let id = self.tabs.items()[self.tabs.active()].id;
        self.open_change_tab(id, ctx);
    }

    pub(super) fn open_change_index(&mut self, index: usize, ctx: &egui::Context) {
        if let Some(id) = self.tabs.items().get(index).map(|tab| tab.id) {
            self.open_change_tab(id, ctx);
        }
    }

    fn start_retarget(&mut self, change: Change, now: f64, ctx: &egui::Context) {
        let (Some(paths), Some(profiles)) = (self.paths.clone(), self.catalog.profiles().cloned())
        else {
            return;
        };
        let Some(subject) = self.subject_of(change.tab) else { return };
        if !self.retarget.busy.insert(change.tab) {
            return;
        }
        self.retarget.waiting.insert(change.tab, (change.clone(), now + EVERY_S));
        let installed = profiles.agent_table().found_on_path(Some(&self.catalog.path));
        let remembered = self
            .retarget
            .agents
            .remembered(change.tab)
            .map(|kept| (kept.cli, kept.session_id.clone()));
        let request = Request {
            subject,
            target: change.target,
            remembered,
            interrupt: change.interrupt,
            profiles,
            installed,
            paths,
            path_var: self.catalog.path.clone(),
            listener: self.hooks.listener(),
            trust: self.trust.grant(),
            wait: Duration::from_secs(END_WAIT_S),
        };
        retarget_jobs::retarget(&self.retarget.jobs, ctx, request);
    }

    fn apply_retarget_done(&mut self, done: Done, now: f64, ctx: &egui::Context) {
        match done {
            Done::Detected(found) => self.retarget.agents.finish(found),
            Done::DetectFailed => self.retarget.agents.failed(),
            Done::Refused { tab, refusal } => {
                self.retarget.busy.remove(&tab);
                let waits = refusal == crate::retarget::Refusal::MidTurn
                    && self.retarget.waiting.contains_key(&tab);
                if waits {
                    if let Some((_, at)) = self.retarget.waiting.get_mut(&tab) {
                        *at = now + EVERY_S;
                    }
                    ctx.request_repaint_after(Duration::from_secs_f64(EVERY_S));
                } else {
                    self.retarget.waiting.remove(&tab);
                    self.show_toast(format!("The tab was not changed: {}.", refusal.text()), ctx);
                }
            }
            Done::Retargeted(landed) => match *landed {
                Retargeted::Agent(done) => {
                    self.retarget.busy.remove(&done.tab);
                    self.retarget.waiting.remove(&done.tab);
                    self.retarget.agents.forget(done.tab);
                    self.swap_to_agent(done, now, ctx);
                }
                Retargeted::Shell { tab, cli, session_id } => {
                    self.retarget.busy.remove(&tab);
                    self.retarget.waiting.remove(&tab);
                    self.swap_to_shell(tab, Remembered { cli, session_id }, ctx);
                }
            },
        }
    }

    fn swap_to_agent(
        &mut self,
        done: crate::restore_jobs::Reopened,
        now: f64,
        ctx: &egui::Context,
    ) {
        let id = done.tab;
        let Some(tab) = self.tabs.get_mut(id) else { return };
        if let Body::Shell(old) = std::mem::replace(&mut tab.pane.body, Body::Creating) {
            std::thread::spawn(move || drop(old));
        }
        self.views.remove(&id);
        let agent = |dir: &std::path::Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let typed = |dir: &std::path::Path, prepared: &Prepared| spawn_typed(ctx, dir, prepared);
        let shell = |dir: &std::path::Path, note: Option<&str>| spawn_shell_noted(ctx, dir, note);
        let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
        let mut toasts = Vec::new();
        reopened(done, &mut self.tabs, &spawn, now, &mut toasts);
        for text in toasts {
            self.show_toast(text, ctx);
        }
        ctx.request_repaint();
    }

    fn swap_to_shell(&mut self, id: u64, kept: Remembered, ctx: &egui::Context) {
        let name = self
            .catalog
            .agent(kept.cli.id())
            .map_or_else(|| kept.cli.id().to_owned(), |a| a.name.clone());
        let Some(index) = self.tabs.index_of(id) else { return };
        let dir = self.tab_dir(&self.tabs.items()[index]);
        let note = format!("{name} stopped. Change this tab to resume the conversation.");
        let tab = &mut self.tabs.items_mut()[index];
        if tab.pane.agent.take().is_some() {
            let fresh = Body::Shell(spawn_shell_noted(ctx, &dir, Some(&note)));
            if let Body::Shell(old) = std::mem::replace(&mut tab.pane.body, fresh) {
                std::thread::spawn(move || drop(old));
            }
            "shell".clone_into(&mut tab.title);
            if let Some(listener) = self.hooks.listener() {
                listener.unregister_tab(id);
            }
            self.views.remove(&id);
        }
        self.retarget.agents.remember(id, kept);
        self.show_toast(
            format!("{name} stopped. Change this tab to resume the conversation."),
            ctx,
        );
    }

    fn drive_dev_change_tab(&mut self) {
        let Some(dev) = self.retarget.dev_open else { return };
        let found = self.tabs.items().iter().find(|tab| match dev {
            DevOpen::Agent => !tab.pinned && kind_of(tab) == TabKind::Agent,
            DevOpen::Shell => !tab.pinned && kind_of(tab) == TabKind::Shell,
        });
        let Some(id) = found.map(|tab| tab.id) else { return };
        if self.change_facts(id).is_none() || !self.catalog.is_loaded() {
            return;
        }
        if dev == DevOpen::Shell
            && self.retarget.agents.fake() != crate::shell_agents::Fake::Off
            && self.retarget.agents.found(id).is_none()
        {
            return;
        }
        self.retarget.dev_open = None;
        self.tabs.select_id(id);
        let entries = self.catalog.entries();
        let builder = |cli: &str| builder_entry(cli, &entries).map(|(key, _)| key);
        if let Some(facts) = self.change_facts(id) {
            self.retarget.open = Some(Open { tab: id, pick: Pick::start(&facts, &builder) });
        }
    }
}

fn detail_of(model: Option<&str>, effort: Option<&str>) -> String {
    match (model.filter(|m| !m.is_empty()), effort) {
        (Some(model), Some(effort)) => format!("{model} \u{b7} {effort}"),
        (Some(model), None) => model.to_owned(),
        (None, Some(effort)) => format!("CLI default \u{b7} {effort}"),
        (None, None) => "CLI default".to_owned(),
    }
}
