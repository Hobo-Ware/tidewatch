use super::*;

const MIN: i64 = 60_000;

fn with(name: &str, done: f64, total: f64) -> Report {
    Report { done: Some(done), total: Some(total), ..Report::named(name) }
}

fn steady(board: &mut JobBoard, name: &str, total: f64, readings: &[(i64, f64)]) {
    for (at, done) in readings {
        board.report(&with(name, *done, total), *at);
    }
}

fn job(board: &JobBoard) -> &Job {
    &board.jobs[0]
}

#[test]
fn rate_comes_from_the_samples_and_needs_two_that_moved() {
    let sample = |at_ms, done| Sample { at_ms, done };
    let cases: [(&[Sample], Option<f64>); 6] = [
        (&[], None),
        (&[sample(0, 5.0)], None),
        (&[sample(0, 5.0), sample(500, 9.0)], None),
        (&[sample(0, 5.0), sample(MIN, 5.0)], None),
        (&[sample(0, 0.0), sample(MIN, 400.0), sample(2 * MIN, 820.0)], Some(410.0)),
        (&[sample(0, 10.0), sample(MIN, 4.0)], None),
    ];
    for (samples, want) in cases {
        assert_eq!(rate_per_min(samples), want, "{samples:?}");
    }
}

#[test]
fn rate_looks_only_at_the_latest_samples() {
    let mut samples: Vec<Sample> =
        (0..30).map(|i| Sample { at_ms: i * MIN, done: (i * 100) as f64 }).collect();
    let slow = samples.last().copied().unwrap();
    for i in 1..=RATE_WINDOW as i64 {
        samples.push(Sample { at_ms: slow.at_ms + i * MIN, done: slow.done + (i * 10) as f64 });
    }
    assert_eq!(rate_per_min(&samples), Some(10.0));
}

#[test]
fn eta_follows_the_rate_and_hides_when_it_would_be_a_guess() {
    let cases = [
        ("three samples, steady rate", 12_431.0, Some(58_650.0), Some(412.0), 3, Some(6_731)),
        ("two samples is enough", 12_431.0, Some(58_650.0), Some(412.0), 2, Some(6_731)),
        ("one sample is not enough", 12_431.0, Some(58_650.0), Some(412.0), 1, None),
        ("no total", 12_431.0, None, Some(412.0), 9, None),
        ("a total of zero", 373.0, Some(0.0), Some(412.0), 9, None),
        ("done past the total", 58_650.0, Some(58_649.0), Some(412.0), 9, None),
        ("no rate", 100.0, Some(200.0), None, 9, None),
        ("a rate of zero", 100.0, Some(200.0), Some(0.0), 9, None),
        ("done equals total is never zero seconds", 200.0, Some(200.0), Some(5.0), 9, None),
        ("longer than a hundred days", 1.0, Some(1e12), Some(1.0), 9, None),
    ];
    for (what, done, total, rate, samples, want) in cases {
        assert_eq!(eta_secs(done, total, rate, samples), want, "{what}");
    }
}

#[test]
fn a_board_shows_no_eta_before_the_second_distinct_reading() {
    let mut board = JobBoard::default();
    board.report(&with("orders", 0.0, 1000.0), 0);
    assert_eq!(job(&board).eta_s, None);
    assert_eq!(job(&board).eta_left(0), None);
    board.report(&with("orders", 100.0, 1000.0), MIN);
    assert_eq!(job(&board).rate_per_min, Some(100.0));
    assert_eq!(job(&board).eta_s, Some(540));
    board.report(&with("orders", 200.0, 1000.0), 2 * MIN);
    assert_eq!(job(&board).eta_s, Some(480));
    assert_eq!(job(&board).eta_left(2 * MIN), Some(480));
    assert_eq!(job(&board).eta_left(2 * MIN + 100_000), Some(380));
}

#[test]
fn an_eta_that_ran_out_is_gone_not_zero_seconds() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 1000.0, &[(0, 0.0), (MIN, 100.0), (2 * MIN, 200.0)]);
    assert_eq!(job(&board).eta_left(2 * MIN + 479_000), Some(1));
    assert_eq!(job(&board).eta_left(2 * MIN + 480_000), None);
    assert_eq!(job(&board).eta_left(10 * MIN), None);
}

