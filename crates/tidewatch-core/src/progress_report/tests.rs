use serde_json::{Value, json};

use super::*;
use crate::progress::{Ledger, PhaseState};

fn args(value: &Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

fn good() -> Value {
    json!({
        "goal": "Link each recap line to its cast and backfill old rows",
        "state": "working",
        "now": "Backfill batch 2 is running",
        "stages": [
            { "name": "Build", "kind": "build", "status": "done" },
            { "name": "Backfill", "kind": "ship", "status": "active" },
            { "name": "Verify", "kind": "verify", "status": "todo" }
        ],
        "steps": [
            { "stage": "Build", "text": "Write the linker", "status": "done", "evidence": "tests pass" },
            { "stage": "Backfill", "text": "Check links", "status": "active" },
            { "stage": "Verify", "text": "Check one row", "status": "todo" }
        ]
    })
}

fn with(change: impl FnOnce(&mut Value)) -> Value {
    let mut value = good();
    change(&mut value);
    value
}

fn problems(value: &Value) -> Vec<String> {
    ProgressReport::check(&args(value), None).expect_err("the report breaks a rule")
}

fn season_recap() -> Value {
    json!({
        "goal": "Link each season recap line to the cast it names, and backfill old rows",
        "state": "needs_you",
        "now": "Backfill step 2 is running. Step 3 waits for your pick.",
        "stages": [
            { "name": "Build", "kind": "build", "status": "done" },
            { "name": "Ship", "kind": "ship", "status": "done" },
            { "name": "Backfill", "kind": "ship", "status": "active" },
            { "name": "Verify", "kind": "verify", "status": "todo" }
        ],
        "steps": [
            { "stage": "Ship", "text": "Deploy builds for c85574ded", "status": "done", "evidence": "both deploys succeeded" },
            { "stage": "Backfill", "text": "Check links against each season's cast", "status": "active" },
            { "stage": "Backfill", "text": "Guarded backfill writes", "status": "blocked" }
        ],
        "needs_you": {
            "question": "When should step 3 run?",
            "options": ["Pause after step 2", "Let it all run now", "Pause right now"],
            "recommended": 0
        },
        "waiting_on": { "job": "recap-people-backfill" }
    })
}

#[test]
fn the_contract_board_example_is_accepted() {
    let report = ProgressReport::check(&args(&season_recap()), None).unwrap();
    assert_eq!(report.state, State::NeedsYou);
    assert_eq!(report.stages.len(), 4);
    assert_eq!(report.waiting_on, Some(WaitingOn::Job("recap-people-backfill".into())));
    assert_eq!(report.needs_you.as_ref().map(|ask| ask.recommended), Some(0));
}

#[test]
fn a_complete_report_saves_with_no_problem_and_keeps_every_field() {
    let report = ProgressReport::check(&args(&good()), None).unwrap();
    assert_eq!(report.goal, "Link each recap line to its cast and backfill old rows");
    assert_eq!(report.stages[1].kind, StageKind::Ship);
    assert_eq!(report.stages[1].status, StageStatus::Active);
    assert_eq!(report.steps[0].evidence.as_deref(), Some("tests pass"));
}

#[test]
fn each_broken_rule_is_one_fix_naming_the_field() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "no state",
            with(|v| drop(v.as_object_mut().unwrap().remove("state"))),
            "state is required",
        ),
        (
            "an unknown state",
            with(|v| v["state"] = json!("asleep")),
            "state \"asleep\" is not one of working, needs_you, waiting, blocked or done",
        ),
        ("no now", with(|v| drop(v.as_object_mut().unwrap().remove("now"))), "now is required"),
        (
            "a now over 120 characters",
            with(|v| v["now"] = json!("x".repeat(121))),
            "now is 121 characters, over the 120 limit",
        ),
        (
            "no stages",
            with(|v| drop(v.as_object_mut().unwrap().remove("stages"))),
            "stages is required",
        ),
        (
            "one stage",
            with(|v| {
                v["stages"].as_array_mut().unwrap().truncate(1);
                v["steps"] = json!([]);
            }),
            "stages has 1: send 2 to 5",
        ),
        (
            "six stages",
            with(|v| {
                v["stages"] = json!((0..6).map(|n| json!({ "name": format!("S{n}"), "kind": "build", "status": "todo" })).collect::<Vec<_>>());
                v["steps"] = json!([]);
            }),
            "stages has 6: send 2 to 5",
        ),
        (
            "an unknown stage kind",
            with(|v| v["stages"][0]["kind"] = json!("deploy")),
            "stages[0].kind \"deploy\" is not one of understand, build, ship, verify or research",
        ),
        (
            "an unknown stage status",
            with(|v| v["stages"][0]["status"] = json!("finished")),
            "stages[0].status \"finished\" is not one of todo, active, done or skipped",
        ),
        (
            "a stage with no name",
            with(|v| v["stages"][0]["name"] = json!(" ")),
            "stages[0].name is required",
        ),
        (
            "a stage named twice",
            with(|v| v["stages"][1]["name"] = json!("build")),
            "stages[1].name \"build\" is used twice",
        ),
        (
            "a step in a stage that does not exist",
            with(|v| v["steps"][0]["stage"] = json!("Deploy")),
            "steps[0].stage \"Deploy\" is not one of your stages (Build, Backfill, Verify)",
        ),
        (
            "a step with no stage",
            with(|v| drop(v["steps"][0].as_object_mut().unwrap().remove("stage"))),
            "steps[0].stage is required",
        ),
        (
            "an unknown step status",
            with(|v| v["steps"][0]["status"] = json!("finished")),
            "steps[0].status \"finished\" is not one of todo, active, done, skipped or blocked",
        ),
        (
            "a step with no text",
            with(|v| v["steps"][0]["text"] = json!("")),
            "steps[0].text is required",
        ),
        ("an unknown argument", with(|v| v["note"] = json!("x")), "unknown argument \"note\""),
        (
            "an unknown task kind",
            with(|v| v["kind"] = json!("both")),
            "kind \"both\" is not read-only or edit",
        ),
    ];
    for (name, value, want) in cases {
        let found = problems(&value);
        assert!(found.iter().any(|problem| problem.contains(want)), "{name}: {found:?}");
        assert!(found.iter().all(|problem| problem.is_ascii()), "{name}: {found:?}");
    }
}

