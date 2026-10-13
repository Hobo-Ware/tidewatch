use std::collections::{BTreeMap, VecDeque};

use tidewatch_core::events::TabEvent;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::runner::{RunContext, StageState};
use tidewatch_core::status::{HookEvent, HookKind};
use tidewatch_core::workflows::{Gate, Stage, Workflow, Workflows};

use super::*;

const LIMIT: Duration = Duration::from_secs(600);

fn context(test_command: Option<&str>) -> RunContext {
    RunContext {
        task: "add a retry".into(),
        base: "main".into(),
        branch: "agent/brave-otter".into(),
        test_command: test_command.map(str::to_owned),
    }
}

fn ship_run(test_command: Option<&str>, file: Option<PathBuf>) -> TabRun {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let runner = Runner::new(&ship, &BTreeMap::new(), context(test_command)).unwrap();
    TabRun::new(runner, file, LIMIT)
}

fn message(generation: u64, event: TabEvent) -> TabMessage {
    TabMessage { tab_id: 1, generation, event }
}

fn done(generation: u64, summary: &str) -> TabMessage {
    message(generation, TabEvent::StageDone { summary: summary.into() })
}

fn finding(generation: u64, blocking: bool, text: &str) -> TabMessage {
    message(
        generation,
        TabEvent::Finding { file: "src/auth.ts".into(), line: 12, blocking, text: text.into() },
    )
}

fn stop_hook(generation: u64, last: Option<&str>) -> TabMessage {
    message(
        generation,
        TabEvent::Hook(HookEvent {
            kind: HookKind::TurnDone,
            session_id: None,
            transcript_path: None,
            message: last.map(str::to_owned),
        }),
    )
}

fn only_launch(execs: &[Exec]) -> &StageLaunch {
    match execs {
        [Exec::Launch(launch)] => launch,
        other => panic!("expected one launch, got {other:?}"),
    }
}

#[test]
fn starting_launches_the_first_stage_as_generation_one() {
    let mut run = ship_run(Some("npm test"), None);
    let execs = run.start();
    let launch = only_launch(&execs);
    assert_eq!((launch.index, launch.attempt, launch.generation), (0, 1, 1));
    assert_eq!(launch.profile.agent, "claude");
    assert!(launch.prompt.starts_with("add a retry\n\nWork in small steps."), "{}", launch.prompt);
    assert_eq!(launch.divider, "--- Stage 1: Work (claude opus, Auto edit) ---");
    assert_eq!((run.generation(), run.launching()), (1, true));
    assert_eq!(run.start(), Vec::new(), "a run starts once");
}

#[test]
fn the_divider_names_the_stage_agent_level_and_round() {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let runner = Runner::new(&ship, &BTreeMap::new(), context(None)).unwrap();
    let review = runner.profile(1).unwrap().clone();
    assert_eq!(
        divider(1, Action::Review, &review, 1),
        "--- Stage 2: Review (gemini, Read only) ---"
    );
    assert_eq!(
        divider(1, Action::Review, &review, 2),
        "--- Stage 2: Review (gemini, Read only, round 2) ---"
    );
}

#[test]
fn what_the_previous_stages_agent_says_after_the_next_stage_started_is_dropped() {
    let mut run = ship_run(Some("npm test"), None);
    run.start();
    run.launched();
    let tests = run.message(&done(1, "built"));
    assert!(matches!(tests.as_slice(), [Exec::RunTests { .. }]), "{tests:?}");
    let next = run.event(RunEvent::TestResult { exit_code: 0, output: String::new() });
    assert_eq!(only_launch(&next).generation, 2);
    assert_eq!(run.message(&stop_hook(1, Some("late"))), Vec::new());
    assert_eq!(run.message(&done(1, "late summary")), Vec::new());
    assert_eq!(run.message(&finding(1, true, "late finding")), Vec::new());
    assert_eq!(run.runner().phase(), &Phase::Working);
    assert_eq!(run.runner().findings().len(), 0);
    assert_eq!(run.runner().summary(1), None);
    let live = run.message(&stop_hook(2, None));
    assert_eq!(live, Vec::new());
    assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::NoStageDone { .. })));
}

#[test]
fn only_the_listeners_stage_events_reach_the_runner() {
    let mut run = ship_run(Some("npm test"), None);
    run.start();
    let permission = message(
        1,
        TabEvent::Hook(HookEvent {
            kind: HookKind::PermissionNeeded,
            session_id: None,
            transcript_path: None,
            message: None,
        }),
    );
    assert_eq!(run.message(&permission), Vec::new());
    let reply = message(1, TabEvent::NoteReply { note_id: 1, text: "ok".into() });
    assert_eq!(run.message(&reply), Vec::new());
    assert_eq!(run.runner().phase(), &Phase::Working);
}

