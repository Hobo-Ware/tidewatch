use super::*;

#[test]
fn the_pill_names_the_time_left_in_the_biggest_unit_that_fits() {
    let cases = [
        (1.0, "1m", "1m"),
        (59.0 * 60.0, "59m", "59m"),
        (42.0 * 60.0 - 5.0, "42m", "42m"),
        (3600.0, "1h", "1h"),
        (3600.0 + 61.0, "1h 2m", "1h"),
        (4.0 * 3600.0, "4h", "4h"),
        (3.0 * 3600.0 + 12.0 * 60.0, "3h 12m", "3h"),
    ];
    for (left, full, short) in cases {
        assert_eq!(clock_texts(left), (full.to_owned(), short.to_owned()), "{left}");
    }
}

#[test]
fn the_label_shortens_then_hides_as_the_gap_closes() {
    let (full, short) = (96.0, 64.0);
    let cases = [
        (400.0, Fit::Full),
        (96.0 - ICON_W + 8.0, Fit::Full),
        (96.0 - ICON_W + 7.9, Fit::Short),
        (64.0 - ICON_W + 8.0, Fit::Short),
        (64.0 - ICON_W + 7.9, Fit::Hidden),
        (0.0, Fit::Hidden),
    ];
    for (gap, expected) in cases {
        assert_eq!(fit(gap, full, short), expected, "gap {gap}");
    }
    assert_eq!(pill_width(Fit::Full, full, short), 96.0);
    assert_eq!(pill_width(Fit::Short, full, short), 64.0);
    assert_eq!(pill_width(Fit::Hidden, full, short), ICON_W);
}

#[test]
fn the_pill_grows_with_a_spring_and_collapses_without_one() {
    let mut slide = Slide::new(false);
    assert_eq!(slide.amount(0.0), 0.0);
    slide.set(true, 10.0);
    assert_eq!(slide.amount(10.0), 0.0);
    let overshoot =
        (0..=26).map(|ms| slide.amount(10.0 + f64::from(ms) / 100.0)).fold(0.0, f64::max);
    assert!(overshoot > 1.0, "the spring passes its width ({overshoot})");
    assert!((slide.amount(10.0 + OPEN_S) - 1.0).abs() < 1e-6);
    assert!(slide.moving(10.1) && !slide.moving(10.3));
    slide.set(false, 20.0);
    let mut last = 1.0;
    for ms in 0..=20 {
        let now = slide.amount(20.0 + f64::from(ms) / 100.0);
        assert!(now <= last + 1e-9 && now >= 0.0, "monotone close at {ms}");
        last = now;
    }
    assert_eq!(slide.amount(21.0), 0.0);
}

#[test]
fn reversing_midway_continues_from_where_the_pill_was() {
    let mut slide = Slide::new(false);
    slide.set(true, 0.0);
    let half = slide.amount(0.1);
    slide.set(false, 0.1);
    assert!((slide.amount(0.1) - half).abs() < 1e-9);
}

#[test]
fn the_label_appears_after_the_pill_has_started_to_open() {
    assert_eq!(label_alpha(0.0), 0.0);
    assert_eq!(label_alpha(0.3), 0.0);
    assert_eq!(label_alpha(1.0), 1.0);
    assert!(label_alpha(0.55) > 0.4 && label_alpha(0.55) < 0.6);
}

#[test]
fn the_progress_line_shows_the_share_of_time_left() {
    assert_eq!(line_fraction(900.0, 1800.0), 0.5);
    assert_eq!(line_fraction(9000.0, 1800.0), 1.0);
    assert_eq!(line_fraction(-1.0, 1800.0), 0.0);
    assert_eq!(line_fraction(10.0, 0.0), 0.0);
}

#[test]
fn the_menu_and_tooltip_say_the_time_left_in_words() {
    let cases = [
        (0.0, "Less than a minute left"),
        (20.0, "1 min left"),
        (42.0 * 60.0 - 5.0, "42 min left"),
        (3600.0, "1 h left"),
        (3600.0 + 12.0 * 60.0, "1 h 12 min left"),
    ];
    for (left, text) in cases {
        assert_eq!(left_text(left), text, "{left}");
    }
}