#[test]
fn several_broken_rules_are_all_listed_in_one_answer() {
    let found = problems(&with(|v| {
        v["state"] = json!("asleep");
        v["now"] = json!("");
        v["stages"][0]["kind"] = json!("deploy");
    }));
    assert_eq!(found.len(), 3, "{found:?}");
}

#[test]
fn a_report_needs_a_goal_or_a_title_the_first_time_only() {
    let first = problems(&with(|v| drop(v.as_object_mut().unwrap().remove("goal"))));
    assert!(first.iter().any(|problem| problem.contains("goal is required on the first report")));
    let saved = ProgressReport::check(&args(&good()), None).unwrap();
    let later = with(|v| drop(v.as_object_mut().unwrap().remove("goal")));
    let report = ProgressReport::check(&args(&later), Some(&saved)).unwrap();
    assert_eq!(report.goal, saved.goal);
    let titled = with(|v| {
        let map = v.as_object_mut().unwrap();
        map.remove("goal");
        map.insert("title".into(), json!("Recap links"));
    });
    assert!(ProgressReport::check(&args(&titled), None).is_ok());
}

#[test]
fn done_is_refused_while_any_step_is_todo_active_or_blocked() {
    for open in ["todo", "active", "blocked"] {
        let value = with(|v| {
            v["state"] = json!("done");
            v["stages"] = json!([
                { "name": "Build", "kind": "build", "status": "done" },
                { "name": "Verify", "kind": "verify", "status": "done" }
            ]);
            v["steps"] = json!([
                { "stage": "Build", "text": "Write it", "status": "done" },
                { "stage": "Verify", "text": "Check it live", "status": open }
            ]);
        });
        let found = problems(&value);
        assert_eq!(found.len(), 1, "{open}: {found:?}");
        assert!(found[0].contains("state is done but 1 step is still open"), "{open}: {found:?}");
        assert!(found[0].contains(&format!("\"Check it live\" ({open})")), "{found:?}");
    }
}

