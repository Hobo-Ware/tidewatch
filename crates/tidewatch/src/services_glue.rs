//! `Host`: the repo services of every repo the window holds, each a pty in the repo's main
//! checkout and never a tab. `service_supervisor.rs` decides; this file spawns, kills and reads
//! exits. `TIDEWATCH_FAKE_SERVICES=1` stands in three services and spawns and writes nothing.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use stdusk_core::handoff::{PaneMeta, capture_pane};
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::service_log::Log;
use tidewatch_core::service_port;
use tidewatch_core::service_supervisor::{Carry, Effect, Event, Phase, Supervisor, Word};
use tidewatch_core::services::{Service, Store, same_name};
use tidewatch_core::services_mcp::{Fact, Listed};

use crate::handoff::{ResumedService, ServiceEntry};

pub(crate) type Spawner = Box<dyn Fn(&Path, &str) -> PtyTerm>;

pub(crate) const FAKE_VAR: &str = "TIDEWATCH_FAKE_SERVICES";
const PORT_SCAN_NEW_SECS: f64 = 0.3;
const PORT_SCAN_KNOWN_SECS: f64 = 2.0;
const PORT_SCAN_LINES: usize = 300;
const FAKE_UP_MAIN: f64 = 8040.0;
const FAKE_UP_API: f64 = 720.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Start,
    Stop,
    Restart,
}

/// What the strip's pill and the list read about one service.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) autostart: bool,
    pub(crate) word: Word,
    pub(crate) line: String,
}

struct Run {
    service: Service,
    sup: Supervisor,
    term: Option<PtyTerm>,
    exit_seen: bool,
    fake_log: Option<Vec<String>>,
    port: Option<u16>,
    scanned_at: f64,
}

impl Run {
    fn new(service: Service) -> Self {
        Self {
            service,
            sup: Supervisor::new(),
            term: None,
            exit_seen: false,
            fake_log: None,
            port: None,
            scanned_at: f64::NEG_INFINITY,
        }
    }

    fn port(&self) -> Option<u16> {
        self.sup.wants_process().then(|| self.port.or(self.service.port)).flatten()
    }

    fn step(&mut self, event: Event, repo: &Path, now: f64, spawn: &Spawner) {
        let mut pending: VecDeque<Effect> = self.sup.step(event).into();
        while let Some(effect) = pending.pop_front() {
            match effect {
                Effect::Kill => {
                    if let Some(term) = self.term.as_mut() {
                        term.kill();
                    }
                }
                Effect::Spawn => {
                    self.exit_seen = false;
                    self.port = None;
                    self.scanned_at = f64::NEG_INFINITY;
                    self.term = Some(spawn(repo, &self.service.command));
                    pending.extend(self.sup.step(Event::Spawned { at: now }));
                }
                Effect::Wake { .. } => {}
            }
        }
    }

    fn drive(&mut self, repo: &Path, now: f64, spawn: &Spawner) -> bool {
        let before = (self.sup.state(), self.sup.last_exit(), self.port);
        if !self.exit_seen
            && let Some(exit) = self.term.as_ref().and_then(PtyTerm::exited)
        {
            self.exit_seen = true;
            self.step(Event::Exited { code: exit.code, at: now }, repo, now, spawn);
        }
        if self.sup.next_wake().is_some_and(|at| now >= at) {
            self.step(Event::Tick { at: now }, repo, now, spawn);
        }
        self.scan_port(now);
        before != (self.sup.state(), self.sup.last_exit(), self.port)
    }

    fn scan_port(&mut self, now: f64) {
        let every = if self.port.is_some() { PORT_SCAN_KNOWN_SECS } else { PORT_SCAN_NEW_SECS };
        if !self.live() || now - self.scanned_at < every {
            return;
        }
        self.scanned_at = now;
        let tail = Log::from_lines(self.lines()).tail(PORT_SCAN_LINES);
        if let Some(found) = service_port::newest(&tail) {
            self.port = Some(found);
        }
    }

