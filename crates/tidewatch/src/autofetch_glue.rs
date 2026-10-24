//! Background fetch: one timer per open repo, one fetch at a time per repo, and the quiet note a
//! failed fetch leaves for the status bar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kelp_core::ops::Op;
use kelp_ui::autofetch::{AutoFetch, Tick};
use tidewatch_core::prefs::{FETCH_MINUTES_DEFAULT, GitPrefs};

use crate::jobs::Jobs;
use crate::repo::Workspace;

const REASON_MAX: usize = 160;

pub(crate) type Fetch = fn(&Path) -> Result<(), String>;

struct Slot {
    timer: AutoFetch,
    running: bool,
    failure: Option<String>,
}

pub(crate) struct Plan {
    pub(crate) start: Vec<PathBuf>,
    pub(crate) wake: Option<Duration>,
}

pub(crate) struct Schedule {
    every: Duration,
    slots: HashMap<PathBuf, Slot>,
}

impl Schedule {
    pub(crate) fn new(every: Duration) -> Self {
        Self { every, slots: HashMap::new() }
    }

    pub(crate) fn set_every(&mut self, every: Duration) {
        self.every = every;
        for slot in self.slots.values_mut() {
            slot.timer.set_every(every);
        }
    }

    pub(crate) fn restart(&mut self, every: Duration, now: Instant) {
        self.every = every;
        for slot in self.slots.values_mut() {
            slot.timer = AutoFetch::new(every, now);
        }
    }

    pub(crate) fn plan(&mut self, roots: &[PathBuf], now: Instant) -> Plan {
        self.slots.retain(|root, slot| slot.running || roots.contains(root));
        let every = self.every;
        let mut start = Vec::new();
        let mut wake: Option<Duration> = None;
        for root in roots {
            let slot = self.slots.entry(root.clone()).or_insert_with(|| Slot {
                timer: AutoFetch::new(every, now),
                running: false,
                failure: None,
            });
            let next = match slot.timer.tick(now, slot.running) {
                Tick::Fetch => {
                    slot.running = true;
                    start.push(root.clone());
                    every
                }
                Tick::Skip => every,
                Tick::Wait(wait) => wait,
            };
            wake = Some(wake.map_or(next, |soonest| soonest.min(next)));
        }
        Plan { start, wake }
    }

    pub(crate) fn finish(&mut self, root: &Path, result: &Result<(), String>) -> bool {
        let Some(slot) = self.slots.get_mut(root) else { return false };
        slot.running = false;
        slot.failure = result.as_ref().err().cloned();
        result.is_ok()
    }

    pub(crate) fn failure(&self, root: &Path) -> Option<&str> {
        self.slots.get(root)?.failure.as_deref()
    }

    pub(crate) fn every(&self) -> Duration {
        self.every
    }
}

struct Done {
    root: PathBuf,
    result: Result<(), String>,
}

pub(crate) struct Host {
    prefs: GitPrefs,
    live: bool,
    schedule: Schedule,
    jobs: Jobs<Done>,
    fetch: Fetch,
}

impl Host {
    pub(crate) fn new(prefs: GitPrefs, live: bool) -> Self {
        Self::with_fetch(prefs, live, fetch_all)
    }

    fn with_fetch(prefs: GitPrefs, live: bool, fetch: Fetch) -> Self {
        let every = prefs.interval().unwrap_or(Duration::from_mins(FETCH_MINUTES_DEFAULT.into()));
        Self { prefs, live, schedule: Schedule::new(every), jobs: Jobs::new(), fetch }
    }

    pub(crate) fn prefs(&self) -> GitPrefs {
        self.prefs
    }

    pub(crate) fn set_prefs(&mut self, prefs: GitPrefs, now: Instant) {
        let was_off = self.prefs.interval().is_none();
        self.prefs = prefs;
        match prefs.interval() {
            Some(every) if was_off => self.schedule.restart(every, now),
            Some(every) => self.schedule.set_every(every),
            None => {}
        }
    }

    /// Collects finished fetches and starts the due ones, once per frame. Returns the repos whose
    /// fetch just succeeded, so the caller can read their counts again.
    pub(crate) fn drive(
        &mut self,
        ctx: &egui::Context,
        roots: &[PathBuf],
        now: Instant,
    ) -> Vec<PathBuf> {
        let fetched = self
            .jobs
            .drain()
            .into_iter()
            .filter(|done| self.schedule.finish(&done.root, &done.result))
            .map(|done| done.root)
            .collect();
        if !self.live || self.prefs.interval().is_none() {
            return fetched;
        }
        let plan = self.schedule.plan(roots, now);
        for root in plan.start {
            let fetch = self.fetch;
            self.jobs.spawn(ctx, move || Done { result: fetch(&root), root });
        }
        if let Some(wake) = plan.wake {
            ctx.request_repaint_after(wake);
        }
        fetched
    }

    pub(crate) fn failure_note(&self, root: &Path) -> Option<String> {
        let reason = self.schedule.failure(root)?;
        let minutes = self.schedule.every().as_secs() / 60;
        Some(format!("{reason}. Trying again in {minutes} min."))
    }
}

/// The repos of the window that get fetched: one per repo group, whatever number of worktree tabs
/// it has. A folder that is not a repo has nothing to fetch.
pub(crate) fn roots(workspaces: &[Workspace]) -> Vec<PathBuf> {
    workspaces
        .iter()
        .filter_map(|workspace| match workspace {
            Workspace::Repo(repo) => Some(repo.root.clone()),
            Workspace::Other { .. } => None,
        })
        .collect()
}

fn fetch_all(root: &Path) -> Result<(), String> {
    Op::Fetch.run(root).map(|_| ()).map_err(|e| reason_of(&format!("{e:#}")))
}

fn reason_of(message: &str) -> String {
    let message = message.split_once(" failed: ").map_or(message, |(_, rest)| rest);
    let mut lines = message.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines
        .clone()
        .rev()
        .find(|line| line.starts_with("fatal:") || line.starts_with("error:"))
        .or_else(|| lines.next_back())
        .unwrap_or("git fetch failed");
    let line =
        line.strip_prefix("fatal: ").or_else(|| line.strip_prefix("error: ")).unwrap_or(line);
    let line = line.trim_end_matches('.');
    match line.char_indices().nth(REASON_MAX) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests;
