use tidewatch_core::job::{Agent, Report};

use super::*;

const NOW: u64 = 1000 * HOUR_MS;
const FOUR_HOURS: u64 = 4 * HOUR_MS;

fn rules(after_ms: Option<u64>) -> Rules {
    Rules { after_ms, fits: Fits { alone: 9, with_pill: 8 } }
}

fn quiet(id: u64, minutes: u64) -> Facts {
    Facts {
        id,
        group: 0,
        role: Role::Plain,
        active: false,
        keep: false,
        needs_you: false,
        park: None,
        touched_ms: NOW - minutes * MINUTE_MS,
        work: Work::None,
        schedule: Schedule::None,
    }
}

fn parked(mut tab: Facts, why: Why) -> Facts {
    tab.park = Some(Park { why, since_ms: tab.touched_ms });
    tab
}

fn day(day: u8) -> Date {
    Date::new(2026, 10, day).unwrap()
}

fn decide_for(tabs: &[Facts], after: Option<u64>) -> Vec<Change> {
    decide(tabs, NOW, &rules(after))
}

#[test]
fn an_untouched_tab_parks_once_the_wait_has_passed_and_not_before() {
    let cases = [(239, false), (240, true), (600, true), (0, false)];
    for (minutes, parks) in cases {
        let got = decide_for(&[quiet(1, minutes)], Some(FOUR_HOURS));
        let want = if parks { vec![Change::Park(1, Why::Idle)] } else { Vec::new() };
        assert_eq!(got, want, "{minutes} minutes");
    }
}

#[test]
fn with_the_setting_off_only_work_parks_a_tab() {
    assert_eq!(decide_for(&[quiet(1, 5000)], None), []);
    let mut waiting = quiet(2, 31);
    waiting.schedule = Schedule::Waiting(day(23));
    assert_eq!(decide_for(&[waiting], None), [Change::Park(2, Why::Waiting)]);
}

#[test]
fn a_long_job_or_a_dated_wait_parks_after_half_an_hour() {
    let mut job = quiet(1, 30);
    job.work = Work::Long;
    let mut wait = quiet(2, 30);
    wait.schedule = Schedule::Waiting(day(23));
    let mut short = quiet(3, 30);
    short.work = Work::Running;
    let mut early = quiet(4, 29);
    early.work = Work::Long;
    let got = decide_for(&[job, wait, short, early], Some(FOUR_HOURS));
    assert_eq!(got, [Change::Park(1, Why::Job), Change::Park(2, Why::Waiting)]);
}

#[test]
fn a_date_wins_over_a_job_and_a_job_over_idle() {
    let mut both = quiet(1, 600);
    both.work = Work::Long;
    both.schedule = Schedule::Waiting(day(23));
    let mut job = quiet(2, 600);
    job.work = Work::Long;
    assert_eq!(
        decide_for(&[both, job], Some(FOUR_HOURS)),
        [Change::Park(1, Why::Waiting), Change::Park(2, Why::Job)]
    );
}

#[test]
fn nothing_parks_the_active_tab_the_pinned_tab_a_draft_a_kept_tab_or_one_that_needs_you() {
    let make = |edit: fn(&mut Facts)| {
        let mut tab = quiet(1, 5000);
        tab.work = Work::Long;
        edit(&mut tab);
        tab
    };
    let tabs = [
        make(|tab| tab.active = true),
        make(|tab| tab.role = Role::Pinned),
        make(|tab| tab.role = Role::Draft),
        make(|tab| tab.keep = true),
        make(|tab| tab.needs_you = true),
    ];
    for tab in tabs {
        assert_eq!(decide_for(&[tab], Some(FOUR_HOURS)), [], "{tab:?}");
    }
}

#[test]
fn a_parked_tab_comes_back_when_its_agent_needs_you_or_it_is_selected_whatever_the_reason() {
    for why in [Why::Idle, Why::Job, Why::Waiting, Why::Manual, Why::Overflow] {
        let mut needs = parked(quiet(1, 5000), why);
        needs.needs_you = true;
        let mut active = parked(quiet(2, 5000), why);
        active.active = true;
        assert_eq!(
            decide_for(&[needs, active], Some(FOUR_HOURS)),
            [Change::Unpark(1), Change::Unpark(2)],
            "{why:?}"
        );
    }
}

