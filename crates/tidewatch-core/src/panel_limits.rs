//! How long each source may stay quiet, and how late a change may arrive and still count as on
//! time (pure). One table, so the "no word since" mark and the live counter agree.

use crate::panel_event::{Fact, Source};

/// "At once": a fact the app learns where it happens, folded within one frame's slack.
pub const AT_ONCE_MS: i64 = 250;
/// A pull request the app polls instead of hearing live may arrive this late.
pub const PR_POLLED_MS: i64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub source: Source,
    /// The source should have spoken within this long of the moment it is expected to: a turn
    /// starting (hook, as the backup window), a turn running (transcript, job) or ending (git), a
    /// pull request being open (pr). `None`: no one expects it to speak on a schedule.
    pub quiet_after_ms: Option<i64>,
    pub on_time_ms: i64,
}

pub const LIMITS: [Limit; 11] = [
    Limit { source: Source::Hook, quiet_after_ms: Some(2_000), on_time_ms: 2_000 },
    Limit { source: Source::Transcript, quiet_after_ms: Some(15_000), on_time_ms: 2_000 },
    Limit { source: Source::Process, quiet_after_ms: None, on_time_ms: 2_000 },
    Limit { source: Source::Terminal, quiet_after_ms: None, on_time_ms: AT_ONCE_MS },
    Limit { source: Source::Report, quiet_after_ms: None, on_time_ms: AT_ONCE_MS },
    Limit { source: Source::Job, quiet_after_ms: Some(60_000), on_time_ms: 1_000 },
    Limit { source: Source::Link, quiet_after_ms: None, on_time_ms: 2_000 },
    Limit { source: Source::Git, quiet_after_ms: Some(2_000), on_time_ms: 2_000 },
    Limit { source: Source::Pr, quiet_after_ms: Some(120_000), on_time_ms: 10_000 },
    Limit { source: Source::Tidewatch, quiet_after_ms: None, on_time_ms: AT_ONCE_MS },
    Limit { source: Source::App, quiet_after_ms: None, on_time_ms: AT_ONCE_MS },
];

const NO_LIMIT: Limit = Limit { source: Source::App, quiet_after_ms: None, on_time_ms: AT_ONCE_MS };

#[must_use]
pub fn limit(source: Source) -> &'static Limit {
    LIMITS.iter().find(|limit| limit.source == source).unwrap_or(&NO_LIMIT)
}

/// How late a fact may be folded after it happened and still be on time. A question and a check
/// Tidewatch ran are at once whatever source told of them.
#[must_use]
pub fn on_time_ms(source: Source, fact: &Fact, polled: bool) -> i64 {
    match fact {
        Fact::AskOpened { .. }
        | Fact::PermissionOpened { .. }
        | Fact::Answered { .. }
        | Fact::QuestionAnswered { .. }
        | Fact::Check { .. } => AT_ONCE_MS,
        Fact::Pr(_) if polled => PR_POLLED_MS,
        _ => limit(source).on_time_ms,
    }
}

/// One change to the panel: when it happened at its source, when it was folded, how late is on time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub changed: bool,
    pub happened_ms: i64,
    pub folded_ms: i64,
    pub limit_ms: i64,
}

impl Delivery {
    #[must_use]
    pub fn of(
        source: Source,
        fact: &Fact,
        polled: bool,
        happened_ms: i64,
        folded_ms: i64,
        changed: bool,
    ) -> Self {
        Self { changed, happened_ms, folded_ms, limit_ms: on_time_ms(source, fact, polled) }
    }

