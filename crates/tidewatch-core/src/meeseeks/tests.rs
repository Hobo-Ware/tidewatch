use super::*;

const MIN: i64 = 60_000;

fn edit(task: &str, criteria: Criteria) -> Meeseeks {
    Meeseeks { kind: Kind::Edit, ..Meeseeks::summon(task, criteria, 0) }
}

fn all_on() -> Meeseeks {
    edit("rename the helper", Criteria::default())
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
        let mut meeseeks = edit("x", criteria);
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
fn a_fix_note_asks_for_the_fix_and_leaves_how_to_save_it_to_the_repos_workflow() {
    let tests = fix_note(Step::Tests, "1 failed");
    let hand_off = fix_note(Step::HandOff, "no pull request");
    for note in [&tests, &hand_off] {
        assert!(note.contains("meeseeks_done again"), "{note}");
        assert!(!note.contains("commit"), "{note}");
    }
    assert!(tests.contains("Fix them, then call meeseeks_done again."), "{tests}");
    assert!(hand_off.contains("fix it, then call meeseeks_done again."), "{hand_off}");
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
    let mut meeseeks = edit("x", Criteria { tests: false, committed: false, pr: false });
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
fn only_the_saved_facts_reach_the_file_and_an_old_file_loads_with_defaults() {
    let mut meeseeks =
        Meeseeks::summon("rename it", Criteria { tests: false, committed: true, pr: true }, 42);
    meeseeks.turns = 9;
    meeseeks.failures = 2;
    meeseeks.phase = Phase::Poofing;
    meeseeks.summary = Some("s".into());
    meeseeks.note = Some("n".into());
    meeseeks.unchanged = true;
    meeseeks.nothing_to_ship = true;
    let text = toml::to_string(&meeseeks).unwrap();
    assert!(
        !text.contains("phase")
            && !text.contains("summary")
            && !text.contains("note")
            && !text.contains("unchanged")
            && !text.contains("nothing_to_ship"),
        "{text}"
    );
    assert!(text.contains("kind = \"read-only\""), "{text}");
    let back: Meeseeks = toml::from_str(&text).unwrap();
    assert_eq!(
        back,
        Meeseeks {
            phase: Phase::Working,
            summary: None,
            note: None,
            unchanged: false,
            nothing_to_ship: false,
            ..meeseeks
        }
    );
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
        let lines = edit("x", criteria).check_lines();
        let got: Vec<bool> = lines.iter().map(|line| line.verdict == Verdict::Skipped).collect();
        let skipped = skipped.to_vec();
        assert_eq!(got, skipped, "{criteria:?}");
    }
}

fn looked(meeseeks: &mut Meeseeks, edited: bool) -> Command {
    meeseeks.apply(Event::Looked { edited }, 0)
}

#[test]
fn a_new_meeseeks_starts_read_only() {
    let meeseeks = Meeseeks::summon("how does the cache work", Criteria::default(), 0);
    assert_eq!(meeseeks.kind, Kind::ReadOnly);
}

#[test]
fn an_old_file_without_a_kind_loads_as_an_edit_and_a_saved_kind_is_kept() {
    let old: Meeseeks = toml::from_str("task = \"x\"\nturns = 3").unwrap();
    assert_eq!(old.kind, Kind::Edit);
    let saved = toml::to_string(&Meeseeks::summon("x", Criteria::default(), 0)).unwrap();
    let back: Meeseeks = toml::from_str(&saved).unwrap();
    assert_eq!(back.kind, Kind::ReadOnly);
    let head = Meeseeks { start_head: Some("abc123".into()), ..back };
    let again: Meeseeks = toml::from_str(&toml::to_string(&head).unwrap()).unwrap();
    assert_eq!(again.start_head.as_deref(), Some("abc123"));
    assert!(!saved.contains("start_head"), "{saved}");
}

#[test]
fn declaring_an_edit_promotes_and_declaring_read_only_changes_nothing() {
    let mut meeseeks = Meeseeks::summon("x", Criteria::default(), 0);
    assert_eq!(meeseeks.apply(Event::Declared(Kind::ReadOnly), 0), Command::Nothing);
    assert_eq!(meeseeks.kind, Kind::ReadOnly);
    assert_eq!(meeseeks.apply(Event::Declared(Kind::Edit), 0), Command::Nothing);
    assert_eq!(meeseeks.kind, Kind::Edit);
}

#[test]
fn a_change_seen_in_the_worktree_promotes() {
    let mut meeseeks = Meeseeks::summon("x", Criteria::default(), 0);
    looked(&mut meeseeks, false);
    assert_eq!(meeseeks.kind, Kind::ReadOnly);
    looked(&mut meeseeks, true);
    assert_eq!(meeseeks.kind, Kind::Edit);
}

#[test]
fn a_promoted_meeseeks_never_goes_back_to_read_only() {
    let mut meeseeks = Meeseeks::summon("x", Criteria::default(), 0);
    looked(&mut meeseeks, true);
    looked(&mut meeseeks, false);
    meeseeks.apply(Event::Declared(Kind::ReadOnly), 0);
    assert_eq!(meeseeks.kind, Kind::Edit);
    assert_eq!(said(&mut meeseeks), Command::Poof { summary: "renamed it".into(), pr_url: None });
}

#[test]
fn a_read_only_meeseeks_runs_no_check_and_poofs_with_its_answer() {
    let mut meeseeks = Meeseeks::summon("how does the cache work", Criteria::default(), 0);
    looked(&mut meeseeks, false);
    let said = meeseeks.apply(Event::Said("It is an LRU over the disk".into()), 0);
    assert_eq!(said, Command::Poof { summary: "It is an LRU over the disk".into(), pr_url: None });
    assert_eq!(meeseeks.phase, Phase::Poofing);
    assert_eq!(meeseeks.stages(), [StageMark::Skipped; 4]);
}

#[test]
fn a_read_only_meeseeks_that_changed_files_is_checked_like_an_edit() {
    let mut meeseeks = Meeseeks::summon("x", Criteria::default(), 0);
    looked(&mut meeseeks, true);
    assert_eq!(said(&mut meeseeks), Command::Check(Step::Change));
    assert!(!meeseeks.nothing_to_ship);
}

#[test]
fn an_edit_with_no_diff_skips_its_checks_instead_of_failing_them() {
    let mut meeseeks = all_on();
    looked(&mut meeseeks, false);
    let done = said(&mut meeseeks);
    assert_eq!(done, Command::Poof { summary: "renamed it".into(), pr_url: None });
    assert!(meeseeks.nothing_to_ship);
    assert_eq!((meeseeks.failures, meeseeks.phase), (0, Phase::Poofing));
    assert_eq!(meeseeks.stages(), [StageMark::Skipped; 4]);
}

#[test]
fn an_edit_that_was_not_looked_at_or_has_a_diff_is_checked_as_usual() {
    let mut unknown = all_on();
    assert_eq!(said(&mut unknown), Command::Check(Step::Change));
    let mut changed = all_on();
    looked(&mut changed, true);
    assert_eq!(said(&mut changed), Command::Check(Step::Change));
    assert!(!changed.nothing_to_ship);
}

#[test]
fn a_blocked_summary_poofs_at_once_with_no_check_whatever_the_kind() {
    let cases = [
        ("Blocked: the staging key is missing", Some("the staging key is missing")),
        ("  blocked:needs a human", Some("needs a human")),
        ("BLOCKED:", Some("")),
        ("Not blocked: all good", None),
        ("Blocked on nothing", None),
    ];
    for kind in [Kind::Edit, Kind::ReadOnly] {
        for (summary, reason) in cases {
            let mut meeseeks = Meeseeks { kind, ..all_on() };
            let command = meeseeks.apply(Event::Said(summary.into()), 0);
            assert_eq!(meeseeks.blocked_reason(), reason, "{summary:?}");
            let poofed = matches!(command, Command::Poof { .. });
            assert_eq!(poofed, reason.is_some() || kind == Kind::ReadOnly, "{kind:?} {summary:?}");
            if let Command::Poof { summary: said, .. } = command {
                assert_eq!(said, summary);
            }
        }
    }
}

#[test]
fn a_blocked_summary_leaves_the_failure_count_alone() {
    let mut meeseeks = all_on();
    meeseeks.apply(Event::Said("Blocked: no access".into()), 0);
    assert_eq!((meeseeks.failures, meeseeks.phase), (0, Phase::Poofing));
}

#[test]
fn an_edit_summary_stays_one_line_of_at_most_200_characters_and_a_read_only_answer_does_not() {
    let long = "x".repeat(MAX_SUMMARY_CHARS + 1);
    let multi = "Line one\nline two".to_owned();
    let fits = "y".repeat(MAX_SUMMARY_CHARS);
    for text in [&long, &multi] {
        let mut edit = all_on();
        let held = edit.apply(Event::Said(text.clone()), 0);
        let Command::Paste(note) = held else { panic!("{held:?}") };
        assert!(note.contains("one line") && note.contains("meeseeks_done"));
        assert_eq!((edit.failures, edit.phase, edit.summary.as_deref()), (0, Phase::Working, None));
        let mut read_only = Meeseeks::summon("how does it work", Criteria::default(), 0);
        let said = read_only.apply(Event::Said(text.clone()), 0);
        assert_eq!(said, Command::Poof { summary: text.clone(), pr_url: None });
    }
    assert_eq!(said_with(&mut all_on(), &fits), Command::Check(Step::Change));
}

fn said_with(meeseeks: &mut Meeseeks, summary: &str) -> Command {
    meeseeks.apply(Event::Said(summary.to_owned()), 0)
}

#[test]
fn a_check_line_says_why_it_did_not_run() {
    let mut read_only = Meeseeks::summon("x", Criteria::default(), 0);
    said(&mut read_only);
    let details = |meeseeks: &Meeseeks| {
        meeseeks.check_lines().iter().map(|line| line.detail.clone()).collect::<Vec<_>>()
    };
    assert_eq!(details(&read_only), ["Read-only, nothing to check"; 4]);
    let mut blocked = all_on();
    said_with(&mut blocked, "Blocked: no access");
    assert_eq!(details(&blocked), ["Blocked, not run"; 4]);
    let mut unchanged = all_on();
    looked(&mut unchanged, false);
    said(&mut unchanged);
    assert_eq!(details(&unchanged), ["Nothing changed"; 4]);
}

#[test]
fn a_read_only_answer_that_leaves_the_user_a_decision_is_held_once_and_told_to_ask() {
    let mut meeseeks = Meeseeks::summon("can we use bun fmt", Criteria::default(), 0);
    looked(&mut meeseeks, false);
    let summary = "bun fmt can replace biome. Waiting on the user to decide.";
    assert_eq!(meeseeks.apply(Event::Said(summary.into()), 0), Command::Paste(ASK_INSTEAD.into()));
    assert_eq!((meeseeks.phase, meeseeks.summary.clone()), (Phase::Working, None));
    let again = meeseeks.apply(Event::Said(summary.into()), 0);
    assert_eq!(again, Command::Poof { summary: summary.into(), pr_url: None });
}

#[test]
fn a_summary_waits_on_the_user_only_when_it_hands_back_a_choice() {
    let cases = [
        ("Waiting on the user to decide.", true),
        ("Should we switch to bun fmt?", true),
        ("Your call: keep Biome or switch.", true),
        ("Let me know which one you want.", true),
        ("It is an LRU over the disk.", false),
        ("Renamed the helper and updated the callers.", false),
    ];
    for (summary, want) in cases {
        assert_eq!(waits_on_user(summary), want, "{summary}");
    }
}

#[test]
fn the_shown_name_is_the_chosen_one_else_a_slug_of_the_task_else_nothing() {
    let mut meeseeks =
        Meeseeks::summon("Fix the https validation on signup", Criteria::default(), 0);
    assert_eq!(meeseeks.shown_name().as_deref(), Some("fix-https-validation-signup"));
    meeseeks.name = Some("signup-https".into());
    assert_eq!(meeseeks.shown_name().as_deref(), Some("signup-https"));
    let wordless = Meeseeks::summon("?! ...", Criteria::default(), 0);
    assert_eq!(wordless.shown_name(), None);
}

#[test]
fn a_chosen_name_changes_the_hash_the_session_signature_reads() {
    let hash = |meeseeks: &Meeseeks| {
        let mut hasher = std::hash::DefaultHasher::new();
        meeseeks.hash(&mut hasher);
        hasher.finish()
    };
    let plain = all_on();
    let named = Meeseeks { name: Some("rename-helper".into()), ..all_on() };
    assert_ne!(hash(&plain), hash(&named));
    assert_eq!(hash(&named), hash(&Meeseeks { name: Some("rename-helper".into()), ..all_on() }));
}
