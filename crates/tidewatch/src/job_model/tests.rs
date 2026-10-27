use tidewatch_core::job::{Agent, Report};

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
fn a_running_job_is_a_bar_card_with_a_percent_counts_and_an_eta() {
    let cards = cards(&orders(), NOON, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.name, "Backfill orders");
    assert_eq!(card.right.as_deref(), Some("21%"));
    assert_eq!(card.detail, "12,420 of 58,650 \u{b7} about 1h 51m left");
    assert_eq!((card.fraction * 1000.0).round(), 212.0);
}

#[test]
fn no_total_means_one_quiet_line_never_a_card_with_a_number() {
    let mut board = JobBoard::default();
    board.report(&Report::named("Worktree setup"), NOON);
    for done in [None, Some(373.0)] {
        board.report(&Report { done, unit: Some("rows".into()), ..Report::named("scan") }, NOON);
    }
    let cards = cards(&board, NOON + 90_000, Agent::Working, &TimeZone::UTC);
    let lines: Vec<_> = cards
        .iter()
        .map(|card| match card {
            Card::Quiet(line) => (line.name.as_str(), line.elapsed.as_str()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(lines, [("Worktree setup", "1m"), ("scan", "1m")]);
    assert_eq!(tab_job(&board, NOON, Agent::Working), None);
}

#[test]
fn a_zero_is_never_the_headline_and_an_eta_waits_for_a_rate() {
    let mut board = JobBoard::default();
    running(&mut board, "orders", 58_650.0, &[(0, 0.0)]);
    let cards = cards(&board, NOON, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!((card.right.as_deref(), card.fraction), (None, 0.0));
    assert_eq!(card.detail, "Starting");
    assert_eq!(tab_job(&board, NOON, Agent::Working), None);
    assert_eq!(lane_line(&board, NOON, Agent::Working).as_deref(), Some("orders"));
    running(&mut board, "orders", 58_650.0, &[(MIN, 100.0)]);
    let cards = super::cards(&board, NOON + MIN, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.right.as_deref(), Some("<1%"));
    assert_eq!(card.detail, "100 of 58,650 \u{b7} about 9h 45m left");
}

#[test]
fn a_percent_reads_whole_never_zero_and_never_100_before_the_end() {
    let cases = [
        (0.0, None),
        (-3.0, None),
        (f64::NAN, None),
        (0.2, Some("<1%")),
        (1.0, Some("1%")),
        (21.2, Some("21%")),
        (97.4, Some("97%")),
        (99.6, Some("99%")),
        (100.0, Some("100%")),
    ];
    for (percent, want) in cases {
        assert_eq!(percent_text(percent).as_deref(), want, "{percent}");
    }
}

#[test]
fn a_timed_job_fills_by_the_clock_and_says_about_how_long_is_left() {
    let mut board = JobBoard::default();
    board.report(&Report { eta_s: Some(1_200), ..Report::named("Deploy staging") }, NOON);
    let cards = cards(&board, NOON + 5 * MIN, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!((card.fraction * 100.0).round(), 25.0);
    assert_eq!(card.right.as_deref(), Some("5m"));
    assert_eq!(card.detail, "about 15m left");
    let past = super::cards(&board, NOON + 25 * MIN, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = past.as_slice() else { panic!("{past:?}") };
    assert_eq!((card.fraction, card.detail.as_str()), (1.0, "overdue by 5m"));
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
    let cards = cards(&board, NOON, Agent::Working, &TimeZone::UTC);
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
    assert!(board.tick(now, Agent::Working));
    let cards = cards(&board, now, Agent::Working, &TimeZone::UTC);
    let [Card::Stalled(card)] = cards.as_slice() else { panic!("{cards:?}") };
    assert_eq!(card.line, "Import users stuck at 97% for 6m");
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
    let lines: Vec<_> = cards(&board, NOON + 9 * MIN, Agent::Working, &TimeZone::UTC)
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
    assert!(board.tick(NOON + 4 * MIN, Agent::Working));
    let kinds: Vec<&str> = cards(&board, NOON + 4 * MIN, Agent::Working, &TimeZone::UTC)
        .iter()
        .map(|card| match card {
            Card::Bar(_) => "bar",
            Card::Segments(_) => "segments",
            Card::Quiet(_) => "quiet",
            Card::Stalled(_) => "stalled",
            Card::Finished(_) => "finished",
        })
        .collect();
    assert_eq!(kinds, ["bar", "segments", "stalled", "finished"]);
}

#[test]
fn the_tab_gets_a_fraction_a_rounded_label_and_a_tip_while_the_lead_job_runs() {
    let now = NOON;
    let tab = tab_job(&orders(), now, Agent::Working).unwrap();
    assert_eq!(tab.label, "21%");
    assert!(!tab.stalled);
    assert!((tab.fraction - 0.212).abs() < 0.001);
    assert_eq!(tab.tip, "Backfill orders: 12,420 / 58,650 \u{b7} 1h 51m left");
    let mut done = orders();
    done.report(&Report { state: Some(JobState::Done), ..Report::named("Backfill orders") }, now);
    assert_eq!(tab_job(&done, now, Agent::Working), None);
}

#[test]
fn a_stalled_lead_turns_the_tab_bar_orange_and_names_the_wait() {
    let mut board = orders();
    let now = NOON + 8 * MIN;
    board.tick(now, Agent::Working);
    let tab = tab_job(&board, now, Agent::Working).unwrap();
    assert!(tab.stalled);
    assert!(tab.tip.ends_with("stuck for 8m"), "{}", tab.tip);
}

#[test]
fn the_lane_line_is_percent_then_eta_and_never_empty() {
    assert_eq!(lane_line(&orders(), NOON, Agent::Working).as_deref(), Some("21% \u{b7} 1h 51m"));
    let mut early = JobBoard::default();
    running(&mut early, "orders", 100.0, &[(0, 4.0)]);
    assert_eq!(lane_line(&early, NOON, Agent::Working).as_deref(), Some("4%"));
    let mut loose = JobBoard::default();
    loose.report(&Report::named("deploy"), NOON);
    assert_eq!(lane_line(&loose, NOON, Agent::Working).as_deref(), Some("deploy"));
    let mut stuck = orders();
    stuck.tick(NOON + 9 * MIN, Agent::Working);
    assert_eq!(
        lane_line(&stuck, NOON + 9 * MIN, Agent::Working).as_deref(),
        Some("21% \u{b7} stuck 9m")
    );
    assert_eq!(lane_line(&JobBoard::default(), NOON, Agent::Working), None);
}

#[test]
fn a_silent_background_launch_is_a_quiet_line_before_the_finished_ones() {
    let mut board = orders();
    board.report(
        &Report { state: Some(tidewatch_core::job::JobState::Done), ..Report::named("tests") },
        NOON,
    );
    let mut all = cards(&board, NOON, Agent::Working, &TimeZone::UTC);
    let mut watch = tidewatch_core::background::Watch::default();
    let signal = tidewatch_core::background::Signal::Started {
        tool_use_id: "toolu_1".into(),
        command: "./import.sh".into(),
    };
    watch.on_signal(signal, NOON - 125 * MIN);
    let silent: Vec<Launch> = watch.silent(None, NOON).into_iter().cloned().collect();
    add_background(&mut all, &silent, NOON);
    let Some(Card::Quiet(line)) = all.iter().find(|card| matches!(card, Card::Quiet(_))) else {
        panic!("no quiet line");
    };
    assert_eq!(line.name, "./import.sh running in the background, no progress reported");
    assert_eq!(line.elapsed, "2h 5m");
    let line_at = all.iter().position(|card| matches!(card, Card::Quiet(_))).unwrap();
    let first_finished = all.iter().position(|card| matches!(card, Card::Finished(_))).unwrap();
    assert!(line_at < first_finished);
    let mut none = Vec::new();
    add_background(&mut none, &[], NOON);
    assert_eq!(none.len(), 0);
}

#[test]
fn the_clock_label_follows_the_zone() {
    let zone = TimeZone::fixed(jiff::tz::offset(3));
    assert_eq!(clock_label(NOON, &TimeZone::UTC).as_deref(), Some("12:00"));
    assert_eq!(clock_label(NOON, &zone).as_deref(), Some("15:00"));
    assert_eq!(clock_label(i64::MAX, &TimeZone::UTC), None);
}

fn wait_script(board: &mut JobBoard, name: &str, readings: &[(i64, f64)], state: JobState) {
    for (at, done) in readings {
        let report = Report {
            done: Some(*done),
            total: Some(100.0),
            state: Some(state),
            ..Report::named(name)
        };
        board.report(&report, NOON + at);
    }
}

fn card_words(cards: &[Card]) -> Vec<String> {
    cards
        .iter()
        .map(|card| match card {
            Card::Finished(line) => {
                format!("finished {} | {} | ended {}", line.name, line.detail, line.ended)
            }
            Card::Stalled(card) => format!("stalled {}", card.line),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn two_wait_scripts_that_never_said_finished_are_no_orange_cards_once_the_agent_is_done() {
    let mut board = JobBoard::default();
    let running = JobState::Running;
    wait_script(&mut board, "deploy api", &[(0, 20.0), (60 * MIN, 100.0)], running);
    wait_script(&mut board, "codex late review", &[(0, 10.0), (60 * MIN, 100.0)], running);
    let want = [
        "finished deploy api | done \u{b7} 1h | ended false",
        "finished codex late review | done \u{b7} 1h | ended false",
    ];
    for agent in [Agent::Done, Agent::Working] {
        let mut board = board.clone();
        let soon = NOON + 61 * MIN;
        board.tick(soon, agent);
        assert_eq!(
            card_words(&cards(&board, soon, Agent::Working, &TimeZone::UTC)),
            want,
            "{agent:?}"
        );
        let later = NOON + 6 * 60 * MIN + 6 * MIN;
        board.tick(later, agent);
        assert_eq!(
            card_words(&cards(&board, later, Agent::Working, &TimeZone::UTC)),
            Vec::<String>::new(),
            "{agent:?}"
        );
    }
}

#[test]
fn a_job_the_agent_left_unfinished_is_a_muted_ended_line_for_a_done_agent_and_orange_for_a_working_one()
 {
    let build = || {
        let mut board = JobBoard::default();
        wait_script(&mut board, "deploy api", &[(0, 20.0), (MIN, 97.0)], JobState::Running);
        board
    };
    let now = NOON + 6 * 60 * MIN + 7 * MIN;
    let (mut working, mut done) = (build(), build());
    working.tick(now, Agent::Working);
    done.tick(now, Agent::Done);
    assert_eq!(
        card_words(&cards(&working, now, Agent::Working, &TimeZone::UTC)),
        ["stalled deploy api stuck at 97% for 6h 6m"]
    );
    assert_eq!(
        card_words(&cards(&done, now, Agent::Working, &TimeZone::UTC)),
        ["finished deploy api | ended, no final report \u{b7} 6h 7m | ended true"]
    );
}

#[test]
fn a_timed_job_past_its_time_says_overdue_and_a_done_agent_gets_an_ended_line_instead() {
    let mut board = JobBoard::default();
    board.report(&Report { eta_s: Some(600), ..Report::named("late review") }, NOON);
    let now = NOON + 12 * MIN;
    let late = cards(&board, now, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = late.as_slice() else { panic!("{late:?}") };
    assert_eq!((card.fraction, card.detail.as_str()), (1.0, "overdue by 2m"));
    let early = cards(&board, NOON + 9 * MIN, Agent::Working, &TimeZone::UTC);
    let [Card::Bar(card)] = early.as_slice() else { panic!("{early:?}") };
    assert_eq!(card.detail, "about 1m left");
    board.tick(now, Agent::Done);
    assert_eq!(
        card_words(&cards(&board, now, Agent::Working, &TimeZone::UTC)),
        ["finished late review | ended, no final report \u{b7} 12m | ended true"]
    );
}

fn codex_review() -> JobBoard {
    let mut board = JobBoard::default();
    let report =
        |done| Report { done: Some(done), total: Some(100.0), ..Report::named("codex-2360") };
    board.report(&report(10.0), NOON);
    board.report(&report(50.0), NOON + MIN);
    board
}

#[test]
fn a_stalled_job_is_an_alarm_only_while_its_agent_works_and_a_muted_line_otherwise() {
    let mut board = codex_review();
    let now = NOON + MIN + 6 * 60 * MIN + 30 * MIN;
    board.tick(now - 31 * MIN, Agent::Working);
    assert_eq!(board.jobs[0].state, JobState::Stalled);
    let working = card_words(&cards(&board, now, Agent::Working, &TimeZone::UTC));
    assert_eq!(working, ["stalled codex-2360 stuck at 50% for 6h 30m"]);
    for agent in [Agent::Idle, Agent::Done] {
        let muted = cards(&board, now, agent, &TimeZone::UTC);
        let [Card::Quiet(line)] = muted.as_slice() else { panic!("{agent:?} {muted:?}") };
        assert_eq!(line.name, "codex-2360 \u{b7} no report for 6h 30m", "{agent:?}");
        assert_eq!(line.elapsed, "", "{agent:?}");
    }
}

#[test]
fn a_job_the_waiting_agent_never_heard_back_on_ends_with_the_time_of_its_last_report() {
    let mut board = codex_review();
    board.tick(NOON + 10 * MIN, Agent::Idle);
    let waiting = cards(&board, NOON + 10 * MIN, Agent::Idle, &TimeZone::UTC);
    let [Card::Quiet(line)] = waiting.as_slice() else { panic!("{waiting:?}") };
    assert_eq!(line.name, "codex-2360 \u{b7} no report for 9m");
    let end = NOON + 31 * MIN;
    board.tick(end, Agent::Idle);
    assert_eq!(
        card_words(&cards(&board, end, Agent::Idle, &TimeZone::UTC)),
        ["finished codex-2360 | no report since 12:01 \u{b7} 31m | ended true"]
    );
    board.tick(end + 10 * MIN, Agent::Idle);
    assert_eq!(
        card_words(&cards(&board, end + 10 * MIN, Agent::Idle, &TimeZone::UTC)),
        Vec::<String>::new()
    );
}

#[test]
fn the_time_of_the_last_report_follows_the_zone() {
    let mut board = codex_review();
    board.tick(NOON + 31 * MIN, Agent::Idle);
    let zone = TimeZone::fixed(jiff::tz::offset(2));
    let line = &cards(&board, NOON + 31 * MIN, Agent::Idle, &zone)[0];
    let Card::Finished(line) = line else { panic!("{line:?}") };
    assert_eq!(line.detail, "no report since 14:01 \u{b7} 31m");
}

#[test]
fn the_tab_and_the_lane_do_not_sound_the_alarm_for_an_agent_that_is_not_working() {
    let mut board = codex_review();
    let now = NOON + 9 * MIN;
    board.tick(now, Agent::Working);
    let alarm = tab_job(&board, now, Agent::Working).unwrap();
    assert!(alarm.stalled);
    assert!(alarm.tip.ends_with("stuck for 8m"), "{}", alarm.tip);
    assert_eq!(lane_line(&board, now, Agent::Working).as_deref(), Some("50% \u{b7} stuck 8m"));
    for agent in [Agent::Idle, Agent::Done] {
        let tab = tab_job(&board, now, agent).unwrap();
        assert!(!tab.stalled, "{agent:?}");
        assert!(tab.tip.ends_with("no report for 8m"), "{}", tab.tip);
        assert_eq!(lane_line(&board, now, agent).as_deref(), Some("50% \u{b7} no report 8m"));
    }
}

#[test]
fn only_a_working_tone_is_a_working_agent() {
    use crate::status_view::Tone;
    let cases = [
        (Tone::Working, Agent::Working),
        (Tone::Done, Agent::Done),
        (Tone::Waiting, Agent::Idle),
        (Tone::Needs, Agent::Idle),
        (Tone::Blocked, Agent::Idle),
        (Tone::Setup, Agent::Idle),
        (Tone::Failed, Agent::Idle),
        (Tone::Stopped, Agent::Idle),
        (Tone::Shell, Agent::Idle),
    ];
    for (tone, want) in cases {
        assert_eq!(agent_of(tone), want, "{tone:?}");
    }
}
