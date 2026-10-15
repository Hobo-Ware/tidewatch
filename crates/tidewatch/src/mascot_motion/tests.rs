use super::*;

const STEP_S: f64 = 0.01;

fn blink_starts(seed: u32, seconds: f64) -> Vec<f64> {
    let mut starts = Vec::new();
    let mut shut = false;
    let mut time = 0.0;
    while time < seconds {
        let now_shut = eye(seed, time).open < 0.5;
        if now_shut && !shut {
            starts.push(time);
        }
        shut = now_shut;
        time += STEP_S;
    }
    starts
}

#[test]
fn it_moves_only_when_drawn_focused_and_visible() {
    for on_screen in [false, true] {
        for focused in [false, true] {
            for visible in [false, true] {
                let expected = on_screen && focused && visible;
                assert_eq!(should_animate(on_screen, focused, visible), expected);
            }
        }
    }
}

#[test]
fn a_standing_still_mascot_asks_for_no_repaint() {
    assert_eq!(next_repaint(false), None);
    assert_eq!(next_repaint(true), Some(Duration::from_millis(33)));
}

#[test]
fn the_creature_bobs_seven_units_up_and_back_every_3_2_seconds() {
    assert_eq!(bob(0.0), 0.0);
    assert!((bob(1.6) + 7.0).abs() < 1e-4);
    assert!(bob(3.2).abs() < 1e-4);
    for step in 0..320 {
        let lift = bob(f64::from(step) * STEP_S);
        assert!((-7.0..=0.0).contains(&lift), "{lift}");
    }
}

#[test]
fn breathing_swells_a_little_and_settles() {
    assert_eq!(breathe(0.0), vec2(1.0, 1.0));
    let full = breathe(2.2);
    assert!((full.x - 1.015).abs() < 1e-4 && (full.y - 1.035).abs() < 1e-4, "{full:?}");
}

#[test]
fn an_eye_blinks_every_four_to_seven_seconds_down_to_almost_shut() {
    for seed in 0..6 {
        let starts = blink_starts(seed, 300.0);
        assert!(starts.len() > 40, "seed {seed}: {}", starts.len());
        for pair in starts.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((3.9..=7.1).contains(&gap), "seed {seed}: gap {gap}");
        }
        let lowest = (0..30_000)
            .map(|step| eye(seed, f64::from(step) * STEP_S).open)
            .fold(1.0_f32, f32::min);
        assert!((0.08 - 1e-4..0.12).contains(&lowest), "seed {seed}: {lowest}");
    }
}

#[test]
fn a_blink_lasts_about_a_sixth_of_a_second() {
    let start = blink_starts(0, 20.0)[0];
    let mut time = start - 0.2;
    let mut shut_for = 0.0;
    while time < start + 0.4 {
        if eye(0, time).open < 0.99 {
            shut_for += STEP_S;
        }
        time += STEP_S;
    }
    assert!((0.1..=0.2).contains(&shut_for), "{shut_for}");
}

#[test]
fn eyes_do_not_blink_in_step() {
    let first = blink_starts(0, 60.0);
    let second = blink_starts(1, 60.0);
    assert_ne!(first, second);
}

#[test]
fn darts_stay_within_three_units_and_every_eye_gets_some() {
    for seed in 0..6 {
        let mut farthest = 0.0_f32;
        for step in 0..30_000 {
            let moved = eye(seed, f64::from(step) * STEP_S).dart;
            farthest = farthest.max(moved.length());
        }
        assert!((1.5..=3.0 + 1e-3).contains(&farthest), "seed {seed}: {farthest}");
    }
}

#[test]
fn the_still_frame_is_eyes_open_and_centered() {
    assert_eq!(EyeFrame::STILL, EyeFrame { open: 1.0, dart: Vec2::ZERO });
}
