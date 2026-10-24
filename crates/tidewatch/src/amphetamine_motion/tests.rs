use super::*;

const SEED: u32 = 7;

fn angle_of(speck: &Speck) -> f32 {
    let d = speck.pos - CENTER;
    d.y.atan2(d.x).to_degrees().rem_euclid(360.0)
}

fn distance_from_eyes(degrees: f32) -> f32 {
    EYE_ANGLES_DEG
        .into_iter()
        .map(|eye| ((degrees - eye + 180.0).rem_euclid(360.0) - 180.0).abs())
        .fold(f32::MAX, f32::min)
}

#[test]
fn the_easings_start_at_zero_end_at_one_and_the_spring_overshoots() {
    for curve in [EASE_OUT, SPRING, STANDARD, PUFF_EASE, DRIFT_EASE] {
        assert!(bezier(curve, 0.0).abs() < 1e-6);
        assert!((bezier(curve, 1.0) - 1.0).abs() < 1e-6);
    }
    assert!((bezier((0.0, 0.0, 1.0, 1.0), 0.37) - 0.37).abs() < 1e-3);
    assert!(bezier(SPRING, 0.6) > 1.0);
    assert!(bezier(EASE_OUT, 0.5) > 0.9);
    assert_eq!(bezier(EASE_OUT, 5.0), bezier(EASE_OUT, 1.0));
}

#[test]
fn off_is_a_smooth_calm_tablet_and_hover_pops_the_eyes_and_lights_the_rim() {
    let off = tablet(Look::Off, 0.0);
    assert_eq!(
        (off.smooth_rim, off.rough_draw, off.wink, off.eyes, off.glow),
        (1.0, 0.0, 0.0, 0.0, 0.0)
    );
    assert_eq!((off.rock_deg, off.specks), (0.0, 0.0));
    assert_eq!((off.rim, off.fill), (brand::MUTED, brand::TEAL.gamma_multiply(0.30)));
    let hover = tablet(Look::Off, 1.0);
    assert_eq!((hover.rim, hover.fill), (brand::TEXT, brand::TEAL.gamma_multiply(0.55)));
    assert_eq!(hover.eyes, 1.0);
    assert_eq!(tablet(Look::Pending, 0.0).eyes, 1.0);
}

#[test]
fn activation_follows_the_spec_timeline() {
    let at = |ms: f64| tablet(Look::On { age: ms / 1000.0 }, 0.0);
    assert_eq!(at(0.0).wink, 0.0);
    assert!((at(200.0).wink - 1.0).abs() < 1e-6 && at(800.0).wink > 0.999);
    assert!(at(100.0).wink > 0.2 && at(100.0).wink < 0.8);
    let mut last = (0.0, 0.0);
    for ms in (0..=800).step_by(20) {
        let t = at(f64::from(ms));
        assert!(t.rough_draw >= last.0 && t.rough_fill >= last.1, "monotone at {ms}");
        last = (t.rough_draw, t.rough_fill);
    }
    assert!((at(800.0).rough_draw - 1.0).abs() < 1e-6 && (at(800.0).rough_fill - 1.0).abs() < 1e-6);
    assert_eq!(at(50.0).rough_fill, 0.0);
    assert_eq!((at(399.0).smooth_rim, at(800.0).smooth_rim), (1.0, 0.0));
    assert_eq!((at(900.0).specks, at(1250.0).specks), (0.0, 1.0));
    assert_eq!((at(0.0).fill, at(0.0).rim), (brand::TEAL, brand::TEAL_TEXT));
}

#[test]
fn turning_off_retracts_the_rim_in_400_ms_and_falls_back_to_the_off_colors() {
    let at = |ms: f64| tablet(Look::Stopping { age: ms / 1000.0 }, 0.0);
    let start = at(0.0);
    assert_eq!((start.rough_draw, start.wink, start.smooth_rim), (1.0, 1.0, 0.0));
    assert_eq!(start.fill, brand::TEAL);
    let end = at(400.0);
    assert_eq!((end.rough_draw, end.rough_fill, end.smooth_rim, end.specks), (0.0, 0.0, 1.0, 0.0));
    assert_eq!(
        (end.fill, end.rim, end.rock_deg, end.glow),
        (tablet(Look::Off, 0.0).fill, brand::MUTED, 0.0, 0.0)
    );
    assert_eq!(end.eyes, 0.0);
    assert_eq!(at(100.0).specks, 0.0);
    assert!(at(100.0).wink < 1.0);
    assert!(at(200.0).wink > at(300.0).wink);
}