#[test]
fn a_name_only_report_is_no_sample() {
    let mut board = JobBoard::default();
    board.report(&Report::named("setup"), 0);
    assert_eq!(job(&board).samples.len(), 0);
    board.report(&Report { done: Some(4.0), ..Report::named("setup") }, MIN);
    assert_eq!(job(&board).samples.len(), 1);
}

#[test]
fn a_repeated_reading_adds_no_sample_and_keeps_the_stall_clock() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 1000.0, &[(0, 10.0), (MIN, 20.0)]);
    assert!(!board.report(&with("orders", 20.0, 1000.0), 2 * MIN));
    assert_eq!(job(&board).samples.len(), 2);
    assert_eq!(job(&board).changed_ms, MIN);
    assert_eq!(job(&board).updated_ms, 2 * MIN);
}

#[test]
fn a_running_job_stalls_after_five_quiet_minutes_and_wakes_when_done_moves() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 1000.0, &[(0, 10.0), (MIN, 20.0)]);
    assert!(!board.tick(MIN + 5 * MIN - 1));
    assert_eq!(job(&board).state, JobState::Running);
    assert_eq!(board.next_change_at(), Some(6 * MIN));
    assert!(board.tick(6 * MIN));
    assert_eq!(job(&board).state, JobState::Stalled);
    assert_eq!(job(&board).quiet_ms(7 * MIN), 6 * MIN);
    assert!(!board.tick(8 * MIN));
    board.report(&with("orders", 20.0, 1000.0), 8 * MIN);
    assert_eq!(job(&board).state, JobState::Stalled, "a heartbeat is not progress");
    board.report(&with("orders", 21.0, 1000.0), 9 * MIN);
    assert_eq!(job(&board).state, JobState::Running);
    assert_eq!(board.next_change_at(), Some(14 * MIN));
}

#[test]
fn finished_jobs_never_stall_and_keep_their_duration() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 10.0, &[(0, 4.0)]);
    board.report(&Report { state: Some(JobState::Done), ..Report::named("orders") }, 3 * MIN);
    assert!(!board.tick(12 * MIN));
    assert_eq!(job(&board).state, JobState::Done);
    assert_eq!(job(&board).elapsed_ms(60 * MIN), 3 * MIN);
    assert_eq!(job(&board).eta_left(3 * MIN), None);
    assert_eq!(board.next_change_at(), Some(13 * MIN));
    assert!(board.lead().is_none());
}

#[test]
fn reaching_the_total_finishes_a_job_that_named_no_state() {
    let mut board = JobBoard::default();
    steady(&mut board, "checks", 5.0, &[(0, 3.0), (MIN, 5.0)]);
    assert_eq!(job(&board).state, JobState::Done);
    assert_eq!(job(&board).finished_ms, Some(MIN));
}

#[test]
fn an_explicit_failure_is_kept_even_below_the_total() {
    let mut board = JobBoard::default();
    steady(&mut board, "checks", 5.0, &[(0, 3.0)]);
    let failed = Report { state: Some(JobState::Failed), ..Report::named("checks") };
    board.report(&failed, MIN);
    assert_eq!(job(&board).state, JobState::Failed);
}

#[test]
fn a_new_run_under_the_same_name_starts_over() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 100.0, &[(0, 10.0), (MIN, 100.0)]);
    assert_eq!(job(&board).state, JobState::Done);
    board.report(&with("orders", 2.0, 100.0), 9 * MIN);
    let again = job(&board);
    assert_eq!(again.state, JobState::Running);
    assert_eq!((again.started_ms, again.finished_ms), (9 * MIN, None));
    assert_eq!(again.samples.len(), 1);
    assert_eq!(board.jobs.len(), 1);
}

#[test]
fn done_going_backwards_restarts_the_job() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 100.0, &[(0, 10.0), (MIN, 50.0)]);
    board.report(&with("orders", 3.0, 100.0), 2 * MIN);
    assert_eq!(job(&board).samples.len(), 1);
    assert_eq!(job(&board).started_ms, 2 * MIN);
}

