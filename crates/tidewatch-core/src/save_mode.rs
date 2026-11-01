//! Save mode (pure, table-tested): while a plan limit is near, Tidewatch holds back what it sends
//! on its own and does not start new Meeseeks. Only what Tidewatch itself spends is paused; the
//! owner's own messages and the agents already working are never touched.
//!
//! The limits are an input. Claude Code gives its 5-hour and weekly usage to its status line
//! command only; `statusline` saves what that command is given, and the app feeds the newest
//! reading in. A reading older than its own reset time is dropped here rather than trusted.

use serde::{Deserialize, Serialize};

pub const THRESHOLD_PCT: f64 = 80.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    FiveHour,
    Week,
}

impl Which {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::FiveHour => "5-hour limit",
            Self::Week => "weekly limit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub used_pct: f64,
    pub resets_at_ms: Option<i64>,
}

impl Window {
    fn live(self, now_ms: i64) -> bool {
        self.used_pct.is_finite() && self.resets_at_ms.is_none_or(|at| at > now_ms)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanLimits {
    pub five_hour: Option<Window>,
    pub week: Option<Window>,
}

impl PlanLimits {
    /// The windows whose reading is still current, five-hour first.
    #[must_use]
    pub fn current(self, now_ms: i64) -> Vec<(Which, Window)> {
        [(Which::FiveHour, self.five_hour), (Which::Week, self.week)]
            .into_iter()
            .filter_map(|(which, window)| Some((which, window.filter(|w| w.live(now_ms))?)))
            .collect()
    }

    /// The fullest current window at or above the threshold.
    #[must_use]
    pub fn near(self, now_ms: i64) -> Option<(Which, Window)> {
        self.current(now_ms)
            .into_iter()
            .filter(|(_, window)| window.used_pct >= THRESHOLD_PCT)
            .max_by(|a, b| a.1.used_pct.total_cmp(&b.1.used_pct))
    }
}

/// What Tidewatch says the first time save mode starts for a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    pub which: Which,
    pub used_pct: u8,
}

impl Notice {
    #[must_use]
    pub fn text(self) -> String {
        format!(
            "Save mode is on: your {} is at {}%. Auto sends, reminders and new Meeseeks wait until it resets. Turn it off in Settings, General.",
            self.which.word(),
            self.used_pct
        )
    }
}

pub const MEESEEKS_HELD: &str = "Save mode is on, so no new Meeseeks start. Turn it off in Settings, General, or try again after the limit resets.";

#[derive(Debug, Default)]
pub struct SaveMode {
    saving: Option<(Which, Window)>,
    told_for: Option<(Which, Option<i64>)>,
}

impl SaveMode {
    /// Takes the switch and the current limits. Returns the notice the first time a window sends
    /// save mode on; the same window dipping below and rising again says nothing more.
    pub fn step(
        &mut self,
        enabled: bool,
        limits: Option<PlanLimits>,
        now_ms: i64,
    ) -> Option<Notice> {
        self.saving = limits.filter(|_| enabled).and_then(|limits| limits.near(now_ms));
        let (which, window) = self.saving?;
        let window_id = (which, window.resets_at_ms);
        if self.told_for == Some(window_id) {
            return None;
        }
        self.told_for = Some(window_id);
        let used_pct = window.used_pct.floor().clamp(0.0, 100.0) as u8;
        Some(Notice { which, used_pct })
    }

    #[must_use]
    pub fn saving(&self) -> Option<(Which, Window)> {
        self.saving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn window(used_pct: f64, resets_in_ms: i64) -> Window {
        Window { used_pct, resets_at_ms: Some(NOW + resets_in_ms) }
    }

    fn limits(five_hour: Option<Window>, week: Option<Window>) -> PlanLimits {
        PlanLimits { five_hour, week }
    }

    #[test]
    fn a_window_at_eighty_percent_or_more_is_near_and_the_fullest_one_is_named() {
        let cases = [
            (limits(Some(window(79.9, 1000)), Some(window(10.0, 1000))), None),
            (limits(Some(window(80.0, 1000)), None), Some(Which::FiveHour)),
            (limits(Some(window(62.0, 1000)), Some(window(81.0, 1000))), Some(Which::Week)),
            (limits(Some(window(95.0, 1000)), Some(window(81.0, 1000))), Some(Which::FiveHour)),
            (limits(None, None), None),
        ];
        for (limits, want) in cases {
            assert_eq!(limits.near(NOW).map(|(which, _)| which), want, "{limits:?}");
        }
    }

    #[test]
    fn a_reading_past_its_own_reset_time_is_dropped() {
        let stale = limits(Some(window(99.0, -1)), Some(window(50.0, 1000)));
        assert_eq!(stale.near(NOW), None);
        assert_eq!(stale.current(NOW).len(), 1);
    }

    #[test]
    fn a_reading_that_is_not_a_number_is_dropped() {
        assert_eq!(limits(Some(window(f64::NAN, 1000)), None).near(NOW), None);
    }

    #[test]
    fn save_mode_starts_at_the_threshold_tells_once_and_ends_below_it() {
        let mut mode = SaveMode::default();
        assert_eq!(mode.step(true, Some(limits(Some(window(62.0, 1000)), None)), NOW), None);
        assert_eq!(mode.saving(), None);
        let began = mode.step(true, Some(limits(Some(window(81.4, 1000)), None)), NOW);
        assert_eq!(began, Some(Notice { which: Which::FiveHour, used_pct: 81 }));
        assert!(mode.saving().is_some());
        assert_eq!(mode.step(true, Some(limits(Some(window(90.0, 1000)), None)), NOW), None);
        assert!(mode.saving().is_some());
        assert_eq!(mode.step(true, Some(limits(Some(window(70.0, 1000)), None)), NOW), None);
        assert_eq!(mode.saving(), None);
        assert_eq!(mode.step(true, Some(limits(Some(window(85.0, 1000)), None)), NOW), None);
        assert!(mode.saving().is_some());
    }

    #[test]
    fn a_new_window_after_a_reset_is_told_again() {
        let mut mode = SaveMode::default();
        assert!(mode.step(true, Some(limits(Some(window(85.0, 1000)), None)), NOW).is_some());
        assert!(mode.step(true, Some(limits(Some(window(85.0, 9000)), None)), NOW).is_some());
    }

    #[test]
    fn the_switch_off_or_no_reading_means_no_save_mode() {
        let mut off = SaveMode::default();
        assert_eq!(off.step(false, Some(limits(Some(window(99.0, 1000)), None)), NOW), None);
        assert_eq!(off.saving(), None);
        let mut unknown = SaveMode::default();
        assert_eq!(unknown.step(true, None, NOW), None);
        assert_eq!(unknown.saving(), None);
    }

    #[test]
    fn turning_it_off_while_saving_ends_it_at_once_and_turning_it_on_does_not_tell_twice() {
        let mut mode = SaveMode::default();
        let high = Some(limits(Some(window(85.0, 1000)), None));
        assert!(mode.step(true, high, NOW).is_some());
        assert_eq!(mode.step(false, high, NOW), None);
        assert_eq!(mode.saving(), None);
        assert_eq!(mode.step(true, high, NOW), None);
        assert!(mode.saving().is_some());
    }

    #[test]
    fn the_notice_names_the_window_what_waits_and_where_to_turn_it_off() {
        let text = Notice { which: Which::Week, used_pct: 83 }.text();
        assert_eq!(
            text,
            "Save mode is on: your weekly limit is at 83%. Auto sends, reminders and new Meeseeks wait until it resets. Turn it off in Settings, General."
        );
    }
}
