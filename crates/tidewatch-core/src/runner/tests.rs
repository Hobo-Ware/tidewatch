use std::collections::{HashMap, VecDeque};
use std::path::Path;

use super::*;
use crate::paths::TidewatchPaths;
use crate::status::HookEvent;
use crate::workflows::{StageRun, Workflows};

fn stage(action: Action, gate: Gate) -> Stage {
    Stage {
        run: StageRun { agent: Some("claude".into()), ..StageRun::default() },
        gate,
        ..Stage::new(action)
    }
}

fn looping(action: Action, gate: Gate, on_fail: usize, max_loops: u8) -> Stage {
    Stage { on_fail: Some(on_fail), max_loops, ..stage(action, gate) }
}

fn workflow(stages: Vec<Stage>) -> Workflow {
    Workflow { id: "t".into(), name: "Test".into(), description: String::new(), stages }
}

fn ctx(test_command: Option<&str>) -> RunContext {
    RunContext {
        task: "add a retry".into(),
        base: "main".into(),
        branch: "agent/otter".into(),
        test_command: test_command.map(str::to_owned),
    }
}

fn runner(stages: Vec<Stage>, test_command: Option<&str>) -> Runner {
    Runner::new(&workflow(stages), &BTreeMap::new(), ctx(test_command)).unwrap()
}

fn started(stages: Vec<Stage>, test_command: Option<&str>) -> (Runner, Command) {
    let mut runner = runner(stages, test_command);
    let first = runner.start().unwrap();
    (runner, first)
}

fn done(summary: &str) -> RunEvent {
    RunEvent::StageDone { summary: summary.into() }
}

fn finding(blocking: bool) -> RunEvent {
    RunEvent::Finding {
        file: "src/a.rs".into(),
        line: 3,
        blocking,
        text: if blocking { "leaks the timer" } else { "rename x" }.into(),
    }
}

fn tests(exit_code: i32) -> RunEvent {
    RunEvent::TestResult { exit_code, output: format!("run exited {exit_code}") }
}

fn gate_failed(index: usize, gate: Gate, detail: &str, loops_used: u8) -> Command {
    Command::AskUser {
        index,
        reason: AskReason::GateFailed { gate, detail: detail.into(), loops_used },
    }
}

fn ask(index: usize, reason: AskReason) -> Command {
    Command::AskUser { index, reason }
}

fn start_of(command: &Command) -> (usize, u8) {
    match command {
        Command::StartStage { index, attempt, .. } => (*index, *attempt),
        other => panic!("expected StartStage, got {other:?}"),
    }
}

fn prompt_of(command: &Command) -> &str {
    match command {
        Command::StartStage { prompt, .. } => prompt,
        other => panic!("expected StartStage, got {other:?}"),
    }
}

fn feed(runner: &mut Runner, events: impl IntoIterator<Item = RunEvent>) -> Option<Command> {
    events.into_iter().fold(None, |last, event| runner.handle(event).or(last))
}

