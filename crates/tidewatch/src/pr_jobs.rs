//! The gh, git and disk work behind the PR cards, on `Jobs` threads (PLAN.md, D4 follow-up): read a
//! tab's saved cards, look at its pull request, count the commits an agent made after a message, and
//! re-run failed checks. Each function starts one job and returns at once; its `Done` comes back
//! through `Jobs::drain`. `Saver` writes the cards on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::pr_cards::Cards;
use tidewatch_core::pr_gh::{self, Polled};
use tidewatch_core::setup_recipe;
use tidewatch_core::tools::{GhError, Tools};
use tidewatch_core::worktree::Worktree;
use tidewatch_core::worktree_cleanup::{self, Report};

use crate::jobs::Jobs;
use crate::pr_watch::Look;

pub(crate) enum Done {
    Loaded {
        tab: u64,
        cards: Box<Cards>,
    },
    Looked {
        tab: u64,
        result: Result<Box<Polled>, GhError>,
    },
    Committed {
        tab: u64,
        found: Option<(String, usize)>,
    },
    AfterMerge {
        tab: u64,
        commits: Option<usize>,
    },
    Reran {
        tab: u64,
        result: Result<(), GhError>,
    },
    /// `remove` is set when the check was the last look before a removal the user asked for.
    Checked {
        tab: u64,
        remove: bool,
        result: Result<Report, String>,
    },
}

pub(crate) fn load(jobs: &Jobs<Done>, ctx: &egui::Context, tab: u64, file: PathBuf) {
    jobs.spawn(ctx, move || Done::Loaded { tab, cards: Box::new(Cards::load(&file)) });
}

pub(crate) fn look(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    look: Look,
    now_ms: i64,
) {
    jobs.spawn(ctx, move || Done::Looked {
        tab,
        result: pr_gh::poll(&tools, &dir, look.number, look.prev.as_ref(), now_ms).map(Box::new),
    });
}

/// How many commits the worktree has beyond `remote_head`, after the agent finished a message.
pub(crate) fn committed(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    remote_head: String,
) {
    jobs.spawn(ctx, move || Done::Committed {
        tab,
        found: pr_gh::unpushed(&tools, &dir, &remote_head),
    });
}

/// How many commits the worktree has beyond the head its pull request merged with.
pub(crate) fn after_merge(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    merged_head: String,
) {
    jobs.spawn(ctx, move || Done::AfterMerge {
        tab,
        commits: pr_gh::unpushed(&tools, &dir, &merged_head).map(|(_, commits)| commits),
    });
}

/// Re-runs the failed jobs of `run`. Writes to GitHub; only the Re-run button starts it.
pub(crate) fn rerun(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    run: u64,
) {
    jobs.spawn(ctx, move || Done::Reran { tab, result: pr_gh::rerun_failed(&tools, &dir, run) });
}

/// Is the merged tab's worktree safe to remove? Local git and disk only.
pub(crate) fn check_cleanup(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    tab: u64,
    worktree: Worktree,
    pr_head: Option<String>,
    remove: bool,
) {
    jobs.spawn(ctx, move || {
        let copied = setup_recipe::recipe_for(&paths, &worktree.repo).copy;
        let result = worktree_cleanup::check(&worktree, pr_head.as_deref(), &copied);
        Done::Checked { tab, remove, result }
    });
}

/// Writes cards off the UI thread. A newer copy of the same file replaces one still waiting.
pub(crate) struct Saver {
    sender: Option<Sender<(PathBuf, Cards)>>,
    worker: Option<JoinHandle<()>>,
}

impl Saver {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || write_loop(&receiver));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    pub(crate) fn save(&self, file: &Path, cards: Cards) {
        if let Some(sender) = &self.sender {
            let _ = sender.send((file.to_path_buf(), cards));
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_loop(receiver: &Receiver<(PathBuf, Cards)>) {
    while let Ok(first) = receiver.recv() {
        let mut latest = vec![first];
        for (file, cards) in receiver.try_iter() {
            match latest.iter_mut().find(|(known, _)| *known == file) {
                Some(slot) => slot.1 = cards,
                None => latest.push((file, cards)),
            }
        }
        for (file, cards) in latest {
            if let Err(e) = cards.save(&file) {
                eprintln!("tidewatch: cannot save {}: {e}", file.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tidewatch_core::pr_status::Body;

    fn wait_for<T: Send + 'static>(jobs: &Jobs<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(done) = jobs.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn saved_cards_come_back_through_a_load_job_and_the_saver_coalesces() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("pr_cards.json");
        let mut cards = Cards::default();
        cards.follow(42);
        cards.add(1, Body::Closed { number: 42 });
        {
            let saver = Saver::new();
            saver.save(&file, Cards::default());
            saver.save(&file, cards.clone());
        }
        let jobs = Jobs::new();
        load(&jobs, &egui::Context::default(), 3, file);
        let Done::Loaded { tab, cards: loaded } = wait_for(&jobs) else { panic!("not a load") };
        assert_eq!(tab, 3);
        assert_eq!(*loaded, cards);
    }

    #[test]
    fn a_look_at_a_gh_that_never_answers_comes_back_stopped_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_exec::write_executable_from_a_child(
            &dir.path().join("gh"),
            "#!/bin/sh\nexec sleep 30\n",
        );
        let path = std::env::join_paths(
            std::iter::once(dir.path().to_path_buf())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap();
        let tools = Tools::with_path(path).with_limit(Duration::from_millis(300));
        let jobs = Jobs::new();
        look(
            &jobs,
            &egui::Context::default(),
            1,
            tools,
            dir.path().to_path_buf(),
            Look { number: 1, prev: None },
            0,
        );
        let Done::Looked { result, .. } = wait_for(&jobs) else { panic!("not a look") };
        assert_eq!(
            result.err(),
            Some(GhError::Stopped("gh took longer than 300 ms and was stopped".to_owned()))
        );
    }

    #[test]
    fn a_look_without_gh_is_an_answer_not_a_hang() {
        let empty = tempfile::tempdir().unwrap();
        let jobs = Jobs::new();
        let tools = Tools::with_path(empty.path());
        look(
            &jobs,
            &egui::Context::default(),
            1,
            tools,
            empty.path().to_path_buf(),
            Look { number: 1, prev: None },
            0,
        );
        let Done::Looked { result, .. } = wait_for(&jobs) else { panic!("not a look") };
        assert!(matches!(result, Err(GhError::Missing)));
    }
}