#[test]
fn done_is_accepted_when_every_step_is_done_or_skipped() {
    let value = with(|v| {
        v["state"] = json!("done");
        v["stages"] = json!([
            { "name": "Build", "kind": "build", "status": "done" },
            { "name": "Verify", "kind": "verify", "status": "skipped" }
        ]);
        v["steps"] = json!([
            { "stage": "Build", "text": "Write it", "status": "done" },
            { "stage": "Verify", "text": "Check it live", "status": "skipped", "reason": "no live route" }
        ]);
    });
    let report = ProgressReport::check(&args(&value), None).unwrap();
    assert_eq!(report.state, State::Done);
    assert_eq!(report.warnings().len(), 0);
}

#[test]
fn a_refused_done_names_at_most_three_steps_and_counts_the_rest() {
    let value = with(|v| {
        v["state"] = json!("done");
        v["steps"] =
            json!((0..5)
            .map(|n| json!({ "stage": "Build", "text": format!("step {n}"), "status": "todo" }))
            .collect::<Vec<_>>());
    });
    let found = problems(&value);
    assert!(found[0].contains("5 steps are still open"), "{found:?}");
    assert!(found[0].contains("and 2 more"), "{found:?}");
}

#[test]
fn a_done_report_with_an_unfinished_stage_is_saved_with_a_warning() {
    let value = with(|v| {
        v["state"] = json!("done");
        v["steps"] = json!([{ "stage": "Build", "text": "x", "status": "done" }]);
    });
    let report = ProgressReport::check(&args(&value), None).unwrap();
    let warnings = report.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("stage `Backfill` is not done or skipped"));
}

#[test]
fn a_report_with_no_steps_is_saved_with_a_warning() {
    let report = ProgressReport::check(
        &args(&with(|v| drop(v.as_object_mut().unwrap().remove("steps")))),
        None,
    )
    .unwrap();
    assert!(report.warnings()[0].contains("no steps"));
}

fn ask() -> Value {
    json!({ "question": "Which one?", "options": ["A", "B"], "recommended": 1 })
}

#[test]
fn needs_you_is_required_when_the_state_says_so_and_only_then() {
    let missing = problems(&with(|v| v["state"] = json!("needs_you")));
    assert!(missing.iter().any(|p| p.contains("state is needs_you, so needs_you is required")));
    let stray = problems(&with(|v| v["needs_you"] = ask()));
    assert!(stray.iter().any(|p| p.contains("needs_you is set but state is working")));
    let ok = with(|v| {
        v["state"] = json!("needs_you");
        v["needs_you"] = ask();
    });
    let report = ProgressReport::check(&args(&ok), None).unwrap();
    assert_eq!(report.needs_you.unwrap().options, ["A", "B"]);
}

#[test]
fn a_question_needs_two_to_four_options_and_a_recommended_index_that_exists() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "one option",
            json!({ "question": "q", "options": ["A"], "recommended": 0 }),
            "needs_you.options has 1: send 2 to 4",
        ),
        (
            "five options",
            json!({ "question": "q", "options": ["A", "B", "C", "D", "E"], "recommended": 0 }),
            "needs_you.options has 5: send 2 to 4",
        ),
        ("no options", json!({ "question": "q", "recommended": 0 }), "needs_you.options has 0"),
        (
            "an empty option",
            json!({ "question": "q", "options": ["A", " "], "recommended": 0 }),
            "must not hold an empty answer",
        ),
        (
            "a recommended past the end",
            json!({ "question": "q", "options": ["A", "B"], "recommended": 2 }),
            "needs_you.recommended must be the index",
        ),
        (
            "no recommended",
            json!({ "question": "q", "options": ["A", "B"] }),
            "needs_you.recommended must be the index",
        ),
        (
            "a negative recommended",
            json!({ "question": "q", "options": ["A", "B"], "recommended": -1 }),
            "needs_you.recommended must be the index",
        ),
        (
            "no question",
            json!({ "options": ["A", "B"], "recommended": 0 }),
            "needs_you.question is required",
        ),
    ];
    for (name, ask, want) in cases {
        let found = problems(&with(|v| {
            v["state"] = json!("needs_you");
            v["needs_you"] = ask;
        }));
        assert!(found.iter().any(|problem| problem.contains(want)), "{name}: {found:?}");
    }
}

