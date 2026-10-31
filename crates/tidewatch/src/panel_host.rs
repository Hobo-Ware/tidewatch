//! The app's side of the panel store: one numbered log per agent tab, fed from every place the app
//! learns something about the tab (hooks, the transcript, the terminal, the process list, reports,
//! jobs, links, checks), written to `panel-log.jsonl` on a background thread and folded in memory.
//! Nothing here draws. The old path still decides what the screens show, with one exception: when
//! the transcript or the process list says a turn began or ended and the hook that should have said
//! so never came, the tab's status follows (`AgentRun::begin_turn_elsewhere`, `settle_ended_turn`). `TIDEWATCH_DEV_STATUS_LOG=1` prints every append
//! and each moment the live tone and the store's tone disagree for more than two seconds.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::agents::reports_by_hooks;
use tidewatch_core::elsewhere_scan::RealSystem;
use tidewatch_core::end_block::EndBlock;
use tidewatch_core::events::{TabEvent, TabMessage};
use tidewatch_core::panel_event::{Event, Fact, PrFacts, Source};
use tidewatch_core::panel_log::{CAP_EVENTS, Log};
use tidewatch_core::panel_state::PanelState;
use tidewatch_core::panel_store::{self, Writer};
use tidewatch_core::panel_view::{PanelView, Tone as StoreTone};
use tidewatch_core::paths::panel_log_file_beside;
use tidewatch_core::status::{AgentStatus, HookEvent, HookKind, ProgressSignal, TerminalSignal};

use crate::agent_process::{self, Hosting, Presence, Schedule, Step, Target};
use crate::agent_run::{Life, TermSnapshot, progress_signal};
use crate::jobs::Jobs;
use crate::pane::Pane;
use crate::status_view::Tone;
use crate::tabs::Tabs;

const OUTPUT_EVERY_MS: i64 = 1_000;
const DISAGREE_MS: i64 = 2_000;

/// The share of hooks `TIDEWATCH_DEV_DROP_HOOKS` throws away at the listener drain, to rehearse
/// the loss the backups exist for.
#[derive(Debug)]
pub(crate) struct Drops {
    percent: u8,
    state: u64,
}

impl Drops {
    pub(crate) fn from_text(text: Option<&str>, seed: u64) -> Self {
        let percent =
            text.and_then(|text| text.trim().parse::<u8>().ok()).map_or(0, |p| p.min(100));
        Self { percent, state: seed | 1 }
    }

    pub(crate) fn from_env() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |since| since.as_nanos() as u64);
        Self::from_text(std::env::var("TIDEWATCH_DEV_DROP_HOOKS").ok().as_deref(), seed)
    }

    pub(crate) fn should_drop(&mut self) -> bool {
        match self.percent {
            0 => false,
            100.. => true,
            percent => {
                self.state ^= self.state << 13;
                self.state ^= self.state >> 7;
                self.state ^= self.state << 17;
                self.state % 100 < u64::from(percent)
            }
        }
    }
}

