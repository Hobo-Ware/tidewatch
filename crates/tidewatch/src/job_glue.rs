//! The app's side of long jobs: per-tab boards fed by the progress file, `report_job` and the terminal.
//! `TIDEWATCH_FAKE_JOBS=1` gives every agent tab six fake jobs and writes nothing.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use stdusk_core::terminal::GridSnap;
use tidewatch_core::background::{Launch, Signal, Watch};
use tidewatch_core::job::{JobBoard, JobState, Report};
use tidewatch_core::job_bar;
use tidewatch_core::job_feed::Feed;
use tidewatch_core::job_store;
use tidewatch_core::paths::{jobs_file_beside, progress_feed_file_beside};

use crate::jobs::Jobs;

const POLL_EVERY_MS: i64 = 1_000;
const RUNNING_POLL_MS: i64 = 2_000;
const SAVE_EVERY_MS: i64 = 3_000;
const SCAN_EVERY_MS: i64 = 1_000;
const SEEN_LINES_CAP: usize = 4_096;
const MIN_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) notes_file: &'a Path,
}

pub(crate) enum Done {
    Loaded { tab: u64, board: Option<Box<JobBoard>>, feed: Feed },
    Saved,
}

struct Files {
    jobs: PathBuf,
    feed: PathBuf,
}

#[derive(Default)]
struct Scan {
    output: u64,
    at_ms: i64,
    seen: HashSet<u64>,
}

#[derive(Default)]
struct Tab {
    board: JobBoard,
    feed: Feed,
    files: Option<Files>,
    loaded: bool,
    pending: Vec<Report>,
    polled_ms: i64,
    dirty: bool,
    saved_ms: i64,
    scan: Scan,
    watch: Watch,
    last_report_ms: Option<i64>,
}

pub(crate) struct Host {
    tabs: HashMap<u64, Tab>,
    jobs: Jobs<Done>,
    writable: bool,
    fake: bool,
}

impl Host {
    pub(crate) fn new(writable: bool, fake: bool) -> Self {
        Self { tabs: HashMap::new(), jobs: Jobs::new(), writable: writable && !fake, fake }
    }

    pub(crate) fn from_env(writable: bool) -> Self {
        let fake =
            std::env::var_os("TIDEWATCH_FAKE_JOBS").is_some_and(|v| !v.is_empty() && v != "0");
        Self::new(writable, fake)
    }

    pub(crate) fn board(&self, tab: u64) -> Option<&JobBoard> {
        self.tabs.get(&tab).map(|tab| &tab.board).filter(|board| !board.is_empty())
    }

    pub(crate) fn report(&mut self, tab: u64, report: Report, now_ms: i64) {
        let Some(tab) = self.tabs.get_mut(&tab) else { return };
        tab.last_report_ms = Some(now_ms);
        if tab.loaded {
            tab.dirty |= tab.board.report(&report, now_ms);
        } else {
            tab.pending.push(report);
        }
    }

    pub(crate) fn background(&mut self, tab: u64, signal: Signal, now_ms: i64) {
        if let Some(tab) = self.tabs.get_mut(&tab) {
            tab.watch.on_signal(signal, now_ms);
        }
    }

    pub(crate) fn background_due(&self, tab: u64, idle: bool, now_ms: i64) -> Option<Launch> {
        let tab = self.tabs.get(&tab)?;
        tab.watch.due(tab.last_report_ms, idle, now_ms).cloned()
    }

    pub(crate) fn background_nudged(&mut self, tab: u64, tool_use_id: &str) {
        if let Some(tab) = self.tabs.get_mut(&tab) {
            tab.watch.nudged(tool_use_id);
        }
    }

    pub(crate) fn background_silent(&self, tab: u64, now_ms: i64) -> Vec<Launch> {
        let Some(tab) = self.tabs.get(&tab) else { return Vec::new() };
        tab.watch.silent(tab.last_report_ms, now_ms).into_iter().cloned().collect()
    }

