//! `Host`: one `gh webhook forward` per GitHub repo the window holds, never per tab. The state
//! machine of `github_forwarder.rs` decides; this file spawns the process in its own group, stops it
//! with SIGTERM, runs the install and the stale-hook clean-up on threads and turns the listener's
//! deliveries into news for the repos' tabs. It blocks on nothing a frame waits for.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::events::{RepoEvent, RepoMessage};
use crate::fsutil::write_atomic;
use crate::github_clone::origin_of;
use crate::github_forwarder::{
    Cleared, Effect, Event, ExitKind, Forwarder, Why, classify, forwarder_hook_ids, stale_to_delete,
};
use crate::github_hook::{EVENTS, Hit};
use crate::github_hooks::RepoHook;
use crate::listener::Listener;
use crate::paths::TidewatchPaths;
use crate::tools::Tools;

const EXTENSION: &str = "cli/gh-webhook";
const STDERR_TAIL_BYTES: usize = 4096;
const EXIT_WAIT: Duration = Duration::from_millis(500);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A delivery that names pull requests, issues and branches of the repo `slug` (`owner/name` in
/// lower case), checked out in `repos`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct News {
    pub slug: String,
    pub repos: Vec<PathBuf>,
    pub hit: Hit,
}

/// What one frame's drive found.
#[derive(Debug, Default, PartialEq)]
pub struct Driven {
    pub news: Vec<News>,
    /// Seconds from now until a retry is due, with nothing else happening.
    pub wake_in: Option<f64>,
}

enum Origin {
    Asked,
    Known(Option<String>),
}

enum Done {
    Origin { repo: PathBuf, slug: Option<String> },
    Installed { key: String, ok: bool },
    Cleared { key: String, outcome: Cleared },
}

struct Exit {
    kind: ExitKind,
}

struct Run {
    pgid: u32,
    exit: Receiver<Exit>,
}

struct Forward {
    slug: String,
    machine: Forwarder,
    run: Option<Run>,
}

pub struct Host {
    tools: Tools,
    paths: Option<TidewatchPaths>,
    wake: Wake,
    epoch: Instant,
    enabled: bool,
    origins: HashMap<PathBuf, Origin>,
    forwards: HashMap<String, Forward>,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
}

fn key_of(slug: &str) -> String {
    slug.to_ascii_lowercase()
}

impl Host {
    /// `enabled` false is a host that never starts anything (a screenshot or background run).
    #[must_use]
    pub fn new(tools: Tools, paths: Option<TidewatchPaths>, wake: Wake, enabled: bool) -> Self {
        let (done_tx, done_rx) = channel();
        Self {
            tools,
            paths,
            wake,
            epoch: Instant::now(),
            enabled,
            origins: HashMap::new(),
            forwards: HashMap::new(),
            done_tx,
            done_rx,
        }
    }

    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn known_slug(&self, repo: &Path) -> Option<&str> {
        match self.origins.get(repo)? {
            Origin::Known(slug) => slug.as_deref(),
            Origin::Asked => None,
        }
    }

    fn paths_of(&self, key: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = self
            .origins
            .iter()
            .filter(
                |(_, origin)| matches!(origin, Origin::Known(Some(slug)) if key_of(slug) == key),
            )
            .map(|(path, _)| path.clone())
            .collect();
        found.sort();
        found
    }

    /// The repos whose forwarder delivers right now.
    #[must_use]
    pub fn live_repos(&self) -> HashSet<PathBuf> {
        self.forwards
            .iter()
            .filter(|(_, forward)| forward.machine.live())
            .flat_map(|(key, _)| self.paths_of(key))
            .collect()
    }

