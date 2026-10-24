//! The time pill that grows left of the Amphetamine icon: the text it shows, whether it fits the
//! gap before the new tab button, and how its width eases open and shut. Pure.

use crate::amphetamine_motion::{POP, STANDARD, bezier};

pub(crate) const ICON_W: f32 = 44.0;
pub(crate) const GAP_MARGIN: f32 = 8.0;
pub(crate) const OPEN_S: f64 = 0.26;
pub(crate) const CLOSE_S: f64 = 0.2;
pub(crate) const ROLL_S: f64 = 0.38;
pub(crate) const INFINITY: &str = "\u{221e}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fit {
    Full,
    Short,
    Hidden,
}

/// The pill's two texts for `left` seconds on the timer: `1h 12m` and `1h`, or `42m` for both.
/// Minutes round up, so the pill never reads `0m` before the session ends.
pub(crate) fn clock_texts(left: f64) -> (String, String) {
    let minutes = (left / 60.0).ceil().max(1.0) as u64;
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, m) => (format!("{m}m"), format!("{m}m")),
        (h, 0) => (format!("{h}h"), format!("{h}h")),
        (h, m) => (format!("{h}h {m}m"), format!("{h}h")),
    }
}

/// The sentence for the tooltip and the menu: `42 min left`, `1 h 12 min left`.
pub(crate) fn left_text(left: f64) -> String {
    let minutes = (left / 60.0).ceil().max(0.0) as u64;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "Less than a minute left".to_owned(),
        (0, m) => format!("{m} min left"),
        (h, 0) => format!("{h} h left"),
        (h, m) => format!("{h} h {m} min left"),
    }
}

/// Full when the gap, less its margin, holds the whole pill's growth, else short, else hidden.
pub(crate) fn fit(gap: f32, full_pill: f32, short_pill: f32) -> Fit {
    let free = gap - GAP_MARGIN;
    if free >= full_pill - ICON_W {
        Fit::Full
    } else if free >= short_pill - ICON_W {
        Fit::Short
    } else {
        Fit::Hidden
    }
}

pub(crate) fn pill_width(fit: Fit, full_pill: f32, short_pill: f32) -> f32 {
    match fit {
        Fit::Full => full_pill,
        Fit::Short => short_pill,
        Fit::Hidden => ICON_W,
    }
}

/// How far open the pill is, 0 (just the icon) to 1 (its width), easing between the two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slide {
    open: bool,
    from: f64,
    since: f64,
}

impl Slide {
    pub(crate) fn new(open: bool) -> Self {
        Self { open, from: if open { 1.0 } else { 0.0 }, since: f64::NEG_INFINITY }
    }

    pub(crate) fn set(&mut self, open: bool, now: f64) {
        if open != self.open {
            self.from = self.amount(now);
            self.open = open;
            self.since = now;
        }
    }

    pub(crate) fn amount(&self, now: f64) -> f64 {
        let (target, span, curve) =
            if self.open { (1.0, OPEN_S, POP) } else { (0.0, CLOSE_S, STANDARD) };
        let progress = (now - self.since) / span;
        if progress >= 1.0 {
            return target;
        }
        self.from + (target - self.from) * bezier(curve, progress)
    }

    pub(crate) fn moving(&self, now: f64) -> bool {
        now - self.since < if self.open { OPEN_S } else { CLOSE_S }
    }
}

/// How visible the label is while the pill opens: it waits for the pill, then fades in.
pub(crate) fn label_alpha(amount: f64) -> f32 {
    ((amount - 0.3) / 0.5).clamp(0.0, 1.0) as f32
}

/// The share of the progress line still showing: time left of the whole session.
pub(crate) fn line_fraction(left: f64, total: f64) -> f32 {
    if total <= 0.0 { 0.0 } else { (left / total).clamp(0.0, 1.0) as f32 }
}

#[cfg(test)]
mod tests;
