//! The "taking long" line of the status bar: which running call it names, its words, whether it
//! fits beside the other line and when the window needs its next frame. Pure.

use std::time::{Duration, Instant};

use tidewatch_core::limited::{Running, SLOW_AFTER};

const BUTTON_GAP: f32 = 10.0;
const TICK: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) id: u64,
    pub(crate) label: String,
    pub(crate) elapsed: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) id: u64,
    pub(crate) text: String,
}

pub(crate) fn calls(running: &[Running], now: Instant) -> Vec<Call> {
    running
        .iter()
        .map(|call| Call {
            id: call.id,
            label: call.label.clone(),
            elapsed: now.saturating_duration_since(call.started),
        })
        .collect()
}

/// `14 s`, `2 min 5 s`.
pub(crate) fn elapsed_text(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 { format!("{secs} s") } else { format!("{} min {} s", secs / 60, secs % 60) }
}

pub(crate) fn text(label: &str, elapsed: Duration, more: usize) -> String {
    let tail = if more == 0 { String::new() } else { format!(" \u{b7} and {more} more") };
    format!("{label} is taking long \u{b7} {}{tail}", elapsed_text(elapsed))
}

pub(crate) fn pick(calls: &[Call]) -> Option<Line> {
    let slow: Vec<&Call> = calls.iter().filter(|call| call.elapsed >= SLOW_AFTER).collect();
    let oldest = slow.iter().copied().reduce(|a, b| if b.elapsed > a.elapsed { b } else { a })?;
    Some(Line { id: oldest.id, text: text(&oldest.label, oldest.elapsed, slow.len() - 1) })
}

pub(crate) fn next_look(calls: &[Call]) -> Option<Duration> {
    let oldest = calls.iter().map(|call| call.elapsed).max()?;
    Some(SLOW_AFTER.checked_sub(oldest).filter(|left| !left.is_zero()).unwrap_or(TICK))
}

pub(crate) fn fits(x: f32, text_w: f32, cancel_w: f32, limit: f32) -> bool {
    x + text_w + BUTTON_GAP + cancel_w <= limit
}

#[cfg(test)]
mod tests;
