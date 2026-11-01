//! What the token budget says on screen (pure, table-tested): a tab's tokens and cost for its
//! tooltip and its Overview card, the plan limit meters of the status bar, and the tokens mark.

use jiff::tz::TimeZone;
use tidewatch_core::save_mode::{PlanLimits, THRESHOLD_PCT, Which, Window};
use tidewatch_core::token_usage::{self, Share, Tally};

const MINUTE_MS: i64 = 60_000;
const MIN_COST_USD: f64 = 0.005;
const SAVING_TIP: &str = "Save mode is on: Auto sends, reminders and new Meeseeks wait until the limit resets. Turn it off in Settings, General.";
const SAVE_OFF_TIP: &str =
    "Save mode is off, so Tidewatch keeps sending. Turn it on in Settings, General.";

/// A tab's tokens: the short form for a card and the lines of its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabTokens {
    pub(crate) short: String,
    pub(crate) tip: String,
}

pub(crate) fn tab_tokens(tally: &Tally, share: Share) -> Option<TabTokens> {
    let short = token_usage::short(tally)?;
    let summary = token_usage::summary(tally)?;
    let tip = match token_usage::share_line(share) {
        Some(line) => format!("{summary}\n{line}"),
        None => summary,
    };
    Some(TabTokens { short, tip })
}

/// The two meters at the right of the status bar, one per window whose reading is current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Meter {
    pub(crate) label: &'static str,
    pub(crate) percent: u8,
    /// The window is at the save-mode threshold: the fill is drawn in the warning ink.
    pub(crate) hot: bool,
    pub(crate) tip: String,
}

impl Meter {
    pub(crate) fn fraction(&self) -> f32 {
        f32::from(self.percent) / 100.0
    }

    pub(crate) fn percent_text(&self) -> String {
        format!("{}%", self.percent)
    }
}

/// `2h 10m`, `45m`: how long until a reset, never `0m`.
fn resets_in(resets_at_ms: i64, now_ms: i64) -> String {
    let minutes = ((resets_at_ms - now_ms).max(0) + MINUTE_MS - 1) / MINUTE_MS;
    let minutes = minutes.max(1);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

/// `Thu 14:00` in the local zone.
fn resets_on(resets_at_ms: i64, zone: &TimeZone) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(resets_at_ms).ok()?.to_zoned(zone.clone());
    Some(at.strftime("%a %H:%M").to_string())
}

fn meter(which: Which, window: Window, save_mode_on: bool, now_ms: i64, zone: &TimeZone) -> Meter {
    let percent = window.used_pct.floor().clamp(0.0, 100.0) as u8;
    let hot = window.used_pct >= THRESHOLD_PCT;
    let (label, name) = match which {
        Which::FiveHour => ("5h", "5-hour limit"),
        Which::Week => ("week", "Week limit"),
    };
    let reset = window.resets_at_ms.and_then(|at| match which {
        Which::FiveHour => Some(format!("resets in {}", resets_in(at, now_ms))),
        Which::Week => resets_on(at, zone).map(|when| format!("resets {when}")),
    });
    let first = match reset {
        Some(reset) => format!("{name} {percent}%, {reset}"),
        None => format!("{name} {percent}%"),
    };
    let save = match (hot, save_mode_on) {
        (true, true) => SAVING_TIP.to_owned(),
        (true, false) => SAVE_OFF_TIP.to_owned(),
        (false, _) => format!("Save mode starts at {THRESHOLD_PCT}% of a limit."),
    };
    Meter { label, percent, hot, tip: format!("{first}\n{save}") }
}

/// Every window whose reading is current, five-hour first.
pub(crate) fn meters(
    limits: Option<PlanLimits>,
    save_mode_on: bool,
    now_ms: i64,
    zone: &TimeZone,
) -> Vec<Meter> {
    limits
        .map(|limits| limits.current(now_ms))
        .unwrap_or_default()
        .into_iter()
        .map(|(which, window)| meter(which, window, save_mode_on, now_ms, zone))
        .collect()
}

/// `$20.51 this session`.
pub(crate) fn cost_line(cost_usd: f64) -> String {
    format!("${cost_usd:.2} this session")
}