    fn row(&self, now: f64) -> Row {
        Row {
            name: self.service.name.clone(),
            command: self.service.command.clone(),
            autostart: self.service.autostart,
            word: self.sup.word(),
            line: self.sup.line(now, self.port()),
        }
    }

    fn lines(&self) -> Vec<String> {
        if let Some(fake) = &self.fake_log {
            return fake.clone();
        }
        self.term
            .as_ref()
            .map(|term| term.buffer_lines().into_iter().map(|(_, line)| line).collect())
            .unwrap_or_default()
    }

    fn live(&self) -> bool {
        self.term.is_some() && self.sup.wants_process()
    }
}

pub(crate) struct Host {
    repos: BTreeMap<PathBuf, Vec<Run>>,
    adopted: Vec<ResumedService<PtyTerm>>,
    paths: Option<TidewatchPaths>,
    spawn: Spawner,
    epoch: Instant,
    autostart: bool,
    fake: bool,
    version: u64,
    told: (u64, u64),
}

impl Host {
    pub(crate) fn new(
        paths: Option<TidewatchPaths>,
        spawn: Spawner,
        autostart: bool,
        fake: bool,
    ) -> Self {
        let mut host = Self {
            repos: BTreeMap::new(),
            adopted: Vec::new(),
            paths,
            spawn,
            epoch: Instant::now(),
            autostart,
            fake,
            version: 1,
            told: (0, 0),
        };
        if fake {
            host.repos.insert(PathBuf::new(), fake_runs(host.now()));
        }
        host
    }

