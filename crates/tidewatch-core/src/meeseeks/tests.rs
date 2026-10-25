use super::*;

const MIN: i64 = 60_000;

fn all_on() -> Meeseeks {
    Meeseeks::summon("rename the helper", Criteria::default(), 0)
}

fn said(meeseeks: &mut Meeseeks) -> Command {
    meeseeks.apply(Event::Said("renamed it".into()), 0)
}

fn pass(meeseeks: &mut Meeseeks, step: Step) -> Command {
    meeseeks.apply(Event::Passed { step, pr_url: None, detail: String::new() }, 0)
}

#[test]
fn the_box_gives_one_task_per_non_blank_line() {
    let cases = [
        ("", Vec::<&str>::new()),
        ("  \n\n", vec![]),
        ("one", vec!["one"]),
        ("  one \n\ntwo\r\n   \nthree  ", vec!["one", "two", "three"]),
    ];
    for (text, want) in cases {
        assert_eq!(tasks_of(text), want, "{text:?}");
    }
}

#[test]
fn every_tick_starts_on() {
    assert_eq!(Criteria::default(), Criteria { tests: true, committed: true, pr: true });
}

#[test]
fn the_three_struggle_rules_fire_at_their_limits_and_failed_checks_win_a_tie() {
    let mut meeseeks = all_on();
    assert_eq!(meeseeks.struggle(0), None);
    meeseeks.failures = MAX_FAILURES - 1;
    meeseeks.turns = MAX_TURNS - 1;
    assert_eq!(meeseeks.struggle(MAX_MINUTES * MIN - 1), None);
    assert_eq!(meeseeks.struggle(MAX_MINUTES * MIN), Some(Struggle::Time));
    meeseeks.turns += 1;
    assert_eq!(meeseeks.struggle(0), Some(Struggle::Turns));
    meeseeks.failures += 1;
    assert_eq!(meeseeks.struggle(MAX_MINUTES * MIN), Some(Struggle::FailedChecks));
}

#[test]
fn a_struggle_says_why_in_the_words_of_the_card() {
    let reasons = [Struggle::FailedChecks, Struggle::Turns, Struggle::Time].map(Struggle::reason);
    assert_eq!(reasons, ["3 failed checks", "40 turns", "60 minutes"]);
}

#[test]
fn a_stop_hook_is_a_turn_and_the_clock_never_runs_backwards() {
    let mut meeseeks = Meeseeks::summon("x", Criteria::default(), 5_000);
    meeseeks.turn();
    meeseeks.turn();
    assert_eq!(meeseeks.turns, 2);
    assert_eq!(meeseeks.elapsed_ms(1_000), 0);
    assert_eq!(meeseeks.elapsed_ms(65_000), MIN);
}

#[test]
fn with_everything_ticked_the_checks_run_in_order_and_the_pr_url_reaches_the_poof() {
    let mut meeseeks = all_on();
    assert_eq!(said(&mut meeseeks), Command::Check(Step::Change));
    assert_eq!(pass(&mut meeseeks, Step::Change), Command::Check(Step::Tests));
    assert_eq!(pass(&mut meeseeks, Step::Tests), Command::Check(Step::Proof));
    assert_eq!(pass(&mut meeseeks, Step::Proof), Command::Check(Step::HandOff));
    let url = Some("https://example.test/pull/7".to_owned());
    let done = meeseeks.apply(
        Event::Passed { step: Step::HandOff, pr_url: url.clone(), detail: String::new() },
        0,
    );
    assert_eq!(done, Command::Poof { summary: "renamed it".into(), pr_url: url });
    assert_eq!(meeseeks.phase, Phase::Poofing);
}

#[test]
fn an_unticked_check_is_skipped_and_the_proof_and_the_poof_always_stay() {
    let cases = [
        (
            Criteria { tests: false, committed: true, pr: true },
            vec![Step::Change, Step::Proof, Step::HandOff],
        ),
        (
            Criteria { tests: true, committed: false, pr: true },
            vec![Step::Tests, Step::Proof, Step::HandOff],
        ),
        (
            Criteria { tests: true, committed: true, pr: false },
            vec![Step::Change, Step::Tests, Step::Proof],
        ),
        (Criteria { tests: false, committed: false, pr: false }, vec![Step::Proof]),
    ];
    for (criteria, want) in cases {
        let mut meeseeks = Meeseeks::summon("x", criteria, 0);
        let mut ran = Vec::new();
        let mut command = said(&mut meeseeks);
        while let Command::Check(step) = command {
            ran.push(step);
            command = pass(&mut meeseeks, step);
        }
        assert_eq!(ran, want, "{criteria:?}");
        assert!(matches!(command, Command::Poof { pr_url: None, .. }), "{criteria:?}");
    }
}

