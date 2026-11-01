//! The plan limits and session cost Tidewatch keeps from Claude Code's status line input (JSON on
//! stdin, given to a status line command only). Field names checked against claude 2.1.316:
//! `used_percentage` is 0 to 100 and `resets_at` unix seconds.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fsutil::{read_json, save_json};
use crate::save_mode::{PlanLimits, Window};

const MS_PER_SEC: f64 = 1000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    #[serde(flatten)]
    pub limits: PlanLimits,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub read_ms: i64,
}

impl Reading {
    #[must_use]
    pub fn has_data(&self) -> bool {
        self.limits != PlanLimits::default() || self.cost_usd.is_some()
    }

    /// Claude Code sends no limits before a session's first answer, so what the new input lacks
    /// stays as saved.
    #[must_use]
    pub fn over(self, before: Option<Self>) -> Self {
        let Some(before) = before else { return self };
        Self {
            limits: PlanLimits {
                five_hour: self.limits.five_hour.or(before.limits.five_hour),
                week: self.limits.week.or(before.limits.week),
            },
            cost_usd: self.cost_usd.or(before.cost_usd),
            read_ms: self.read_ms,
        }
    }
}

fn window(value: Option<&Value>) -> Option<Window> {
    let value = value?;
    let used = value.get("used_percentage")?.as_f64().filter(|used| used.is_finite())?;
    let resets_at_ms = value
        .get("resets_at")
        .and_then(Value::as_f64)
        .filter(|secs| secs.is_finite() && *secs >= 0.0)
        .map(|secs| (secs * MS_PER_SEC) as i64);
    Some(Window { used_pct: used.clamp(0.0, 100.0), resets_at_ms })
}

#[must_use]
pub fn parse(input: &str, now_ms: i64) -> Option<Reading> {
    let root: Value = serde_json::from_str(input).ok()?;
    root.as_object()?;
    let limits = root.get("rate_limits");
    Some(Reading {
        limits: PlanLimits {
            five_hour: window(limits.and_then(|limits| limits.get("five_hour"))),
            week: window(limits.and_then(|limits| limits.get("seven_day"))),
        },
        cost_usd: root
            .pointer("/cost/total_cost_usd")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0),
        read_ms: now_ms,
    })
}

#[must_use]
pub fn load(file: &Path) -> Option<Reading> {
    read_json(file)
}

pub fn save(file: &Path, reading: &Reading) -> std::io::Result<()> {
    save_json(file, reading)
}

#[must_use]
pub fn newest_limits<'a>(readings: impl IntoIterator<Item = &'a Reading>) -> Option<PlanLimits> {
    readings
        .into_iter()
        .filter(|reading| reading.limits != PlanLimits::default())
        .max_by_key(|reading| reading.read_ms)
        .map(|reading| reading.limits)
}

#[cfg(test)]
mod tests;