/// The lines of a tab's tooltip about what it spent: its tokens, then its cost once it is a cent.
pub(crate) fn tab_tip(tab: Option<&TabTokens>, cost_usd: Option<f64>) -> Option<String> {
    let cost = cost_usd.filter(|cost| *cost >= MIN_COST_USD);
    let lines: Vec<String> =
        tab.map(|tab| tab.tip.clone()).into_iter().chain(cost.map(cost_line)).collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The one mark at the left of the status bar: the active tab's tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) text: String,
    pub(crate) tip: String,
}

pub(crate) fn bar_mark(tab: Option<&TabTokens>, cost_usd: Option<f64>) -> Option<Mark> {
    let tab = tab?;
    Some(Mark { text: tab.short.clone(), tip: tab_tip(Some(tab), cost_usd)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::token_usage::Usage;

    const NOW: i64 = 1_793_000_000_000;
    const HOUR: i64 = 3_600_000;

    fn window(used_pct: f64, resets_in_ms: i64) -> Window {
        Window { used_pct, resets_at_ms: Some(NOW + resets_in_ms) }
    }

    fn tally() -> Tally {
        Tally {
            total: Usage {
                input: 2_000,
                cache_read: 1_100_000,
                cache_write: 40_000,
                output: 58_000,
            },
            calls: 40,
            turn_count: 18,
            turns: Vec::new(),
        }
    }

    #[test]
    fn a_tab_with_tokens_has_a_short_form_and_a_tooltip_that_adds_what_tidewatch_cost() {
        assert_eq!(tab_tokens(&Tally::default(), Share::default()), None);
        let plain = tab_tokens(&tally(), Share::default()).unwrap();
        assert_eq!(plain.short, "1.2M tokens");
        assert_eq!(plain.tip, "1.2M tokens \u{b7} 96% cached \u{b7} 18 turns");
        let share = Share { messages: 4, usage: Usage { output: 14_000, ..Usage::default() } };
        let with = tab_tokens(&tally(), share).unwrap();
        assert_eq!(
            with.tip,
            "1.2M tokens \u{b7} 96% cached \u{b7} 18 turns\nTidewatch cost this tab 14k tokens (4 messages)"
        );
    }

    #[test]
    fn a_reset_is_counted_down_in_hours_and_minutes_and_is_never_zero() {
        let cases = [
            (2 * HOUR + 10 * 60_000, "2h 10m"),
            (HOUR, "1h"),
            (45 * 60_000, "45m"),
            (59 * 60_000 + 1, "1h"),
            (1_000, "1m"),
            (0, "1m"),
            (-5, "1m"),
        ];
        for (ahead, want) in cases {
            assert_eq!(resets_in(NOW + ahead, NOW), want, "{ahead}");
        }
    }

    #[test]
    fn a_week_reset_reads_as_a_weekday_and_a_clock_in_the_zone() {
        assert_eq!(resets_on(NOW + 3 * 24 * HOUR, &TimeZone::UTC).as_deref(), Some("Thu 07:33"));
        assert_eq!(resets_on(NOW + 25 * HOUR, &TimeZone::UTC).as_deref(), Some("Tue 08:33"));
    }

    fn both(five: f64, week: f64) -> PlanLimits {
        PlanLimits {
            five_hour: Some(window(five, 2 * HOUR + 10 * 60_000)),
            week: Some(window(week, 3 * 24 * HOUR)),
        }
    }

    #[test]
    fn each_current_window_is_a_meter_with_its_percent_and_its_reset_in_the_tooltip() {
        let utc = TimeZone::UTC;
        let got = meters(Some(both(31.4, 70.0)), true, NOW, &utc);
        assert_eq!(
            got.iter().map(|m| (m.label, m.percent_text(), m.hot)).collect::<Vec<_>>(),
            [("5h", "31%".to_owned(), false), ("week", "70%".to_owned(), false)]
        );
        assert_eq!(
            got[0].tip,
            "5-hour limit 31%, resets in 2h 10m\nSave mode starts at 80% of a limit."
        );
        assert_eq!(
            got[1].tip,
            "Week limit 70%, resets Thu 07:33\nSave mode starts at 80% of a limit."
        );
        assert_eq!(got[0].fraction(), 0.31);
    }

    #[test]
    fn a_window_with_no_reading_or_a_reset_in_the_past_has_no_meter() {
        let utc = TimeZone::UTC;
        assert_eq!(meters(None, true, NOW, &utc), Vec::<Meter>::new());
        assert_eq!(meters(Some(PlanLimits::default()), true, NOW, &utc), Vec::<Meter>::new());
        let week_only = PlanLimits { five_hour: None, week: both(0.0, 12.0).week };
        assert_eq!(meters(Some(week_only), true, NOW, &utc).len(), 1);
        let stale = PlanLimits {
            five_hour: Some(window(99.0, -HOUR)),
            week: Some(window(50.0, 3 * 24 * HOUR)),
        };
        let got = meters(Some(stale), true, NOW, &utc);
        assert_eq!(got.iter().map(|m| m.label).collect::<Vec<_>>(), ["week"]);
    }

    #[test]
    fn a_meter_turns_hot_at_the_save_mode_threshold_and_says_what_save_mode_does() {
        let utc = TimeZone::UTC;
        let cases = [(79.9, false), (80.0, true), (84.0, true), (100.0, true)];
        for (used, hot) in cases {
            let got = meters(Some(both(10.0, used)), true, NOW, &utc);
            assert_eq!((got[0].hot, got[1].hot), (false, hot), "{used}");
        }
        let on = meters(Some(both(10.0, 84.0)), true, NOW, &utc);
        assert_eq!(on[1].tip.lines().nth(1), Some(SAVING_TIP));
        let off = meters(Some(both(10.0, 84.0)), false, NOW, &utc);
        assert_eq!(off[1].tip.lines().nth(1), Some(SAVE_OFF_TIP));
        assert_eq!(on[1].percent_text(), "84%");
    }

    #[test]
    fn a_percent_shows_whole_and_never_rounds_up_into_the_next_threshold() {
        let utc = TimeZone::UTC;
        let got = meters(Some(both(79.99, 100.4)), true, NOW, &utc);
        assert_eq!((got[0].percent, got[0].hot), (79, false));
        assert_eq!(got[1].percent, 100);
    }

    #[test]
    fn a_window_without_a_reset_time_has_a_tooltip_of_its_percent_alone() {
        let limits = PlanLimits {
            five_hour: Some(Window { used_pct: 40.0, resets_at_ms: None }),
            week: None,
        };
        let got = meters(Some(limits), true, NOW, &TimeZone::UTC);
        assert_eq!(got[0].tip.lines().next(), Some("5-hour limit 40%"));
    }

    #[test]
    fn the_cost_reads_in_dollars_and_cents_and_joins_the_tokens_in_a_tab_tooltip() {
        assert_eq!(cost_line(20.514), "$20.51 this session");
        assert_eq!(cost_line(0.004), "$0.00 this session");
        let tab = tab_tokens(&tally(), Share::default()).unwrap();
        assert_eq!(
            tab_tip(Some(&tab), Some(20.51)).as_deref(),
            Some("1.2M tokens \u{b7} 96% cached \u{b7} 18 turns\n$20.51 this session")
        );
        assert_eq!(tab_tip(None, Some(1.0)).as_deref(), Some("$1.00 this session"));
        assert_eq!(tab_tip(Some(&tab), None).as_deref(), Some(tab.tip.as_str()));
        assert_eq!(tab_tip(None, None), None);
        assert_eq!(tab_tip(None, Some(0.0)), None, "a fresh session has cost nothing yet");
        assert_eq!(tab_tip(Some(&tab), Some(0.004)).as_deref(), Some(tab.tip.as_str()));
    }

    #[test]
    fn the_bar_shows_the_active_tabs_tokens_with_its_cost_in_the_tooltip_or_nothing() {
        assert_eq!(bar_mark(None, Some(3.0)), None);
        let tab = tab_tokens(&tally(), Share::default()).unwrap();
        let mark = bar_mark(Some(&tab), None).unwrap();
        assert_eq!((mark.text.as_str(), mark.tip.as_str()), ("1.2M tokens", tab.tip.as_str()));
        let priced = bar_mark(Some(&tab), Some(20.51)).unwrap();
        assert_eq!(priced.tip.lines().last(), Some("$20.51 this session"));
    }
}