#[test]
fn a_failure_while_a_launch_is_on_its_way_is_the_old_agents() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    assert_eq!(run.agent_failed("exit 1".into()), Vec::new());
    assert_eq!(run.runner().phase(), &Phase::Working);
    run.launched();
    assert_eq!(run.agent_failed("exit 1".into()), Vec::new());
    assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::AgentFailed { .. })));
    assert_eq!(run.agent_failed("exit 2".into()), Vec::new(), "only while the stage works");
}

#[test]
fn a_launch_that_cannot_start_asks_the_user() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    assert_eq!(run.launch_failed("claude: not found".into()), Vec::new());
    assert!(!run.launching());
    let ask = run.ask().unwrap();
    assert_eq!(ask.title, "Work 1/4: the agent failed");
    assert_eq!(ask.detail.as_deref(), Some("claude: not found"));
}

#[test]
fn rejecting_asks_for_the_reason_first_and_sends_the_work_back_with_it() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    let passed = || RunEvent::TestResult { exit_code: 0, output: String::new() };
    run.message(&done(1, "built"));
    run.event(passed());
    run.message(&done(2, "reviewed"));
    run.message(&done(3, "tidied"));
    run.event(passed());
    run.message(&done(4, "feat: retry"));
    assert_eq!(run.ask().unwrap().title, "Hand off 4/4 is waiting for your approval");
    assert_eq!(run.choose(Choice::Reject), Vec::new());
    assert!(run.ui.rejecting && run.ui.focus_field);
    assert_eq!(run.choose(Choice::Reject), Vec::new(), "no reason, no rejection");
    assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::Approve)));
    run.ui.feedback = "mention the retry limit".into();
    assert_eq!(run.choose(Choice::Reject), Vec::new());
    assert_eq!(run.runner().phase(), &Phase::Stopped(StopReason::Rejected { stage: 3 }));
    assert_eq!(
        run.stopped().as_deref(),
        Some("Stage 4 (Hand off) was rejected and has no earlier stage to go back to")
    );
}

#[test]
fn clicking_a_stage_opens_its_summary_and_clicking_it_again_closes_it() {
    let mut run = ship_run(Some("t"), None);
    run.toggle_stage(1);
    assert_eq!(run.ui.open_stage, Some(1));
    run.toggle_stage(2);
    assert_eq!(run.ui.open_stage, Some(2));
    run.toggle_stage(2);
    assert_eq!(run.ui.open_stage, None);
}

#[test]
fn backing_out_of_a_rejection_clears_what_was_typed() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    run.ui.rejecting = true;
    run.ui.feedback = "half a thought".into();
    run.cancel_reject();
    assert_eq!((run.ui.rejecting, run.ui.feedback.as_str()), (false, ""));
}

#[test]
fn the_field_on_the_card_takes_the_keyboard_once_it_is_shown() {
    let mut run = ship_run(None, None);
    run.start();
    run.message(&done(1, "built"));
    assert!(run.ui.focus_field, "the test command field");
    run.ui.focus_field = false;
    let mut other = ship_run(Some("t"), None);
    other.start();
    other.message(&stop_hook(1, None));
    assert!(!other.ui.focus_field);
}

#[test]
fn a_test_command_typed_on_the_card_runs_the_tests_again() {
    let mut run = ship_run(None, None);
    run.start();
    run.message(&done(1, "built"));
    assert!(run.ask().unwrap().test_command);
    run.ui.test_command = "  cargo test ".into();
    let execs = run.choose(Choice::Retry);
    assert_eq!(execs, [Exec::RunTests { command: "cargo test".into(), timeout: LIMIT }]);
}

#[test]
fn choosing_with_nothing_typed_asks_the_gate_to_be_skipped_or_stopped() {
    let mut run = ship_run(None, None);
    run.start();
    run.message(&done(1, "built"));
    let execs = run.choose(Choice::Approve);
    assert_eq!(only_launch(&execs).index, 1, "the skipped gate moves on to Review");
    let mut other = ship_run(None, None);
    other.start();
    other.message(&done(1, "built"));
    assert_eq!(other.choose(Choice::Stop), Vec::new());
    assert_eq!(other.runner().phase(), &Phase::Stopped(StopReason::User));
}

#[test]
fn a_new_question_clears_what_was_typed_for_the_last_one() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    run.ui.feedback = "old".into();
    run.ui.rejecting = true;
    run.ui.open_stage = Some(2);
    run.message(&stop_hook(1, None));
    assert_eq!((run.ui.feedback.as_str(), run.ui.rejecting), ("", false));
    assert_eq!(run.ui.open_stage, Some(2), "an open stage stays open");
}

