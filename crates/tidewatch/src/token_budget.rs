//! What the token budget says on screen (pure, table-tested): a tab's tokens for its tooltip and
//! its Overview card, the plan limits for the status bar, and the one mark the bar draws.

use jiff::tz::TimeZone;
use tidewatch_core::save_mode::{PlanLimits, THRESHOLD_PCT, Which, Window};
use tidewatch_core::token_usage::{self, Share, Tally};

const DAY_MS: i64 = 86_400_000;
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

/// `16:40` when the window resets within a day, else the weekday (`Mon`).
pub(crate) fn reset_text(resets_at_ms: i64, now_ms: i64, zone: &TimeZone) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(resets_at_ms).ok()?.to_zoned(zone.clone());
    let pattern = if resets_at_ms - now_ms < DAY_MS { "%H:%M" } else { "%a" };
    Some(at.strftime(pattern).to_string())
}

fn window_text(which: Which, window: Window, now_ms: i64, zone: &TimeZone) -> String {
    let label = match which {
        Which::FiveHour => "5h",
        Which::Week => "week",
    };
    let used = format!("{label} {}%", window.used_pct.round());
    match window.resets_at_ms.and_then(|at| reset_text(at, now_ms, zone)) {
        Some(reset) => format!("{used} \u{b7} resets {reset}"),
        None => used,
    }
}

/// `5h 62% · resets 16:40  ·  week 81% · resets Mon`: every window whose reading is current.
pub(crate) fn limits_line(limits: PlanLimits, now_ms: i64, zone: &TimeZone) -> Option<String> {
    let parts: Vec<String> = limits
        .current(now_ms)
        .into_iter()
        .map(|(which, window)| window_text(which, window, now_ms, zone))
        .collect();
    (!parts.is_empty()).then(|| parts.join("  \u{b7}  "))
}

/// The one mark at the left of the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) text: String,
    pub(crate) tip: String,
    /// A window is at the save-mode threshold: the mark is drawn in the warning ink.
    pub(crate) hot: bool,
}

pub(crate) struct BarInput<'a> {
    pub(crate) limits: Option<PlanLimits>,
    pub(crate) save_mode_on: bool,
    pub(crate) tab: Option<&'a TabTokens>,
}

pub(crate) fn bar_mark(input: &BarInput<'_>, now_ms: i64, zone: &TimeZone) -> Option<Mark> {
    let tab_tip = input.tab.map(|tab| tab.tip.clone());
    let Some(line) = input.limits.and_then(|limits| limits_line(limits, now_ms, zone)) else {
        return input.tab.map(|tab| Mark {
            text: tab.short.clone(),
            tip: tab.tip.clone(),
            hot: false,
        });
    };
    let near = input.limits.is_some_and(|limits| limits.near(now_ms).is_some());
    let save = match (near, input.save_mode_on) {
        (true, true) => Some(SAVING_TIP.to_owned()),
        (true, false) => Some(SAVE_OFF_TIP.to_owned()),
        (false, _) => None,
    };
    let note = format!("Save mode starts at {THRESHOLD_PCT}% of a limit.");
    let tip = [Some(line.clone()), save.or(Some(note)), tab_tip]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Some(Mark { text: line, tip: tip.join("\n"), hot: near })
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
    fn a_reset_within_a_day_reads_as_a_clock_and_a_later_one_as_a_weekday() {
        let utc = TimeZone::UTC;
        let cases = [
            (NOW + 2 * HOUR, "09:33"),
            (NOW + 23 * HOUR, "06:33"),
            (NOW + 25 * HOUR, "Tue"),
            (NOW + 3 * 24 * HOUR, "Thu"),
        ];
        for (at, want) in cases {
            assert_eq!(reset_text(at, NOW, &utc).as_deref(), Some(want), "{at}");
        }
    }

    #[test]
    fn the_limits_line_names_each_current_window_with_its_percent_and_reset() {
        let utc = TimeZone::UTC;
        let both = PlanLimits {
            five_hour: Some(window(62.4, 2 * HOUR)),
            week: Some(window(81.0, 3 * 24 * HOUR)),
        };
        assert_eq!(
            limits_line(both, NOW, &utc).as_deref(),
            Some("5h 62% \u{b7} resets 09:33  \u{b7}  week 81% \u{b7} resets Thu")
        );
        let week_only = PlanLimits { five_hour: None, week: Some(window(10.0, 3 * 24 * HOUR)) };
        assert_eq!(
            limits_line(week_only, NOW, &utc).as_deref(),
            Some("week 10% \u{b7} resets Thu")
        );
        let stale = PlanLimits { five_hour: Some(window(99.0, -HOUR)), week: None };
        assert_eq!(limits_line(stale, NOW, &utc), None);
        assert_eq!(limits_line(PlanLimits::default(), NOW, &utc), None);
    }

    fn input(limits: Option<PlanLimits>, save: bool, tab: Option<&TabTokens>) -> BarInput<'_> {
        BarInput { limits, save_mode_on: save, tab }
    }

    #[test]
    fn with_no_limits_the_bar_shows_the_tab_tokens_or_nothing() {
        let utc = TimeZone::UTC;
        assert_eq!(bar_mark(&input(None, true, None), NOW, &utc), None);
        let tab = tab_tokens(&tally(), Share::default()).unwrap();
        let mark = bar_mark(&input(None, true, Some(&tab)), NOW, &utc).unwrap();
        assert_eq!((mark.text.as_str(), mark.hot), ("1.2M tokens", false));
        assert_eq!(mark.tip, tab.tip);
    }

    #[test]
    fn with_limits_the_bar_shows_them_and_turns_hot_at_the_save_mode_threshold() {
        let utc = TimeZone::UTC;
        let calm = PlanLimits {
            five_hour: Some(window(62.0, 2 * HOUR)),
            week: Some(window(70.0, 3 * 24 * HOUR)),
        };
        let mark = bar_mark(&input(Some(calm), true, None), NOW, &utc).unwrap();
        assert!(!mark.hot);
        assert_eq!(
            mark.tip,
            "5h 62% \u{b7} resets 09:33  \u{b7}  week 70% \u{b7} resets Thu\nSave mode starts at 80% of a limit."
        );
        let near = PlanLimits { week: Some(window(81.0, 3 * 24 * HOUR)), ..calm };
        let tab = tab_tokens(&tally(), Share::default()).unwrap();
        let hot = bar_mark(&input(Some(near), true, Some(&tab)), NOW, &utc).unwrap();
        assert!(hot.hot);
        assert_eq!(hot.tip.lines().nth(1), Some(SAVING_TIP));
        assert_eq!(hot.tip.lines().last(), Some(tab.tip.as_str()));
        let off = bar_mark(&input(Some(near), false, None), NOW, &utc).unwrap();
        assert!(off.hot);
        assert_eq!(off.tip.lines().nth(1), Some(SAVE_OFF_TIP));
    }
}
