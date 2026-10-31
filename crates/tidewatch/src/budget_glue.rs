//! `Host`: the token budget of the window: the plan limits, save mode and its switch. Nothing here
//! reads a limit from Claude Code yet (see `save_mode`), so in a real run the limits stay unknown
//! and save mode never starts; `TIDEWATCH_FAKE_LIMITS` stands in a reading for captures.

use tidewatch_core::save_mode::{Notice, PlanLimits, SaveMode, Window};
use tidewatch_core::token_usage::{Share, Tally, Usage};

use crate::token_budget::{TabTokens, tab_tokens};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

#[derive(Debug)]
pub(crate) struct Host {
    enabled: bool,
    limits: Option<PlanLimits>,
    mode: SaveMode,
}

impl Host {
    pub(crate) fn new(enabled: bool, now_ms: i64) -> Self {
        let limits =
            std::env::var("TIDEWATCH_FAKE_LIMITS").ok().and_then(|text| fake(&text, now_ms));
        Self { enabled, limits, mode: SaveMode::default() }
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

    /// Whether Tidewatch holds back what it sends on its own right now.
    pub(crate) fn saving(&self) -> bool {
        self.mode.saving().is_some()
    }

    /// One look per frame: the notice to show the first time save mode starts for a window.
    pub(crate) fn tick(&mut self, now_ms: i64) -> Option<Notice> {
        self.mode.step(self.enabled, self.limits, now_ms)
    }
}

/// `TIDEWATCH_FAKE_TOKENS=1`: every agent tab reads as having spent 1.2M tokens, 14k of them on
/// four messages of Tidewatch's, for captures. Nothing is read or saved.
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

    #[test]
    fn save_mode_follows_the_limits_and_the_switch() {
        let mut host =
            Host { enabled: true, limits: fake("85@2h,10@3d", NOW), mode: SaveMode::default() };
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
        let mut host = Host { enabled: true, limits: None, mode: SaveMode::default() };
        assert_eq!(host.tick(NOW), None);
        assert!(!host.saving());
    }
}