#[test]
fn a_tab_parked_for_its_job_returns_when_the_job_finishes_or_stalls_and_not_while_it_runs() {
    let mut running = parked(quiet(1, 60), Why::Job);
    running.work = Work::Long;
    let mut shorter = parked(quiet(2, 60), Why::Job);
    shorter.work = Work::Running;
    let gone = parked(quiet(3, 60), Why::Job);
    assert_eq!(decide_for(&[running, shorter, gone], Some(FOUR_HOURS)), [Change::Unpark(3)]);
}

#[test]
fn a_tab_parked_for_a_date_returns_when_the_item_comes_due_or_goes_away() {
    let mut future = parked(quiet(1, 60), Why::Waiting);
    future.schedule = Schedule::Waiting(day(23));
    let mut due = parked(quiet(2, 60), Why::Waiting);
    due.schedule = Schedule::Due;
    let none = parked(quiet(3, 60), Why::Waiting);
    assert_eq!(
        decide_for(&[future, due, none], Some(FOUR_HOURS)),
        [Change::Unpark(2), Change::Unpark(3)]
    );
}

#[test]
fn an_idle_or_manual_park_stays_until_the_tab_is_opened() {
    let mut idle = parked(quiet(1, 9000), Why::Idle);
    idle.work = Work::Long;
    let mut manual = parked(quiet(2, 9000), Why::Manual);
    manual.schedule = Schedule::Due;
    assert_eq!(decide_for(&[idle, manual], Some(FOUR_HOURS)), []);
}

#[test]
fn a_tab_that_came_back_by_itself_is_not_parked_again_by_the_old_quiet_time() {
    let mut returned = parked(quiet(1, 9000), Why::Job);
    returned.needs_you = false;
    let first = decide_for(&[returned], Some(FOUR_HOURS));
    assert_eq!(first, [Change::Unpark(1)]);
    let mut again = quiet(1, 0);
    again.touched_ms = NOW;
    assert_eq!(decide_for(&[again], Some(FOUR_HOURS)), []);
}

fn crowd(count: u64, group: usize) -> Vec<Facts> {
    let mut tabs: Vec<Facts> = (0..count)
        .map(|n| {
            let mut tab = quiet(n + 100 * group as u64, 10 + n);
            tab.group = group;
            tab
        })
        .collect();
    tabs[0].role = Role::Pinned;
    tabs
}

#[test]
fn tabs_that_do_not_fit_park_the_least_recently_touched_first_until_they_fit() {
    let tabs = crowd(11, 0);
    let got = decide(&tabs, NOW, &rules(None));
    assert_eq!(
        got,
        [Change::Park(10, Why::Overflow), Change::Park(9, Why::Overflow)],
        "10 agent tabs against 8 with the pill, the main tab takes no place"
    );
}

#[test]
fn overflow_never_parks_what_the_rules_forbid() {
    let mut tabs = crowd(11, 0);
    tabs[10].needs_you = true;
    tabs[9].keep = true;
    tabs[8].active = true;
    let got = decide(&tabs, NOW, &rules(None));
    assert_eq!(got, [Change::Park(7, Why::Overflow), Change::Park(6, Why::Overflow)]);
}

#[test]
fn a_strip_that_fits_alone_parks_nothing_and_one_more_tab_does() {
    assert_eq!(decide(&crowd(10, 0), NOW, &rules(None)), []);
    assert_eq!(decide(&crowd(11, 0), NOW, &rules(None)).len(), 2);
}

#[test]
fn with_a_pill_already_showing_the_smaller_room_counts() {
    let mut tabs = crowd(10, 0);
    tabs[9] = parked(tabs[9], Why::Manual);
    let got = decide(&tabs, NOW, &rules(None));
    assert_eq!(got, [], "eight shown, eight fit with the pill");
    let mut tabs = crowd(11, 0);
    tabs[10] = parked(tabs[10], Why::Manual);
    assert_eq!(decide(&tabs, NOW, &rules(None)), [Change::Park(9, Why::Overflow)]);
}

#[test]
fn overflow_is_counted_per_group() {
    let mut tabs = crowd(11, 0);
    tabs.extend(crowd(3, 1));
    let got = decide(&tabs, NOW, &rules(None));
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|change| matches!(change, Change::Park(id, _) if *id < 100)));
}