#[test]
fn waiting_on_is_required_when_waiting_and_names_exactly_one_thing() {
    let missing = problems(&with(|v| v["state"] = json!("waiting")));
    assert!(missing.iter().any(|p| p.contains("state is waiting, so waiting_on is required")));
    let cases = [
        (json!({ "job": "backfill" }), WaitingOn::Job("backfill".into())),
        (json!({ "pr": 2340 }), WaitingOn::Pr(2340)),
        (json!({ "date": "2026-11-01" }), WaitingOn::Date(Date::parse("2026-11-01").unwrap())),
        (json!({ "event": "the partner replies" }), WaitingOn::Event("the partner replies".into())),
    ];
    for (waiting, want) in cases {
        let value = with(|v| {
            v["state"] = json!("waiting");
            v["waiting_on"] = waiting;
        });
        assert_eq!(ProgressReport::check(&args(&value), None).unwrap().waiting_on, Some(want));
    }
}

#[test]
fn a_waiting_on_that_is_malformed_is_refused() {
    let cases = [
        (
            "two things",
            json!({ "job": "a", "pr": 1 }),
            "waiting_on must hold exactly one of job, pr, date or event",
        ),
        ("nothing", json!({}), "waiting_on must hold exactly one of"),
        ("an unknown key", json!({ "ticket": "a" }), "waiting_on must hold exactly one of"),
        (
            "a bad date",
            json!({ "date": "2026-13-45" }),
            "waiting_on.date must be a date written YYYY-MM-DD",
        ),
        (
            "a date after the word on",
            json!({ "date": "on: 2026-11-01" }),
            "waiting_on.date must be a date",
        ),
        ("a pr of zero", json!({ "pr": 0 }), "waiting_on.pr must be a pull request number"),
        ("an empty job", json!({ "job": "" }), "waiting_on.job must be a non-empty string"),
    ];
    for (name, waiting, want) in cases {
        let found = problems(&with(|v| {
            v["state"] = json!("waiting");
            v["waiting_on"] = waiting;
        }));
        assert!(found.iter().any(|problem| problem.contains(want)), "{name}: {found:?}");
    }
}

#[test]
fn a_job_wait_may_ride_along_with_a_question_and_work_but_never_with_done() {
    for state in ["working", "needs_you", "blocked"] {
        let value = with(|v| {
            v["state"] = json!(state);
            v["waiting_on"] = json!({ "job": "backfill" });
            if state == "needs_you" {
                v["needs_you"] = ask();
            }
        });
        assert!(ProgressReport::check(&args(&value), None).is_ok(), "{state}");
    }
    let done = problems(&with(|v| {
        v["state"] = json!("done");
        v["steps"] = json!([]);
        v["waiting_on"] = json!({ "job": "backfill" });
    }));
    assert!(done.iter().any(|p| p.contains("waiting_on is set but state is done")), "{done:?}");
}

#[test]
fn scheduled_takes_each_typed_when_and_refuses_a_bad_one() {
    let value = with(|v| {
        v["scheduled"] = json!([
            { "when": { "date": "2026-11-01" }, "text": "a" },
            { "when": { "about": "2026-11-07" }, "text": "b", "why": "w" },
            { "when": { "event": "the partner replies" }, "text": "c" },
            { "when": { "parked": "until the rewrite" }, "text": "d" },
            { "when": { "you": true }, "text": "e" }
        ]);
    });
    let report = ProgressReport::check(&args(&value), None).unwrap();
    let whens: Vec<When> = report.scheduled.iter().map(|item| item.when.to_when()).collect();
    assert!(matches!(whens[0], When::Date(_)));
    assert!(matches!(whens[1], When::About(_)));
    assert!(matches!(whens[2], When::On(_)));
    assert!(matches!(whens[3], When::Parked(_)));
    assert_eq!(whens[4], When::You);
    let cases = [
        (
            "a date after the word on",
            json!({ "when": { "date": "on: 2026-11-01" }, "text": "x" }),
            "scheduled[0].when.date must be a date written YYYY-MM-DD",
        ),
        (
            "someday",
            json!({ "when": { "someday": true }, "text": "x" }),
            "scheduled[0].when must hold exactly one of date, about, event, parked or you",
        ),
        (
            "two whens",
            json!({ "when": { "date": "2026-11-01", "you": true }, "text": "x" }),
            "scheduled[0].when must hold exactly one of",
        ),
        ("no when", json!({ "text": "x" }), "scheduled[0].when is required"),
        ("no text", json!({ "when": { "you": true } }), "scheduled[0].text is required"),
    ];
    for (name, item, want) in cases {
        let found = problems(&with(|v| v["scheduled"] = json!([item])));
        assert!(found.iter().any(|problem| problem.contains(want)), "{name}: {found:?}");
    }
}

