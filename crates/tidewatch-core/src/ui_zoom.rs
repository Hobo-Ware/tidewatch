//! The interface size: the steps Settings offers, how a stored value is made safe and how the
//! zoom shortcuts step through them (pure).

pub const MIN: f32 = 0.8;
pub const MAX: f32 = 1.5;
pub const DEFAULT: f32 = 1.0;
pub const STEPS: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];

/// The size that reads as 100%: the egui zoom of 90%, which owners found right on a laptop screen.
pub const BASE: f32 = 0.9;

const SAME: f32 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Bigger,
    Smaller,
    Reset,
}

/// `value` held to `MIN..=MAX`; anything that is not a number is `DEFAULT`.
#[must_use]
pub fn sane(value: f32) -> f32 {
    if value.is_nan() { DEFAULT } else { value.clamp(MIN, MAX) }
}

/// The egui zoom factor a stored size draws at.
#[must_use]
pub fn render_factor(value: f32) -> f32 {
    sane(value) * BASE
}

/// The next step the shortcut lands on: one up or down from `current` (a value between steps goes
/// to the nearest step in that direction), never past the ends, or `DEFAULT` for a reset.
#[must_use]
pub fn step(current: f32, direction: Direction) -> f32 {
    let current = sane(current);
    match direction {
        Direction::Reset => DEFAULT,
        Direction::Bigger => STEPS.into_iter().find(|step| *step > current + SAME).unwrap_or(MAX),
        Direction::Smaller => {
            STEPS.into_iter().rev().find(|step| *step < current - SAME).unwrap_or(MIN)
        }
    }
}

/// The index of `value` among `STEPS`, if it is one of them.
#[must_use]
pub fn position(value: f32) -> Option<usize> {
    STEPS.iter().position(|step| (step - value).abs() < SAME)
}

#[must_use]
pub fn label(value: f32) -> String {
    format!("{}%", (value * 100.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_value_is_held_to_the_range_and_nan_is_the_default() {
        let cases = [
            (1.0, 1.0),
            (0.5, MIN),
            (3.0, MAX),
            (0.0, MIN),
            (-2.0, MIN),
            (f32::INFINITY, MAX),
            (f32::NEG_INFINITY, MIN),
            (f32::NAN, DEFAULT),
        ];
        for (value, want) in cases {
            assert_eq!(sane(value), want, "{value}");
        }
    }

    #[test]
    fn a_hundred_percent_draws_at_the_old_ninety() {
        let cases = [(1.0, 0.9), (0.8, 0.72), (1.5, 1.35), (f32::NAN, 0.9)];
        for (value, want) in cases {
            assert!((render_factor(value) - want).abs() < 1e-6, "{value}");
        }
    }

    fn walk(from: f32, direction: Direction) -> Vec<f32> {
        let mut seen = vec![from];
        loop {
            let next = step(*seen.last().unwrap(), direction);
            if next == *seen.last().unwrap() {
                return seen;
            }
            seen.push(next);
        }
    }

    #[test]
    fn bigger_and_smaller_walk_the_steps_and_stop_at_the_ends() {
        assert_eq!(walk(MIN, Direction::Bigger), STEPS);
        let mut reversed = STEPS.to_vec();
        reversed.reverse();
        assert_eq!(walk(MAX, Direction::Smaller), reversed);
    }

    #[test]
    fn a_value_between_steps_goes_to_the_nearest_step_in_the_direction_asked() {
        assert_eq!(step(1.05, Direction::Bigger), 1.1);
        assert_eq!(step(1.05, Direction::Smaller), 1.0);
        assert_eq!(step(1.3, Direction::Bigger), 1.5);
        assert_eq!(step(f32::NAN, Direction::Bigger), 1.1);
    }

    #[test]
    fn reset_is_one_hundred_percent_from_anywhere() {
        for value in STEPS {
            assert_eq!(step(value, Direction::Reset), 1.0);
        }
    }

    #[test]
    fn the_labels_and_positions_follow_the_steps() {
        let labels: Vec<String> = STEPS.iter().map(|value| label(*value)).collect();
        assert_eq!(labels, ["80%", "90%", "100%", "110%", "125%", "150%"]);
        assert_eq!(position(1.25), Some(4));
        assert_eq!(position(1.05), None);
    }
}
