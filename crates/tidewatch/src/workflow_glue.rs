//! `Host`: the workflow profile of every repo the window holds. A saved profile is the answer; a
//! repo with none gets a detection on a `Jobs` thread, whose values are used for messages and
//! offered once on a card until the owner accepts them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::workflow_detect::{self, Detected, Pick, Source};
use tidewatch_core::workflow_profile::{CatchUp, Fixes, PullRequests, Store, Workflow};

use crate::jobs::Jobs;

pub(crate) const FAKE_VAR: &str = "TIDEWATCH_DEV_WORKFLOW_CARD";

enum Entry {
    Reading,
    Proposed(Detected),
    Dismissed(Detected),
}

enum Done {
    Loaded(Store),
    Detected(PathBuf, Detected),
}

/// What the host may do on this run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Reads, detects and shows the card; writes only when the owner accepts or changes.
    Live,
    /// A screenshot or background run: reads and detects, never shows the card or writes.
    Quiet,
    /// `TIDEWATCH_DEV_WORKFLOW_CARD`: stand-in values, the card shown, nothing read or written.
    Fake,
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    paths: Option<TidewatchPaths>,
    mode: Mode,
    load_asked: bool,
    store: Option<Store>,
    entries: HashMap<PathBuf, Entry>,
}

fn key(repo: &Path) -> PathBuf {
    repo.components().collect()
}

pub(crate) fn stand_in() -> Detected {
    let found = |quote: &str, file: &str| Source::Found { quote: quote.into(), file: file.into() };
    Detected {
        catch_up: Pick {
            value: CatchUp::Rebase,
            source: found("rebase --autosquash origin/main", "~/.claude/CLAUDE.md"),
        },
        fixes: Pick { value: Fixes::Fixup, source: found("--fixup", ".agents/rules/workflow.md") },
        pull_requests: Pick {
            value: PullRequests::Draft,
            source: Source::Default {
                why: "Tidewatch has always asked for drafts, and the Draft PRs guard (off by default) only refuses a pull request that is not one",
            },
        },
    }
}

impl Host {
    pub(crate) fn new(paths: Option<TidewatchPaths>, mode: Mode) -> Self {
        let fake = mode == Mode::Fake;
        Self {
            jobs: Jobs::new(),
            paths,
            mode,
            load_asked: fake,
            store: fake.then(Store::default),
            entries: HashMap::new(),
        }
    }

    pub(crate) fn mode_from_env(quiet: bool) -> Mode {
        if std::env::var(FAKE_VAR).is_ok_and(|value| value == "1") {
            Mode::Fake
        } else if quiet {
            Mode::Quiet
        } else {
            Mode::Live
        }
    }

    /// Starts what the repos need: the saved file once, then one detection per repo with no
    /// saved profile.
    pub(crate) fn sync(&mut self, repos: &[&Path], ctx: &egui::Context) {
        if !self.load_asked
            && let Some(paths) = self.paths.clone()
        {
            self.load_asked = true;
            self.jobs.spawn(ctx, move || Done::Loaded(Store::load(&paths)));
        }
        let Some(store) = &self.store else { return };
        for repo in repos {
            let at = key(repo);
            if store.get(repo).is_some() || self.entries.contains_key(&at) {
                continue;
            }
            if self.mode == Mode::Fake {
                self.entries.insert(at, Entry::Proposed(stand_in()));
            } else if let Some(paths) = self.paths.clone() {
                self.entries.insert(at.clone(), Entry::Reading);
                self.jobs.spawn(ctx, move || {
                    let detected = workflow_detect::detect_repo(&paths, &at);
                    Done::Detected(at, detected)
                });
            }
        }
    }

    pub(crate) fn drive(&mut self) {
        for done in self.jobs.drain() {
            match done {
                Done::Loaded(store) => self.store = Some(store),
                Done::Detected(repo, detected) => {
                    let entry = if self.mode == Mode::Live {
                        Entry::Proposed(detected)
                    } else {
                        Entry::Dismissed(detected)
                    };
                    self.entries.insert(repo, entry);
                }
            }
        }
    }

    /// What messages for `repo` say: the saved profile, else what detection proposed, else the
    /// defaults.
    pub(crate) fn profile(&self, repo: &Path) -> Workflow {
        if let Some(saved) = self.store.as_ref().and_then(|store| store.get(repo)) {
            return saved;
        }
        self.detected(repo).map_or_else(Workflow::default, Detected::workflow)
    }

    pub(crate) fn detected(&self, repo: &Path) -> Option<&Detected> {
        match self.entries.get(&key(repo))? {
            Entry::Reading => None,
            Entry::Proposed(detected) | Entry::Dismissed(detected) => Some(detected),
        }
    }

    /// The detection to offer on the card: a repo with no saved profile whose card is still open.
    pub(crate) fn proposal(&self, repo: &Path) -> Option<&Detected> {
        if self.confirmed(repo) {
            return None;
        }
        match self.entries.get(&key(repo))? {
            Entry::Proposed(detected) => Some(detected),
            Entry::Reading | Entry::Dismissed(_) => None,
        }
    }

    pub(crate) fn confirmed(&self, repo: &Path) -> bool {
        self.store.as_ref().is_some_and(|store| store.get(repo).is_some())
    }

    pub(crate) fn accept(&mut self, repo: &Path) {
        if let Some(detected) = self.proposal(repo) {
            let workflow = detected.workflow();
            self.set(repo, workflow);
        }
    }

    /// The card goes away until the next launch; the detected values stay in use.
    pub(crate) fn dismiss(&mut self, repo: &Path) {
        let at = key(repo);
        if let Some(Entry::Proposed(detected)) = self.entries.remove(&at) {
            self.entries.insert(at, Entry::Dismissed(detected));
        }
    }

    /// Saves the owner's choice for `repo`: it counts as confirmed.
    pub(crate) fn set(&mut self, repo: &Path, workflow: Workflow) {
        self.dismiss(repo);
        if let Some(store) = &mut self.store {
            store.set(repo, workflow);
        }
        let Some(paths) = self.paths.clone().filter(|_| self.mode == Mode::Live) else { return };
        let repo = repo.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) = Store::update(&paths, |store| store.set(&repo, workflow)) {
                eprintln!("tidewatch: cannot save workflow.toml: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests;