#[test]
fn the_three_note_lists_take_text_and_why_and_refuse_an_empty_text() {
    let value = with(|v| {
        v["verified"] = json!([{ "text": "tests", "why": "412 passed" }]);
        v["not_verified"] = json!([{ "text": "live" }]);
        v["decisions"] = json!([{ "text": "batches", "why": "lock timeout" }]);
    });
    let report = ProgressReport::check(&args(&value), None).unwrap();
    assert_eq!(report.verified[0].why.as_deref(), Some("412 passed"));
    assert_eq!(report.not_verified[0].why, None);
    let found = problems(&with(|v| v["verified"] = json!([{ "text": " " }])));
    assert!(found.iter().any(|p| p.contains("verified[0].text is required")), "{found:?}");
}

#[test]
fn a_shape_that_is_not_the_schema_is_one_problem_not_a_crash() {
    let found = problems(&with(|v| v["stages"] = json!("Build, Ship")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("the arguments do not match the schema"), "{found:?}");
}

#[test]
fn a_report_round_trips_through_json() {
    let report = ProgressReport::check(&args(&season_recap()), None).unwrap();
    let back: ProgressReport = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(back, report);
}

#[test]
fn a_typed_report_maps_onto_the_ledger_the_screens_read() {
    let ledger = ProgressReport::check(&args(&season_recap()), None).unwrap().to_ledger();
    assert_eq!(ledger.status.state, Some(State::NeedsYou));
    assert_eq!(ledger.status.note, "Backfill step 2 is running. Step 3 waits for your pick.");
    let titles: Vec<_> = ledger.checklist.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["Build", "Ship", "Backfill", "Verify"]);
    assert_eq!(ledger.checklist[2].kind, Some(StageKind::Ship));
    assert_eq!(ledger.checklist[2].state(), PhaseState::Blocked);
    assert_eq!(ledger.checklist[1].state(), PhaseState::Done);
    assert_eq!(
        ledger.checklist[0].state(),
        PhaseState::Done,
        "a stage with no steps is done as stated"
    );
    assert_eq!(ledger.checklist[3].state(), PhaseState::Todo);
    assert_eq!(ledger.current_phase().map(|p| p.title.as_str()), Some("Backfill"));
    assert_eq!(ledger.checklist[1].items[0].why.as_deref(), Some("both deploys succeeded"));
    assert!(ledger.has_question());
    assert_eq!(ledger.needs_you[0].options.iter().filter(|c| c.recommended).count(), 1);
    assert!(ledger.needs_you[0].options[0].recommended);
    assert_eq!(ledger.waiting_on, Some(WaitingOn::Job("recap-people-backfill".into())));
    assert!(ledger.awaits_owner());
}

#[test]
fn a_step_finds_its_stage_whatever_the_case_of_the_name() {
    let value = with(|v| v["steps"][1]["stage"] = json!("  backfill "));
    let report = ProgressReport::check(&args(&value), None).unwrap();
    assert_eq!(report.steps[1].stage, "Backfill");
    assert_eq!(report.to_ledger().checklist[1].items.len(), 1);
}

