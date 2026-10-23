//! The seams between the PR watches and the app (PLAN.md, D4 follow-up): one `Watch` per agent
//! worktree tab, its GitHub jobs and the clicks on its cards, handed back as [`Applied`] values.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::pr_cards::Cards;
use tidewatch_core::pr_status::Body;
use tidewatch_core::prefs::GithubPrefs;
use tidewatch_core::tools::Tools;
use tidewatch_core::worktree::{BRANCH_PREFIX, Worktree};

use crate::jobs::Jobs;
use crate::merge_cleanup::Cleanup;
use crate::pr_cards_view::Data;
use crate::pr_jobs::{self, Done, Saver};
use crate::pr_watch::{Facts, Out, Watch};

/// What the app must do for a tab.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) tab: u64,
    pub(crate) out: Out,
}

/// `TIDEWATCH_DEV_PR_EVENTS=<file>`: the cards of the first agent tab are loaded from a JSON file (a
/// `Cards` value) instead of its state folder, for captures. Without `TIDEWATCH_DEV_GH_DIR` as well
/// no look, re-run or page opening reaches the outside: the cards are only drawn.
#[derive(Debug, Default)]
pub(crate) struct Dev {
    seed: Option<Cards>,
    fake_gh: bool,
    /// `TIDEWATCH_DEV_PR_POPOVER=1`: the strip's popover is open.
    popover: bool,
}

impl Dev {
    pub(crate) fn from_env() -> Self {
        let seed = std::env::var_os("TIDEWATCH_DEV_PR_EVENTS")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .and_then(|path| match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<Cards>(&bytes)
                    .map_err(|e| eprintln!("tidewatch: {} is not PR cards ({e})", path.display()))
                    .ok(),
                Err(e) => {
                    eprintln!("tidewatch: cannot read {} ({e})", path.display());
                    None
                }
            });
        let fake_gh = std::env::var_os("TIDEWATCH_DEV_GH_DIR").is_some_and(|dir| !dir.is_empty());
        let popover =
            std::env::var("TIDEWATCH_DEV_PR_POPOVER").is_ok_and(|v| !v.is_empty() && v != "0");
        Self { seed, fake_gh, popover }
    }

    /// Whether `gh` may be run and pages opened: always, except for cards from a file that no fake
    /// `gh` stands behind.
    pub(crate) fn outside_reachable(&self) -> bool {
        self.seed.is_none() || self.fake_gh
    }
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    tabs: HashMap<u64, Watch>,
    default: GithubPrefs,
    tools: Tools,
    saver: Saver,
    dev: Dev,
}

impl Host {
    pub(crate) fn new(tools: Tools, default: GithubPrefs, dev: Dev) -> Self {
        Self { jobs: Jobs::new(), tabs: HashMap::new(), default, tools, saver: Saver::new(), dev }
    }

    pub(crate) fn default_prefs(&self) -> GithubPrefs {
        self.default
    }

    pub(crate) fn set_default(&mut self, prefs: GithubPrefs) {
        self.default = prefs;
    }

    pub(crate) fn outside_reachable(&self) -> bool {
        self.dev.outside_reachable()
    }

    pub(crate) fn open_popover(&self) -> bool {
        self.dev.popover
    }

    pub(crate) fn watch(&self, tab: u64) -> Option<&Watch> {
        self.tabs.get(&tab)
    }

    /// What the Reader draws for the tab, `None` while there is nothing to draw.
    pub(crate) fn data<'a>(&'a self, tab: u64, profile: &'a str, now_ms: i64) -> Option<Data<'a>> {
        let watch = self.tabs.get(&tab)?;
        let data = Data { watch, profile, mode: watch.policy(self.default).mode, now_ms };
        (!watch.cards.cards().is_empty() || data.has_tail()).then_some(data)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.tabs.remove(&tab);
    }

    /// The tab's choices: its own, or the default it started from.
    pub(crate) fn policy(&self, tab: u64) -> GithubPrefs {
        self.tabs.get(&tab).map_or(self.default, |watch| watch.policy(self.default))
    }