#[test]
fn a_failed_check_pastes_the_fix_note_counts_once_and_the_next_done_starts_over() {
    let mut meeseeks = all_on();
    said(&mut meeseeks);
    pass(&mut meeseeks, Step::Change);
    let failed =
        meeseeks.apply(Event::Failed { step: Step::Tests, detail: "1 failed\nassert".into() }, 0);
    let Command::Paste(text) = failed else { panic!("{failed:?}") };
    assert!(text.contains("tests") && text.contains("assert") && text.contains("meeseeks_done"));
    assert_eq!((meeseeks.failures, meeseeks.phase), (1, Phase::Working));
    assert_eq!(meeseeks.note.as_deref(), Some("Tests failed: 1 failed"));
    assert_eq!(said(&mut meeseeks), Command::Check(Step::Change));
}

#[test]
fn an_event_for_another_step_or_phase_changes_nothing() {
    let mut meeseeks = all_on();
    let stale = [
        Event::Passed { step: Step::Change, pr_url: None, detail: String::new() },
        Event::Failed { step: Step::Tests, detail: "x".into() },
        Event::PoofRefused("dirty".into()),
    ];
    for event in stale.clone() {
        assert_eq!(meeseeks.apply(event, 0), Command::Nothing);
    }
    said(&mut meeseeks);
    let stale_while_checking = [
        Event::Passed { step: Step::Tests, pr_url: None, detail: String::new() },
        Event::Failed { step: Step::Tests, detail: "x".into() },
        Event::PoofRefused("dirty".into()),
    ];
    for event in stale_while_checking {
        assert_eq!(meeseeks.apply(event, 0), Command::Nothing);
    }
    assert_eq!((meeseeks.failures, meeseeks.phase), (0, Phase::Checking(Step::Change)));
    assert_eq!(said(&mut meeseeks), Command::Nothing, "a second done while checking is ignored");
}

#[test]
fn three_failed_checks_make_it_struggle_and_a_struggling_one_is_never_checked_again() {
    let mut meeseeks = all_on();
    for round in 1..=MAX_FAILURES {
        assert_eq!(said(&mut meeseeks), Command::Check(Step::Change), "round {round}");
        let failed =
            meeseeks.apply(Event::Failed { step: Step::Change, detail: "dirty".into() }, 0);
        assert!(matches!(failed, Command::Paste(_)));
    }
    assert_eq!(meeseeks.struggle(0), Some(Struggle::FailedChecks));
    assert_eq!(said(&mut meeseeks), Command::Nothing);
    assert_eq!(meeseeks.phase, Phase::Working);
    assert!(meeseeks.note.as_deref().unwrap().contains("Waiting for you"));
}

#[test]
fn a_refused_poof_stays_put_and_asks_the_agent_to_commit() {
    let mut meeseeks =
        Meeseeks::summon("x", Criteria { tests: false, committed: false, pr: false }, 0);
    said(&mut meeseeks);
    let poof = pass(&mut meeseeks, Step::Proof);
    assert!(matches!(poof, Command::Poof { .. }));
    let refused = meeseeks.apply(Event::PoofRefused("2 files with uncommitted changes".into()), 0);
    let Command::Paste(text) = refused else { panic!("{refused:?}") };
    assert!(text.contains("2 files with uncommitted changes"));
    assert_eq!((meeseeks.failures, meeseeks.phase), (1, Phase::Working));
}

#[test]
fn the_stage_row_follows_the_phase_and_dims_what_was_not_asked_for() {
    use StageMark::{Active, Done, Pending, Skipped};
    let mut meeseeks = all_on();
    assert_eq!(meeseeks.stages(), [Active, Pending, Pending, Pending]);
    let cases = [
        (Phase::Checking(Step::Change), [Active, Pending, Pending, Pending]),
        (Phase::Checking(Step::Tests), [Done, Active, Pending, Pending]),
        (Phase::Checking(Step::Proof), [Done, Done, Active, Pending]),
        (Phase::Checking(Step::HandOff), [Done, Done, Done, Active]),
        (Phase::Poofing, [Done, Done, Done, Active]),
    ];
    for (phase, want) in cases {
        meeseeks.phase = phase;
        assert_eq!(meeseeks.stages(), want, "{phase:?}");
    }
    meeseeks.criteria.tests = false;
    meeseeks.phase = Phase::Checking(Step::Proof);
    assert_eq!(meeseeks.stages(), [Done, Skipped, Active, Pending]);
}

