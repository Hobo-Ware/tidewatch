//! `Host`: the token budget of the window: the plan limits, save mode and its switch. The limits
//! are the newest reading the status line command of any agent tab saved (`plan-limits.json` in
//! the tab's state folder, see `tidewatch_core::statusline`), read on a `Jobs` thread at most every
//! 5 s on frames that happen anyway. `TIDEWATCH_FAKE_LIMITS` stands in a reading for captures.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::paths::plan_limits_file_beside;
use tidewatch_core::save_mode::{Notice, PlanLimits, SaveMode, Window};
use tidewatch_core::statusline::{self, Reading};
use tidewatch_core::token_usage::{Share, Tally, Usage};

use crate::jobs::Jobs;
use crate::token_budget::{TabTokens, tab_tokens};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const LOOK_EVERY_MS: i64 = 5_000;
const FAKE_COST_USD: f64 = 20.51;

type Looked = Vec<(u64, Reading)>;

pub(crate) struct Host {
    enabled: bool,
    fake: Option<PlanLimits>,
    readings: HashMap<u64, Reading>,
    limits: Option<PlanLimits>,
    mode: SaveMode,
    jobs: Jobs<Looked>,
    looking: bool,
    last_look_ms: Option<i64>,
}

impl Host {
    pub(crate) fn new(enabled: bool, now_ms: i64) -> Self {
        let fake = std::env::var("TIDEWATCH_FAKE_LIMITS").ok().and_then(|text| fake(&text, now_ms));
        Self {
            enabled,
            fake,
            readings: HashMap::new(),
            limits: fake,
            mode: SaveMode::default(),
            jobs: Jobs::new(),
            looking: false,
            last_look_ms: None,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn limits(&self) -> Option<PlanLimits> {
        self.limits
    }

    /// The tab's session cost in USD, when its status line has reported one.
    pub(crate) fn cost_of(&self, tab: u64) -> Option<f64> {
        self.readings.get(&tab).and_then(|reading| reading.cost_usd).or_else(fake_cost)
    }

    /// Whether Tidewatch holds back what it sends on its own right now.
    pub(crate) fn saving(&self) -> bool {
        self.mode.saving().is_some()
    }

    /// When the soonest current window resets, so the bar can drop it without another event.
    pub(crate) fn next_reset_ms(&self, now_ms: i64) -> Option<i64> {
        self.limits?.current(now_ms).into_iter().filter_map(|(_, window)| window.resets_at_ms).min()
    }

    /// Takes what a finished look found, and starts another when the last one was 5 s ago. `tabs`
    /// are the agent tabs and the notes file each keeps its state beside.
    pub(crate) fn look(&mut self, tabs: &[(u64, PathBuf)], now_ms: i64, ctx: &egui::Context) {
        for found in self.jobs.drain() {
            self.looking = false;
            self.readings = found.into_iter().collect();
            self.limits = self.fake.or_else(|| statusline::newest_limits(self.readings.values()));
        }
        let due = self.last_look_ms.is_none_or(|at| now_ms - at >= LOOK_EVERY_MS);
        if self.looking || !due || tabs.is_empty() {
            return;
        }
        self.looking = true;
        self.last_look_ms = Some(now_ms);
        let files: Vec<(u64, PathBuf)> =
            tabs.iter().map(|(tab, notes)| (*tab, plan_limits_file_beside(notes))).collect();
        self.jobs.spawn(ctx, move || {
            files
                .into_iter()
                .filter_map(|(tab, file)| Some((tab, statusline::load(&file)?)))
                .collect()
        });
    }

    /// One look per frame: the notice to show the first time save mode starts for a window.
    pub(crate) fn tick(&mut self, now_ms: i64) -> Option<Notice> {
        self.mode.step(self.enabled, self.limits, now_ms)
    }
}

/// `TIDEWATCH_FAKE_TOKENS=1`: every agent tab reads as having spent 1.2M tokens, 14k of them on
/// four messages of Tidewatch's, and $20.51, for captures. Nothing is read or saved.
pub(crate) fn fake_tokens() -> Option<TabTokens> {
    std::env::var_os("TIDEWATCH_FAKE_TOKENS")?;
    let tally = Tally {
        total: Usage { input: 2_000, cache_read: 1_100_000, cache_write: 40_000, output: 58_000 },
        calls: 40,
        turn_count: 18,
        turns: Vec::new(),
    };
    let share = Share { messages: 4, usage: Usage { output: 14_000, ..Usage::default() } };
    tab_tokens(&tally, share)
}

fn fake_cost() -> Option<f64> {
    std::env::var_os("TIDEWATCH_FAKE_TOKENS").map(|_| FAKE_COST_USD)
}

/// `62@2h,81@3d`: the five-hour and the weekly reading as a percent and how long until it resets.
fn fake(text: &str, now_ms: i64) -> Option<PlanLimits> {
    let mut windows = text.split(',').map(|part| {
        let (percent, reset) = part.trim().split_once('@')?;
        let (number, unit) = reset.split_at(reset.len().checked_sub(1)?);
        let span = number.parse::<i64>().ok()? * if unit == "d" { DAY_MS } else { HOUR_MS };
        Some(Window { used_pct: percent.parse().ok()?, resets_at_ms: Some(now_ms + span) })
    });
    let five_hour = windows.next()?;
    let week = windows.next().unwrap_or(None);
    Some(PlanLimits { five_hour, week })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const NOW: i64 = 5_000_000;

    #[test]
    fn the_fake_reading_is_a_percent_and_a_reset_for_each_window() {
        let limits = fake("62@2h,81@3d", NOW).unwrap();
        assert_eq!(
            limits.five_hour,
            Some(Window { used_pct: 62.0, resets_at_ms: Some(NOW + 2 * HOUR_MS) })
        );
        assert_eq!(
            limits.week,
            Some(Window { used_pct: 81.0, resets_at_ms: Some(NOW + 3 * DAY_MS) })
        );
        assert_eq!(fake("90@1h", NOW).unwrap().week, None);
        assert_eq!(fake("nonsense", NOW).unwrap().five_hour, None);
    }

    fn host(enabled: bool, limits: Option<PlanLimits>) -> Host {
        Host {
            enabled,
            fake: limits,
            readings: HashMap::new(),
            limits,
            mode: SaveMode::default(),
            jobs: Jobs::new(),
            looking: false,
            last_look_ms: None,
        }
    }

    #[test]
    fn save_mode_follows_the_limits_and_the_switch() {
        let mut host = host(true, fake("85@2h,10@3d", NOW));
        assert!(!host.saving());
        let notice = host.tick(NOW).unwrap();
        assert_eq!(notice.used_pct, 85);
        assert!(host.saving());
        assert_eq!(host.tick(NOW), None);
        host.set_enabled(false);
        assert_eq!(host.tick(NOW), None);
        assert!(!host.saving());
    }

    #[test]
    fn with_no_reading_save_mode_never_starts() {
        let mut host = host(true, None);
        assert_eq!(host.tick(NOW), None);
        assert!(!host.saving());
    }

    fn save_reading(dir: &Path, name: &str, input: &str, read_ms: i64) -> PathBuf {
        let notes = dir.join(name).join("notes.json");
        let file = plan_limits_file_beside(&notes);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        statusline::save(&file, &statusline::parse(input, read_ms).unwrap()).unwrap();
        notes
    }

    fn settle(host: &mut Host, tabs: &[(u64, PathBuf)], now_ms: i64) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        host.look(tabs, now_ms, &ctx);
        while host.looking && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            host.look(tabs, now_ms, &ctx);
        }
        assert!(!host.looking, "the look never finished");
    }

