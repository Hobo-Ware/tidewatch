//! The Amphetamine tablet's motion as pure functions of time: how far each part of the activation
//! has come, the steady rock, glow and blink, and the specks that puff and fizz off the rim. Units
//! are the 24 unit glyph grid of the approved board; nothing here reads a clock or draws.

use std::f64::consts::PI;
use std::sync::OnceLock;

use egui::{Color32, Vec2, vec2};

use crate::brand;
use crate::mascot_motion::unit;

pub(crate) const CENTER: Vec2 = vec2(12.0, 12.8);
pub(crate) const RADIUS: f32 = 9.0;
pub(crate) const TILT_DEG: f32 = 30.0;
pub(crate) const ACTIVATE_S: f64 = 0.8;
pub(crate) const DEACTIVATE_S: f64 = 0.4;
pub(crate) const PUFF_COUNT: usize = 16;
pub(crate) const FIZZ_COUNT: usize = 15;

const WINK_CLOSE_S: f64 = 0.2;
const WINK_OPEN_S: f64 = 0.35;
const FIZZ_START_S: f64 = 0.95;
const FIZZ_FADE_S: f64 = 0.3;
const ROCK_DEG: f64 = 5.0;
const ROCK_PERIOD_S: f64 = 3.0;
const ROCK_RAMP_S: f64 = 0.5;
const GLOW_PERIOD_S: f64 = 2.6;
const BLINK_PERIOD_S: f64 = 3.6;
const DART_PERIOD_S: f64 = 4.2;
const EYE_POP_S: f64 = 0.25;
const EYE_DELAYS: [f64; 2] = [0.0, 1.1];
const DART_DELAYS: [f64; 2] = [0.0, 1.7];
const DART_UNITS: f32 = 0.6;
const POINTS: usize = 72;
const EYE_ANGLES_DEG: [f32; 2] = [278.0, 331.0];
const EYE_CLEAR_DEG: f32 = 17.0;
const SIZES: [f32; 5] = [0.9, 1.0, 1.2, 1.4, 1.7];
const LIVES_S: [f64; 5] = [3.6, 4.1, 4.6, 5.1, 5.6];
const EASE_OUT: (f64, f64, f64, f64) = (0.22, 1.0, 0.36, 1.0);
const SPRING: (f64, f64, f64, f64) = (0.34, 1.56, 0.64, 1.0);
const PUFF_EASE: (f64, f64, f64, f64) = (0.2, 0.7, 0.3, 1.0);
const DRIFT_EASE: (f64, f64, f64, f64) = (0.0, 0.0, 0.58, 1.0);
const OFF_WHITE: Color32 = Color32::from_rgb(0xf6, 0xfa, 0xf9);

pub(crate) const STANDARD: (f64, f64, f64, f64) = (0.4, 0.0, 0.2, 1.0);
pub(crate) const POP: (f64, f64, f64, f64) = SPRING;

/// What the tablet is doing. `age` is seconds since the session came on or began to stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Look {
    Off,
    /// The password prompt is open: the eyes peek, nothing else moves.
    Pending,
    On {
        age: f64,
    },
    Stopping {
        age: f64,
    },
}

impl Look {
    /// Whether the tablet needs frames: on, or still easing off.
    pub(crate) fn animating(self) -> bool {
        match self {
            Self::Off | Self::Pending => false,
            Self::On { .. } => true,
            Self::Stopping { age } => age < DEACTIVATE_S,
        }
    }
}

/// Every part of the tablet at one instant, as amounts the painter scales its shapes by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tablet {
    pub(crate) fill: Color32,
    pub(crate) rim: Color32,
    pub(crate) face: Color32,
    pub(crate) smooth_rim: f32,
    pub(crate) rough_fill: f32,
    pub(crate) rough_draw: f32,
    pub(crate) wink: f32,
    pub(crate) eyes: f32,
    pub(crate) glow: f32,
    pub(crate) rock_deg: f32,
    pub(crate) specks: f32,
}

