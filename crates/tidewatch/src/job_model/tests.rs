use tidewatch_core::job::Report;

use super::*;

const MIN: i64 = 60_000;
const NOON: i64 = 1_760_875_200_000;

fn running(board: &mut JobBoard, name: &str, total: f64, readings: &[(i64, f64)]) {
    for (at, done) in readings {
        let report = Report { done: Some(*done), total: Some(total), ..Report::named(name) };
        board.report(&report, NOON + at);
    }
}

fn orders() -> JobBoard {
    let mut board = JobBoard::default();
    running(
        &mut board,
        "Backfill orders",
        58_650.0,
        &[(-30 * MIN, 0.0), (-20 * MIN, 4_140.0), (-10 * MIN, 8_280.0), (0, 12_420.0)],
    );
    board
}

#[test]
fn durations_read_in_the_two_largest_units() {
    let cases = [
        (0, "0s"),
        (59, "59s"),
        (60, "1m"),
        (1_800, "30m"),
        (3_599, "59m"),
        (3_600, "1h"),
        (6_731, "1h 52m"),
        (86_399, "23h 59m"),
        (86_400, "1d"),
        (100_000, "1d 3h"),
    ];
    for (secs, want) in cases {
        assert_eq!(duration_text(secs), want, "{secs}");
    }
}

#[test]
fn counts_get_thousands_separators_and_one_decimal_when_fractional() {
    let cases = [
        (0.0, "0"),
        (999.0, "999"),
        (1_000.0, "1,000"),
        (12_431.0, "12,431"),
        (1_234_567.0, "1,234,567"),
        (2.5, "2.5"),
    ];
    for (value, want) in cases {
        assert_eq!(count_text(value), want, "{value}");
    }
}

#[test]
fn a_running_job_is_a_bar_card_with_the_numbers_of_the_mockup() {
    let cards = cards(&orders(), NOON, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.name, "Backfill orders");
    assert_eq!(card.elapsed, "30m");
    assert_eq!(card.percent.as_deref(), Some("21.2%"));
    assert_eq!(card.counts, "12,420 / 58,650");
    assert_eq!(card.rate.as_deref(), Some("414/min"));
    assert_eq!(card.eta.as_deref(), Some("1h 51m"));
    assert_eq!(card.done_at.as_deref(), Some("13:51"));
    assert_eq!(card.fraction.map(|f| (f * 1000.0).round()), Some(212.0));
    assert_eq!(card.series.len(), 3);
}