#[test]
fn a_wait_is_not_a_wait_on_the_owner_unless_a_question_stands_with_it() {
    let value = with(|v| {
        v["state"] = json!("waiting");
        v["waiting_on"] = json!({ "pr": 2340 });
    });
    let ledger = ProgressReport::check(&args(&value), None).unwrap().to_ledger();
    assert!(!ledger.awaits_owner());
    let old = Ledger::parse("# Goal\ng\n\n## Status\nstate: waiting\n");
    assert!(!old.awaits_owner(), "an old waiting ledger that asks nothing is still a wait");
    let asking =
        Ledger::parse("# Goal\ng\n\n## Status\nstate: waiting\n\n## Needs you\n- Pick one\n");
    assert!(asking.awaits_owner());
}

const OLD: &str = "# Goal\nShip the retry fix\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the 429 test\n\n## Checklist\n### 1. Understand\n- [x] Read the retry code -- found the loop\n### 2. Build\n- [x] Add backoff\n- [~] Add a test for a 429\n- [ ] Wire the config\n### 3. PR\n- [ ] Open the PR\n### 4. Done\n- [!] Verify live -- waits on the deploy\n\n## Decisions\n- Use jitter -- avoids a thundering herd\n\n## Verified\n- Unit tests -- cargo test\n\n## Not verified\n- Live route -- needs the deploy\n\n## Needs you\n- Which backoff cap? -- it changes the worst case -- options: 30s | 60s* | 5m\n\n## Scheduled\n- [ ] 2026-11-01 -- Drop the flag -- nothing reads it\n- [ ] on: the partner confirms -- Delete the endpoint\n- [x] 2026-10-01 -- Old item\n";

#[test]
fn an_old_markdown_ledger_maps_onto_the_four_fixed_stages_and_keeps_its_facts() {
    let report = ProgressReport::from_ledger(&Ledger::parse(OLD));
    let stages: Vec<_> =
        report.stages.iter().map(|s| (s.name.as_str(), s.kind, s.status)).collect();
    assert_eq!(
        stages,
        [
            ("Understand", StageKind::Understand, StageStatus::Done),
            ("Build", StageKind::Build, StageStatus::Active),
            ("PR", StageKind::Ship, StageStatus::Todo),
            ("Done", StageKind::Verify, StageStatus::Active),
        ]
    );
    assert_eq!(report.state, State::Working);
    assert_eq!(report.now, "writing the 429 test");
    assert_eq!(report.steps.len(), 6);
    assert_eq!(report.steps[0].evidence.as_deref(), Some("found the loop"));
    assert_eq!(report.steps[5].status, StepStatus::Blocked);
    assert_eq!(report.steps[5].reason.as_deref(), Some("waits on the deploy"));
    let ask = report.needs_you.as_ref().unwrap();
    assert_eq!((ask.options.len(), ask.recommended), (3, 1));
    assert_eq!(ask.why.as_deref(), Some("it changes the worst case"));
    assert_eq!(report.scheduled.len(), 2, "a finished item is not later work");
    assert_eq!(report.verified.len() + report.not_verified.len() + report.decisions.len(), 3);
}

#[test]
fn the_phase_an_old_ledger_names_stays_the_current_one_when_nothing_in_it_started() {
    let old = "# Goal\ng\n\n## Status\nphase: 2\nstate: working\n\n## Checklist\n### 1. Understand\n- [ ] a\n### 2. Build\n- [ ] b\n";
    let ledger = ProgressReport::from_ledger(&Ledger::parse(old)).to_ledger();
    assert_eq!(ledger.current_phase().map(|p| p.title.as_str()), Some("Build"));
}

#[test]
fn an_old_ledger_with_odd_phases_maps_without_being_refused() {
    let cases = [
        ("no checklist", "# Goal\ng\n", 0),
        ("one headless list", "# Goal\ng\n\n## Checklist\n- [ ] a\n", 1),
        (
            "seven phases",
            &format!("# Goal\ng\n\n## Checklist\n{}", "### 1. P\n- [ ] x\n".repeat(7)),
            7,
        ),
        (
            "one title twice",
            "# Goal\ng\n\n## Checklist\n### 1. Build\n- [ ] a\n### 2. Build\n- [ ] b\n",
            2,
        ),
    ];
    for (name, text, stages) in cases {
        let report = ProgressReport::from_ledger(&Ledger::parse(text));
        assert_eq!(report.stages.len(), stages, "{name}");
        assert!(report.now.chars().count() <= MAX_NOW_CHARS, "{name}");
        assert_eq!(report.to_ledger().checklist.len(), stages, "{name}");
    }
    let twice = ProgressReport::from_ledger(&Ledger::parse(
        "# Goal\ng\n\n## Checklist\n### 1. Build\n- [ ] a\n### 2. Build\n- [ ] b\n",
    ));
    assert_eq!(twice.stages[1].name, "Build (2)");
    assert_eq!(twice.steps[1].stage, "Build (2)");
}