    /// The repos (`owner/name` in lower case) whose forwarder delivers right now, whether or not a
    /// folder of the window shows them.
    #[must_use]
    pub fn live_slugs(&self) -> HashSet<String> {
        self.forwards
            .iter()
            .filter(|(_, forward)| forward.machine.live())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Why a repo is polled, when it has a forwarder that fell back.
    #[must_use]
    pub fn why_polled(&self, repo: &Path) -> Option<Why> {
        let slug = self.known_slug(repo)?;
        self.forwards.get(&key_of(slug))?.machine.why()
    }

    fn background(&self, work: impl FnOnce() -> Option<Done> + Send + 'static) {
        let (done, wake) = (self.done_tx.clone(), Arc::clone(&self.wake));
        std::thread::spawn(move || {
            if let Some(result) = work() {
                let _ = done.send(result);
            }
            wake();
        });
    }

    /// Looks at the repos the window holds, the finished jobs, the forwarders' exits and the
    /// listener's deliveries. `listener` is `None` when it could not start: nothing is forwarded then.
    pub fn drive(&mut self, repos: &[PathBuf], listener: Option<&Listener>) -> Driven {
        if !self.enabled {
            return Driven::default();
        }
        self.ask_origins(repos);
        let now = self.now();
        while let Ok(done) = self.done_rx.try_recv() {
            self.finished(done, listener, now);
        }
        self.sync(repos, listener, now);
        self.read_exits(listener, now);
        let news = self.read_deliveries(listener, now);
        let wake_in = self.tick(listener, now);
        Driven { news, wake_in }
    }

    fn ask_origins(&mut self, repos: &[PathBuf]) {
        self.origins.retain(|path, _| repos.contains(path));
        for repo in repos {
            if self.origins.contains_key(repo) {
                continue;
            }
            self.origins.insert(repo.clone(), Origin::Asked);
            let (tools, repo) = (self.tools.clone(), repo.clone());
            self.background(move || {
                let slug = origin_of(&tools, &repo).map(|origin| origin.slug());
                Some(Done::Origin { repo, slug })
            });
        }
    }

    fn finished(&mut self, done: Done, listener: Option<&Listener>, now: f64) {
        match done {
            Done::Origin { repo, slug } => {
                self.origins.entry(repo).and_modify(|origin| *origin = Origin::Known(slug));
            }
            Done::Installed { key, ok } => {
                self.step(&key, Event::Installed { ok, at: now }, listener, now);
            }
            Done::Cleared { key, outcome } => {
                self.step(&key, Event::Cleared { outcome, at: now }, listener, now);
            }
        }
    }

    fn sync(&mut self, repos: &[PathBuf], listener: Option<&Listener>, now: f64) {
        let wanted: HashMap<String, String> = if listener.is_some() {
            repos
                .iter()
                .filter_map(|repo| self.known_slug(repo))
                .map(|slug| (key_of(slug), slug.to_owned()))
                .collect()
        } else {
            HashMap::new()
        };
        let gone: Vec<String> =
            self.forwards.keys().filter(|key| !wanted.contains_key(*key)).cloned().collect();
        for key in gone {
            self.step(&key, Event::Close, listener, now);
            if let Some(listener) = listener {
                listener.unregister_repo(&key);
            }
            self.forwards.remove(&key);
        }
        for (key, slug) in wanted {
            if !self.forwards.contains_key(&key) {
                self.forwards
                    .insert(key.clone(), Forward { slug, machine: Forwarder::new(), run: None });
                self.step(&key, Event::Open { at: now }, listener, now);
            }
        }
    }

    fn read_exits(&mut self, listener: Option<&Listener>, now: f64) {
        let exited: Vec<(String, ExitKind)> = self
            .forwards
            .iter()
            .filter_map(|(key, forward)| {
                let exit = forward.run.as_ref()?.exit.try_recv().ok()?;
                Some((key.clone(), exit.kind))
            })
            .collect();
        for (key, kind) in exited {
            if let Some(forward) = self.forwards.get_mut(&key) {
                forward.run = None;
            }
            self.step(&key, Event::Exited { kind, at: now }, listener, now);
        }
    }

    fn read_deliveries(&mut self, listener: Option<&Listener>, now: f64) -> Vec<News> {
        let Some(listener) = listener else { return Vec::new() };
        let mut news = Vec::new();
        for RepoMessage { repo, event } in listener.drain_github() {
            let key = key_of(&repo);
            if !self.forwards.contains_key(&key) {
                continue;
            }
            match event {
                RepoEvent::Ping { hook_id } => {
                    self.step(&key, Event::Ping { hook_id, at: now }, Some(listener), now);
                }
                RepoEvent::News(hit) => {
                    news.push(News { repos: self.paths_of(&key), slug: key, hit });
                }
            }
        }
        news
    }

    fn tick(&mut self, listener: Option<&Listener>, now: f64) -> Option<f64> {
        let due: Vec<String> = self
            .forwards
            .iter()
            .filter(|(_, forward)| forward.machine.next_wake().is_some_and(|at| now >= at))
            .map(|(key, _)| key.clone())
            .collect();
        for key in due {
            self.step(&key, Event::Tick { at: now }, listener, now);
        }
        self.forwards
            .values()
            .filter_map(|forward| forward.machine.next_wake())
            .reduce(f64::min)
            .map(|at| (at - now).max(0.0))
    }

    fn step(&mut self, key: &str, event: Event, listener: Option<&Listener>, now: f64) {
        let Some(forward) = self.forwards.get_mut(key) else { return };
        let mut pending: VecDeque<Effect> = forward.machine.step(event).into();
        while let Some(effect) = pending.pop_front() {
            let Some(forward) = self.forwards.get_mut(key) else { return };
            match effect {
                Effect::Spawn => {
                    let Some(listener) = listener else { continue };
                    let hook = listener.register_repo(&key_of(&forward.slug));
                    match spawn(&self.tools, &forward.slug, &hook, &self.wake) {
                        Ok(run) => forward.run = Some(run),
                        Err(kind) => {
                            pending.extend(forward.machine.step(Event::Exited { kind, at: now }));
                        }
                    }
                }
                Effect::Kill => {
                    if let Some(run) = forward.run.take() {
                        terminate(run.pgid);
                    }
                }
                Effect::Install => {
                    let (tools, key) = (self.tools.clone(), key.to_owned());
                    self.background(move || {
                        Some(Done::Installed { ok: install_extension(&tools), key })
                    });
                }
                Effect::ClearStale => {
                    let (tools, paths) = (self.tools.clone(), self.paths.clone());
                    let (slug, key) = (forward.slug.clone(), key.to_owned());
                    self.background(move || {
                        Some(Done::Cleared {
                            outcome: clear_stale(&tools, paths.as_ref(), &slug),
                            key,
                        })
                    });
                }
                Effect::Record { hook_id } => {
                    if let Some(paths) = self.paths.clone() {
                        let slug = forward.slug.clone();
                        self.background(move || {
                            record_hook(&paths, &slug, hook_id);
                            None
                        });
                    }
                }
            }
        }
    }

    /// Stops every forwarder with SIGTERM and waits a moment for each to remove its webhook.
    pub fn shutdown(&mut self, listener: Option<&Listener>) {
        let runs: Vec<Run> =
            self.forwards.values_mut().filter_map(|forward| forward.run.take()).collect();
        for run in &runs {
            terminate(run.pgid);
        }
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        for run in runs {
            let left = deadline.saturating_duration_since(Instant::now());
            let _ = run.exit.recv_timeout(left);
        }
        if let Some(listener) = listener {
            for key in self.forwards.keys() {
                listener.unregister_repo(key);
            }
        }
        self.forwards.clear();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for run in self.forwards.values_mut().filter_map(|forward| forward.run.take()) {
            terminate(run.pgid);
        }
    }
}

fn spawn(tools: &Tools, slug: &str, hook: &RepoHook, wake: &Wake) -> Result<Run, ExitKind> {
    let mut command = tools.gh(&std::env::temp_dir());
    command
        .args(["webhook", "forward"])
        .arg(format!("--repo={slug}"))
        .arg(format!("--events={EVENTS}"))
        .arg(format!("--url={}", hook.url))
        .arg(format!("--secret={}", hook.secret))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ExitKind::NoGh,
        _ => ExitKind::Other,
    })?;
    let pgid = child.id();
    let stderr = child.stderr.take();
    let (tail_tx, tail_rx) = channel();
    std::thread::spawn(move || {
        let mut tail = String::new();
        if let Some(mut stderr) = stderr {
            let mut chunk = [0u8; 1024];
            while let Ok(read) = stderr.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if tail.len() > STDERR_TAIL_BYTES {
                    let cut = tail.len() - STDERR_TAIL_BYTES;
                    tail = tail.chars().skip(cut).collect();
                }
            }
        }
        let _ = tail_tx.send(tail);
    });
    let (exit_tx, exit_rx) = channel();
    let wake = Arc::clone(wake);
    std::thread::spawn(move || {
        let _ = child.wait();
        let tail = tail_rx.recv_timeout(EXIT_WAIT).unwrap_or_default();
        let _ = exit_tx.send(Exit { kind: classify(&tail) });
        wake();
    });
    Ok(Run { pgid, exit: exit_rx })
}