    pub(crate) fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn key<'a>(&self, repo: &'a Path) -> &'a Path {
        if self.fake { Path::new("") } else { repo }
    }

    fn runs(&self, repo: &Path) -> &[Run] {
        self.repos.get(self.key(repo)).map_or(&[], Vec::as_slice)
    }

    fn find(&mut self, repo: &Path, name: &str) -> Option<&mut Run> {
        let key = self.key(repo).to_path_buf();
        self.repos.get_mut(&key)?.iter_mut().find(|run| same_name(&run.service.name, name))
    }

    /// Opens the repos the window holds that are not open yet (their saved services, the
    /// autostart ones started, the ones a replaced window kept running taken as they are) and
    /// stops the services of a repo that is no longer held.
    pub(crate) fn sync(&mut self, repos: &[&Path]) {
        if self.fake {
            return;
        }
        let before = self.repos.len();
        self.repos.retain(|repo, _| repos.contains(&repo.as_path()));
        let mut changed = self.repos.len() != before;
        for repo in repos {
            if !self.repos.contains_key(*repo) {
                self.open(repo);
                changed = true;
            }
        }
        self.adopted.clear();
        if changed {
            self.version += 1;
        }
    }

    fn open(&mut self, repo: &Path) {
        let saved = self.paths.as_ref().map(|paths| Store::load(paths).services(repo));
        let now = self.now();
        let mut runs = Vec::new();
        for service in saved.unwrap_or_default() {
            let kept = self
                .adopted
                .iter()
                .position(|a| a.entry.repo == repo && same_name(&a.entry.name, &service.name));
            let mut run = Run::new(service);
            if let Some(at) = kept {
                let resumed = self.adopted.swap_remove(at);
                run.sup = Supervisor::from_carry(&resumed.entry.carry, now);
                run.term = resumed.term;
                if run.sup.wants_process() && run.term.is_none() {
                    run.sup = Supervisor::new();
                    run.step(Event::Start, repo, now, &self.spawn);
                }
            } else if run.service.autostart && self.autostart {
                run.step(Event::Start, repo, now, &self.spawn);
            }
            runs.push(run);
        }
        self.repos.insert(repo.to_path_buf(), runs);
    }

    /// Reads exits and backoff timers once a frame. The answer is when it needs another frame
    /// with nothing else happening, in seconds from now.
    pub(crate) fn drive(&mut self) -> Option<f64> {
        if self.fake {
            return None;
        }
        let now = self.now();
        let mut changed = false;
        let mut wake: Option<f64> = None;
        for (repo, runs) in &mut self.repos {
            for run in runs {
                changed |= run.drive(repo, now, &self.spawn);
                wake = wake.into_iter().chain(run.sup.next_wake()).reduce(f64::min);
            }
        }
        if changed {
            self.version += 1;
        }
        wake.map(|at| (at - now).max(0.0))
    }

    /// The line each agent tab's next prompt carries, only when a state or the tabs changed since
    /// the last call.
    pub(crate) fn lines_for<'a>(
        &mut self,
        tabs: impl Iterator<Item = (u64, &'a Path)> + Clone,
    ) -> Option<HashMap<u64, String>> {
        let ids = tabs.clone().fold(0u64, |sum, (id, _)| sum.wrapping_mul(31).wrapping_add(id + 1));
        if self.told == (self.version, ids) {
            return None;
        }
        self.told = (self.version, ids);
        Some(
            tabs.filter_map(|(id, repo)| {
                tidewatch_core::services_mcp::context_line(&self.facts(repo)).map(|line| (id, line))
            })
            .collect(),
        )
    }

    pub(crate) fn rows(&self, repo: &Path) -> Vec<Row> {
        let now = self.now();
        self.runs(repo).iter().map(|run| run.row(now)).collect()
    }

    pub(crate) fn facts(&self, repo: &Path) -> Vec<Fact> {
        self.runs(repo)
            .iter()
            .map(|run| Fact {
                name: run.service.name.clone(),
                port: run.port(),
                word: run.sup.word(),
            })
            .collect()
    }

    pub(crate) fn listed(&self, repo: &Path) -> Vec<Listed> {
        let now = self.now();
        self.runs(repo)
            .iter()
            .map(|run| {
                Listed::new(
                    &run.service,
                    run.port(),
                    run.sup.word(),
                    run.sup.uptime(now),
                    run.sup.last_exit(),
                )
            })
            .collect()
    }

    fn unknown(&self, repo: &Path, name: &str) -> String {
        let known: Vec<&str> =
            self.runs(repo).iter().map(|run| run.service.name.as_str()).collect();
        if known.is_empty() {
            format!(
                "There is no service called `{name}`: this repo has none. Use add_service to save one."
            )
        } else {
            format!(
                "There is no service called `{name}`. This repo has: {}.",
                known.iter().map(|known| format!("`{known}`")).collect::<Vec<_>>().join(", ")
            )
        }
    }

    pub(crate) fn command(
        &mut self,
        repo: &Path,
        name: &str,
        command: Command,
    ) -> Result<String, String> {
        let now = self.now();
        let key = self.key(repo).to_path_buf();
        if self.fake {
            return Err("This is a demo run: services are stand-ins and nothing starts.".into());
        }
        let Some(run) = self
            .repos
            .get_mut(&key)
            .and_then(|runs| runs.iter_mut().find(|run| same_name(&run.service.name, name)))
        else {
            return Err(self.unknown(repo, name));
        };
        let shown = run.service.name.clone();
        let was_running = run.sup.wants_process();
        let event = match command {
            Command::Start => Event::Start,
            Command::Stop => Event::Stop,
            Command::Restart => Event::Restart,
        };
        run.step(event, &key, now, &self.spawn);
        self.version += 1;
        Ok(match command {
            Command::Start if was_running => format!("`{shown}` was already running."),
            Command::Start => format!("Started `{shown}`."),
            Command::Stop if !was_running => format!("`{shown}` was not running."),
            Command::Stop => {
                format!("Stopped `{shown}`. It stays stopped until it is started again.")
            }
            Command::Restart => format!("Restarted `{shown}`."),
        })
    }

    fn saved(
        &self,
        change: impl FnOnce(&mut Store) -> Result<(), Vec<String>>,
    ) -> Result<(), Vec<String>> {
        if self.fake {
            return Err(vec!["This is a demo run: nothing is saved.".to_owned()]);
        }
        let Some(paths) = &self.paths else {
            return Err(vec!["Tidewatch has no home folder to save services in.".to_owned()]);
        };
        Store::update(paths, change)
    }

    /// Saves a new service and starts it, even when its autostart is off.
    pub(crate) fn add(&mut self, repo: &Path, service: &Service) -> Result<(), Vec<String>> {
        self.saved(|store| store.add(repo, service))?;
        let service = Service::new(&service.name, &service.command, service.autostart);
        let now = self.now();
        let mut run = Run::new(service);
        run.step(Event::Start, repo, now, &self.spawn);
        self.repos.entry(repo.to_path_buf()).or_default().push(run);
        self.version += 1;
        Ok(())
    }

    /// Replaces the service called `old`; a running one restarts when its command changed.
    pub(crate) fn edit(
        &mut self,
        repo: &Path,
        old: &str,
        service: &Service,
    ) -> Result<(), Vec<String>> {
        self.saved(|store| store.replace(repo, old, service))?;
        let service = Service::new(&service.name, &service.command, service.autostart);
        let now = self.now();
        let spawn = &self.spawn;
        if let Some(run) = self
            .repos
            .get_mut(repo)
            .and_then(|runs| runs.iter_mut().find(|run| same_name(&run.service.name, old)))
        {
            let command_changed = run.service.command != service.command;
            run.service = service;
            if command_changed && run.sup.wants_process() {
                run.step(Event::Restart, repo, now, spawn);
            }
        }
        self.version += 1;
        Ok(())
    }

    /// Stops the service and forgets it.
    pub(crate) fn remove(&mut self, repo: &Path, name: &str) -> Result<(), Vec<String>> {
        self.saved(|store| {
            store.remove(repo, name);
            Ok(())
        })?;
        if let Some(runs) = self.repos.get_mut(repo) {
            runs.retain(|run| !same_name(&run.service.name, name));
        }
        self.version += 1;
        Ok(())
    }

    /// The last `lines` lines a service printed, cleaned; `None` for a name the repo lacks.
    pub(crate) fn log_tail(&self, repo: &Path, name: &str, lines: usize) -> Option<Vec<String>> {
        let run = self.runs(repo).iter().find(|run| same_name(&run.service.name, name))?;
        Some(Log::from_lines(run.lines()).tail(lines))
    }

    pub(crate) fn log_text(&self, repo: &Path, name: &str, lines: usize) -> Result<String, String> {
        match self.log_tail(repo, name, lines) {
            None => Err(self.unknown(repo, name)),
            Some(tail) if tail.is_empty() => Ok(format!("`{name}` has printed nothing yet.")),
            Some(tail) => Ok(tail.join("\n")),
        }
    }

    /// The service's live terminal, for the Logs view.
    pub(crate) fn term_mut(&mut self, repo: &Path, name: &str) -> Option<&mut PtyTerm> {
        self.find(repo, name)?.term.as_mut()
    }

    pub(crate) fn fake_lines(&self, repo: &Path, name: &str) -> Option<Vec<String>> {
        let run = self.runs(repo).iter().find(|run| same_name(&run.service.name, name))?;
        run.fake_log.as_ref().map(|lines| Log::from_lines(lines).tail(usize::MAX))
    }

    pub(crate) fn has(&self, repo: &Path, name: &str) -> bool {
        self.runs(repo).iter().any(|run| same_name(&run.service.name, name))
    }

    /// Every service as the replacement window needs it, in the order [`Self::captures`] sends
    /// the live ones.
    pub(crate) fn entries(&self) -> Vec<ServiceEntry> {
        let now = self.now();
        self.repos
            .iter()
            .filter(|_| !self.fake)
            .flat_map(|(repo, runs)| {
                runs.iter().map(move |run| ServiceEntry {
                    repo: repo.clone(),
                    name: run.service.name.clone(),
                    live: run.live(),
                    carry: run.sup.carry(now),
                })
            })
            .collect()
    }

    pub(crate) fn captures(&self) -> Result<Vec<(PaneMeta, BorrowedFd<'_>)>, String> {
        self.repos
            .values()
            .filter(|_| !self.fake)
            .flatten()
            .filter(|run| run.live())
            .map(|run| {
                run.term
                    .as_ref()
                    .and_then(capture_pane)
                    .ok_or_else(|| "a service cannot be handed over".to_owned())
            })
            .collect()
    }

    /// The replacement window took the ptys: this window must not stop them any more.
    pub(crate) fn release(&mut self) {
        for run in self.repos.values_mut().flatten() {
            if let Some(term) = run.term.as_mut().filter(|_| run.sup.wants_process()) {
                term.mark_handed_off();
            }
        }
    }

    /// The services an old window left running, waiting for their repos to open.
    pub(crate) fn adopt(&mut self, kept: Vec<ResumedService<PtyTerm>>) {
        self.adopted = kept;
    }

    pub(crate) fn shutdown(&mut self) {
        for run in self.repos.values_mut().flatten() {
            if let Some(term) = run.term.as_mut() {
                term.kill();
            }
        }
    }
}