type GateCase = (Gate, &'static [bool], Option<i32>, Command, StageState);

#[test]
fn every_gate_passes_and_fails_by_the_facts_not_the_agents_word() {
    let cases: Vec<GateCase> = vec![
        (Gate::Tests, &[], Some(0), Command::Finished, StageState::Done),
        (
            Gate::Tests,
            &[],
            Some(1),
            gate_failed(0, Gate::Tests, "tests failed (exit 1)", 0),
            StageState::Failed,
        ),
        (
            Gate::Tests,
            &[],
            Some(-1),
            gate_failed(0, Gate::Tests, "tests failed (exit -1)", 0),
            StageState::Failed,
        ),
        (Gate::TestPasses, &[], Some(0), Command::Finished, StageState::Done),
        (
            Gate::TestPasses,
            &[],
            Some(2),
            gate_failed(0, Gate::TestPasses, "tests failed (exit 2)", 0),
            StageState::Failed,
        ),
        (Gate::SameBehavior, &[], Some(0), Command::Finished, StageState::Done),
        (
            Gate::SameBehavior,
            &[],
            Some(101),
            gate_failed(0, Gate::SameBehavior, "tests failed (exit 101)", 0),
            StageState::Failed,
        ),
        (Gate::TestFails, &[], Some(1), Command::Finished, StageState::Done),
        (
            Gate::TestFails,
            &[],
            Some(0),
            gate_failed(0, Gate::TestFails, "the tests passed, but a failing test was expected", 0),
            StageState::Failed,
        ),
        (Gate::NoBlocking, &[], None, Command::Finished, StageState::Done),
        (Gate::NoBlocking, &[false, false], None, Command::Finished, StageState::Done),
        (
            Gate::NoBlocking,
            &[true],
            None,
            gate_failed(0, Gate::NoBlocking, "1 blocking finding", 0),
            StageState::Failed,
        ),
        (
            Gate::NoBlocking,
            &[true, false, true],
            None,
            gate_failed(0, Gate::NoBlocking, "2 blocking findings", 0),
            StageState::Failed,
        ),
        (Gate::Approve, &[], None, ask(0, AskReason::Approve), StageState::NeedsYou),
    ];
    for (gate, findings, test_exit, expected, state) in cases {
        let (mut runner, _) = started(vec![stage(Action::Work, gate)], Some("cargo test"));
        let mut last = feed(&mut runner, findings.iter().map(|b| finding(*b)));
        assert_eq!(last, None, "{gate:?}: findings alone decide nothing");
        last = runner.handle(done("did it"));
        if let Some(Command::RunTests { command }) = &last {
            assert_eq!(command, "cargo test", "{gate:?}");
            last = runner.handle(tests(test_exit.expect("a test gate needs a result")));
        }
        assert_eq!(last, Some(expected), "{gate:?} {findings:?} {test_exit:?}");
        assert_eq!(runner.states(), [state], "{gate:?} {findings:?} {test_exit:?}");
    }
}

#[test]
fn only_the_test_gates_run_the_test_command() {
    for gate in Gate::ALL {
        let (mut runner, _) = started(vec![stage(Action::Work, gate)], Some("make check"));
        let command = runner.handle(done("x"));
        let ran = matches!(command, Some(Command::RunTests { .. }));
        assert_eq!(ran, gate.runs_tests(), "{gate:?}");
    }
}

#[test]
fn approving_an_approval_gate_finishes_and_a_blocking_finding_does_not_block_it() {
    let (mut runner, _) = started(vec![stage(Action::Handoff, Gate::Approve)], None);
    feed(&mut runner, [finding(true), done("pr text")]);
    assert_eq!(runner.handle(RunEvent::UserApprove), Some(Command::Finished));
    assert_eq!(runner.states(), [StageState::Done]);
    assert!(runner.is_over());
}

#[test]
fn a_test_gate_without_a_test_command_asks_and_never_guesses() {
    let (mut runner, _) = started(vec![stage(Action::Work, Gate::Tests)], None);
    assert_eq!(runner.handle(done("x")), Some(ask(0, AskReason::NoTestCommand)));
    assert_eq!(runner.states(), [StageState::NeedsYou]);

    assert_eq!(
        runner.handle(RunEvent::UserRetry),
        Some(ask(0, AskReason::NoTestCommand)),
        "still no command"
    );
    runner.set_test_command(Some("cargo test".into()));
    assert_eq!(
        runner.handle(RunEvent::UserRetry),
        Some(Command::RunTests { command: "cargo test".into() })
    );
    assert_eq!(runner.handle(tests(0)), Some(Command::Finished));
}

#[test]
fn passing_a_missing_test_command_skips_the_gate() {
    let stages = vec![stage(Action::Work, Gate::Tests), stage(Action::Handoff, Gate::Approve)];
    let (mut runner, _) = started(stages, None);
    runner.handle(done("x"));
    let next = runner.handle(RunEvent::UserApprove).unwrap();
    assert_eq!(start_of(&next), (1, 1));
    assert_eq!(runner.states(), [StageState::Done, StageState::Running]);
}

#[test]
fn a_failed_gate_loops_back_and_resets_the_stages_in_between() {
    let stages = vec![
        stage(Action::Work, Gate::Tests),
        stage(Action::Refactor, Gate::SameBehavior),
        looping(Action::Review, Gate::NoBlocking, 0, 2),
    ];
    let (mut runner, first) = started(stages, Some("t"));
    assert_eq!(start_of(&first), (0, 1));
    runner.handle(done("built"));
    runner.handle(tests(0));
    runner.handle(done("tidied"));
    runner.handle(tests(0));
    assert_eq!(runner.states(), [StageState::Done, StageState::Done, StageState::Running]);

    let back = feed(&mut runner, [finding(true), done("one blocker")]).unwrap();
    assert_eq!(start_of(&back), (0, 2));
    assert_eq!(runner.states(), [StageState::Running, StageState::Todo, StageState::Todo]);
    assert_eq!((runner.position(), runner.loops_used(2), runner.loops_used(0)), (0, 1, 0));
}

#[test]
fn the_loop_limit_asks_after_exactly_max_loops_trips_back() {
    for max_loops in 0..=3u8 {
        let stages = vec![
            stage(Action::Work, Gate::Tests),
            looping(Action::Review, Gate::NoBlocking, 0, max_loops),
        ];
        let (mut runner, _) = started(stages, Some("t"));
        let mut reviews = 0;
        let asked = loop {
            runner.handle(done("built"));
            let review = runner.handle(tests(0)).unwrap();
            assert_eq!(start_of(&review).0, 1);
            reviews += 1;
            let next = feed(&mut runner, [finding(true), done("blocked")]).unwrap();
            if !matches!(next, Command::StartStage { .. }) {
                break next;
            }
        };
        assert_eq!(reviews, u32::from(max_loops) + 1, "max_loops {max_loops}");
        assert_eq!(asked, gate_failed(1, Gate::NoBlocking, "1 blocking finding", max_loops));
        assert_eq!(runner.states()[1], StageState::Failed);
    }
}

#[test]
fn a_loop_that_ends_in_a_pass_moves_on() {
    let stages = vec![
        stage(Action::Work, Gate::Tests),
        looping(Action::Review, Gate::NoBlocking, 0, 2),
        stage(Action::Refactor, Gate::SameBehavior),
    ];
    let (mut runner, _) = started(stages, Some("t"));
    runner.handle(done("built"));
    runner.handle(tests(0));
    feed(&mut runner, [finding(true), done("blocked")]);
    runner.handle(done("fixed"));
    let review = runner.handle(tests(0)).unwrap();
    assert_eq!(start_of(&review), (1, 2));
    assert_eq!(runner.findings(), [], "the old findings went with the old attempt");
    let refactor = runner.handle(done("clean")).unwrap();
    assert_eq!(start_of(&refactor), (2, 1));
    assert_eq!(runner.states(), [StageState::Done, StageState::Done, StageState::Running]);
}

#[test]
fn retry_after_the_limit_gives_another_round_and_pass_overrides_the_gate() {
    let stages = vec![
        stage(Action::Work, Gate::Tests),
        looping(Action::Review, Gate::NoBlocking, 0, 1),
        stage(Action::Handoff, Gate::Approve),
    ];
    let reach_the_limit = || {
        let (mut runner, _) = started(stages.clone(), Some("t"));
        runner.handle(done("a"));
        runner.handle(tests(0));
        feed(&mut runner, [finding(true), done("b")]);
        runner.handle(done("c"));
        runner.handle(tests(0));
        let asked = feed(&mut runner, [finding(true), done("d")]).unwrap();
        assert_eq!(asked, gate_failed(1, Gate::NoBlocking, "1 blocking finding", 1));
        runner
    };

    let mut retried = reach_the_limit();
    let back = retried.handle(RunEvent::UserRetry).unwrap();
    assert_eq!(start_of(&back), (0, 3));
    assert_eq!(retried.loops_used(1), 1, "counted again from zero");

    let mut passed = reach_the_limit();
    let handoff = passed.handle(RunEvent::UserApprove).unwrap();
    assert_eq!(start_of(&handoff), (2, 1));
    assert_eq!(passed.states()[1], StageState::Done);

    let mut stopped = reach_the_limit();
    assert_eq!(
        stopped.handle(RunEvent::UserStop),
        Some(Command::Stopped { reason: StopReason::User })
    );
    assert_eq!(stopped.states()[1], StageState::Failed);
}

#[test]
fn a_failed_gate_without_on_fail_asks_at_once_and_retry_runs_the_stage_again() {
    let (mut runner, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    runner.handle(done("x"));
    assert_eq!(
        runner.handle(tests(1)),
        Some(gate_failed(0, Gate::Tests, "tests failed (exit 1)", 0))
    );
    let again = runner.handle(RunEvent::UserRetry).unwrap();
    assert_eq!(start_of(&again), (0, 2));
    assert!(prompt_of(&again).contains("Output of the failing test run:\nrun exited 1"));
}

#[test]
fn stopping_without_stage_done_asks_and_the_user_can_pass_retry_or_stop() {
    let stages = || vec![stage(Action::Work, Gate::Tests)];
    let stopped = RunEvent::AgentStopped { last_message: Some("I think this is done".into()) };
    let asked =
        ask(0, AskReason::NoStageDone { last_message: Some("I think this is done".into()) });

    let (mut passing, _) = started(stages(), Some("t"));
    assert_eq!(passing.handle(stopped.clone()), Some(asked.clone()));
    assert_eq!(passing.states(), [StageState::NeedsYou]);
    assert_eq!(
        passing.handle(RunEvent::UserApprove),
        Some(Command::RunTests { command: "t".into() }),
        "pass still runs the gate"
    );
    assert_eq!(passing.summary(0), Some("I think this is done"));
    assert_eq!(passing.handle(tests(0)), Some(Command::Finished));

    let (mut retrying, _) = started(stages(), Some("t"));
    retrying.handle(stopped.clone());
    let again = retrying.handle(RunEvent::UserRetry).unwrap();
    assert_eq!(start_of(&again), (0, 2));

    let (mut stopping, _) = started(stages(), Some("t"));
    stopping.handle(stopped);
    assert_eq!(
        stopping.handle(RunEvent::UserStop),
        Some(Command::Stopped { reason: StopReason::User })
    );
}

#[test]
fn a_stage_done_or_finding_that_arrives_late_is_taken_while_asking() {
    let (mut runner, _) = started(vec![stage(Action::Review, Gate::NoBlocking)], None);
    runner.handle(RunEvent::AgentStopped { last_message: None });
    assert_eq!(runner.handle(finding(true)), None);
    let outcome = runner.handle(done("late"));
    assert_eq!(outcome, Some(gate_failed(0, Gate::NoBlocking, "1 blocking finding", 0)));
    assert_eq!(runner.summary(0), Some("late"));
}

#[test]
fn passing_after_a_silent_stop_keeps_an_existing_summary() {
    let (mut runner, _) = started(vec![stage(Action::Review, Gate::NoBlocking)], None);
    runner.handle(RunEvent::AgentStopped { last_message: Some("chatter".into()) });
    runner.handle(RunEvent::UserApprove);
    assert_eq!(runner.summary(0), Some("chatter"));
    assert!(runner.is_over());
}

#[test]
fn a_failed_agent_fails_the_gate_so_it_loops_back_or_asks() {
    let failed = RunEvent::AgentFailed { reason: "exit 1".into() };
    let stages =
        vec![stage(Action::Work, Gate::Tests), looping(Action::Review, Gate::NoBlocking, 0, 1)];

    let (mut runner, _) = started(stages.clone(), Some("t"));
    runner.handle(done("a"));
    runner.handle(tests(0));
    let back = runner.handle(failed.clone()).unwrap();
    assert_eq!(start_of(&back), (0, 2));
    runner.handle(done("b"));
    runner.handle(tests(0));
    let asked = runner.handle(failed.clone());
    assert_eq!(asked, Some(ask(1, AskReason::AgentFailed { reason: "exit 1".into() })));
    assert_eq!(runner.states()[1], StageState::Failed);

    let (mut plain, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    assert_eq!(
        plain.handle(failed.clone()),
        Some(ask(0, AskReason::AgentFailed { reason: "exit 1".into() }))
    );
    assert_eq!(start_of(&plain.handle(RunEvent::UserRetry).unwrap()), (0, 2));

    let (mut passing, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    passing.handle(failed);
    assert_eq!(passing.handle(RunEvent::UserApprove), Some(Command::Finished));
}

#[test]
fn rejecting_goes_back_with_the_reason_or_stops_when_there_is_nowhere_to_go() {
    let stages =
        vec![stage(Action::Work, Gate::Tests), looping(Action::Handoff, Gate::Approve, 0, 1)];
    let (mut runner, _) = started(stages, Some("t"));
    runner.handle(done("a"));
    runner.handle(tests(0));
    assert_eq!(runner.handle(done("pr text")), Some(ask(1, AskReason::Approve)));

    let back = runner.handle(RunEvent::UserReject { feedback: "title is vague".into() }).unwrap();
    assert_eq!(start_of(&back), (0, 2));
    assert!(prompt_of(&back).contains("Open findings to address:\n- [blocking] title is vague"));

    runner.handle(done("b"));
    runner.handle(tests(0));
    runner.handle(done("pr text 2"));
    let limit = runner.handle(RunEvent::UserReject { feedback: "still vague".into() });
    assert_eq!(limit, Some(gate_failed(1, Gate::Approve, "rejected: still vague", 1)));

    let (mut lone, _) = started(vec![stage(Action::Handoff, Gate::Approve)], None);
    lone.handle(done("x"));
    assert_eq!(
        lone.handle(RunEvent::UserReject { feedback: "no".into() }),
        Some(Command::Stopped { reason: StopReason::Rejected { stage: 0 } })
    );
    assert_eq!(lone.states(), [StageState::Failed]);
    assert!(lone.is_over());
}

#[test]
fn an_approve_flag_waits_once_after_the_gate_passes() {
    let flagged = Stage { approve: true, ..stage(Action::Work, Gate::Tests) };
    let (mut runner, _) = started(vec![flagged, stage(Action::Handoff, Gate::Approve)], Some("t"));
    runner.handle(done("x"));
    assert_eq!(runner.handle(tests(0)), Some(ask(0, AskReason::Approve)));
    assert_eq!(runner.states(), [StageState::NeedsYou, StageState::Todo]);
    assert_eq!(start_of(&runner.handle(RunEvent::UserApprove).unwrap()), (1, 1));

    let double = Stage { approve: true, ..stage(Action::Handoff, Gate::Approve) };
    let (mut once, _) = started(vec![double], None);
    assert_eq!(once.handle(done("x")), Some(ask(0, AskReason::Approve)));
    assert_eq!(once.handle(RunEvent::UserApprove), Some(Command::Finished));
}

#[test]
fn a_failing_gate_comes_before_the_approve_flag() {
    let flagged = Stage { approve: true, ..stage(Action::Work, Gate::Tests) };
    let (mut runner, _) = started(vec![flagged], Some("t"));
    runner.handle(done("x"));
    assert_eq!(
        runner.handle(tests(1)),
        Some(gate_failed(0, Gate::Tests, "tests failed (exit 1)", 0))
    );
}

#[test]
fn retry_means_nothing_to_an_approval_question() {
    let (mut runner, _) = started(vec![stage(Action::Handoff, Gate::Approve)], None);
    runner.handle(done("x"));
    assert_eq!(runner.handle(RunEvent::UserRetry), None);
    assert_eq!(runner.phase(), &Phase::Asking(AskReason::Approve));
}

#[test]
fn stop_works_in_every_running_phase_and_nothing_works_after() {
    let stages = || vec![stage(Action::Work, Gate::Tests)];
    let mut runs = Vec::new();
    let (working, _) = started(stages(), Some("t"));
    runs.push(("working", working));
    let (mut testing, _) = started(stages(), Some("t"));
    testing.handle(done("x"));
    runs.push(("testing", testing));
    let (mut asking, _) = started(stages(), Some("t"));
    asking.handle(RunEvent::AgentStopped { last_message: None });
    runs.push(("asking", asking));
    for (name, mut runner) in runs {
        assert_eq!(
            runner.handle(RunEvent::UserStop),
            Some(Command::Stopped { reason: StopReason::User }),
            "{name}"
        );
        assert_eq!(runner.states(), [StageState::Failed], "{name}");
        assert!(runner.is_over(), "{name}");
        for event in
            [done("x"), RunEvent::UserApprove, RunEvent::UserRetry, RunEvent::UserStop, tests(0)]
        {
            assert_eq!(runner.handle(event), None, "{name}");
        }
    }
}

#[test]
fn events_that_do_not_fit_the_phase_are_ignored() {
    let mut idle = runner(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    for event in [done("x"), RunEvent::UserApprove, RunEvent::UserStop, tests(0)] {
        assert_eq!(idle.handle(event), None, "idle");
    }
    assert_eq!(idle.phase(), &Phase::Idle);

    let all_events = || {
        vec![
            done("x"),
            finding(true),
            RunEvent::AgentStopped { last_message: None },
            RunEvent::AgentFailed { reason: "r".into() },
            tests(0),
            RunEvent::UserApprove,
            RunEvent::UserReject { feedback: "f".into() },
            RunEvent::UserRetry,
        ]
    };
    let fits_working = |e: &RunEvent| {
        matches!(
            e,
            RunEvent::StageDone { .. }
                | RunEvent::Finding { .. }
                | RunEvent::AgentStopped { .. }
                | RunEvent::AgentFailed { .. }
        )
    };
    for event in all_events().into_iter().filter(|e| !fits_working(e)) {
        let (mut working, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
        assert_eq!(working.handle(event.clone()), None, "working: {event:?}");
        assert_eq!(working.phase(), &Phase::Working, "working: {event:?}");
    }
    for event in all_events().into_iter().filter(|e| !matches!(e, RunEvent::TestResult { .. })) {
        let (mut testing, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
        testing.handle(done("x"));
        assert_eq!(testing.handle(event.clone()), None, "testing: {event:?}");
        assert_eq!(testing.phase(), &Phase::Testing, "testing: {event:?}");
    }
    let (mut asking, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    asking.handle(done("x"));
    asking.handle(tests(1));
    for event in [done("x"), finding(true), tests(0), RunEvent::UserReject { feedback: "f".into() }]
    {
        assert_eq!(asking.handle(event.clone()), None, "asking: {event:?}");
    }
    assert!(matches!(asking.phase(), Phase::Asking(AskReason::GateFailed { .. })));
    assert_eq!(asking.start(), None, "start only works once");
}

#[test]
fn blocking_findings_count_only_for_the_stage_that_reported_them() {
    let second = Stage {
        prompt: Some("Look at:\n{findings}".into()),
        ..stage(Action::Review, Gate::NoBlocking)
    };
    let (mut runner, _) = started(vec![stage(Action::Review, Gate::NoBlocking), second], None);
    let asked = feed(&mut runner, [finding(true), done("first")]);
    assert_eq!(asked, Some(gate_failed(0, Gate::NoBlocking, "1 blocking finding", 0)));
    let next = runner.handle(RunEvent::UserApprove).unwrap();
    assert_eq!(prompt_of(&next), "Look at:\n- [blocking] src/a.rs:3 leaks the timer");
    assert_eq!(runner.handle(done("second")), Some(Command::Finished));
}

#[test]
fn prompts_fill_every_placeholder_from_the_run() {
    let custom = Stage {
        prompt: Some(
            "{task}|{base}|{branch}|{prev}|{previous_summary}|{findings}|{test_output}".into(),
        ),
        ..stage(Action::Fix, Gate::TestPasses)
    };
    let stages = vec![stage(Action::Reproduce, Gate::TestFails), custom];
    let (mut runner, first) = started(stages, Some("t"));
    assert_eq!(
        prompt_of(&first),
        "add a retry\n\nWrite one test that fails because of this bug. Do not fix the bug yet."
    );
    feed(&mut runner, [finding(false), done("wrote test_retry")]);
    let second = runner.handle(RunEvent::TestResult { exit_code: 1, output: "1 failed\n".into() });
    assert_eq!(
        prompt_of(&second.unwrap()),
        "add a retry|main|agent/otter|wrote test_retry|wrote test_retry|\
         - [nit] src/a.rs:3 rename x|1 failed"
    );
}

#[test]
fn a_placeholder_in_the_template_is_not_repeated_by_the_loop_back_note() {
    let template = Stage {
        prompt: Some("Fix this: {findings}\nOutput: {test_output}".into()),
        ..stage(Action::Work, Gate::Tests)
    };
    let stages = vec![template, looping(Action::Review, Gate::NoBlocking, 0, 2)];
    let (mut runner, _) = started(stages, Some("t"));
    runner.handle(done("a"));
    runner.handle(tests(0));
    let back = feed(&mut runner, [finding(true), done("b")]).unwrap();
    assert_eq!(
        prompt_of(&back),
        "Fix this: - [blocking] src/a.rs:3 leaks the timer\nOutput: (none)"
    );
}

#[test]
fn findings_render_with_their_place_and_kind() {
    let make = |file: &str, line: u32, blocking: bool, text: &str| Finding {
        stage: 0,
        file: file.into(),
        line,
        blocking,
        text: text.into(),
    };
    let findings = [
        make("src/a.rs", 3, true, "leak"),
        make("README.md", 0, false, "typo"),
        make("", 0, true, "general"),
    ];
    assert_eq!(
        format_findings(findings.iter()),
        "- [blocking] src/a.rs:3 leak\n- [nit] README.md typo\n- [blocking] general"
    );
    assert_eq!(format_findings([].iter()), "");
}

#[test]
fn only_the_end_of_a_long_test_output_reaches_the_prompt() {
    let mut long = String::new();
    for n in 0..2_000 {
        writeln!(long, "line {n}").unwrap();
    }
    let stages = vec![stage(Action::Work, Gate::Tests)];
    let (mut runner, _) = started(stages, Some("t"));
    runner.handle(done("x"));
    runner.handle(RunEvent::TestResult { exit_code: 1, output: long.clone() });
    let again = runner.handle(RunEvent::UserRetry).unwrap();
    let shown = prompt_of(&again).split("failing test run:\n").nth(1).unwrap();
    assert!(shown.starts_with("...\n"));
    assert!(shown.ends_with("line 1999"));
    assert!(shown.chars().count() <= TEST_OUTPUT_TAIL_CHARS + 4);

    assert_eq!(tail("short\n\n", 10), "short");
    let accents = "é".repeat(5_000);
    assert_eq!(tail(&accents, 3), "...\nééé");
}

#[test]
fn a_passing_run_clears_the_old_failure_output() {
    let (mut runner, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    runner.handle(done("x"));
    runner.handle(tests(1));
    runner.handle(RunEvent::UserRetry);
    runner.handle(done("y"));
    assert_eq!(runner.handle(tests(0)), Some(Command::Finished));
    let (mut next, _) = started(
        vec![stage(Action::Work, Gate::Tests), stage(Action::Refactor, Gate::Tests)],
        Some("t"),
    );
    next.handle(done("x"));
    let second = next.handle(tests(0)).unwrap();
    assert!(!prompt_of(&second).contains("failing test run"));
}

#[test]
fn start_stage_carries_the_resolved_profile_and_action() {
    let reviewer = Profile {
        name: "Reviewer".into(),
        agent: "gemini".into(),
        model: None,
        effort: None,
        level: crate::agents::Level::Plan,
        setup: String::new(),
        never_push: true,
        extra_args: crate::agents::ArgList::default(),
        house_style: true,
    };
    let profiles = BTreeMap::from([("reviewer".to_owned(), reviewer.clone())]);
    let by_profile = Stage {
        run: StageRun { profile: Some("reviewer".into()), ..StageRun::default() },
        ..Stage::new(Action::Review)
    };
    let mut runner = Runner::new(&workflow(vec![by_profile]), &profiles, ctx(None)).unwrap();
    let Some(Command::StartStage { action, profile, .. }) = runner.start() else {
        panic!("expected StartStage");
    };
    assert_eq!((action, profile), (Action::Review, reviewer));
}

#[test]
fn a_run_tells_the_profile_each_stage_launches_with_and_what_it_is_about() {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let runner = Runner::new(&ship, &BTreeMap::new(), ctx(Some("npm test"))).unwrap();
    let agents: Vec<_> = (0..4).map(|i| runner.profile(i).unwrap().agent.as_str()).collect();
    assert_eq!(agents, ["claude", "gemini", "claude", "claude"]);
    assert_eq!(runner.profile(4), None);
    assert_eq!(runner.context().task, "add a retry");
    assert_eq!(runner.context().test_command.as_deref(), Some("npm test"));
}

#[test]
fn a_run_refuses_a_workflow_that_cannot_run() {
    let unknown = Stage {
        run: StageRun { profile: Some("ghost".into()), ..StageRun::default() },
        ..Stage::new(Action::Work)
    };
    let problems = Runner::new(&workflow(vec![unknown]), &BTreeMap::new(), ctx(None)).unwrap_err();
    assert_eq!(problems, ["stage 1 (Work): unknown profile \"ghost\""]);

    let nobody =
        Runner::new(&workflow(vec![Stage::new(Action::Work)]), &BTreeMap::new(), ctx(None));
    assert_eq!(nobody.unwrap_err(), ["stage 1 (Work) needs a profile or an agent"]);
    let empty = Runner::new(&workflow(Vec::new()), &BTreeMap::new(), ctx(None));
    assert_eq!(empty.unwrap_err(), ["has no stages"]);
}

fn hook(kind: HookKind, message: Option<&str>) -> TabEvent {
    TabEvent::Hook(HookEvent {
        kind,
        session_id: None,
        transcript_path: None,
        message: message.map(str::to_owned),
    })
}

#[test]
fn listener_messages_map_to_run_events() {
    let cases = [
        (
            TabEvent::StageDone { summary: "s".into() },
            Some(RunEvent::StageDone { summary: "s".into() }),
        ),
        (
            TabEvent::Finding { file: "f".into(), line: 2, blocking: true, text: "t".into() },
            Some(RunEvent::Finding { file: "f".into(), line: 2, blocking: true, text: "t".into() }),
        ),
        (
            hook(HookKind::TurnDone, Some("all done")),
            Some(RunEvent::AgentStopped { last_message: Some("all done".into()) }),
        ),
        (hook(HookKind::TurnDone, None), Some(RunEvent::AgentStopped { last_message: None })),
        (
            hook(HookKind::TurnFailed("rate_limit".into()), None),
            Some(RunEvent::AgentFailed { reason: "rate_limit".into() }),
        ),
        (hook(HookKind::SessionStart, None), None),
        (hook(HookKind::PermissionNeeded, Some("run rm")), None),
        (hook(HookKind::IdleWaiting, None), None),
        (TabEvent::NoteReply { note_id: 1, text: "ok".into() }, None),
    ];
    for (event, expected) in cases {
        assert_eq!(RunEvent::from_tab_event(&event), expected, "{event:?}");
    }
}

fn round_trips(runner: &Runner, command: &Command, dir: &Path) {
    let file = TidewatchPaths::new(dir).stages_file(Path::new("/work/web"), "brave-otter");
    runner.save(&file).unwrap();
    let loaded = Runner::load(&file).expect("a saved run loads");
    assert_eq!(&loaded, runner);
    assert_eq!(loaded.resume().as_ref(), Some(command), "resume repeats the last command");
}

struct Script {
    turns: HashMap<(usize, u8), Vec<RunEvent>>,
    test_runs: VecDeque<(i32, &'static str)>,
    started: Vec<(usize, u8)>,
    prompts: Vec<String>,
    tests_run: usize,
}

impl Script {
    fn new(turns: Vec<((usize, u8), Vec<RunEvent>)>, test_runs: Vec<(i32, &'static str)>) -> Self {
        Self {
            turns: turns.into_iter().collect(),
            test_runs: test_runs.into(),
            started: Vec::new(),
            prompts: Vec::new(),
            tests_run: 0,
        }
    }

    fn drive(&mut self, runner: &mut Runner, first: Command, dir: &Path) -> Command {
        let mut command = first;
        loop {
            round_trips(runner, &command, dir);
            let events = match &command {
                Command::StartStage { index, attempt, prompt, .. } => {
                    self.started.push((*index, *attempt));
                    self.prompts.push(prompt.clone());
                    self.turns.remove(&(*index, *attempt)).expect("a scripted turn")
                }
                Command::RunTests { .. } => {
                    self.tests_run += 1;
                    let (exit_code, output) = self.test_runs.pop_front().expect("a scripted run");
                    vec![RunEvent::TestResult { exit_code, output: output.into() }]
                }
                _ => return command,
            };
            command = feed(runner, events).expect("every scripted turn ends in a command");
        }
    }
}

fn blocking_finding(file: &str, line: u32, text: &str) -> RunEvent {
    RunEvent::Finding { file: file.into(), line, blocking: true, text: text.into() }
}

#[test]
fn ship_a_feature_catches_a_planted_bug_loops_once_then_waits_at_hand_off() {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let mut runner = Runner::new(&ship, &BTreeMap::new(), ctx(Some("npm test"))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut script = Script::new(
        vec![
            ((0, 1), vec![done("Added the refresh timer")]),
            (
                (1, 1),
                vec![
                    blocking_finding("src/auth.ts", 12, "the timer keeps running after sign-out"),
                    RunEvent::Finding {
                        file: "src/auth.ts".into(),
                        line: 40,
                        blocking: false,
                        text: "rename x".into(),
                    },
                    done("One blocking finding, one nit"),
                ],
            ),
            ((0, 2), vec![done("Cleared the timer in signOut")]),
            ((1, 2), vec![done("Nothing blocking")]),
            ((2, 1), vec![done("Removed the duplicate in session.ts")]),
            ((3, 1), vec![done("feat: refresh the token before it expires")]),
        ],
        vec![(0, "ok"), (0, "ok"), (0, "ok")],
    );

    let first = runner.start().unwrap();
    let waiting = script.drive(&mut runner, first, dir.path());

    assert_eq!(waiting, ask(3, AskReason::Approve), "Hand off waits for the user");
    assert_eq!(script.started, [(0, 1), (1, 1), (0, 2), (1, 2), (2, 1), (3, 1)]);
    assert_eq!(script.tests_run, 3, "work, work again, refactor");
    assert_eq!(runner.loops_used(1), 1);
    assert_eq!(
        runner.states(),
        [StageState::Done, StageState::Done, StageState::Done, StageState::NeedsYou]
    );
    assert_eq!(runner.summary(3), Some("feat: refresh the token before it expires"));

    let looped = &script.prompts[2];
    assert!(looped.starts_with("add a retry\n\nWork in small steps."), "{looped}");
    assert!(
        looped.ends_with(
            "Open findings to address:\n\
             - [blocking] src/auth.ts:12 the timer keeps running after sign-out\n\
             - [nit] src/auth.ts:40 rename x"
        ),
        "{looped}"
    );
    assert!(script.prompts[3].starts_with("Review main..HEAD"));
    assert!(!script.prompts[3].contains("sign-out"), "a re-run reviewer starts clean");
    assert!(script.prompts[5].starts_with("Write a PR title and description for main..HEAD."));

    let finished = runner.handle(RunEvent::UserApprove).unwrap();
    assert_eq!(finished, Command::Finished);
    assert_eq!(runner.states(), [StageState::Done; 4]);
    round_trips(&runner, &finished, dir.path());
}

#[test]
fn ship_a_feature_stops_and_asks_when_the_reviewer_keeps_finding_the_bug() {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let mut runner = Runner::new(&ship, &BTreeMap::new(), ctx(Some("npm test"))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blocked = |attempt| {
        ((1, attempt), vec![blocking_finding("src/auth.ts", 12, "still leaks"), done("blocked")])
    };
    let mut script = Script::new(
        vec![
            ((0, 1), vec![done("a")]),
            blocked(1),
            ((0, 2), vec![done("b")]),
            blocked(2),
            ((0, 3), vec![done("c")]),
            blocked(3),
        ],
        vec![(0, ""), (0, ""), (0, "")],
    );
    let first = runner.start().unwrap();
    let asked = script.drive(&mut runner, first, dir.path());
    assert_eq!(asked, gate_failed(1, Gate::NoBlocking, "1 blocking finding", 2));
    assert_eq!(runner.states()[1], StageState::Failed);
    assert_eq!(runner.loops_used(1), 2);
}

#[test]
fn fix_a_bug_runs_reproduce_fix_review_and_hands_the_failure_to_the_fix() {
    let fix = Workflows::defaults().workflow.remove("fix").unwrap();
    let mut runner = Runner::new(&fix, &BTreeMap::new(), ctx(Some("cargo test"))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut script = Script::new(
        vec![
            ((0, 1), vec![done("Wrote retry_is_capped")]),
            ((1, 1), vec![done("Capped the retries")]),
            ((2, 1), vec![done("Looks right")]),
        ],
        vec![(101, "retry_is_capped panicked: left 5, right 3"), (0, "ok")],
    );
    let first = runner.start().unwrap();
    let end = script.drive(&mut runner, first, dir.path());
    assert_eq!(end, Command::Finished);
    assert!(
        script.prompts[1].contains("Output of the failing test run:\nretry_is_capped panicked")
    );
    assert!(script.prompts[1].starts_with("Make the failing test pass"));
    assert_eq!(runner.states(), [StageState::Done; 3]);
}

#[test]
fn fix_a_bug_asks_when_the_reproduction_does_not_fail_or_the_fix_does_not_pass() {
    let fix = Workflows::defaults().workflow.remove("fix").unwrap();
    let dir = tempfile::tempdir().unwrap();

    let mut no_repro = Runner::new(&fix, &BTreeMap::new(), ctx(Some("t"))).unwrap();
    let first = no_repro.start().unwrap();
    let mut script = Script::new(vec![((0, 1), vec![done("wrote it")])], vec![(0, "passes")]);
    let asked = script.drive(&mut no_repro, first, dir.path());
    assert_eq!(
        asked,
        gate_failed(0, Gate::TestFails, "the tests passed, but a failing test was expected", 0)
    );

    let mut no_fix = Runner::new(&fix, &BTreeMap::new(), ctx(Some("t"))).unwrap();
    let first = no_fix.start().unwrap();
    let mut script = Script::new(
        vec![((0, 1), vec![done("wrote it")]), ((1, 1), vec![done("tried")])],
        vec![(1, "red"), (1, "still red")],
    );
    let asked = script.drive(&mut no_fix, first, dir.path());
    assert_eq!(asked, gate_failed(1, Gate::TestPasses, "tests failed (exit 1)", 0));
}

#[test]
fn review_a_pr_asks_on_a_blocking_finding_then_passes_on_and_waits_for_the_summary_approval() {
    let pr = Workflows::defaults().workflow.remove("pr").unwrap();
    let mut runner = Runner::new(&pr, &BTreeMap::new(), ctx(None)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut script = Script::new(
        vec![
            ((0, 1), vec![blocking_finding("a.rs", 1, "race"), done("one blocker")]),
            ((1, 1), vec![done("agree")]),
            ((2, 1), vec![done("summary")]),
        ],
        Vec::new(),
    );
    let first = runner.start().unwrap();
    let asked = script.drive(&mut runner, first, dir.path());
    assert_eq!(asked, gate_failed(0, Gate::NoBlocking, "1 blocking finding", 0));
    let next = runner.handle(RunEvent::UserApprove).unwrap();
    let waiting = script.drive(&mut runner, next, dir.path());
    assert_eq!(waiting, ask(2, AskReason::Approve));
}

#[test]
fn resume_repeats_the_command_the_run_was_waiting_on_at_every_step() {
    let stages = vec![
        Stage { approve: true, ..stage(Action::Work, Gate::Tests) },
        stage(Action::Handoff, Gate::Approve),
    ];
    let dir = tempfile::tempdir().unwrap();
    let mut runner = runner(stages, Some("t"));
    assert_eq!(runner.resume(), None, "not started");
    let mut command = runner.start().unwrap();
    round_trips(&runner, &command, dir.path());
    let events = [done("a"), tests(0), RunEvent::UserApprove, done("b"), RunEvent::UserApprove];
    for event in events {
        command = runner.handle(event).unwrap();
        round_trips(&runner, &command, dir.path());
    }
    assert_eq!(command, Command::Finished);
}

#[test]
fn resume_covers_testing_failed_gates_stops_and_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let (mut testing, command) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    round_trips(&testing, &command, dir.path());
    let run_tests = testing.handle(done("x")).unwrap();
    round_trips(&testing, &run_tests, dir.path());
    let failed = testing.handle(tests(2)).unwrap();
    round_trips(&testing, &failed, dir.path());
    let stopped = testing.handle(RunEvent::UserStop).unwrap();
    assert_eq!(stopped, Command::Stopped { reason: StopReason::User });
    round_trips(&testing, &stopped, dir.path());

    let (mut rejected, _) = started(vec![stage(Action::Handoff, Gate::Approve)], None);
    rejected.handle(done("x"));
    let end = rejected.handle(RunEvent::UserReject { feedback: "no".into() }).unwrap();
    round_trips(&rejected, &end, dir.path());

    let (mut silent, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    let asked = silent.handle(RunEvent::AgentStopped { last_message: Some("hm".into()) }).unwrap();
    round_trips(&silent, &asked, dir.path());
}

#[test]
fn a_resumed_run_carries_on_where_it_left_off() {
    let dir = tempfile::tempdir().unwrap();
    let file = TidewatchPaths::new(dir.path()).stages_file(Path::new("/r"), "t1");
    let stages =
        vec![stage(Action::Work, Gate::Tests), looping(Action::Review, Gate::NoBlocking, 0, 2)];
    let (mut before, _) = started(stages, Some("t"));
    before.handle(done("built"));
    before.handle(tests(0));
    feed(&mut before, [blocking_finding("a.rs", 1, "bug"), done("blocked")]);
    before.save(&file).unwrap();
    drop(before);

    let mut after = Runner::load(&file).unwrap();
    let again = after.resume().unwrap();
    assert_eq!(start_of(&again), (0, 2));
    assert!(prompt_of(&again).contains("- [blocking] a.rs:1 bug"));
    after.handle(done("fixed"));
    let review = after.handle(tests(0)).unwrap();
    assert_eq!(start_of(&review), (1, 2));
    assert_eq!(after.loops_used(1), 1);
}

#[test]
fn a_missing_or_bad_state_file_is_no_run_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t").join("stages.json");
    assert_eq!(Runner::load(&file), None);

    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    for garbage in ["", "not json", "{}", "[1,2]", "{\"stages\": 3}"] {
        std::fs::write(&file, garbage).unwrap();
        assert_eq!(Runner::load(&file), None, "{garbage:?}");
    }
}

#[test]
fn a_state_file_that_does_not_match_its_workflow_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("stages.json");
    let (good, _) = started(
        vec![stage(Action::Work, Gate::Tests), looping(Action::Review, Gate::NoBlocking, 0, 2)],
        Some("t"),
    );
    good.save(&file).unwrap();
    assert_eq!(Runner::load(&file), Some(good.clone()));

    let tampered = [
        "position",
        "states",
        "loops",
        "no stages",
        "latest summary",
        "finding stage",
        "forward on_fail",
    ];
    for name in tampered {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        match name {
            "position" => value["position"] = 9.into(),
            "states" => value["states"] = serde_json::json!(["Todo"]),
            "loops" => value["loops"] = serde_json::json!([]),
            "no stages" => {
                for key in ["stages", "states", "loops", "attempts", "summaries"] {
                    value[key] = serde_json::json!([]);
                }
            }
            "latest summary" => value["latest_summary"] = 7.into(),
            "finding stage" => {
                value["findings"] = serde_json::json!([
                    {"stage": 5, "file": "", "line": 0, "blocking": true, "text": "x"}
                ]);
            }
            _ => value["stages"][1]["stage"]["on_fail"] = 2.into(),
        }
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, value.to_string()).unwrap();
        assert_eq!(Runner::load(&bad), None, "{name}");
    }
}

#[test]
fn saving_replaces_the_file_atomically_and_creates_the_tab_dir() {
    let dir = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(dir.path());
    let file = paths.stages_file(Path::new("/work/web"), "brave-otter");
    let (mut runner, _) = started(vec![stage(Action::Work, Gate::Tests)], Some("t"));
    runner.save(&file).unwrap();
    runner.handle(done("x"));
    runner.save(&file).unwrap();
    assert_eq!(Runner::load(&file).unwrap().phase(), &Phase::Testing);
    let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["stages.json"]);
}

#[test]
fn a_real_test_command_drives_the_gate_through_run_in() {
    let worktree = tempfile::tempdir().unwrap();
    let command = "test -f ok.marker || { echo 'ok.marker is missing' 1>&2; exit 1; }";
    let (mut runner, _) = started(vec![stage(Action::Work, Gate::Tests)], Some(command));
    let Some(Command::RunTests { command }) = runner.handle(done("x")) else {
        panic!("expected RunTests");
    };

    let run = crate::test_command::run_in(worktree.path(), &command);
    let asked =
        runner.handle(RunEvent::TestResult { exit_code: run.exit_code, output: run.output });
    assert_eq!(asked, Some(gate_failed(0, Gate::Tests, "tests failed (exit 1)", 0)));

    let again = runner.handle(RunEvent::UserRetry).unwrap();
    assert!(prompt_of(&again).ends_with("ok.marker is missing"), "{}", prompt_of(&again));
    std::fs::write(worktree.path().join("ok.marker"), "").unwrap();
    let Some(Command::RunTests { command }) = runner.handle(done("y")) else {
        panic!("expected RunTests");
    };
    let run = crate::test_command::run_in(worktree.path(), &command);
    let end = runner.handle(RunEvent::TestResult { exit_code: run.exit_code, output: run.output });
    assert_eq!(end, Some(Command::Finished));
}