/// The cubic-bezier easing of CSS: `x1 y1 x2 y2` and the progress `t` in 0 to 1.
pub(crate) fn bezier((x1, y1, x2, y2): (f64, f64, f64, f64), t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let axis = |a: f64, b: f64, s: f64| {
        3.0 * a * (1.0 - s).powi(2) * s + 3.0 * b * (1.0 - s) * s * s + s.powi(3)
    };
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = f64::midpoint(low, high);
        if axis(x1, x2, mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    axis(y1, y2, f64::midpoint(low, high))
}

fn smooth(progress: f64) -> f64 {
    let p = progress.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn mix(from: Color32, to: Color32, amount: f64) -> Color32 {
    from.lerp_to_gamma(to, amount.clamp(0.0, 1.0) as f32)
}

fn off_colors(hover: f64) -> (Color32, Color32, Color32) {
    (
        mix(brand::TEAL.gamma_multiply(0.30), brand::TEAL.gamma_multiply(0.55), hover),
        mix(brand::MUTED, brand::TEXT, hover),
        mix(brand::MUTED.gamma_multiply(0.85), brand::TEXT.gamma_multiply(0.85), hover),
    )
}

fn on_colors() -> (Color32, Color32, Color32) {
    (brand::TEAL, brand::TEAL_TEXT, mix(brand::TEAL, brand::GROUND, 0.6))
}

/// The tablet for `look` while the pointer is `hover` (0 to 1) over it.
pub(crate) fn tablet(look: Look, hover: f64) -> Tablet {
    let (fill, rim, face) = off_colors(hover);
    let still = Tablet {
        fill,
        rim,
        face,
        smooth_rim: 1.0,
        rough_fill: 0.0,
        rough_draw: 0.0,
        wink: 0.0,
        eyes: bezier(POP, hover) as f32,
        glow: 0.0,
        rock_deg: 0.0,
        specks: 0.0,
    };
    match look {
        Look::Off => still,
        Look::Pending => Tablet { eyes: 1.0, ..still },
        Look::On { age } => {
            let (fill, rim, face) = on_colors();
            Tablet {
                fill,
                rim,
                face,
                smooth_rim: (1.0 - smooth((age - 0.4) / 0.4)) as f32,
                rough_fill: bezier(EASE_OUT, (age - 0.1) / 0.7) as f32,
                rough_draw: bezier(EASE_OUT, age / ACTIVATE_S) as f32,
                wink: smooth(age / WINK_CLOSE_S) as f32,
                eyes: 1.0,
                glow: glow(age),
                rock_deg: rock(age),
                specks: smooth((age - FIZZ_START_S) / FIZZ_FADE_S) as f32,
            }
        }
        Look::Stopping { age } => {
            let gone = smooth(age / DEACTIVATE_S);
            let (on_fill, on_rim, on_face) = on_colors();
            let retract = 1.0 - bezier(STANDARD, age / DEACTIVATE_S);
            Tablet {
                fill: mix(on_fill, fill, gone),
                rim: mix(on_rim, rim, gone),
                face: mix(on_face, face, gone),
                smooth_rim: smooth(age / 0.3) as f32,
                rough_fill: retract as f32,
                rough_draw: retract as f32,
                wink: (1.0 - smooth(age / WINK_OPEN_S)) as f32,
                eyes: 1.0 - ((1.0 - bezier(POP, hover)) * smooth(age / EYE_POP_S)) as f32,
                glow: (glow(age) * (1.0 - gone) as f32).max(0.0),
                rock_deg: rock(age) * (1.0 - gone) as f32,
                specks: 0.0,
            }
        }
    }
}

pub(crate) fn rock(age: f64) -> f32 {
    let amount = ROCK_DEG * smooth(age / ROCK_RAMP_S);
    (-amount * (2.0 * PI * age / ROCK_PERIOD_S).cos()) as f32
}

pub(crate) fn glow(age: f64) -> f32 {
    (0.5 + 0.45 * (0.5 - 0.5 * (2.0 * PI * age / GLOW_PERIOD_S).cos())) as f32
}

/// How open peeking eye `slot` is at `age` (1 open, 0.1 shut): a blink every 3.6 s.
pub(crate) fn blink(slot: usize, age: f64) -> f32 {
    let phase = ((age + EYE_DELAYS[slot]) / BLINK_PERIOD_S).rem_euclid(1.0);
    let shut = 0.1;
    if !(0.88..1.0).contains(&phase) {
        return 1.0;
    }
    let value = if phase < 0.93 {
        1.0 - (1.0 - shut) * smooth((phase - 0.88) / 0.05)
    } else {
        shut + (1.0 - shut) * smooth((phase - 0.93) / 0.07)
    };
    value as f32
}

/// How far eye `slot`'s pupil has moved at `age`, in units: it looks one way, the other, and back.
pub(crate) fn dart(slot: usize, age: f64) -> Vec2 {
    let phase = ((age + DART_DELAYS[slot]) / DART_PERIOD_S).rem_euclid(1.0);
    let toward = |from: f64, to: f64| smooth((phase - from) / (to - from));
    let side = if phase < 0.18 {
        0.0
    } else if phase < 0.28 {
        toward(0.18, 0.28)
    } else if phase < 0.52 {
        1.0
    } else if phase < 0.62 {
        1.0 - 2.0 * toward(0.52, 0.62)
    } else if phase < 0.86 {
        -1.0
    } else {
        -1.0 + toward(0.86, 1.0)
    };
    vec2(DART_UNITS * side as f32, -0.5 * DART_UNITS * side as f32)
}

/// The rim of the crumbled tablet and its inner shade ring, `POINTS` units each, clockwise from the
/// right.
pub(crate) struct Rims {
    pub(crate) rough: [Vec2; POINTS],
    pub(crate) shade: [Vec2; POINTS],
}

const L2_NICKS: [(f32, f32, f32); 4] =
    [(0.4, 0.12, 0.6), (2.0, 0.10, 0.5), (3.6, 0.13, 0.6), (5.3, 0.10, 0.5)];

fn rim(amp: f32, nicks: [(f32, f32, f32); 4], phases: [f32; 3]) -> [Vec2; POINTS] {
    std::array::from_fn(|i| {
        let t = std::f32::consts::TAU * i as f32 / POINTS as f32;
        let mut r = RADIUS
            + amp
                * (0.55 * (7.0 * t + phases[0]).sin()
                    + 0.3 * (13.0 * t + phases[1]).sin()
                    + 0.2 * (23.0 * t + phases[2]).sin());
        for (center, width, depth) in nicks {
            let d = (t - center + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            r -= depth * (-(d / width).powi(2)).exp();
        }
        CENTER + vec2(r * t.cos(), r * t.sin())
    })
}

pub(crate) fn rims() -> &'static Rims {
    static RIMS: OnceLock<Rims> = OnceLock::new();
    RIMS.get_or_init(|| Rims {
        rough: rim(0.3, L2_NICKS, [2.9803, 4.1289, 4.1851]),
        shade: rim(0.18, L2_NICKS.map(|(c, w, d)| (c, w * 1.2, d * 0.5)), [1.6266, 4.3034, 4.296]),
    })
}

/// The flat chalk specks on the tablet: position, radius and whether it is a light one.
pub(crate) const CHALK: [(f32, f32, f32, bool); 11] = [
    (5.76, 13.81, 0.55, true),
    (10.18, 9.66, 0.55, false),
    (17.01, 13.14, 0.4, true),
    (10.96, 19.64, 0.55, false),
    (9.78, 7.13, 0.55, true),
    (6.08, 10.53, 0.4, false),
    (15.27, 17.04, 0.55, true),
    (15.81, 18.99, 0.55, false),
    (18.48, 15.31, 0.4, true),
    (17.75, 15.71, 0.55, false),
    (9.31, 17.22, 0.55, true),
];

/// One speck in the air around the tablet, in glyph units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Speck {
    pub(crate) pos: Vec2,
    pub(crate) radius: f32,
    pub(crate) alpha: f32,
    pub(crate) ink: Color32,
}

fn clear_of_eyes(degrees: f32) -> f32 {
    EYE_ANGLES_DEG.into_iter().fold(degrees, |degrees, eye| {
        let off = (degrees - eye + 180.0).rem_euclid(360.0) - 180.0;
        if off.abs() < EYE_CLEAR_DEG {
            (eye + (EYE_CLEAR_DEG + 0.5).copysign(off)).rem_euclid(360.0)
        } else {
            degrees
        }
    })
}

fn between(seed: u32, slot: i64, salt: u32, low: f32, high: f32) -> f32 {
    low + (high - low) * unit(seed, slot, salt) as f32
}

fn ink(index: usize) -> Color32 {
    if index.is_multiple_of(2) { brand::TEAL_TEXT } else { OFF_WHITE }
}

fn on_rim(degrees: f32, radius: f32, travel: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    CENTER + vec2(cos, sin) * (radius + travel)
}

/// Speck `index` of the first puff `age` seconds into an activation, if it is in flight.
pub(crate) fn puff(seed: u32, index: usize, age: f64) -> Option<Speck> {
    let slot = index as i64;
    let degrees =
        clear_of_eyes(index as f32 * 360.0 / PUFF_COUNT as f32 + between(seed, slot, 1, -9.0, 9.0));
    let delay = f64::from(between(seed, slot, 2, 0.25, 0.45));
    let life = f64::from(between(seed, slot, 3, 0.8, 1.1));
    let progress = (age - delay) / life;
    if !(0.0..1.0).contains(&progress) {
        return None;
    }
    let travel = between(seed, slot, 4, 5.0, 9.0) * bezier(PUFF_EASE, progress) as f32;
    let radius = between(seed, slot, 5, 1.0, 1.7) * (0.6 - 0.3 * progress as f32);
    let alpha = if progress < 0.15 { progress / 0.15 } else { 1.0 - (progress - 0.15) / 0.85 };
    Some(Speck {
        pos: on_rim(degrees, RADIUS - 0.4, travel),
        radius,
        alpha: alpha as f32,
        ink: ink(index),
    })
}

/// Speck `index` of the steady fizz `age` seconds into an activation.
pub(crate) fn fizz(seed: u32, index: usize, age: f64) -> Option<Speck> {
    if age < FIZZ_START_S {
        return None;
    }
    let life = LIVES_S[(index * 2) % 5];
    let local = age - FIZZ_START_S + index as f64 / FIZZ_COUNT as f64 * life;
    let cycle = (local / life).floor();
    let progress = (local / life - cycle) as f32;
    let slot = (index as i64) * 100_000 + cycle as i64;
    let degrees = clear_of_eyes(
        (index as f32 * 360.0 / FIZZ_COUNT as f32 + between(seed, slot, 1, -8.0, 8.0))
            .rem_euclid(360.0),
    );
    let far = index % 5 == 3;
    let travel =
        if far { between(seed, slot, 2, 7.0, 8.5) } else { between(seed, slot, 2, 3.5, 6.0) };
    let drift = bezier(DRIFT_EASE, f64::from(progress)) as f32;
    let alpha = if progress < 0.14 {
        progress / 0.14 * 0.95
    } else {
        0.95 * (1.0 - (progress - 0.14) / 0.86)
    };
    Some(Speck {
        pos: on_rim(degrees, RADIUS - 0.3, travel * drift),
        radius: SIZES[(index * 3) % 5] * (0.7 - 0.45 * progress),
        alpha,
        ink: ink(index),
    })
}

#[cfg(test)]
mod tests;
