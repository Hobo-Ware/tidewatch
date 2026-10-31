//! The weekly tally behind "Panel on time" (pure): per source, how many changes reached a tab's
//! panel within their limit, how many came late and how many a backup source had to catch. The
//! week is a UTC week starting Monday; a clock that jumps backward neither rolls the week over
//! nor makes a change late.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::panel_event::Source;
use crate::panel_limits::Delivery;

const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    OnTime,
    Late,
    /// The source never said it; another source caught the change.
    Missed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub on_time: u32,
    pub late: u32,
    pub missed: u32,
}

impl Counts {
    #[must_use]
    pub fn total(self) -> u32 {
        self.on_time + self.late + self.missed
    }

    #[must_use]
    pub fn failed(self) -> u32 {
        self.late + self.missed
    }

    fn add(&mut self, other: Self) {
        self.on_time += other.on_time;
        self.late += other.late;
        self.missed += other.missed;
    }

    fn bump(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::OnTime => self.on_time += 1,
            Outcome::Late => self.late += 1,
            Outcome::Missed => self.missed += 1,
        }
    }
}

/// The Monday (as a day number since 1970-01-01) of the UTC week `ms` falls in.
#[must_use]
pub fn week_of(ms: i64) -> i64 {
    let day = ms.div_euclid(DAY_MS);
    day - (day + 3).rem_euclid(7)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counter {
    week: Option<i64>,
    sources: BTreeMap<Source, Counts>,
}

impl Counter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A file that is missing, cut or not a tally is an empty tally.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn roll(&mut self, now_ms: i64) {
        let week = week_of(now_ms);
        match self.week {
            Some(known) if week <= known => {}
            _ => {
                self.week = Some(week);
                self.sources.clear();
            }
        }
    }

    pub fn record(&mut self, now_ms: i64, source: Source, outcome: Outcome) {
        self.roll(now_ms);
        self.sources.entry(source).or_default().bump(outcome);
    }

    /// Counts a change folded into a panel; an event that changed nothing is not a change.
    pub fn record_delivery(&mut self, source: Source, delivery: &Delivery) {
        if !delivery.changed {
            return;
        }
        let outcome = if delivery.is_on_time() { Outcome::OnTime } else { Outcome::Late };
        self.record(delivery.folded_ms, source, outcome);
    }

    /// Adds `other` to this tally when both are of the same week; the newer week wins otherwise.
    pub fn merge(&mut self, other: Self) {
        match (self.week, other.week) {
            (_, None) => {}
            (Some(mine), Some(theirs)) if theirs < mine => {}
            (Some(mine), Some(theirs)) if theirs == mine => {
                for (source, counts) in other.sources {
                    self.sources.entry(source).or_default().add(counts);
                }
            }
            _ => *self = other,
        }
    }

    fn is_this_week(&self, now_ms: i64) -> bool {
        self.week.is_some_and(|week| week_of(now_ms) <= week)
    }

    #[must_use]
    pub fn counts(&self, now_ms: i64) -> Counts {
        if !self.is_this_week(now_ms) {
            return Counts::default();
        }
        self.sources.values().fold(Counts::default(), |mut all, counts| {
            all.add(*counts);
            all
        })
    }

    /// Share of this week's changes on time, `None` before there was a change.
    #[must_use]
    pub fn percent(&self, now_ms: i64) -> Option<f64> {
        let counts = self.counts(now_ms);
        (counts.total() > 0).then(|| f64::from(counts.on_time) * 100.0 / f64::from(counts.total()))
    }

    /// The source with the most late and missed changes this week, the first on a tie.
    #[must_use]
    pub fn worst(&self, now_ms: i64) -> Option<(Source, u32)> {
        if !self.is_this_week(now_ms) {
            return None;
        }
        self.sources
            .iter()
            .map(|(source, counts)| (*source, counts.failed()))
            .filter(|(_, failed)| *failed > 0)
            .fold(None, |best: Option<(Source, u32)>, next| match best {
                Some(known) if known.1 >= next.1 => Some(known),
                _ => Some(next),
            })
    }
}

#[must_use]
pub const fn label(source: Source) -> &'static str {
    match source {
        Source::Hook => "hooks",
        Source::Transcript => "transcript",
        Source::Process => "process",
        Source::Terminal => "terminal",
        Source::Report => "reports",
        Source::Job => "jobs",
        Source::Link => "links",
        Source::Git => "git",
        Source::Pr => "pull requests",
        Source::Tidewatch => "tidewatch",
        Source::App => "app",
    }
}

#[must_use]
pub fn load(file: &Path) -> Counter {
    std::fs::read_to_string(file).map(|text| Counter::parse(&text)).unwrap_or_default()
}

pub fn save(file: &Path, counter: &Counter) -> std::io::Result<()> {
    fsutil::write_atomic(file, &counter.to_text())
}

#[cfg(test)]
mod tests;