#[test]
fn only_a_session_that_is_on_or_easing_off_needs_frames() {
    assert!(!Look::Off.animating() && !Look::Pending.animating());
    assert!(Look::On { age: 100.0 }.animating());
    assert!(Look::Stopping { age: 0.39 }.animating() && !Look::Stopping { age: 0.4 }.animating());
}

#[test]
fn the_rock_stays_within_five_degrees_and_starts_from_rest() {
    assert_eq!(rock(0.0), 0.0);
    let peak = (0..600).map(|i| rock(f64::from(i) / 100.0).abs()).fold(0.0, f32::max);
    assert!(peak <= 5.0 + 1e-4 && peak > 4.9, "{peak}");
    assert!((rock(1.5 + ROCK_RAMP_S * 4.0) - rock(1.5 + ROCK_RAMP_S * 4.0 + 3.0)).abs() < 1e-4);
}

#[test]
fn the_glow_pulses_between_half_and_nearly_full_every_2_6_seconds() {
    let values: Vec<f32> = (0..260).map(|i| glow(f64::from(i) / 100.0)).collect();
    let (low, high) = values.iter().fold((f32::MAX, 0.0_f32), |(l, h), v| (l.min(*v), h.max(*v)));
    assert!((low - 0.5).abs() < 1e-3 && (high - 0.95).abs() < 1e-3, "{low} {high}");
    assert!((glow(0.0) - glow(2.6)).abs() < 1e-4);
}

#[test]
fn each_eye_blinks_for_a_fifth_of_a_second_every_3_6_seconds_and_the_eyes_are_out_of_step() {
    for (slot, delay) in EYE_DELAYS.into_iter().enumerate() {
        let shut: Vec<f64> =
            (0..3600).map(|i| f64::from(i) / 1000.0).filter(|t| blink(slot, *t) < 0.5).collect();
        assert_ne!(shut.len(), 0);
        assert!(shut.last().unwrap() - shut.first().unwrap() < 0.2, "{slot}");
        assert_eq!(blink(slot, 100.0 * 3.6 - delay), 1.0);
    }
    let first_shut =
        |slot: usize| (0..3600).map(|i| f64::from(i) / 1000.0).find(|t| blink(slot, *t) < 0.5);
    assert_ne!(first_shut(0), first_shut(1));
}

#[test]
fn the_pupils_move_six_tenths_of_a_unit_and_come_back() {
    assert_eq!(dart(0, 0.0), Vec2::ZERO);
    let reach = (0..420).map(|i| dart(0, f64::from(i) / 100.0).length()).fold(0.0, f32::max);
    assert!(reach > 0.6 && reach < 0.7, "{reach}");
    assert!(dart(0, 4.199).length() < 0.05);
}

#[test]
fn the_crumbled_rim_has_72_points_a_wobble_and_four_nicks() {
    let rims = rims();
    assert_eq!(rims.rough.len(), 72);
    let radii: Vec<f32> = rims.rough.iter().map(|p| (*p - CENTER).length()).collect();
    assert!(radii.iter().all(|r| *r > 7.5 && *r < 9.5), "{radii:?}");
    let at = |radians: f32| radii[(radians / std::f32::consts::TAU * 72.0).round() as usize % 72];
    for nick in [0.4, 2.0, 3.6, 5.3] {
        let beside = at(nick + 0.6).min(at(nick - 0.6));
        assert!(at(nick) < beside - 0.15, "nick at {nick}");
    }
    let shade: Vec<f32> = rims.shade.iter().map(|p| (*p - CENTER).length()).collect();
    let spread = |v: &[f32]| {
        v.iter().fold(0.0_f32, |m, x| m.max(*x)) - v.iter().fold(f32::MAX, |m, x| m.min(*x))
    };
    assert!(spread(&shade) < spread(&radii));
}

#[test]
fn the_specks_are_the_same_for_the_same_seed_and_differ_for_another() {
    let all = |seed: u32| {
        (0..FIZZ_COUNT)
            .map(|i| fizz(seed, i, 12.3))
            .chain((0..PUFF_COUNT).map(|i| puff(seed, i, 0.6)))
            .collect::<Vec<_>>()
    };
    assert_eq!(all(SEED), all(SEED));
    assert_ne!(all(SEED), all(SEED + 1));
}