    const FIVE: &str = r#"{"cost":{"total_cost_usd":20.51},"rate_limits":{"five_hour":{"used_percentage":31,"resets_at":5000}}}"#;
    const BOTH: &str = r#"{"cost":{"total_cost_usd":3.5},"rate_limits":{"five_hour":{"used_percentage":45,"resets_at":5000},"seven_day":{"used_percentage":84,"resets_at":900000}}}"#;

    #[test]
    fn the_limits_are_the_newest_reading_of_any_tab_and_each_tab_keeps_its_own_cost() {
        let dir = tempfile::tempdir().unwrap();
        let old = save_reading(dir.path(), "old", FIVE, NOW - 9_000);
        let new = save_reading(dir.path(), "new", BOTH, NOW);
        let none = dir.path().join("none").join("notes.json");
        let tabs = [(1, old), (2, new), (3, none)];
        let mut host = host(true, None);
        assert_eq!(host.limits(), None);
        settle(&mut host, &tabs, NOW);
        let limits = host.limits().unwrap();
        assert_eq!(limits.five_hour.map(|w| w.used_pct), Some(45.0));
        assert_eq!(limits.week.map(|w| w.used_pct), Some(84.0));
        assert_eq!(
            (host.cost_of(1), host.cost_of(2), host.cost_of(3)),
            (Some(20.51), Some(3.5), None)
        );
    }