/// Whether a transcript's end mark may close the turn the hooks opened: a mark the CLI stamped
/// before that turn began belongs to an earlier turn.
pub(crate) fn ends_after(mark_ms: Option<i64>, hook_opened_ms: Option<i64>) -> bool {
    match (mark_ms, hook_opened_ms) {
        (Some(mark), Some(opened)) => mark >= opened,
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Agree,
    Wait(i64),
    Tell { for_ms: i64 },
    Told,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shadow {
    live: Tone,
    store: Tone,
    since_ms: i64,
    told: bool,
}

impl Shadow {
    pub(crate) fn observe(
        slot: &mut Option<Self>,
        live: Tone,
        store: Tone,
        now_ms: i64,
    ) -> Verdict {
        if live == store {
            *slot = None;
            return Verdict::Agree;
        }
        let shadow = match slot {
            Some(known) if known.live == live && known.store == store => known,
            _ => slot.insert(Self { live, store, since_ms: now_ms, told: false }),
        };
        let for_ms = now_ms - shadow.since_ms;
        if shadow.told {
            Verdict::Told
        } else if for_ms > DISAGREE_MS {
            shadow.told = true;
            Verdict::Tell { for_ms }
        } else {
            Verdict::Wait(shadow.since_ms + DISAGREE_MS + 1)
        }
    }
}

pub(crate) fn tone_of(store: StoreTone) -> Tone {
    match store {
        StoreTone::Setup => Tone::Setup,
        StoreTone::Working => Tone::Working,
        StoreTone::Needs => Tone::Needs,
        StoreTone::Waiting => Tone::Waiting,
        StoreTone::Blocked => Tone::Blocked,
        StoreTone::Done => Tone::Done,
        StoreTone::Failed => Tone::Failed,
        StoreTone::Stopped => Tone::Stopped,
        StoreTone::Shell => Tone::Shell,
    }
}

fn working(status: &AgentStatus) -> bool {
    matches!(status, AgentStatus::Working | AgentStatus::NeedsYou(_))
}

fn bare(kind: HookKind) -> HookEvent {
    HookEvent { kind, session_id: None, transcript_path: None, message: None }
}

struct Pending {
    source: Source,
    fact: Fact,
    at_ms: i64,
}

struct Ready {
    log: Log,
    state: PanelState,
    writer: Option<Writer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pushed {
    pub(crate) running_before: bool,
    pub(crate) running_after: bool,
}

impl Ready {
    fn new(log: Log, writer: Option<Writer>) -> Self {
        let state = log.replay();
        Self { log, state, writer }
    }

    fn push(&mut self, source: Source, fact: Fact, at_ms: i64) -> (Event, Pushed) {
        let running_before = self.state.turn().running;
        let event = self.log.append(source, fact, at_ms);
        self.state.apply(&event);
        if let Some(writer) = &self.writer {
            writer.append(event.clone());
        }
        if self.log.len() >= CAP_EVENTS {
            panel_store::compact(&mut self.log);
        }
        (event, Pushed { running_before, running_after: self.state.turn().running })
    }
}

#[derive(Default)]
struct Terminal {
    progress: ProgressSignal,
    title: Option<String>,
    title_changed: bool,
    printed: bool,
    emitted_ms: i64,
    exited: bool,
}

struct TabPanel {
    file: PathBuf,
    ready: Option<Ready>,
    pending: Vec<Pending>,
    terminal: Terminal,
    asking: bool,
    presence: Option<Presence>,
    hook_opened_ms: Option<i64>,
    started: bool,
    shadow: Option<Shadow>,
    pr: Option<PrFacts>,
}

struct Loaded {
    tab: u64,
    log: Log,
}

pub(crate) struct Host {
    tabs: HashMap<u64, TabPanel>,
    loads: Jobs<Loaded>,
    probes: Jobs<Option<Vec<(Target, Presence)>>>,
    schedule: Schedule,
    gone: Vec<u64>,
    writes: bool,
    echo: bool,
    drops: Drops,
}

impl Host {
    pub(crate) fn new(writes: bool, echo: bool, drops: Drops) -> Self {
        Self {
            tabs: HashMap::new(),
            loads: Jobs::new(),
            probes: Jobs::new(),
            schedule: Schedule::default(),
            gone: Vec::new(),
            writes,
            echo,
            drops,
        }
    }

    pub(crate) fn from_env(writes: bool, echo: bool) -> Self {
        Self::new(writes, echo, Drops::from_env())
    }

    pub(crate) fn drops_hook(&mut self) -> bool {
        self.drops.should_drop()
    }

    pub(crate) fn view(&self, tab: u64, now_ms: i64) -> Option<PanelView> {
        let ready = self.tabs.get(&tab)?.ready.as_ref()?;
        Some(ready.state.view(now_ms))
    }

    pub(crate) fn sync<T>(&mut self, tabs: &Tabs<Pane<T>>, ctx: &egui::Context) {
        let live: Vec<(u64, PathBuf)> = tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let notes = tab.pane.agent.as_ref()?.run.notes_file.as_deref()?;
                Some((tab.id, panel_log_file_beside(notes)))
            })
            .collect();
        self.tabs.retain(|id, _| live.iter().any(|(tab, _)| tab == id));
        for (id, file) in live {
            if self.tabs.contains_key(&id) {
                continue;
            }
            let mut panel = TabPanel {
                file: file.clone(),
                ready: None,
                pending: Vec::new(),
                terminal: Terminal::default(),
                asking: false,
                presence: None,
                hook_opened_ms: None,
                started: false,
                shadow: None,
                pr: None,
            };
            if self.writes {
                self.loads.spawn(ctx, move || Loaded { tab: id, log: panel_store::load(&file) });
            } else {
                panel.ready = Some(Ready::new(Log::new(), None));
            }
            self.tabs.insert(id, panel);
        }
        self.drain_loaded();
    }

    fn drain_loaded(&mut self) {
        for loaded in self.loads.drain() {
            let writes = self.writes;
            let Some(panel) = self.tabs.get_mut(&loaded.tab) else { continue };
            let writer = writes.then(|| Writer::new(panel.file.clone()));
            panel.started = !loaded.log.is_empty();
            panel.ready = Some(Ready::new(loaded.log, writer));
            for pending in std::mem::take(&mut panel.pending) {
                self.push(loaded.tab, pending.source, pending.fact, pending.at_ms);
            }
        }
    }

    /// Folds one fact into the tab's log. `None` while the log is still being read from disk (the
    /// fact waits and is folded when it arrives) or for a tab with no panel.
    pub(crate) fn push(
        &mut self,
        tab: u64,
        source: Source,
        fact: Fact,
        at_ms: i64,
    ) -> Option<Pushed> {
        let echo = self.echo;
        let panel = self.tabs.get_mut(&tab)?;
        let Some(ready) = panel.ready.as_mut() else {
            panel.pending.push(Pending { source, fact, at_ms });
            return None;
        };
        let opens_by_hook = source == Source::Hook;
        let started = matches!(fact, Fact::Started { .. });
        let (event, pushed) = ready.push(source, fact, at_ms);
        if echo {
            eprintln!(
                "tidewatch: panel tab={tab} seq={} src={} fact={} at {at_ms}",
                event.seq,
                event.source.word(),
                event.fact.name()
            );
        }
        if started {
            panel.started = true;
        }
        if opens_by_hook && !pushed.running_before && pushed.running_after {
            panel.hook_opened_ms = Some(at_ms);
        } else if pushed.running_before && !pushed.running_after {
            panel.hook_opened_ms = None;
        }
        Some(pushed)
    }

    pub(crate) fn heard(&mut self, message: &TabMessage, now_ms: i64) {
        match &message.event {
            TabEvent::Hook(hook) => {
                self.push(message.tab_id, Source::Hook, Fact::Hook(hook.clone()), now_ms);
            }
            TabEvent::Ask { questions, .. } => {
                let fact = Fact::AskOpened {
                    summary: questions.first().map(|q| q.text.clone()).unwrap_or_default(),
                    n: u32::try_from(questions.len()).unwrap_or(u32::MAX),
                };
                self.push(message.tab_id, Source::Tidewatch, fact, now_ms);
            }
            _ => {}
        }
    }

    pub(crate) fn pr(&mut self, tab: u64, facts: PrFacts, now_ms: i64) {
        let Some(panel) = self.tabs.get_mut(&tab) else { return };
        if panel.pr.as_ref() == Some(&facts) {
            return;
        }
        panel.pr = Some(facts.clone());
        self.push(tab, Source::Pr, Fact::Pr(facts), now_ms);
    }

    pub(crate) fn collect<T>(&mut self, tabs: &mut Tabs<Pane<T>>, now: f64, now_ms: i64) {
        self.drain_probed(now_ms);
        for tab in tabs.items_mut() {
            let id = tab.id;
            let Some(agent) = tab.pane.agent.as_mut() else { continue };
            if !self.tabs.contains_key(&id) {
                continue;
            }
            let hooked = reports_by_hooks(&agent.run.agent_id);
            for life in agent.run.take_life() {
                if let Some(panel) = self.tabs.get_mut(&id)
                    && life == Life::Started
                {
                    panel.terminal = Terminal::default();
                    panel.asking = false;
                    panel.presence = None;
                    panel.hook_opened_ms = None;
                }
                let (source, fact) = match life {
                    Life::Started => (Source::App, Fact::Started { hooked }),
                    Life::Resumed => (Source::App, Fact::Hook(bare(HookKind::IdleWaiting))),
                    Life::KeptRunning => (Source::App, Fact::Restart),
                };
                self.push(id, source, fact, now_ms);
            }
            self.question_ended(id, agent.run.is_asking(), now_ms);
            agent.reader.set_working(working(agent.run.status()));
            let gone = self.gone.contains(&id);
            let mut ended = false;
            for timed in agent.reader.take_panel_facts() {
                let mark = match timed.fact {
                    Fact::Turn { started, ts_ms } => Some((started, ts_ms)),
                    _ => None,
                };
                let opened = self.tabs.get(&id).and_then(|panel| panel.hook_opened_ms);
                let wrote_ms = timed.ts_ms.map_or(now_ms, |ts| ts.min(now_ms));
                let pushed = self.push(id, Source::Transcript, timed.fact, wrote_ms);
                let (Some((started, ts)), Some(pushed)) = (mark, pushed) else { continue };
                if started && pushed.running_after {
                    ended = false;
                    if agent.run.begin_turn_elsewhere(now) && self.echo {
                        eprintln!("tidewatch: panel begin tab={id} at {now_ms}");
                    }
                } else if !started && !pushed.running_after && ends_after(ts, opened) {
                    ended = true;
                }
            }
            if gone || ended {
                let block = self.end_block(id, now_ms);
                if agent.run.settle_ended_turn(block, now) && self.echo {
                    eprintln!("tidewatch: panel settle tab={id} at {now_ms}");
                }
            }
        }
        self.gone.clear();
    }

    fn end_block(&self, tab: u64, now_ms: i64) -> Option<EndBlock> {
        self.view(tab, now_ms)?.end_block
    }

    fn question_ended(&mut self, tab: u64, asking: bool, now_ms: i64) {
        let Some(panel) = self.tabs.get_mut(&tab) else { return };
        let was = std::mem::replace(&mut panel.asking, asking);
        if was && !asking {
            self.push(tab, Source::Tidewatch, Fact::AskClosed, now_ms);
        }
    }

    /// Output and title changes are logged at most once a second; the return value is when the last
    /// of them is due, so a quiet window sleeps.
    pub(crate) fn terminal(&mut self, seen: &[(u64, TermSnapshot)], now_ms: i64) -> Option<i64> {
        let mut due: Option<i64> = None;
        for (id, snapshot) in seen {
            let Some(panel) = self.tabs.get_mut(id) else { continue };
            let mut facts: Vec<(Source, Fact)> = Vec::new();
            let memory = &mut panel.terminal;
            let signal = progress_signal(snapshot.progress);
            if signal != memory.progress {
                memory.progress = signal;
                facts.push((Source::Terminal, Fact::Terminal(TerminalSignal::Progress(signal))));
            }
            if snapshot.bell {
                facts.push((Source::Terminal, Fact::Terminal(TerminalSignal::Bell)));
            }
            if snapshot.title != memory.title {
                memory.title.clone_from(&snapshot.title);
                memory.title_changed = true;
            }
            memory.printed |= snapshot.activity;
            if memory.printed || memory.title_changed {
                let at = memory.emitted_ms + OUTPUT_EVERY_MS;
                if now_ms >= at {
                    if memory.title_changed {
                        facts
                            .push((Source::Terminal, Fact::Terminal(TerminalSignal::TitleChanged)));
                    }
                    if memory.printed {
                        facts.push((Source::Terminal, Fact::Output));
                    }
                    memory.title_changed = false;
                    memory.printed = false;
                    memory.emitted_ms = now_ms;
                } else {
                    due = Some(due.map_or(at, |soonest| soonest.min(at)));
                }
            }
            if let Some(code) = snapshot.exit_code
                && !memory.exited
            {
                memory.exited = true;
                facts
                    .push((Source::Process, Fact::Process { alive: false, exit_code: Some(code) }));
            }
            for (source, fact) in facts {
                self.push(*id, source, fact, now_ms);
            }
        }
        due
    }

    /// Returns the milliseconds until the next ask is due, when one is waiting.
    pub(crate) fn probe(
        &mut self,
        targets: Vec<Target>,
        wanted: bool,
        ctx: &egui::Context,
        now_ms: i64,
    ) -> Option<i64> {
        match self.schedule.step(now_ms, wanted) {
            Step::Rest => None,
            Step::Wait(ms) => Some(ms),
            Step::Probe => {
                self.probes.spawn(ctx, move || agent_process::probe(&RealSystem::new(), &targets));
                None
            }
        }
    }

    fn drain_probed(&mut self, now_ms: i64) {
        for answer in self.probes.drain() {
            self.schedule.finished();
            for (target, presence) in answer.into_iter().flatten() {
                self.presence(target.tab, presence, target.hosting, now_ms);
            }
        }
    }

    /// An agent started from a shell leaves the shell alive, so its going is the end of a turn, not
    /// an exit.
    fn presence(&mut self, tab: u64, presence: Presence, hosting: Hosting, now_ms: i64) {
        let Some(panel) = self.tabs.get_mut(&tab) else { return };
        if panel.presence == Some(presence) {
            return;
        }
        panel.presence = Some(presence);
        let fact = match (presence, hosting) {
            (Presence::Alive, _) => Fact::Process { alive: true, exit_code: None },
            (Presence::Gone, Hosting::Launcher) => Fact::Process { alive: false, exit_code: None },
            (Presence::Gone, Hosting::Shell) => Fact::Turn { started: false, ts_ms: None },
        };
        self.push(tab, Source::Process, fact, now_ms);
        if presence == Presence::Gone {
            self.gone.push(tab);
        }
    }

    pub(crate) fn shadow<T>(&mut self, tabs: &Tabs<Pane<T>>, now_ms: i64) -> Option<i64> {
        if !self.echo {
            return None;
        }
        let mut again: Option<i64> = None;
        for tab in tabs.items() {
            let Some(agent) = tab.pane.agent.as_ref() else { continue };
            let Some(panel) = self.tabs.get_mut(&tab.id) else { continue };
            let Some(ready) = &panel.ready else { continue };
            let live = agent.run.shown_tone();
            if !panel.started || matches!(live, Tone::Setup | Tone::Shell) {
                panel.shadow = None;
                continue;
            }
            let store = tone_of(ready.state.view(now_ms).tone);
            match Shadow::observe(&mut panel.shadow, live, store, now_ms) {
                Verdict::Wait(at) => again = Some(again.map_or(at, |soonest| soonest.min(at))),
                Verdict::Tell { for_ms } => eprintln!(
                    "tidewatch: panel shadow tab={} live={live:?} store={store:?} for {for_ms}ms at {now_ms}",
                    tab.id
                ),
                Verdict::Agree | Verdict::Told => {}
            }
        }
        again
    }

    pub(crate) fn targets(
        tabs: &Tabs<Pane<stdusk_core::terminal::PtyTerm>>,
    ) -> (Vec<Target>, bool) {
        let mut targets = Vec::new();
        let mut open = false;
        for tab in tabs.items() {
            let Some(agent) = tab.pane.agent.as_ref() else { continue };
            let crate::pane::Body::Shell(term) = &tab.pane.body else { continue };
            let Some(root) = term.shell_pid() else { continue };
            open |= working(agent.run.status());
            let hosting =
                if agent.run.is_shell_hosted() { Hosting::Shell } else { Hosting::Launcher };
            targets.push(Target { tab: tab.id, root, hosting });
        }
        (targets, open)
    }
}

#[cfg(test)]
mod tests;