#[test]
fn reports_merge_by_name_ignoring_case_and_keep_what_they_leave_out() {
    let mut board = JobBoard::default();
    let first = Report {
        unit: Some("rows".into()),
        phase: Some("write".into()),
        ..with("Users backfill", 10.0, 100.0)
    };
    board.report(&first, 0);
    board.report(&Report { done: Some(20.0), ..Report::named("  users   BACKFILL ") }, MIN);
    assert_eq!(board.jobs.len(), 1);
    let merged = job(&board);
    assert_eq!(merged.name, "Users backfill");
    assert_eq!((merged.done, merged.total), (20.0, Some(100.0)));
    assert_eq!((merged.unit.as_str(), merged.phase.as_str()), ("rows", "write"));
    board
        .report(&Report { phase: Some(String::new()), ..Report::named("users backfill") }, 2 * MIN);
    assert_eq!(job(&board).phase, "");
}

#[test]
fn nonsense_numbers_are_cleaned_before_they_count() {
    let mut board = JobBoard::default();
    board.report(&with("odd", 373.0, 0.0), 0);
    assert_eq!(job(&board).total, None);
    assert_eq!(job(&board).percent(), None);
    board.report(&with("odd", f64::NAN, f64::INFINITY), MIN);
    assert_eq!(job(&board).done, 373.0);
    board.report(&with("odd", -5.0, 50.0), 2 * MIN);
    assert_eq!(job(&board).done, 0.0);
    assert!(!board.report(&Report::named("   "), 3 * MIN));
    assert_eq!(board.jobs.len(), 1);
}

#[test]
fn percent_clamps_and_a_segment_total_is_small_and_whole() {
    let mut board = JobBoard::default();
    board.report(&with("over", 58_650.0, 58_649.0), 0);
    assert_eq!(board.jobs[0].percent(), Some(100.0));
    assert_eq!(board.jobs[0].eta_s, None);
    assert_eq!(board.jobs[0].state, JobState::Running);
    let cases = [
        (Some(5.0), true),
        (Some(12.0), true),
        (Some(13.0), false),
        (Some(1.0), false),
        (Some(2.5), false),
        (None, false),
    ];
    for (total, want) in cases {
        let mut job = board.jobs[0].clone();
        job.total = total;
        assert_eq!(job.is_segmented(), want, "{total:?}");
    }
}

#[test]
fn a_hint_from_the_agent_counts_down_and_gives_way_to_the_rate() {
    let mut board = JobBoard::default();
    let hinted = Report { eta_s: Some(600), ..Report::named("deploy") };
    board.report(&hinted, 0);
    assert_eq!(job(&board).eta_left(0), Some(600));
    assert_eq!(job(&board).eta_left(100_000), Some(500));
    steady(&mut board, "deploy", 1000.0, &[(MIN, 100.0), (2 * MIN, 200.0), (3 * MIN, 300.0)]);
    assert_eq!(job(&board).eta_left(3 * MIN), Some(420));
}

#[test]
fn lead_is_the_moving_job_with_a_percent_and_never_a_finished_one() {
    let mut board = JobBoard::default();
    board.report(&Report::named("loose"), 5 * MIN);
    board.report(&with("early", 1.0, 100.0), MIN);
    board.report(&with("late", 1.0, 100.0), 2 * MIN);
    assert_eq!(board.lead().map(|job| job.name.as_str()), Some("late"));
    board.report(&Report { state: Some(JobState::Done), ..Report::named("late") }, 3 * MIN);
    assert_eq!(board.lead().map(|job| job.name.as_str()), Some("early"));
}

#[test]
fn the_board_is_capped_and_drops_a_finished_job_first() {
    let mut board = JobBoard::default();
    for i in 0..MAX_JOBS {
        board.report(&with(&format!("job {i}"), 1.0, 100.0), i as i64);
    }
    board.report(&Report { state: Some(JobState::Done), ..Report::named("job 5") }, 99);
    board.report(&with("extra", 1.0, 100.0), 100);
    assert_eq!(board.jobs.len(), MAX_JOBS);
    assert!(board.jobs.iter().all(|job| job.name != "job 5"));
    assert!(board.jobs.iter().any(|job| job.name == "extra"));
}