#[test]
fn a_job_with_no_total_shows_its_count_and_no_bar_or_eta() {
    let mut board = JobBoard::default();
    for (at, done) in [(0, 10.0), (MIN, 20.0), (2 * MIN, 30.0), (3 * MIN, 373.0)] {
        board.report(
            &Report { done: Some(done), unit: Some("rows".into()), ..Report::named("scan") },
            NOON + at,
        );
    }
    let cards = cards(&board, NOON + 3 * MIN, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(
        (card.counts.as_str(), card.percent.as_deref(), card.fraction),
        ("373 rows", None, None)
    );
    assert_eq!((card.eta.as_deref(), card.done_at.as_deref()), (None, None));
    assert_eq!(tab_job(&board, NOON + 3 * MIN), None);
}

#[test]
fn a_small_whole_total_is_segments_with_the_active_one_after_the_done_ones() {
    let mut board = JobBoard::default();
    let report = Report {
        done: Some(3.0),
        total: Some(5.0),
        unit: Some("checks".into()),
        phase: Some("macOS running".into()),
        eta_s: Some(180),
        ..Report::named("PR checks")
    };
    board.report(&report, NOON);
    let cards = cards(&board, NOON, &TimeZone::UTC);
    let [Card::Segments(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.label, "3 of 5 checks");
    assert_eq!(card.sub, "macOS running \u{b7} about 3m left");
    assert_eq!(
        card.segments,
        [Segment::Done, Segment::Done, Segment::Done, Segment::Active, Segment::Todo]
    );
}

#[test]
fn a_stalled_job_says_where_it_stuck_and_carries_the_message_to_send() {
    let mut board = JobBoard::default();
    running(&mut board, "Import users", 21_940.0, &[(0, 10_000.0), (MIN, 21_370.0)]);
    let now = NOON + 7 * MIN;
    assert!(board.tick(now));
    let cards = cards(&board, now, &TimeZone::UTC);
    let [Card::Stalled(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.line, "Stuck at 97.4% for 6m");
    assert_eq!(card.ask, "Import users has not moved for 6m. Check on it.");
}

#[test]
fn a_finished_job_is_one_line_with_its_count_and_duration_and_failures_say_so() {
    let mut board = JobBoard::default();
    let start =
        Report { done: Some(0.0), unit: Some("passed".into()), ..Report::named("cargo test") };
    board.report(&start, NOON);
    let end =
        Report { done: Some(2_246.0), state: Some(JobState::Done), ..Report::named("cargo test") };
    board.report(&end, NOON + 4 * MIN);
    board
        .report(&Report { state: Some(JobState::Failed), ..Report::named("lint") }, NOON + 5 * MIN);
    let lines: Vec<_> = cards(&board, NOON + 9 * MIN, &TimeZone::UTC)
        .into_iter()
        .map(|card| match card {
            Card::Finished(line) => (line.name, line.failed, line.detail),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("lint".to_owned(), true, "failed \u{b7} 0s".to_owned()),
            ("cargo test".to_owned(), false, "2,246 passed \u{b7} 4m".to_owned()),
        ]
    );
}

#[test]
fn cards_run_running_then_stalled_then_finished() {
    let mut board = orders();
    running(&mut board, "Import users", 21_940.0, &[(-20 * MIN, 5.0), (-19 * MIN, 21_370.0)]);
    board.report(&Report { state: Some(JobState::Done), ..Report::named("old") }, NOON);
    running(&mut board, "PR checks", 5.0, &[(0, 3.0)]);
    assert!(board.tick(NOON + 4 * MIN));
    let kinds: Vec<&str> = cards(&board, NOON + 4 * MIN, &TimeZone::UTC)
        .iter()
        .map(|card| match card {
            Card::Bar(_) => "bar",
            Card::Segments(_) => "segments",
            Card::Stalled(_) => "stalled",
            Card::Finished(_) => "finished",
        })
        .collect();
    assert_eq!(kinds, ["bar", "segments", "stalled", "finished"]);
}

#[test]
fn the_tab_gets_a_fraction_a_rounded_label_and_a_tip_while_the_lead_job_runs() {
    let now = NOON;
    let tab = tab_job(&orders(), now).unwrap();
    assert_eq!(tab.label, "21%");
    assert!(!tab.stalled);
    assert!((tab.fraction - 0.212).abs() < 0.001);
    assert_eq!(tab.tip, "Backfill orders: 12,420 / 58,650 \u{b7} 1h 51m left");
    let mut done = orders();
    done.report(&Report { state: Some(JobState::Done), ..Report::named("Backfill orders") }, now);
    assert_eq!(tab_job(&done, now), None);
}

#[test]
fn a_stalled_lead_turns_the_tab_bar_orange_and_names_the_wait() {
    let mut board = orders();
    let now = NOON + 8 * MIN;
    board.tick(now);
    let tab = tab_job(&board, now).unwrap();
    assert!(tab.stalled);
    assert!(tab.tip.ends_with("stuck for 8m"), "{}", tab.tip);
}

#[test]
fn the_lane_line_is_percent_then_eta_and_never_empty() {
    assert_eq!(lane_line(&orders(), NOON).as_deref(), Some("21% \u{b7} 1h 51m"));
    let mut early = JobBoard::default();
    running(&mut early, "orders", 100.0, &[(0, 4.0)]);
    assert_eq!(lane_line(&early, NOON).as_deref(), Some("4%"));
    let mut loose = JobBoard::default();
    loose.report(&Report::named("deploy"), NOON);
    assert_eq!(lane_line(&loose, NOON).as_deref(), Some("deploy"));
    let mut stuck = orders();
    stuck.tick(NOON + 9 * MIN);
    assert_eq!(lane_line(&stuck, NOON + 9 * MIN).as_deref(), Some("21% \u{b7} stuck 9m"));
    assert_eq!(lane_line(&JobBoard::default(), NOON), None);
}

#[test]
fn the_clock_label_follows_the_zone() {
    let zone = TimeZone::fixed(jiff::tz::offset(3));
    assert_eq!(clock_label(NOON, &TimeZone::UTC).as_deref(), Some("12:00"));
    assert_eq!(clock_label(NOON, &zone).as_deref(), Some("15:00"));
    assert_eq!(clock_label(i64::MAX, &TimeZone::UTC), None);
}