fn terminate(pgid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn install_extension(tools: &Tools) -> bool {
    let mut command = tools.gh(&std::env::temp_dir());
    command.args(["extension", "install", EXTENSION]);
    tools.finish(command).is_ok_and(|done| done.success)
}

fn existing_hook_ids(tools: &Tools, slug: &str) -> Option<Vec<u64>> {
    let mut command = tools.gh(&std::env::temp_dir());
    command.args(["api", &format!("repos/{slug}/hooks")]);
    let done = tools.finish(command).ok().filter(|done| done.success)?;
    Some(forwarder_hook_ids(&done.stdout))
}

fn delete_hook(tools: &Tools, slug: &str, id: u64) -> bool {
    let mut command = tools.gh(&std::env::temp_dir());
    command.args(["api", "-X", "DELETE", &format!("repos/{slug}/hooks/{id}")]);
    tools.finish(command).is_ok_and(|done| done.success)
}

fn clear_stale(tools: &Tools, paths: Option<&TidewatchPaths>, slug: &str) -> Cleared {
    let Some(existing) = existing_hook_ids(tools, slug) else { return Cleared::Failed };
    let recorded = paths.and_then(|paths| recorded_hook(paths, slug));
    match stale_to_delete(recorded, &existing) {
        None => Cleared::NotOurs,
        Some(id) if delete_hook(tools, slug, id) => Cleared::Deleted,
        Some(_) => Cleared::Failed,
    }
}

static STORE: Mutex<()> = Mutex::new(());

fn read_records(paths: &TidewatchPaths) -> HashMap<String, u64> {
    std::fs::read_to_string(paths.github_hooks_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The webhook id the last forwarder of `slug` reported in its `ping`.
#[must_use]
pub fn recorded_hook(paths: &TidewatchPaths, slug: &str) -> Option<u64> {
    let _guard = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    read_records(paths).get(&key_of(slug)).copied()
}

pub fn record_hook(paths: &TidewatchPaths, slug: &str, id: u64) {
    let _guard = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut records = read_records(paths);
    records.insert(key_of(slug), id);
    if let Ok(text) = serde_json::to_string(&records) {
        let _ = write_atomic(&paths.github_hooks_file(), &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_hook_reads_back_per_repo_ignoring_case_and_a_new_one_replaces_it() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(recorded_hook(&paths, "Acme/Web"), None);
        record_hook(&paths, "Acme/Web", 5);
        record_hook(&paths, "acme/api", 6);
        assert_eq!(recorded_hook(&paths, "acme/web"), Some(5));
        record_hook(&paths, "ACME/WEB", 9);
        assert_eq!(recorded_hook(&paths, "Acme/Web"), Some(9));
        assert_eq!(recorded_hook(&paths, "acme/api"), Some(6));
    }

    #[test]
    fn a_broken_record_file_reads_as_no_record() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::write(paths.github_hooks_file(), "{ not json").unwrap();
        assert_eq!(recorded_hook(&paths, "a/b"), None);
        record_hook(&paths, "a/b", 1);
        assert_eq!(recorded_hook(&paths, "a/b"), Some(1));
    }
}