#[test]
fn the_tail_keeps_the_last_twelve_lines_and_cannot_close_a_fence_or_carry_control_bytes() {
    let output =
        format!("{}\n\n", (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n"));
    let tail = tail_of(&output);
    assert_eq!(tail.lines().count(), 12);
    assert!(tail.starts_with("line 9") && tail.ends_with("line 20"));
    assert_eq!(tail_of("a ``` b\x1b[31m red\x07"), "a ''' b[31m red");
    assert_eq!(tail_of(&"x".repeat(500)).chars().count(), LINE_CHARS);
    assert_eq!(tail_of(""), "");
}

#[test]
fn elapsed_time_reads_in_minutes_then_hours() {
    let cases = [
        (-5, "0m"),
        (0, "0m"),
        (12 * MIN + 999, "12m"),
        (60 * MIN, "1h 00m"),
        (125 * MIN, "2h 05m"),
    ];
    for (ms, want) in cases {
        assert_eq!(elapsed_text(ms), want);
    }
}

#[test]
fn only_the_five_saved_facts_reach_the_file_and_an_old_file_loads_with_defaults() {
    let mut meeseeks =
        Meeseeks::summon("rename it", Criteria { tests: false, committed: true, pr: true }, 42);
    meeseeks.turns = 9;
    meeseeks.failures = 2;
    meeseeks.phase = Phase::Poofing;
    meeseeks.summary = Some("s".into());
    meeseeks.note = Some("n".into());
    let text = toml::to_string(&meeseeks).unwrap();
    assert!(
        !text.contains("phase") && !text.contains("summary") && !text.contains("note"),
        "{text}"
    );
    let back: Meeseeks = toml::from_str(&text).unwrap();
    assert_eq!(back, Meeseeks { phase: Phase::Working, summary: None, note: None, ..meeseeks });
    let old: Meeseeks = toml::from_str("task = \"x\"").unwrap();
    assert_eq!((old.turns, old.failures, old.criteria), (0, 0, Criteria::default()));
}

fn run_through(meeseeks: &mut Meeseeks, steps: &[(Step, &str)]) {
    said(meeseeks);
    for (step, detail) in steps {
        meeseeks
            .apply(Event::Passed { step: *step, pr_url: None, detail: (*detail).to_owned() }, 0);
    }
}

#[test]
fn the_check_lines_keep_the_detail_each_check_said_and_the_latest_pass_wins() {
    let mut meeseeks = all_on();
    run_through(
        &mut meeseeks,
        &[
            (Step::Change, "2 commits ahead of main"),
            (Step::Tests, "`cargo test` passed"),
            (Step::Proof, "Worktree clean"),
            (Step::HandOff, "Opened PR #7"),
        ],
    );
    let lines = meeseeks.check_lines();
    let said: Vec<(Step, Verdict, &str)> =
        lines.iter().map(|line| (line.step, line.verdict, line.detail.as_str())).collect();
    assert_eq!(
        said,
        [
            (Step::Change, Verdict::Passed, "2 commits ahead of main"),
            (Step::Tests, Verdict::Passed, "`cargo test` passed"),
            (Step::Proof, Verdict::Passed, "Worktree clean"),
            (Step::HandOff, Verdict::Passed, "Opened PR #7"),
        ]
    );
}

#[test]
fn a_check_that_passes_again_after_a_failure_keeps_only_its_latest_line() {
    let mut meeseeks = all_on();
    run_through(&mut meeseeks, &[(Step::Change, "old")]);
    meeseeks.apply(Event::Failed { step: Step::Tests, detail: "red".into() }, 0);
    run_through(&mut meeseeks, &[(Step::Change, "new")]);
    assert_eq!(meeseeks.check_lines()[0].detail, "new");
}

#[test]
fn unticked_checks_and_a_pull_request_nobody_asked_for_are_skipped() {
    let cases = [
        (Criteria { tests: false, committed: true, pr: true }, [false, true, false, false]),
        (Criteria { tests: true, committed: false, pr: true }, [true, false, false, false]),
        (Criteria { tests: true, committed: true, pr: false }, [false, false, false, true]),
    ];
    for (criteria, skipped) in cases {
        let lines = Meeseeks::summon("x", criteria, 0).check_lines();
        let got: Vec<bool> = lines.iter().map(|line| line.verdict == Verdict::Skipped).collect();
        let skipped = skipped.to_vec();
        assert_eq!(got, skipped, "{criteria:?}");
    }
}