    pub(crate) fn tick(
        &mut self,
        tabs: &[Facts<'_>],
        ctx: &egui::Context,
        now_ms: i64,
    ) -> Option<i64> {
        self.tabs.retain(|id, _| tabs.iter().any(|facts| facts.id == *id));
        for done in self.jobs.drain() {
            if let Done::Loaded { tab, board, feed } = done {
                self.loaded(tab, board.map(|board| *board), feed, now_ms);
            }
        }
        for facts in tabs {
            if !self.tabs.contains_key(&facts.id) {
                self.open(facts, ctx, now_ms);
            }
        }
        let mut wake: Option<i64> = None;
        let mut soonest = |at: i64| wake = Some(wake.map_or(at, |first| first.min(at)));
        for tab in self.tabs.values_mut() {
            if !tab.loaded {
                continue;
            }
            if now_ms - tab.polled_ms >= POLL_EVERY_MS
                && let Some(files) = &tab.files
            {
                tab.polled_ms = now_ms;
                for report in tab.feed.poll(&files.feed) {
                    tab.last_report_ms = Some(now_ms);
                    tab.dirty |= tab.board.report(&report, now_ms);
                }
            }
            tab.dirty |= tab.board.tick(now_ms);
            if tab.board.has_running() {
                soonest(now_ms + RUNNING_POLL_MS);
            }
            if let Some(at) = tab.board.next_change_at() {
                soonest(at);
            }
            if let Some(at) = tab.watch.next_change_ms(tab.last_report_ms, now_ms) {
                soonest(at);
            }
            if tab.dirty && self.writable {
                let due = tab.saved_ms + SAVE_EVERY_MS;
                if now_ms >= due {
                    tab.dirty = false;
                    tab.saved_ms = now_ms;
                    if let Some(files) = &tab.files {
                        let (file, board) = (files.jobs.clone(), tab.board.clone());
                        self.jobs.spawn(ctx, move || {
                            let _ = job_store::save(&file, &board);
                            Done::Saved
                        });
                    }
                } else {
                    soonest(due);
                }
            }
        }
        wake
    }

    fn open(&mut self, facts: &Facts<'_>, ctx: &egui::Context, now_ms: i64) {
        let files = Files {
            jobs: jobs_file_beside(facts.notes_file),
            feed: progress_feed_file_beside(facts.notes_file),
        };
        let mut tab = Tab { polled_ms: now_ms, saved_ms: now_ms, ..Tab::default() };
        if self.fake {
            tab.board = fake_board(now_ms);
            tab.loaded = true;
        } else {
            let (id, jobs_file, feed_file) = (facts.id, files.jobs.clone(), files.feed.clone());
            self.jobs.spawn(ctx, move || {
                let board = job_store::load(&jobs_file);
                let feed =
                    if board.is_some() { Feed::from_end(&feed_file) } else { Feed::default() };
                Done::Loaded { tab: id, board: board.map(Box::new), feed }
            });
        }
        tab.files = Some(files);
        self.tabs.insert(facts.id, tab);
    }

    fn loaded(&mut self, id: u64, board: Option<JobBoard>, feed: Feed, now_ms: i64) {
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if let Some(board) = board {
            tab.board = board;
        }
        tab.feed = feed;
        tab.loaded = true;
        for report in std::mem::take(&mut tab.pending) {
            tab.dirty |= tab.board.report(&report, now_ms);
        }
        tab.polled_ms = 0;
    }

    pub(crate) fn scan(
        &mut self,
        id: u64,
        output: u64,
        now_ms: i64,
        rows: impl FnOnce() -> Vec<String>,
    ) -> Option<i64> {
        let tab = self.tabs.get_mut(&id).filter(|tab| tab.loaded)?;
        if output == tab.scan.output {
            return None;
        }
        let due = tab.scan.at_ms + SCAN_EVERY_MS;
        if now_ms < due {
            return Some(due);
        }
        tab.scan.output = output;
        tab.scan.at_ms = now_ms;
        for line in rows() {
            let mut hasher = DefaultHasher::new();
            line.hash(&mut hasher);
            if !tab.scan.seen.insert(hasher.finish()) {
                continue;
            }
            if let Some(report) = job_bar::parse(&line) {
                tab.dirty |= tab.board.report_known(&report, now_ms);
            }
        }
        if tab.scan.seen.len() > SEEN_LINES_CAP {
            tab.scan.seen.clear();
        }
        None
    }
}

pub(crate) fn screen_rows(snap: &GridSnap) -> Vec<String> {
    snap.cells
        .chunks(snap.cols.max(1))
        .map(|row| row.iter().map(|cell| cell.c).filter(|c| *c != '\0').collect::<String>())
        .map(|text| text.trim_end().to_owned())
        .collect()
}

pub(crate) fn fake_board(now_ms: i64) -> JobBoard {
    let mut board = JobBoard::default();
    let rates = [300.0, 340.0, 330.0, 370.0, 360.0, 400.0, 390.0, 420.0, 410.0, 430.0, 424.0];
    let mut done = 0.0;
    let start = now_ms - 30 * MIN_MS;
    for (index, rate) in rates.iter().enumerate() {
        let at = start + (index as i64) * 3 * MIN_MS;
        board.report(
            &Report { done: Some(done), total: Some(58_650.0), ..Report::named("Backfill orders") },
            at,
        );
        done += rate * 3.0;
    }
    board.report(
        &Report {
            done: Some(12_431.0),
            total: Some(58_650.0),
            unit: Some("rows".into()),
            ..Report::named("Backfill orders")
        },
        now_ms,
    );
    board.report(
        &Report {
            done: Some(3.0),
            total: Some(5.0),
            unit: Some("checks".into()),
            phase: Some("macOS running".into()),
            eta_s: Some(180),
            ..Report::named("PR checks")
        },
        now_ms - MIN_MS,
    );
    for (minutes_ago, count) in [(20, 1_000.0), (6, 21_370.0)] {
        board.report(
            &Report {
                done: Some(count),
                total: Some(21_940.0),
                unit: Some("users".into()),
                ..Report::named("Import users")
            },
            now_ms - minutes_ago * MIN_MS,
        );
    }
    board.report(
        &Report { done: Some(0.0), unit: Some("passed".into()), ..Report::named("cargo test") },
        now_ms - 4 * MIN_MS,
    );
    board.report(&Report::named("Worktree setup"), now_ms - 2 * MIN_MS);
    board.report(
        &Report { eta_s: Some(1_200), ..Report::named("Deploy staging") },
        now_ms - 6 * MIN_MS,
    );
    board.report(
        &Report { done: Some(2_246.0), state: Some(JobState::Done), ..Report::named("cargo test") },
        now_ms,
    );
    board.tick(now_ms);
    board
}

#[cfg(test)]
mod tests;