fn fake_runs(now: f64) -> Vec<Run> {
    let run = |name: &str, command: &str, phase: Phase, log: &[&str]| {
        let mut run = Run::new(Service::new(name, command, true));
        let carry = Carry { phase, streak: 0, exits_ago: Vec::new(), last_exit: None };
        run.sup = Supervisor::from_carry(&carry, now);
        run.fake_log = Some(log.iter().map(|line| (*line).to_owned()).collect());
        run.port = service_port::newest(&Log::from_lines(log).tail(PORT_SCAN_LINES));
        run
    };
    vec![
        run(
            "db",
            "pscale connect shop main --port 3309",
            Phase::Running { up: FAKE_UP_MAIN },
            &[
                "\u{1b}[2mSecure connection to shop/main established\u{1b}[0m",
                "Local address to connect your application: 127.0.0.1:3309",
                "\u{1b}[32mready\u{1b}[0m: accepting connections",
                "client 127.0.0.1:50412 connected",
                "client 127.0.0.1:50419 connected",
            ],
        ),
        run(
            "api",
            "npm run dev --workspace api",
            Phase::Running { up: FAKE_UP_API },
            &[
                "> api@0.4.0 dev",
                "> tsx watch src/server.ts",
                "\u{1b}[36m[api]\u{1b}[0m listening on http://localhost:3000",
                "\u{1b}[36m[api]\u{1b}[0m GET /health 200 2ms",
                "\u{1b}[36m[api]\u{1b}[0m restarted after change in src/routes/orders.ts",
            ],
        ),
        {
            let mut tunnel = run(
                "tunnel",
                "cloudflared tunnel run shop-dev",
                Phase::Crashed { exit: 1, ago: 95.0 },
                &[
                    "INF Starting tunnel tunnelID=6f2a",
                    "\u{1b}[31mERR\u{1b}[0m Unable to reach the origin service: connection refused",
                    "\u{1b}[31mERR\u{1b}[0m Retrying in 8s",
                    "ERR giving up after 5 failed starts",
                ],
            );
            tunnel.sup = Supervisor::from_carry(
                &Carry {
                    phase: Phase::Crashed { exit: 1, ago: 95.0 },
                    streak: 5,
                    exits_ago: Vec::new(),
                    last_exit: Some(1),
                },
                now,
            );
            tunnel
        },
    ]
}

#[cfg(test)]
mod tests;
