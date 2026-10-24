//! The mascot's motion as pure functions of time, and the rule for when it may move at all.

use std::f64::consts::PI;
use std::time::Duration;

use egui::{Vec2, vec2};

const BOB_UNITS: f32 = 7.0;
const BOB_PERIOD_S: f64 = 3.2;
const BREATHE_PERIOD_S: f64 = 4.4;
const BREATHE_GROWTH: Vec2 = vec2(0.015, 0.035);
const BLINK_SLOT_S: f64 = 5.5;
const BLINK_SPREAD_S: f64 = 1.5;
const BLINK_LENGTH_S: f64 = 0.15;
const BLINK_SHUT: f32 = 0.08;
const DART_SLOT_S: f64 = 6.0;
const DART_SPREAD_S: f64 = 3.0;
const DART_GO_S: f64 = 0.12;
const DART_HOLD_S: f64 = 1.0;
const DART_BACK_S: f64 = 0.25;
const DART_REACH: (f64, f64) = (2.0, 3.0);
const DART_FLATTEN: f64 = 0.4;
const FRAME_EVERY: Duration = Duration::from_millis(33);

/// What one real eye does at an instant: how open it is (1 open, near 0 shut) and how far it has
/// darted, both in the pose's viewBox units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EyeFrame {
    pub(crate) open: f32,
    pub(crate) dart: Vec2,
}

impl EyeFrame {
    pub(crate) const STILL: Self = Self { open: 1.0, dart: Vec2::ZERO };
}

/// The mascot moves only while it is drawn, its window has focus and the window is visible.
pub(crate) fn should_animate(on_screen: bool, focused: bool, visible: bool) -> bool {
    on_screen && focused && visible
}

/// How long to wait before the next frame of the animation; `None` when it stands still, so the
/// app asks for no repaint at all.
pub(crate) fn next_repaint(animating: bool) -> Option<Duration> {
    animating.then_some(FRAME_EVERY)
}

/// How far the whole creature is lifted (zero or less, in viewBox units).
pub(crate) fn bob(time: f64) -> f32 {
    -BOB_UNITS * swell(time, BOB_PERIOD_S)
}

/// The scale of a sleeping creature, anchored at its base.
pub(crate) fn breathe(time: f64) -> Vec2 {
    Vec2::splat(1.0) + BREATHE_GROWTH * swell(time, BREATHE_PERIOD_S)
}

/// Eye `seed` at `time`: it blinks every 4 to 7 seconds and darts a little now and then, on a
/// schedule of its own.
pub(crate) fn eye(seed: u32, time: f64) -> EyeFrame {
    EyeFrame { open: blink(seed, time), dart: dart(seed, time) }
}

fn swell(time: f64, period: f64) -> f32 {
    (0.5 - 0.5 * (2.0 * PI * time / period).cos()) as f32
}

fn blink(seed: u32, time: f64) -> f32 {
    let (slot, into) = slot_of(seed, time, BLINK_SLOT_S);
    let start = BLINK_SPREAD_S * unit(seed, slot, 1);
    let progress = (into - start) / BLINK_LENGTH_S;
    if !(0.0..1.0).contains(&progress) {
        return 1.0;
    }
    1.0 - (1.0 - BLINK_SHUT) * (PI * progress).sin() as f32
}

fn dart(seed: u32, time: f64) -> Vec2 {
    let (slot, into) = slot_of(seed, time, DART_SLOT_S);
    let since = into - DART_SPREAD_S * unit(seed, slot, 2);
    let held = if since < 0.0 {
        0.0
    } else if since < DART_GO_S {
        smooth(since / DART_GO_S)
    } else if since < DART_GO_S + DART_HOLD_S {
        1.0
    } else {
        1.0 - smooth((since - DART_GO_S - DART_HOLD_S) / DART_BACK_S)
    };
    let reach = DART_REACH.0 + (DART_REACH.1 - DART_REACH.0) * unit(seed, slot, 3);
    let angle = 2.0 * PI * unit(seed, slot, 4);
    vec2((reach * angle.cos() * held) as f32, (reach * angle.sin() * DART_FLATTEN * held) as f32)
}

fn smooth(progress: f64) -> f64 {
    let p = progress.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn slot_of(seed: u32, time: f64, length: f64) -> (i64, f64) {
    let shifted = time + length * unit(seed, 0, 0);
    let slot = (shifted / length).floor();
    (slot as i64, shifted - slot * length)
}

pub(crate) fn unit(seed: u32, slot: i64, salt: u32) -> f64 {
    let mut z = (u64::from(seed) << 40) ^ (u64::from(salt) << 32) ^ (slot as u64);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests;