#[test]
fn the_status_is_held_while_the_runner_has_the_next_move() {
    let mut run = ship_run(Some("t"), None);
    assert_eq!(run.hold(), None);
    run.start();
    let starting = run.hold().unwrap();
    assert_eq!(starting.status, AgentStatus::SettingUp);
    assert_eq!(starting.detail.as_deref(), Some("Starting Work 1/4"));
    run.launched();
    assert_eq!(run.hold(), None, "a working stage shows its agent's own status");
    run.message(&done(1, "built"));
    let testing = run.hold().unwrap();
    assert_eq!(testing.status, AgentStatus::Working);
    assert_eq!(testing.detail.as_deref(), Some("Running the tests"));
    run.event(RunEvent::TestResult { exit_code: 1, output: "boom".into() });
    run.event(RunEvent::UserStop);
    assert_eq!(run.hold().unwrap().status, AgentStatus::Failed("Workflow stopped by you".into()));
}

#[test]
fn a_question_holds_the_tab_as_needing_you() {
    let mut run = ship_run(Some("t"), None);
    run.start();
    run.message(&stop_hook(1, Some("done?")));
    let hold = run.hold().unwrap();
    assert_eq!(hold.status, AgentStatus::NeedsYou(Reason::Input));
    assert_eq!(
        hold.detail.as_deref(),
        Some("Work 1/4: the agent stopped without saying it was done")
    );
}

#[test]
fn the_position_follows_the_run() {
    let mut run = ship_run(Some("t"), None);
    assert_eq!(run.position(), "Ship a feature: Work 1/4");
    run.start();
    run.message(&done(1, "built"));
    run.event(RunEvent::TestResult { exit_code: 0, output: String::new() });
    assert_eq!(run.position(), "Ship a feature: Review 2/4");
    run.event(RunEvent::UserStop);
    assert_eq!(run.position(), "Ship a feature: stopped");
}

#[test]
fn a_run_that_cannot_be_saved_says_so() {
    let mut run = ship_run(Some("t"), Some(PathBuf::from("/dev/null/stages.json")));
    run.start();
    assert!(run.save_error.as_deref().is_some_and(|e| e.starts_with("Cannot save the run")));
    let home = tempfile::tempdir().unwrap();
    let file =
        TidewatchPaths::new(home.path()).stages_file(std::path::Path::new("/w/web"), "otter");
    let mut fine = ship_run(Some("t"), Some(file));
    fine.start();
    assert_eq!(fine.save_error, None);
}

#[test]
fn the_last_summary_is_the_last_stage_that_reported_one() {
    let mut run = ship_run(Some("t"), None);
    assert_eq!(run.last_summary(), None);
    run.start();
    run.message(&done(1, "built"));
    assert_eq!(run.last_summary(), Some("built"));
}

#[derive(Default)]
struct Fake {
    launches: Vec<(usize, u8, u64)>,
    prompts: Vec<String>,
    dividers: Vec<String>,
    tests: VecDeque<(i32, &'static str)>,
    tests_run: usize,
}

impl Fake {
    fn play(&mut self, run: &mut TabRun, mut execs: Vec<Exec>) {
        while let Some(exec) = (!execs.is_empty()).then(|| execs.remove(0)) {
            match exec {
                Exec::Launch(launch) => {
                    self.launches.push((launch.index, launch.attempt, launch.generation));
                    self.prompts.push(launch.prompt.clone());
                    self.dividers.push(launch.divider.clone());
                    run.launched();
                    for message in Self::script(&launch) {
                        execs.extend(run.message(&message));
                    }
                }
                Exec::RunTests { .. } => {
                    self.tests_run += 1;
                    let (exit_code, output) = self.tests.pop_front().expect("a scripted run");
                    execs.extend(
                        run.event(RunEvent::TestResult { exit_code, output: output.into() }),
                    );
                }
            }
        }
    }