#[test]
fn samples_are_capped() {
    let mut board = JobBoard::default();
    for i in 0..(MAX_SAMPLES as i64 + 20) {
        board.report(&with("long", i as f64, 1e9), i * 1000);
    }
    assert_eq!(job(&board).samples.len(), MAX_SAMPLES);
}

#[test]
fn state_words_are_read_in_the_usual_spellings() {
    let cases = [
        ("Running", Some(JobState::Running)),
        ("in_progress", Some(JobState::Running)),
        ("done", Some(JobState::Done)),
        ("Completed", Some(JobState::Done)),
        ("passed", Some(JobState::Done)),
        ("error", Some(JobState::Failed)),
        ("stuck", Some(JobState::Stalled)),
        ("sleeping", None),
        ("", None),
    ];
    for (word, want) in cases {
        assert_eq!(JobState::parse(word), want, "{word:?}");
    }
}

#[test]
fn a_board_round_trips_through_json() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 100.0, &[(0, 1.0), (MIN, 2.0), (2 * MIN, 3.0)]);
    let text = serde_json::to_string(&board).unwrap();
    assert_eq!(serde_json::from_str::<JobBoard>(&text).unwrap(), board);
}

#[test]
fn a_finished_job_is_dropped_ten_minutes_after_it_finished() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 10.0, &[(0, 4.0)]);
    board.report(&Report { state: Some(JobState::Done), ..Report::named("orders") }, 3 * MIN);
    assert!(!board.tick(13 * MIN - 1));
    assert_eq!(board.jobs.len(), 1);
    assert!(board.tick(13 * MIN));
    assert!(board.is_empty());
}

#[test]
fn a_job_with_no_total_is_dropped_after_five_quiet_minutes_and_never_stalls() {
    let mut board = JobBoard::default();
    board.report(&Report::named("setup"), 0);
    assert!(!board.tick(5 * MIN - 1));
    assert_eq!(job(&board).state, JobState::Running);
    board.report(&Report::named("setup"), 2 * MIN);
    assert!(!board.tick(7 * MIN - 1), "a heartbeat keeps it");
    assert_eq!(board.next_change_at(), Some(7 * MIN));
    assert!(board.tick(7 * MIN));
    assert!(board.is_empty());
}

#[test]
fn a_job_with_a_total_is_never_dropped_for_being_quiet() {
    let mut board = JobBoard::default();
    steady(&mut board, "orders", 1000.0, &[(0, 10.0), (MIN, 20.0)]);
    board.tick(100 * MIN);
    assert_eq!(board.jobs.len(), 1);
    assert_eq!(job(&board).state, JobState::Stalled);
}

#[test]
fn a_timed_job_runs_its_clock_and_leaves_five_minutes_after_the_expected_end() {
    let mut board = JobBoard::default();
    let hinted = Report { eta_s: Some(600), ..Report::named("deploy") };
    board.report(&hinted, 0);
    assert!(job(&board).is_timed());
    assert_eq!(job(&board).time_fraction(0), Some(0.0));
    assert_eq!(job(&board).time_fraction(5 * MIN), Some(0.5));
    assert_eq!(job(&board).time_fraction(30 * MIN), Some(1.0));
    assert!(!board.tick(15 * MIN - 1));
    assert!(board.tick(15 * MIN));
}

#[test]
fn a_job_with_a_total_is_not_timed() {
    let mut board = JobBoard::default();
    board.report(&Report { eta_s: Some(600), ..with("checks", 3.0, 5.0) }, 0);
    assert!(!job(&board).is_timed());
    assert_eq!(job(&board).time_fraction(MIN), None);
}

#[test]
fn a_terminal_line_only_updates_a_job_that_is_already_there() {
    let mut board = JobBoard::default();
    assert!(!board.report_known(&with("orders", 5.0, 10.0), 0));
    assert!(board.is_empty());
    board.report(&with("orders", 1.0, 10.0), 0);
    assert!(board.report_known(&with("Orders", 5.0, 10.0), MIN));
    assert_eq!(job(&board).done, 5.0);
    board.report(&Report { state: Some(JobState::Done), ..Report::named("orders") }, 2 * MIN);
    assert!(!board.report_known(&with("orders", 1.0, 10.0), 3 * MIN));
    assert_eq!(job(&board).state, JobState::Done);
}
