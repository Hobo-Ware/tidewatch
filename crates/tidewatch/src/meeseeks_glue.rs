//! The app's side of Meeseeks: checks and poof as jobs, text held for an idle agent, the poofed
//! list, the branch facts of the summary tabs, and `TIDEWATCH_FAKE_MEESEEKS=on|struggling|poofed`
//! for captures.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use jiff::tz::TimeZone;
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::civil_date::Date;
use tidewatch_core::meeseeks::{CheckLine, Criteria, MAX_FAILURES, Meeseeks, Phase, Step, Verdict};
use tidewatch_core::meeseeks_branch::{self, Branch, Changed, Found};
use tidewatch_core::meeseeks_check::{self as check, Checked};
use tidewatch_core::meeseeks_log::{self, Log, Poofed, StageLine};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, Worktree};

use crate::jobs::Jobs;

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

pub(crate) enum Done<T> {
    Checked { tab: u64, step: Step, result: Result<Checked, String> },
    Branch { tab: u64, branch: Branch },
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

pub(crate) fn run_check(step: Step, spec: &CheckSpec, pr: bool) -> Result<Checked, String> {
    let (tools, wt) = (&spec.tools, &spec.worktree);
    match step {
        Step::Change => check::change(tools, &wt.path, &wt.branch),
        Step::Tests => check::tests(&wt.repo, &wt.path),
        Step::Proof => check::proof(tools, &wt.path),
        Step::HandOff if pr => check::hand_off(tools, &wt.path, &wt.branch, &spec.summary),
        Step::HandOff => Ok(Checked { detail: String::new(), pr_url: None }),
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
    leaving: HashMap<u64, Poofed>,
    branches: HashMap<u64, Option<Branch>>,
    struggled: HashSet<u64>,
    loaded: Vec<Poofed>,
    added: Vec<Poofed>,
    file: Option<PathBuf>,
    writable: bool,
    asked: bool,
    fake: Fake,
    fake_shown: HashSet<&'static str>,
    zone: TimeZone,
}

impl Host {
    pub(crate) fn new(file: Option<PathBuf>, writable: bool, fake: Fake) -> Self {
        Self {
            jobs: Jobs::new(),
            pending: HashMap::new(),
            leaving: HashMap::new(),
            branches: HashMap::new(),
            struggled: HashSet::new(),
            loaded: Vec::new(),
            added: Vec::new(),
            file: file.filter(|_| !fake.is_on()),
            writable: writable && !fake.is_on(),
            asked: false,
            fake,
            fake_shown: HashSet::new(),
            zone: TimeZone::system(),
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
                Done::Branch { tab, branch } => {
                    self.branches.insert(tab, Some(branch));
                }
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
        leaving: Poofed,
    ) {
        let (tab, shell, worktree, trust) = job;
        self.leaving.insert(tab, leaving);
        self.jobs.spawn(ctx, move || run_poof(&paths, tab, shell, &worktree, trust.as_ref()));
    }

    pub(crate) fn leaving(&mut self, tab: u64) -> Option<Poofed> {
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
        self.branches.remove(&tab);
        self.struggled.remove(&tab);
    }

    pub(crate) fn first_struggle(&mut self, tab: u64) -> bool {
        self.struggled.insert(tab)
    }

    /// The branch of a summary tab: asked once, from the repo's main checkout, and kept until the
    /// tab goes. `None` while git is still being asked.
    pub(crate) fn branch(&self, tab: u64) -> Option<&Branch> {
        self.branches.get(&tab)?.as_ref()
    }

    pub(crate) fn read_branch(
        &mut self,
        ctx: &egui::Context,
        tab: u64,
        tools: Tools,
        poofed: &Poofed,
    ) {
        if self.branches.contains_key(&tab) {
            return;
        }
        if self.fake == Fake::Poofed {
            self.branches.insert(tab, Some(fake_branch()));
            return;
        }
        self.branches.insert(tab, None);
        let (repo, name) = (poofed.repo.clone(), poofed.branch.clone());
        self.jobs.spawn(ctx, move || Done::Branch {
            tab,
            branch: meeseeks_branch::read(&tools, &repo, &name),
        });
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
        (self.fake == Fake::Poofed && self.fake_shown.insert("toast")).then_some("vite-6-3 poofed")
    }

    pub(crate) fn zone(&self) -> &TimeZone {
        &self.zone
    }

    pub(crate) fn first_fake_summary(&mut self) -> bool {
        self.fake_shown.insert("summary")
    }

    pub(crate) fn poofed_on(&self, day: Date) -> Vec<Poofed> {
        if self.fake == Fake::Poofed {
            return fake_poofed(day, crate::flow_glue::unix_now() * 1000);
        }
        let added = self.added.iter().filter(|entry| !self.loaded.contains(entry));
        let all: Vec<&Poofed> = self.loaded.iter().chain(added).collect();
        all.into_iter().rev().filter(|entry| entry.date == day).cloned().collect()
    }
}

pub(crate) fn fake_branch() -> Branch {
    Branch::Found(Found {
        base: "main".to_owned(),
        ahead: 2,
        changed: Changed { files: 3, added: 42, removed: 7 },
    })
}

pub(crate) fn fake_poofed(day: Date, now_ms: i64) -> Vec<Poofed> {
    let line = |step, detail: &str| CheckLine {
        step,
        verdict: Verdict::Passed,
        detail: detail.to_owned(),
    };
    let entry = |name: &str, task: &str, number: u32, secs: u64| {
        let url = format!("https://github.com/acme/web/pull/{number}");
        let mut poofed = Poofed::bare(name, &format!("Did {name}."), Some(&url), secs, day);
        task.clone_into(&mut poofed.task);
        poofed.repo = PathBuf::from("/code/web");
        poofed.branch = format!("agent/{name}");
        poofed.ended_ms = now_ms - 90_000;
        poofed.checks = vec![
            line(Step::Change, "2 commits ahead of main"),
            line(Step::Tests, "cargo test passed"),
            line(Step::Proof, "Worktree clean"),
            line(Step::HandOff, &format!("Opened PR #{number} against main")),
        ];
        poofed.done = vec![format!("Finished `{name}` and opened PR #{number}")];
        poofed.next = vec!["Review the pull request".to_owned()];
        poofed.stages = ["Understand", "Build", "PR", "Done"]
            .map(|name| StageLine { name: name.to_owned(), done: 2, total: 2 })
            .to_vec();
        poofed
    };
    vec![
        entry("promo-test", "Add a test for the promo banner", 213, 360),
        entry("lint-fix", "Fix the lint warnings in the billing module", 211, 180),
    ]
}

#[cfg(test)]
mod tests;