#[test]
fn the_mapped_now_comes_from_the_note_then_the_step_in_progress_then_the_goal_and_is_cut() {
    let note = ProgressReport::from_ledger(&Ledger::parse("# Goal\ng\n\n## Status\nnote: said\n"));
    assert_eq!(note.now, "said");
    let step = ProgressReport::from_ledger(&Ledger::parse(
        "# Goal\ng\n\n## Checklist\n### 1. Build\n- [ ] later\n- [~] doing it\n",
    ));
    assert_eq!(step.now, "doing it");
    let goal = ProgressReport::from_ledger(&Ledger::parse("# Goal\nthe goal\n"));
    assert_eq!(goal.now, "the goal");
    let long =
        ProgressReport::from_ledger(&Ledger::parse(&format!("# Goal\n{}\n", "x".repeat(300))));
    assert_eq!(long.now.chars().count(), MAX_NOW_CHARS);
    assert!(long.now.ends_with("..."));
}

#[test]
fn an_old_ledger_without_a_state_reads_as_working() {
    assert_eq!(ProgressReport::from_ledger(&Ledger::parse("# Goal\ng\n")).state, State::Working);
}

#[test]
fn a_report_loaded_back_reads_the_same_as_the_ledger_it_was_saved_from() {
    let ledger = ProgressReport::from_ledger(&Ledger::parse(OLD)).to_ledger();
    let again = ProgressReport::from_ledger(&ledger).to_ledger();
    assert_eq!(again, ledger);
}

fn link(url: &str, title: &str) -> Value {
    json!({ "url": url, "title": title })
}

fn link_problems(links: &Value) -> Vec<String> {
    problems(&with(|v| v["links"] = links.clone()))
}

#[test]
fn a_report_without_links_has_none_and_a_good_one_keeps_each_field() {
    let checked = ProgressReport::check_with_links(&args(&good()), None).unwrap();
    assert_eq!(checked.links, Vec::new());
    let value = with(|v| {
        v["links"] = json!([
            { "url": " https://claude.ai/artifact/abc ", "title": " Split options ", "for": "the split question" },
            { "url": "http://localhost:3000/", "title": "Preview" },
        ]);
    });
    let checked = ProgressReport::check_with_links(&args(&value), None).unwrap();
    assert_eq!(
        checked.links,
        [
            LinkReport {
                url: "https://claude.ai/artifact/abc".into(),
                title: "Split options".into(),
                note: Some("the split question".into()),
            },
            LinkReport {
                url: "http://localhost:3000/".into(),
                title: "Preview".into(),
                note: None
            },
        ]
    );
}