    /// Looks at every agent tab and starts what each needs. Returns what to carry out and the next
    /// time a frame is needed with no other news.
    pub(crate) fn tick(
        &mut self,
        facts: &[Facts<'_>],
        paths: &TidewatchPaths,
        ctx: &egui::Context,
        now: f64,
        now_ms: i64,
    ) -> (Vec<Applied>, Option<f64>) {
        self.tabs.retain(|id, _| facts.iter().any(|f| f.id == *id));
        let mut applied = Vec::new();
        let mut wake: Option<f64> = None;
        for fact in facts {
            let default = self.default;
            let watch = self
                .tabs
                .entry(fact.id)
                .or_insert_with(|| Watch::new(paths.pr_cards_file(fact.repo, fact.name)));
            if watch.dir != fact.dir {
                fact.dir.clone_into(&mut watch.dir);
            }
            if watch.title != fact.title {
                fact.title.clone_into(&mut watch.title);
            }
            if watch.take_load() {
                pr_jobs::load(&self.jobs, ctx, fact.id, watch.file.clone());
            }
            let mut outs = Vec::new();
            if let Some((number, head)) = fact.pr
                && watch.learn(number, head, now)
            {
                outs.push(Out::Save);
            }
            if watch.observe(fact.status)
                && self.dev.outside_reachable()
                && let Some(head) = watch.cards.seen.as_ref().map(|seen| seen.head.clone())
            {
                pr_jobs::committed(
                    &self.jobs,
                    ctx,
                    fact.id,
                    self.tools.clone(),
                    fact.dir.to_path_buf(),
                    head,
                );
            }
            outs.extend(watch.flush(default, now, now_ms));
            if watch.cleanup.wants_check() && watch.merge_offered() {
                watch.cleanup = Cleanup::Checking;
                let worktree = Worktree {
                    repo: fact.repo.to_path_buf(),
                    path: fact.dir.to_path_buf(),
                    branch: format!("{BRANCH_PREFIX}{}", fact.name),
                };
                pr_jobs::check_cleanup(
                    &self.jobs,
                    ctx,
                    paths.clone(),
                    fact.id,
                    worktree,
                    watch.pr_head(),
                    false,
                );
            }
            if self.dev.outside_reachable()
                && let Some(look) = watch.should_look(default, now)
            {
                watch.look_started();
                pr_jobs::look(
                    &self.jobs,
                    ctx,
                    fact.id,
                    self.tools.clone(),
                    fact.dir.to_path_buf(),
                    look,
                    now_ms,
                );
            }
            let soonest = [
                if self.dev.outside_reachable() { watch.next_look(default) } else { None },
                watch.next_flush(now),
            ]
            .into_iter()
            .flatten()
            .reduce(f64::min);
            wake = [wake, soonest].into_iter().flatten().reduce(f64::min);
            applied.extend(self.finish(fact.id, outs));
        }
        (applied, wake)
    }

    pub(crate) fn drain(&mut self, now: f64, now_ms: i64) -> Vec<Applied> {
        let mut applied = Vec::new();
        for done in self.jobs.drain() {
            let default = self.default;
            let (tab, outs) = match done {
                Done::Loaded { tab, cards } => {
                    let Some(watch) = self.tabs.get_mut(&tab) else { continue };
                    let cards = match self.dev.seed.take() {
                        Some(seed) => Box::new(seed),
                        None => cards,
                    };
                    watch.loaded(*cards, now);
                    (tab, Vec::new())
                }
                Done::Looked { tab, result } => {
                    let Some(watch) = self.tabs.get_mut(&tab) else { continue };
                    let title = watch.title.clone();
                    (tab, watch.polled(result.map(|polled| *polled), default, &title, now, now_ms))
                }
                Done::Committed { tab, found } => {
                    let Some(watch) = self.tabs.get_mut(&tab) else { continue };
                    let outs =
                        if watch.committed(found, now_ms) { vec![Out::Save] } else { vec![] };
                    (tab, outs)
                }
                Done::Checked { tab, remove, result } => {
                    let Some(watch) = self.tabs.get_mut(&tab) else { continue };
                    (tab, watch.checked(remove, result))
                }
                Done::Reran { tab, result } => {
                    let Some(watch) = self.tabs.get_mut(&tab) else { continue };
                    watch.look_soon(now);
                    let text = match result {
                        Ok(()) => "Re-running the failed checks on GitHub".to_owned(),
                        Err(error) => format!("Could not re-run the checks: {error}"),
                    };
                    (tab, vec![Out::Toast(text)])
                }
            };
            applied.extend(self.finish(tab, outs));
        }
        applied
    }

    fn finish(&self, tab: u64, outs: Vec<Out>) -> Vec<Applied> {
        let mut applied = Vec::new();
        for out in outs {
            match out {
                Out::Save => {
                    if let Some(watch) = self.tabs.get(&tab) {
                        self.saver.save(&watch.file, watch.cards.clone());
                    }
                }
                other => applied.push(Applied { tab, out: other }),
            }
        }
        applied
    }

    /// `Ask <profile> to ...` on the card `card`.
    pub(crate) fn ask(&mut self, tab: u64, card: u64, now: f64, now_ms: i64) -> Vec<Applied> {
        let Some(watch) = self.tabs.get_mut(&tab) else { return Vec::new() };
        let outs = watch.ask(card, now, now_ms).unwrap_or_else(|reason| vec![Out::Toast(reason)]);
        self.finish(tab, outs)
    }

    pub(crate) fn send_now(
        &mut self,
        tab: u64,
        pending: u64,
        now: f64,
        now_ms: i64,
    ) -> Vec<Applied> {
        let Some(watch) = self.tabs.get_mut(&tab) else { return Vec::new() };
        let outs =
            watch.send_now(pending, now, now_ms).unwrap_or_else(|reason| vec![Out::Toast(reason)]);
        self.finish(tab, outs)
    }

    pub(crate) fn cancel(&mut self, tab: u64, pending: u64) {
        if let Some(watch) = self.tabs.get_mut(&tab) {
            watch.cancel(pending);
        }
    }

    /// Hides the buttons of a card (Keep it), or drops it (Not now).
    pub(crate) fn dismiss(&mut self, tab: u64, card: u64, remove: bool) {
        let Some(watch) = self.tabs.get_mut(&tab) else { return };
        if remove {
            watch.cards.remove(card);
        } else {
            watch.cards.handle(card);
        }
        self.saver.save(&watch.file, watch.cards.clone());
    }

    /// Looks at the merged tab's worktree once more before it is removed.
    pub(crate) fn check_before_removal(
        &mut self,
        tab: u64,
        worktree: Worktree,
        paths: &TidewatchPaths,
        ctx: &egui::Context,
    ) {
        let Some(watch) = self.tabs.get_mut(&tab) else { return };
        watch.cleanup = Cleanup::Checking;
        pr_jobs::check_cleanup(
            &self.jobs,
            ctx,
            paths.clone(),
            tab,
            worktree,
            watch.pr_head(),
            true,
        );
    }

    /// The body of a card, for the app to act on (the failed run to re-run, the commits to push).
    pub(crate) fn body(&self, tab: u64, card: u64) -> Option<Body> {
        self.tabs.get(&tab)?.cards.get(card).map(|card| card.body.clone())
    }

    pub(crate) fn rerun(&self, tab: u64, run: u64, ctx: &egui::Context) -> Vec<Applied> {
        let Some(watch) = self.tabs.get(&tab) else { return Vec::new() };
        if !self.dev.outside_reachable() {
            return vec![Applied { tab, out: Out::Toast("Re-run is off in this dev run".into()) }];
        }
        pr_jobs::rerun(&self.jobs, ctx, tab, self.tools.clone(), watch.dir.clone(), run);
        Vec::new()
    }

    pub(crate) fn set_policy(&mut self, tab: u64, policy: GithubPrefs, now: f64) {
        let Some(watch) = self.tabs.get_mut(&tab) else { return };
        watch.set_policy(policy, now);
        self.saver.save(&watch.file, watch.cards.clone());
    }
}

#[cfg(test)]
mod tests;