    fn script(launch: &StageLaunch) -> Vec<TabMessage> {
        let key = (launch.index, launch.attempt);
        let g = launch.generation;
        let late = launch.generation.saturating_sub(1);
        let mut messages = match key {
            (0, 1) => vec![done(g, "Added the refresh timer"), stop_hook(g, None)],
            (1, 1) => vec![
                finding(g, true, "the timer keeps running after sign-out"),
                done(g, "One blocking finding"),
                stop_hook(g, None),
            ],
            (0, 2) => vec![done(g, "Cleared the timer in signOut"), stop_hook(g, None)],
            (1, 2) => vec![done(g, "Nothing blocking"), stop_hook(g, None)],
            (2, 1) => vec![done(g, "Removed the duplicate in session.ts"), stop_hook(g, None)],
            (3, 1) => vec![done(g, "feat: retry the token refresh"), stop_hook(g, None)],
            other => panic!("no scripted turn for {other:?}"),
        };
        messages.insert(0, stop_hook(late, Some("the previous agent still talking")));
        messages.insert(1, done(late, "a stale summary"));
        messages
    }
}

#[test]
fn ship_a_feature_through_the_app_glue_loops_once_and_waits_at_hand_off() {
    let home = tempfile::tempdir().unwrap();
    let file =
        TidewatchPaths::new(home.path()).stages_file(std::path::Path::new("/w/web"), "otter");
    let mut run = ship_run(Some("npm test"), Some(file.clone()));
    let mut fake =
        Fake { tests: VecDeque::from([(0, "ok"), (0, "ok"), (0, "ok")]), ..Fake::default() };
    let first = run.start();
    fake.play(&mut run, first);

    assert_eq!(fake.launches, [(0, 1, 1), (1, 1, 2), (0, 2, 3), (1, 2, 4), (2, 1, 5), (3, 1, 6)]);
    assert_eq!(fake.tests_run, 3);
    assert_eq!(run.runner().loops_used(1), 1);
    assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::Approve)));
    assert_eq!(
        run.runner().states(),
        [StageState::Done, StageState::Done, StageState::Done, StageState::NeedsYou]
    );
    assert_eq!(run.runner().summary(1), Some("Nothing blocking"), "stale summaries never landed");
    assert_eq!(run.runner().summary(3), Some("feat: retry the token refresh"));
    assert_eq!(run.last_summary(), Some("feat: retry the token refresh"));
    assert_eq!(fake.dividers[2], "--- Stage 1: Work (claude opus, Auto edit, round 2) ---");
    assert_eq!(fake.dividers[1], "--- Stage 2: Review (gemini, Read only) ---");
    assert!(fake.prompts[2].contains("src/auth.ts:12 the timer keeps running after sign-out"));
    let ask = run.ask().unwrap();
    assert_eq!(ask.title, "Hand off 4/4 is waiting for your approval");
    assert_eq!(ask.detail.as_deref(), Some("feat: retry the token refresh"));

    let saved = Runner::load(&file).expect("the run was saved after the last event");
    assert_eq!(&saved, run.runner());

    fake.play(&mut run, Vec::new());
    assert_eq!(run.choose(Choice::Approve), Vec::new());
    assert!(run.finished());
    assert_eq!(run.position(), "Ship a feature: finished");
    assert_eq!(run.hold().unwrap().status, AgentStatus::Done);
    assert_eq!(Runner::load(&file).unwrap().phase(), &Phase::Finished);
}

#[test]
fn a_saved_run_resumes_with_the_command_it_was_waiting_on() {
    let home = tempfile::tempdir().unwrap();
    let file =
        TidewatchPaths::new(home.path()).stages_file(std::path::Path::new("/w/web"), "otter");
    let mut run = ship_run(Some("npm test"), Some(file.clone()));
    run.start();
    run.launched();
    run.message(&done(1, "built"));
    run.event(RunEvent::TestResult { exit_code: 0, output: String::new() });

    let (loaded, _) = tidewatch_core::restore::restore_workflow(&file).expect("a saved run");
    let mut resumed = TabRun::new(loaded, Some(file), LIMIT);
    let execs = resumed.resume();
    let launch = only_launch(&execs);
    assert_eq!((launch.index, launch.attempt, launch.generation), (1, 1, 1));
    assert!(launch.prompt.starts_with("Review main..HEAD"), "{}", launch.prompt);
    assert!(resumed.launching());
}

fn one_stage_run() -> TabRun {
    let workflow = Workflow {
        id: "w".into(),
        name: "One".into(),
        description: String::new(),
        stages: vec![Stage {
            gate: Gate::NoBlocking,
            run: tidewatch_core::workflows::StageRun {
                agent: Some("claude".into()),
                ..Default::default()
            },
            ..Stage::new(Action::Review)
        }],
    };
    TabRun::new(Runner::new(&workflow, &BTreeMap::new(), context(None)).unwrap(), None, LIMIT)
}

#[test]
fn a_one_stage_run_finishes_without_launching_again() {
    let mut run = one_stage_run();
    run.start();
    assert_eq!(run.message(&done(1, "clean")), Vec::new());
    assert!(run.finished());
    assert_eq!(run.hold().unwrap().status, AgentStatus::Done);
}