#[test]
fn room_that_opens_up_brings_back_overflow_tabs_most_recently_touched_first() {
    let mut tabs = crowd(6, 0);
    for index in [3, 4, 5] {
        tabs[index] = parked(tabs[index], Why::Overflow);
    }
    tabs[2] = parked(tabs[2], Why::Manual);
    let got = decide(&tabs, NOW, &rules(None));
    assert_eq!(
        got,
        [Change::Refill(3), Change::Refill(4), Change::Refill(5)],
        "one shown, eight fit, only overflow parks return"
    );
    let roomy = Rules { fits: Fits { alone: 9, with_pill: 3 }, ..rules(None) };
    assert_eq!(decide(&tabs, NOW, &roomy), [Change::Refill(3), Change::Refill(4)], "two free");
}

#[test]
fn an_overflow_tab_that_is_also_quiet_takes_the_quiet_reason_instead_of_coming_back() {
    let mut tabs = crowd(3, 0);
    tabs[2] = parked(quiet(2, 5000), Why::Overflow);
    tabs[2].group = 0;
    let got = decide(&tabs, NOW, &rules(Some(FOUR_HOURS)));
    assert_eq!(got, [Change::Park(2, Why::Idle)]);
}

#[test]
fn touches_follow_the_selection_and_a_tab_never_seen_counts_from_now() {
    let mut touches = Touches::default();
    assert_eq!(touches.of(1, 50), 50);
    touches.observe(1, 60);
    touches.observe(1, 90);
    assert_eq!(touches.of(1, 999), 90, "still selected, touched as of the last look");
    touches.observe(2, 120);
    assert_eq!(touches.of(1, 999), 120, "touched until it was left");
    touches.observe(2, 400);
    assert_eq!(touches.of(1, 999), 120);
    touches.set(1, 500);
    assert_eq!(touches.of(1, 999), 500);
    touches.forget(1);
    assert_eq!(touches.of(1, 777), 777);
}

fn board(eta_s: Option<u64>, state: Option<JobState>, started_ago_min: i64) -> JobBoard {
    let now = 10_000_000_i64;
    let mut board = JobBoard::default();
    let at = now - started_ago_min * 60_000;
    board.report(
        &Report { done: Some(10.0), total: Some(100.0), eta_s, state, ..Report::named("orders") },
        at,
    );
    board
}

#[test]
fn a_job_is_long_by_its_eta_or_by_how_long_it_has_run() {
    let now = 10_000_000_i64;
    let cases = [
        (board(Some(7200), None, 1), Work::Long),
        (board(Some(600), None, 1), Work::Running),
        (board(None, None, 45), Work::Long),
        (board(None, None, 5), Work::Running),
        (board(Some(7200), Some(JobState::Done), 1), Work::None),
        (board(Some(7200), Some(JobState::Failed), 1), Work::None),
        (JobBoard::default(), Work::None),
    ];
    for (index, (board, want)) in cases.iter().enumerate() {
        assert_eq!(work_of(Some(board), now), *want, "case {index}");
    }
    assert_eq!(work_of(None, now), Work::None);
}

#[test]
fn a_stalled_job_is_not_work_that_keeps_a_tab_parked() {
    let now = 10_000_000_i64;
    let mut stalled = board(Some(7200), None, 1);
    stalled.tick(now + 6 * 60_000, Agent::Working);
    assert_eq!(work_of(Some(&stalled), now + 6 * 60_000), Work::None);
}

fn ledger(scheduled: &str) -> Ledger {
    Ledger::parse(&format!("# Goal\nx\n\n## Status\nstate: working\n\n## Scheduled\n{scheduled}\n"))
}