#[test]
fn each_broken_link_rule_is_one_fix_naming_the_field() {
    let long_title = "t".repeat(121);
    let long_for = "f".repeat(121);
    let cases: Vec<(&str, Value, &str)> = vec![
        ("no url", json!([{ "title": "A" }]), "links[0].url is required"),
        ("an empty url", json!([link("  ", "A")]), "links[0].url is required"),
        (
            "an ftp url",
            json!([link("ftp://a.dev/x", "A")]),
            "links[0].url \"ftp://a.dev/x\" must be an http or https link",
        ),
        (
            "a script url",
            json!([link("javascript:alert(1)", "A")]),
            "links[0].url \"javascript:alert(1)\" must be an http or https link",
        ),
        (
            "a url with no host",
            json!([link("https://", "A")]),
            "links[0].url \"https://\" must be an http or https link",
        ),
        (
            "a url with a space",
            json!([link("https://a.dev/a b", "A")]),
            "must be an http or https link",
        ),
        ("no title", json!([{ "url": "https://a.dev" }]), "links[0].title is required"),
        ("a blank title", json!([link("https://a.dev", " ")]), "links[0].title is required"),
        (
            "a two line title",
            json!([link("https://a.dev", "one\ntwo")]),
            "links[0].title must be one line",
        ),
        (
            "a title of 121 characters",
            json!([link("https://a.dev", &long_title)]),
            "links[0].title is 121 characters, over the 120 limit",
        ),
        (
            "a two line for",
            json!([{ "url": "https://a.dev", "title": "A", "for": "a\nb" }]),
            "links[0].for must be one line",
        ),
        (
            "a for of 121 characters",
            json!([{ "url": "https://a.dev", "title": "A", "for": long_for }]),
            "links[0].for is 121 characters, over the 120 limit",
        ),
        (
            "the second link broken",
            json!([link("https://a.dev", "A"), link("nope", "B")]),
            "links[1].url \"nope\" must be an http or https link",
        ),
    ];
    for (name, links, want) in cases {
        let found = link_problems(&links);
        assert!(found.iter().any(|problem| problem.contains(want)), "{name}: {found:?}");
        assert_eq!(found.len(), 1, "{name}: {found:?}");
    }
}

#[test]
fn a_title_of_exactly_120_characters_is_accepted() {
    let value = with(|v| v["links"] = json!([link("https://a.dev", &"t".repeat(120))]));
    assert_eq!(ProgressReport::check_with_links(&args(&value), None).unwrap().links.len(), 1);
}

#[test]
fn a_call_takes_at_most_twenty_links() {
    let many = |count: usize| {
        Value::Array((0..count).map(|n| link(&format!("https://a.dev/{n}"), "A")).collect())
    };
    let twenty = with(|v| v["links"] = many(20));
    assert_eq!(ProgressReport::check_with_links(&args(&twenty), None).unwrap().links.len(), 20);
    let found = link_problems(&many(21));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("hold 21 links together: send at most 20"), "{found:?}");
    let split = with(|v| {
        v["state"] = json!("needs_you");
        v["links"] = many(11);
        v["needs_you"] = json!({ "question": "Which?", "options": ["A", "B"], "recommended": 0, "links": many(10) });
    });
    let found = problems(&split);
    assert!(found.iter().any(|problem| problem.contains("hold 21 links together")), "{found:?}");
}

#[test]
fn links_sent_with_a_question_are_stored_for_that_question() {
    let value = with(|v| {
        v["state"] = json!("needs_you");
        v["links"] = json!([{ "url": "https://a.dev/1", "title": "Own", "for": "own use" }]);
        v["needs_you"] = json!({
            "question": "Which split?",
            "options": ["A", "B"],
            "recommended": 0,
            "links": [{ "url": "https://a.dev/2", "title": "Options", "for": "ignored" }, link("https://a.dev/3", "Costs")],
        });
    });
    let checked = ProgressReport::check_with_links(&args(&value), None).unwrap();
    let notes: Vec<_> =
        checked.links.iter().map(|link| (link.title.as_str(), link.note.as_deref())).collect();
    assert_eq!(
        notes,
        [
            ("Own", Some("own use")),
            ("Options", Some("Which split?")),
            ("Costs", Some("Which split?"))
        ]
    );
    assert_eq!(checked.report.needs_you.unwrap().question, "Which split?");
}

#[test]
fn a_broken_link_in_needs_you_is_named_with_its_path_and_every_rule_is_listed_back() {
    let value = with(|v| {
        v["state"] = json!("needs_you");
        v["links"] = json!([link("nope", "A")]);
        v["needs_you"] = json!({
            "question": "Which?",
            "options": ["A", "B"],
            "recommended": 0,
            "links": [{ "url": "https://a.dev" }],
        });
    });
    let found = problems(&value);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().any(|p| p.contains("links[0].url \"nope\"")), "{found:?}");
    assert!(found.iter().any(|p| p.contains("needs_you.links[0].title is required")), "{found:?}");
}

#[test]
fn links_are_not_part_of_the_saved_report_json() {
    let value = with(|v| v["links"] = json!([link("https://a.dev", "A")]));
    let report = ProgressReport::check(&args(&value), None).unwrap();
    assert!(!report.to_json().contains("\"links\""));
}