    #[must_use]
    pub fn is_on_time(&self) -> bool {
        (self.folded_ms - self.happened_ms).max(0) <= self.limit_ms
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub changes: u32,
    pub on_time: u32,
}

impl Tally {
    /// Share of changes folded on time, `None` before there was a change.
    #[must_use]
    pub fn percent(&self) -> Option<f64> {
        (self.changes > 0).then(|| f64::from(self.on_time) * 100.0 / f64::from(self.changes))
    }
}

/// Counts the changes among `deliveries` and how many were on time; an event that changed nothing
/// is not a change and counts for neither.
#[must_use]
pub fn on_time(deliveries: &[Delivery]) -> Tally {
    deliveries.iter().filter(|delivery| delivery.changed).fold(Tally::default(), |tally, d| Tally {
        changes: tally.changes + 1,
        on_time: tally.on_time + u32::from(d.is_on_time()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_event::{ChecksWord, PrFacts, PrWord};

    #[test]
    fn every_source_has_exactly_one_limit() {
        for source in Source::ALL {
            assert_eq!(LIMITS.iter().filter(|l| l.source == source).count(), 1, "{source:?}");
            assert_eq!(limit(source).source, source);
        }
    }

    #[test]
    fn the_board_limits_are_in_the_table() {
        let quiet = |source| limit(source).quiet_after_ms;
        assert_eq!(quiet(Source::Hook), Some(2_000));
        assert_eq!(quiet(Source::Transcript), Some(15_000));
        assert_eq!(quiet(Source::Job), Some(60_000));
        assert_eq!(quiet(Source::Git), Some(2_000));
        assert_eq!(quiet(Source::Pr), Some(120_000));
        assert_eq!(quiet(Source::Report), None);
        let late = |source| limit(source).on_time_ms;
        assert_eq!(late(Source::Hook), 2_000);
        assert_eq!(late(Source::Transcript), 2_000);
        assert_eq!(late(Source::Git), 2_000);
        assert_eq!(late(Source::Job), 1_000);
        assert_eq!(late(Source::Pr), 10_000);
    }

    #[test]
    fn a_question_a_check_and_a_polled_pull_request_have_their_own_limits() {
        let pr =
            Fact::Pr(PrFacts { number: 1, state: PrWord::Open, checks: Some(ChecksWord::Running) });
        let ask = Fact::AskOpened { summary: String::new(), n: 1 };
        let check = Fact::Check { name: "tests".into(), passed: true };
        let cases = [
            (Source::Pr, &pr, false, 10_000),
            (Source::Pr, &pr, true, PR_POLLED_MS),
            (Source::Hook, &ask, false, AT_ONCE_MS),
            (Source::Transcript, &ask, false, AT_ONCE_MS),
            (Source::Tidewatch, &check, false, AT_ONCE_MS),
            (Source::Job, &Fact::Output, false, 1_000),
        ];
        for (source, fact, polled, want) in cases {
            assert_eq!(on_time_ms(source, fact, polled), want, "{source:?} polled={polled}");
        }
    }

    fn change(limit_ms: i64, late_by: i64) -> Delivery {
        Delivery { changed: true, happened_ms: 10_000, folded_ms: 10_000 + late_by, limit_ms }
    }

    #[test]
    fn the_tally_counts_changes_and_the_ones_folded_within_their_limit() {
        let cases = [
            ("nothing", vec![], Tally { changes: 0, on_time: 0 }, None),
            (
                "one on time",
                vec![change(2_000, 100)],
                Tally { changes: 1, on_time: 1 },
                Some(100.0),
            ),
            (
                "exactly at the limit",
                vec![change(2_000, 2_000)],
                Tally { changes: 1, on_time: 1 },
                Some(100.0),
            ),
            (
                "one past the limit",
                vec![change(2_000, 2_001)],
                Tally { changes: 1, on_time: 0 },
                Some(0.0),
            ),
            (
                "three of four",
                vec![
                    change(1_000, 0),
                    change(1_000, 999),
                    change(2_000, 1_999),
                    change(1_000, 5_000),
                ],
                Tally { changes: 4, on_time: 3 },
                Some(75.0),
            ),
            (
                "an event that changed nothing is no change",
                vec![Delivery { changed: false, ..change(1_000, 9_000) }, change(1_000, 1)],
                Tally { changes: 1, on_time: 1 },
                Some(100.0),
            ),
            (
                "a clock that went backward is not late",
                vec![change(1_000, -30_000)],
                Tally { changes: 1, on_time: 1 },
                Some(100.0),
            ),
        ];
        for (name, deliveries, want, percent) in cases {
            let got = on_time(&deliveries);
            assert_eq!(got, want, "{name}");
            assert_eq!(got.percent(), percent, "{name}");
        }
    }

    #[test]
    fn a_delivery_takes_the_limit_of_its_source_and_fact() {
        let done = Delivery::of(Source::Job, &Fact::Output, false, 1_000, 2_500, true);
        assert_eq!(done.limit_ms, 1_000);
        assert!(!done.is_on_time());
        let live = Delivery::of(Source::Hook, &Fact::Output, false, 1_000, 2_500, true);
        assert!(live.is_on_time());
    }
}