#[test]
fn the_puff_is_sixteen_specks_that_start_between_250_and_450_ms_and_last_under_1_1_s() {
    let first_seen =
        |i: usize| (0..1600).map(|ms| f64::from(ms) / 1000.0).find(|t| puff(SEED, i, *t).is_some());
    for i in 0..PUFF_COUNT {
        let start = first_seen(i).unwrap();
        assert!((0.25..=0.46).contains(&start), "speck {i} starts at {start}");
        assert!(puff(SEED, i, start + 1.2).is_none());
    }
    assert_eq!(puff(SEED, PUFF_COUNT, 0.6).map(|_| ()), puff(SEED, PUFF_COUNT, 0.6).map(|_| ()));
    assert!((0..PUFF_COUNT).all(|i| puff(SEED, i, 0.1).is_none() && puff(SEED, i, 1.6).is_none()));
}

#[test]
fn a_puff_speck_flies_straight_out_shrinks_and_fades() {
    let i = 3;
    let start =
        (0..1600).map(|ms| f64::from(ms) / 1000.0).find(|t| puff(SEED, i, *t).is_some()).unwrap();
    let early = puff(SEED, i, start + 0.01).unwrap();
    let late = puff(SEED, i, start + 0.6).unwrap();
    assert!((late.pos - CENTER).length() > (early.pos - CENTER).length() + 3.0);
    assert!(late.radius < early.radius && early.alpha < 0.3);
    assert!((angle_of(&early) - angle_of(&late)).abs() < 0.5);
    assert!((1.0 * 0.3..=1.7 * 0.6).contains(&late.radius) || late.radius < 1.02);
}

#[test]
fn the_fizz_starts_at_950_ms_with_fifteen_specks_in_the_air_after_that() {
    assert!((0..FIZZ_COUNT).all(|i| fizz(SEED, i, 0.9).is_none()));
    for age in [1.0, 4.0, 30.0, 123.4] {
        let live = (0..FIZZ_COUNT).filter_map(|i| fizz(SEED, i, age)).count();
        assert_eq!(live, FIZZ_COUNT, "{age}");
    }
    let visible =
        (0..FIZZ_COUNT).filter_map(|i| fizz(SEED, i, 50.0)).filter(|s| s.alpha > 0.1).count();
    assert!((10..=15).contains(&visible), "{visible}");
}

#[test]
fn the_fizz_spawns_about_three_and_a_third_specks_a_second() {
    let seconds = 460.0;
    let births: f64 = (0..FIZZ_COUNT).map(|i| seconds / LIVES_S[(i * 2) % 5]).sum();
    let rate = births / seconds;
    assert!((3.0..3.6).contains(&rate), "{rate}");
}

#[test]
fn no_speck_leaves_the_rim_where_a_peeking_eye_sits() {
    let mut seen = 0;
    for step in 0..2000 {
        let age = 1.0 + f64::from(step) * 0.37;
        for i in 0..FIZZ_COUNT {
            let speck = fizz(SEED, i, age).unwrap();
            assert!(distance_from_eyes(angle_of(&speck)) >= 17.0 - 0.5, "fizz {i} at {age}");
            seen += 1;
        }
    }
    for i in 0..PUFF_COUNT {
        for seed in 0..200 {
            let t = (0..1600)
                .map(|ms| f64::from(ms) / 1000.0)
                .find(|t| puff(seed, i, *t).is_some())
                .unwrap();
            let speck = puff(seed, i, t + 0.3).unwrap();
            assert!(distance_from_eyes(angle_of(&speck)) >= 16.5, "puff {i} seed {seed}");
            seen += 1;
        }
    }
    assert!(seen > 30_000);
}

#[test]
fn the_specks_use_the_five_spec_sizes_and_alternate_teal_and_off_white() {
    let sizes: std::collections::BTreeSet<u32> =
        SIZES.iter().map(|s| (s * 10.0).round() as u32).collect();
    assert_eq!(sizes.into_iter().collect::<Vec<_>>(), [9, 10, 12, 14, 17]);
    assert_eq!(ink(0), brand::TEAL_TEXT);
    assert_eq!(ink(1), OFF_WHITE);
    assert_eq!(CHALK.len(), 11);
}