#[test]
fn the_schedule_says_what_a_tab_waits_for() {
    let today = day(20);
    let cases = [
        ("", Schedule::None),
        ("- [ ] 2026-10-23 -- Check the rollout -- why", Schedule::Waiting(day(23))),
        (
            "- [ ] 2026-10-25 -- later -- why\n- [ ] ~2026-10-22 -- sooner -- why",
            Schedule::Waiting(day(22)),
        ),
        ("- [ ] 2026-10-20 -- today -- why", Schedule::Due),
        ("- [ ] 2026-10-18 -- overdue -- why", Schedule::Due),
        ("- [ ] 2026-10-25 -- later -- why\n- [ ] 2026-10-19 -- late -- why", Schedule::Due),
        ("- [x] 2026-10-18 -- done -- why", Schedule::None),
        ("- [-] 2026-10-18 -- skipped -- why", Schedule::None),
        ("- [~] 2026-10-26 -- under way -- why", Schedule::Waiting(day(26))),
        ("- [ ] on: the beta ships -- event -- why", Schedule::None),
        ("- [ ] parked: until asked -- parked -- why", Schedule::None),
        ("- [ ] you -- decide -- why", Schedule::None),
    ];
    for (text, want) in cases {
        assert_eq!(schedule_of(&ledger(text), today), want, "{text}");
    }
}

#[test]
fn every_reason_reads_in_its_tone() {
    let park = |why| Park { why, since_ms: NOW - 6 * HOUR_MS };
    let job = JobLine { percent: Some(64), eta_secs: Some(3 * 3600 + 10 * 60) };
    let cases = [
        (park(Why::Job), job, Schedule::None, "Running 64% \u{b7} ETA 3h 10m", Tone::Teal),
        (
            park(Why::Job),
            JobLine { percent: Some(64), eta_secs: None },
            Schedule::None,
            "Running 64%",
            Tone::Teal,
        ),
        (park(Why::Job), JobLine::default(), Schedule::None, "Running", Tone::Teal),
        (
            park(Why::Waiting),
            JobLine::default(),
            Schedule::Waiting(day(23)),
            "Waiting until Fri 23 Oct",
            Tone::Blue,
        ),
        (park(Why::Waiting), JobLine::default(), Schedule::Due, "Waiting", Tone::Blue),
        (park(Why::Idle), JobLine::default(), Schedule::None, "Idle 6h", Tone::Muted),
        (park(Why::Manual), JobLine::default(), Schedule::None, "Parked by you", Tone::Muted),
        (
            park(Why::Overflow),
            JobLine::default(),
            Schedule::None,
            "No room in the strip",
            Tone::Muted,
        ),
    ];
    for (park, job, schedule, text, tone) in cases {
        let line = reason_line(park, NOW, job, schedule);
        assert_eq!((line.text.as_str(), line.tone), (text, tone), "{park:?}");
    }
}

#[test]
fn idle_time_reads_in_minutes_hours_or_days() {
    let cases = [
        (0, "1m"),
        (59 * MINUTE_MS, "59m"),
        (HOUR_MS, "1h"),
        (47 * HOUR_MS + 59 * MINUTE_MS, "47h"),
        (48 * HOUR_MS, "2d"),
        (200 * HOUR_MS, "8d"),
    ];
    for (ms, want) in cases {
        assert_eq!(idle_text(ms), want, "{ms}");
    }
}

#[test]
fn a_job_line_reads_the_leading_jobs_percent_and_time_left() {
    let now = 10_000_000_i64;
    let line = job_line(Some(&board(Some(3600), None, 1)), now);
    assert_eq!(line.percent, Some(10));
    assert!(line.eta_secs.is_some_and(|secs| (3500..=3600).contains(&secs)), "{line:?}");
    assert_eq!(job_line(None, now), JobLine::default());
}

#[test]
fn an_idle_window_wakes_for_the_soonest_quiet_threshold_of_a_tab_that_can_still_be_parked() {
    let mut job = quiet(1, 10);
    job.work = Work::Long;
    let plain = quiet(2, 100);
    let mut needs = quiet(3, 0);
    needs.needs_you = true;
    let mut gone = parked(quiet(4, 0), Why::Manual);
    gone.touched_ms = NOW;
    let cases = [
        (vec![job], Some(FOUR_HOURS), Some(20 * MINUTE_MS + 500)),
        (vec![plain], Some(FOUR_HOURS), Some(140 * MINUTE_MS + 500)),
        (vec![job, plain], Some(FOUR_HOURS), Some(20 * MINUTE_MS + 500)),
        (vec![plain], None, None),
        (vec![needs, gone], Some(FOUR_HOURS), None),
        (vec![quiet(5, 300)], Some(FOUR_HOURS), None),
    ];
    for (tabs, after, want) in cases {
        assert_eq!(next_look_ms(&tabs, NOW, &rules(after)), want, "{tabs:?}");
    }
}