    #[test]
    fn a_reading_at_the_threshold_starts_save_mode_and_its_reset_passing_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let notes = save_reading(dir.path(), "tab", BOTH, 1);
        let tabs = [(1, notes)];
        let mut host = host(true, None);
        settle(&mut host, &tabs, 1);
        assert_eq!(host.tick(1).map(|notice| notice.used_pct), Some(84));
        assert!(host.saving());
        assert_eq!(host.next_reset_ms(1), Some(5_000_000));
        assert_eq!(host.tick(900_000_001), None);
        assert!(!host.saving());
    }

    #[test]
    fn a_look_runs_at_most_every_five_seconds_and_never_without_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let notes = save_reading(dir.path(), "tab", FIVE, NOW);
        let tabs = [(1, notes.clone())];
        let mut host = host(true, None);
        host.look(&[], NOW, &egui::Context::default());
        assert_eq!(host.last_look_ms, None);
        settle(&mut host, &tabs, NOW);
        assert_eq!(host.last_look_ms, Some(NOW));
        host.look(&tabs, NOW + 4_999, &egui::Context::default());
        assert!(!host.looking);
        assert_eq!(host.last_look_ms, Some(NOW));
        statusline::save(
            &plan_limits_file_beside(&notes),
            &statusline::parse(BOTH, NOW + 1).unwrap(),
        )
        .unwrap();
        settle(&mut host, &tabs, NOW + 5_000);
        assert_eq!(host.limits().and_then(|l| l.week).map(|w| w.used_pct), Some(84.0));
    }

    #[test]
    fn a_closed_tab_no_longer_counts() {
        let dir = tempfile::tempdir().unwrap();
        let notes = save_reading(dir.path(), "tab", BOTH, NOW);
        let mut host = host(true, None);
        settle(&mut host, &[(1, notes)], NOW);
        assert!(host.limits().is_some());
        let other = dir.path().join("other").join("notes.json");
        settle(&mut host, &[(2, other)], NOW + 5_000);
        assert_eq!(host.limits(), None);
        assert_eq!(host.cost_of(1), None);
    }

    #[test]
    fn a_fake_reading_wins_over_what_the_tabs_saved() {
        let dir = tempfile::tempdir().unwrap();
        let notes = save_reading(dir.path(), "tab", FIVE, NOW);
        let mut host = host(true, fake("90@1h", NOW));
        settle(&mut host, &[(1, notes)], NOW);
        assert_eq!(host.limits().and_then(|l| l.five_hour).map(|w| w.used_pct), Some(90.0));
    }
}
