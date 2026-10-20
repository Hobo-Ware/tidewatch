//! The app's side of Meeseeks: checks and poof as jobs, text held for an idle agent, the fade, the
//! poofed list, and `TIDEWATCH_FAKE_MEESEEKS=on|struggling|poofed` for captures.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::civil_date::Date;
use tidewatch_core::meeseeks::{Criteria, MAX_FAILURES, Meeseeks, Phase, Step};
use tidewatch_core::meeseeks_check as check;
use tidewatch_core::meeseeks_log::{self, Log, Poofed};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, Worktree};

use crate::jobs::Jobs;

pub(crate) const FADE_SECS: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Off,
    On,
    Struggling,
    Poofed,
}

impl Fake {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("on") => Self::On,
            Some("struggling") => Self::Struggling,
            Some("poofed") => Self::Poofed,
            _ => Self::Off,
        }
    }

    pub(crate) fn is_on(self) -> bool {
        self != Self::Off
    }

    pub(crate) fn state(self, now_ms: i64) -> Option<Meeseeks> {
        let (minutes, turns, failures, phase, task) = match self {
            Self::On => (12, 8, 0, Phase::Checking(Step::Tests), "Bump vite to 6.3"),
            Self::Struggling => {
                (38, 12, MAX_FAILURES, Phase::Working, "Rename getUserData to loadProfile")
            }
            Self::Off | Self::Poofed => return None,
        };
        Some(Meeseeks {
            turns,
            failures,
            phase,
            ..Meeseeks::summon(task, Criteria::default(), now_ms - minutes * 60_000)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Leaving {
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) pr_url: Option<String>,
    pub(crate) duration_secs: u64,
}

pub(crate) enum Done<T> {
    Checked { tab: u64, step: Step, result: Result<Option<String>, String> },
    Poofed { tab: u64 },
    Refused { tab: u64, why: String, shell: Option<T> },
    Loaded(Log),
    Logged,
}

pub(crate) struct CheckSpec {
    pub(crate) tools: Tools,
    pub(crate) worktree: Worktree,
    pub(crate) summary: String,
}

pub(crate) fn run_check(step: Step, spec: &CheckSpec, pr: bool) -> Result<Option<String>, String> {
    let (tools, wt) = (&spec.tools, &spec.worktree);
    match step {
        Step::Change => check::change(tools, &wt.path, &wt.branch).map(|()| None),
        Step::Tests => check::tests(&wt.repo, &wt.path).map(|()| None),
        Step::Proof => check::proof(tools, &wt.path).map(|()| None),
        Step::HandOff if pr => {
            check::hand_off(tools, &wt.path, &wt.branch, &spec.summary).map(Some)
        }
        Step::HandOff => Ok(None),
    }
}

pub(crate) fn run_poof<T>(
    paths: &TidewatchPaths,
    tab: u64,
    shell: T,
    worktree: &Worktree,
    trust: Option<&Grant>,
) -> Done<T> {
    let changed = worktree::inspect(worktree).changed;
    if changed > 0 {
        let files = if changed == 1 { "file" } else { "files" };
        let why = format!("{changed} {files} with uncommitted changes");
        return Done::Refused { tab, why, shell: Some(shell) };
    }
    drop(shell);
    match check::poof(paths, worktree) {
        Ok(()) => {
            if let Some(grant) = trust {
                let _ = grant.revoke(Some(&worktree.path));
            }
            Done::Poofed { tab }
        }
        Err(why) => Done::Refused { tab, why, shell: None },
    }
}

pub(crate) struct Host {
    jobs: Jobs<Done<PtyTerm>>,
    pending: HashMap<u64, String>,
    leaving: HashMap<u64, Leaving>,
    fading: HashMap<u64, f64>,
    struggled: HashSet<u64>,
    loaded: Vec<Poofed>,
    added: Vec<Poofed>,
    file: Option<PathBuf>,
    writable: bool,
    asked: bool,
    fake: Fake,
    fake_toasted: bool,
}

impl Host {
    pub(crate) fn new(file: Option<PathBuf>, writable: bool, fake: Fake) -> Self {
        Self {
            jobs: Jobs::new(),
            pending: HashMap::new(),
            leaving: HashMap::new(),
            fading: HashMap::new(),
            struggled: HashSet::new(),
            loaded: Vec::new(),
            added: Vec::new(),
            file: file.filter(|_| !fake.is_on()),
            writable: writable && !fake.is_on(),
            asked: false,
            fake,
            fake_toasted: false,
        }
    }

    pub(crate) fn from_env(paths: Option<&TidewatchPaths>, writable: bool) -> Self {
        let fake = Fake::parse(std::env::var("TIDEWATCH_FAKE_MEESEEKS").ok().as_deref());
        Self::new(paths.map(TidewatchPaths::meeseeks_file), writable, fake)
    }

    pub(crate) fn fake(&self) -> Fake {
        self.fake
    }

    pub(crate) fn drain(&mut self, ctx: &egui::Context) -> Vec<Done<PtyTerm>> {
        if !std::mem::replace(&mut self.asked, true)
            && let Some(file) = self.file.clone()
        {
            self.jobs.spawn(ctx, move || Done::Loaded(Log::load(&file)));
        }
        let mut finished = Vec::new();
        for done in self.jobs.drain() {
            match done {
                Done::Loaded(log) => self.loaded = log.entries,
                Done::Logged => {}
                other => finished.push(other),
            }
        }
        finished
    }

    pub(crate) fn check(
        &self,
        ctx: &egui::Context,
        tab: u64,
        step: Step,
        spec: CheckSpec,
        pr: bool,
    ) {
        self.jobs.spawn(ctx, move || Done::Checked {
            tab,
            step,
            result: run_check(step, &spec, pr),
        });
    }

    pub(crate) fn poof(
        &mut self,
        ctx: &egui::Context,
        paths: TidewatchPaths,
        job: (u64, PtyTerm, Worktree, Option<Grant>),
        leaving: Leaving,
    ) {
        let (tab, shell, worktree, trust) = job;
        self.leaving.insert(tab, leaving);
        self.jobs.spawn(ctx, move || run_poof(&paths, tab, shell, &worktree, trust.as_ref()));
    }

    pub(crate) fn leaving(&mut self, tab: u64) -> Option<Leaving> {
        self.leaving.remove(&tab)
    }

    pub(crate) fn queue_paste(&mut self, tab: u64, text: String) {
        self.pending.insert(tab, text);
    }

    pub(crate) fn pending_tabs(&self) -> Vec<u64> {
        self.pending.keys().copied().collect()
    }

    pub(crate) fn take_paste(&mut self, tab: u64) -> Option<String> {
        self.pending.remove(&tab)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.pending.remove(&tab);
        self.leaving.remove(&tab);
        self.fading.remove(&tab);
        self.struggled.remove(&tab);
    }

    pub(crate) fn first_struggle(&mut self, tab: u64) -> bool {
        self.struggled.insert(tab)
    }

    pub(crate) fn start_fade(&mut self, tab: u64, now: f64) {
        self.fading.insert(tab, now);
    }

    pub(crate) fn fade(&self, tab: u64, now: f64) -> Option<f32> {
        let started = self.fading.get(&tab)?;
        Some(((now - started) / FADE_SECS).clamp(0.0, 1.0) as f32)
    }

    pub(crate) fn fading(&self) -> bool {
        !self.fading.is_empty()
    }

    pub(crate) fn faded(&mut self, now: f64) -> Vec<u64> {
        let done: Vec<u64> = self
            .fading
            .iter()
            .filter(|(_, at)| now - **at >= FADE_SECS)
            .map(|(tab, _)| *tab)
            .collect();
        for tab in &done {
            self.fading.remove(tab);
        }
        done
    }

    pub(crate) fn record(&mut self, ctx: &egui::Context, entry: Poofed) {
        self.added.push(entry.clone());
        if let (Some(file), true) = (self.file.clone(), self.writable) {
            self.jobs.spawn(ctx, move || {
                if let Err(e) = meeseeks_log::append(&file, entry) {
                    eprintln!("tidewatch: cannot write {}: {e}", file.display());
                }
                Done::Logged
            });
        }
    }

    pub(crate) fn fake_toast(&mut self) -> Option<&'static str> {
        (self.fake == Fake::Poofed && !std::mem::replace(&mut self.fake_toasted, true))
            .then_some("vite-6-3 poofed")
    }

    pub(crate) fn poofed_on(&self, day: Date) -> Vec<Poofed> {
        if self.fake == Fake::Poofed {
            return fake_poofed(day);
        }
        let added = self.added.iter().filter(|entry| !self.loaded.contains(entry));
        let all: Vec<&Poofed> = self.loaded.iter().chain(added).collect();
        all.into_iter().rev().filter(|entry| entry.date == day).cloned().collect()
    }
}

fn fake_poofed(day: Date) -> Vec<Poofed> {
    let entry = |name: &str, number: u32, secs: u64| Poofed {
        name: name.to_owned(),
        summary: format!("Did {name}."),
        pr_url: Some(format!("https://github.com/acme/web/pull/{number}")),
        duration_secs: secs,
        date: day,
    };
    vec![entry("promo-test", 213, 360), entry("lint-fix", 211, 180)]
}

#[cfg(test)]
mod tests;
