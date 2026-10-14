//! What the stepper on a workflow tab shows, as plain data: one pill per stage with its state, the
//! loop counter, the gate the run is at, and, for a clicked stage, its summary and findings.
//! `stepper_view` draws it.

use std::time::Duration;

use tidewatch_core::runner::{Finding, Phase, Runner, StageState};
use tidewatch_core::workflows::Gate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub(crate) label: &'static str,
    pub(crate) agent_id: String,
    pub(crate) state: StageState,
    /// How many times this stage's gate sent the work back, and how many times it may.
    pub(crate) loops: Option<(u8, u8)>,
    pub(crate) tip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Model {
    pub(crate) name: String,
    pub(crate) pills: Vec<Pill>,
    /// What the run is doing right now.
    pub(crate) now: String,
    /// The gate of the stage the run is at, while the run goes on.
    pub(crate) gate: Option<String>,
    /// The loop counter of the stage that sent work back last: `Loop 1 of 2`.
    pub(crate) loop_text: Option<String>,
}

fn minutes(timeout: Duration) -> String {
    let secs = timeout.as_secs();
    if secs >= 120 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

fn who(runner: &Runner, index: usize) -> String {
    let Some(profile) = runner.profile(index) else { return String::new() };
    let model = profile.model.as_deref().map(|model| format!(" {model}")).unwrap_or_default();
    format!("{}{model}, {}", profile.agent, profile.level.label())
}

fn tip(runner: &Runner, index: usize) -> String {
    let Some(stage) = runner.stage(index) else { return String::new() };
    let approval = if stage.waits_for_user() && stage.gate != Gate::Approve {
        ", then your approval"
    } else {
        ""
    };
    format!(
        "{} - {} - gate: {}{approval}",
        stage.action.label(),
        who(runner, index),
        stage.gate.label()
    )
}

fn now_text(runner: &Runner, timeout: Duration) -> String {
    let count = runner.stage_count();
    let number = runner.position() + 1;
    match runner.phase() {
        Phase::Idle => "Waiting for a task".to_owned(),
        Phase::Working => format!("Stage {number} of {count} is working"),
        Phase::Testing => format!("Running the tests (stops after {})", minutes(timeout)),
        Phase::Asking(_) => "Waiting for you".to_owned(),
        Phase::Finished => "Finished".to_owned(),
        Phase::Stopped(_) => "Stopped".to_owned(),
    }
}

/// The loop counter: the stage that sent work back most recently (the latest stage with loops used).
fn loop_text(runner: &Runner) -> Option<String> {
    let index = (0..runner.stage_count()).rev().find(|i| runner.loops_used(*i) > 0)?;
    let max = runner.stage(index)?.max_loops;
    Some(format!("Loop {} of {max}", runner.loops_used(index)))
}

pub(crate) fn model(runner: &Runner, timeout: Duration) -> Model {
    let pills = (0..runner.stage_count())
        .filter_map(|index| {
            let stage = runner.stage(index)?;
            let loops = (stage.on_fail.is_some() && runner.loops_used(index) > 0)
                .then(|| (runner.loops_used(index), stage.max_loops));
            Some(Pill {
                label: stage.action.label(),
                agent_id: runner.profile(index)?.agent.clone(),
                state: runner.states().get(index).copied().unwrap_or_default(),
                loops,
                tip: tip(runner, index),
            })
        })
        .collect();
    let gate = (!runner.is_over())
        .then(|| runner.stage(runner.position()).map(|stage| stage.gate.label().to_owned()))
        .flatten();
    Model {
        name: runner.workflow_name().to_owned(),
        pills,
        now: now_text(runner, timeout),
        gate,
        loop_text: loop_text(runner),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindingLine {
    pub(crate) blocking: bool,
    pub(crate) text: String,
}

/// What a click on a stage opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detail {
    pub(crate) title: String,
    pub(crate) who: String,
    pub(crate) gate: String,
    pub(crate) summary: Option<String>,
    pub(crate) findings: Vec<FindingLine>,
}

fn finding_line(finding: &Finding) -> FindingLine {
    let place = match (finding.file.as_str(), finding.line) {
        ("", _) => String::new(),
        (file, 0) => format!("{file} "),
        (file, line) => format!("{file}:{line} "),
    };
    FindingLine { blocking: finding.blocking, text: format!("{place}{}", finding.text) }
}

pub(crate) fn detail(runner: &Runner, index: usize) -> Option<Detail> {
    let stage = runner.stage(index)?;
    Some(Detail {
        title: format!("Stage {} of {}: {}", index + 1, runner.stage_count(), stage.action.label()),
        who: who(runner, index),
        gate: stage.gate.label().to_owned(),
        summary: runner.summary(index).map(str::to_owned),
        findings: runner
            .findings()
            .iter()
            .filter(|finding| finding.stage == index)
            .map(finding_line)
            .collect(),
    })
}

/// `Review 2/4`, the stage the run is at, for the Overview and the ask card.
pub(crate) fn position_label(runner: &Runner) -> String {
    let action = runner.stage(runner.position()).map_or("", |stage| stage.action.label());
    format!("{action} {}/{}", runner.position() + 1, runner.stage_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tidewatch_core::runner::{RunContext, RunEvent};
    use tidewatch_core::workflows::Workflows;

    fn ship() -> Runner {
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let ctx = RunContext {
            task: "add a retry".into(),
            base: "main".into(),
            branch: "agent/brave-otter".into(),
            test_command: Some("npm test".into()),
        };
        Runner::new(&ship, &BTreeMap::new(), ctx).unwrap()
    }

    fn states(model: &Model) -> Vec<StageState> {
        model.pills.iter().map(|pill| pill.state).collect()
    }

    fn done(summary: &str) -> RunEvent {
        RunEvent::StageDone { summary: summary.into() }
    }

    fn blocking(text: &str) -> RunEvent {
        RunEvent::Finding {
            file: "src/auth.ts".into(),
            line: 12,
            blocking: true,
            text: text.into(),
        }
    }

    const LIMIT: Duration = Duration::from_secs(600);

    #[test]
    fn a_fresh_run_shows_four_stages_to_do() {
        let model = model(&ship(), LIMIT);
        assert_eq!(model.name, "Ship a feature");
        let labels: Vec<_> = model.pills.iter().map(|p| p.label).collect();
        assert_eq!(labels, ["Work", "Review", "Refactor", "Hand off"]);
        assert_eq!(states(&model), [StageState::Todo; 4]);
        assert_eq!(model.now, "Waiting for a task");
        assert_eq!(model.loop_text, None);
    }

    #[test]
    fn the_running_stage_and_its_gate_show_while_the_agent_works() {
        let mut runner = ship();
        runner.start();
        let model = model(&runner, LIMIT);
        assert_eq!(
            states(&model),
            [StageState::Running, StageState::Todo, StageState::Todo, StageState::Todo]
        );
        assert_eq!(model.now, "Stage 1 of 4 is working");
        assert_eq!(model.gate.as_deref(), Some("Tests pass"));
        assert_eq!(model.pills[0].agent_id, "claude");
        assert_eq!(model.pills[1].agent_id, "gemini");
    }

    #[test]
    fn running_the_tests_says_how_long_they_may_take() {
        let mut runner = ship();
        runner.start();
        runner.handle(done("x"));
        assert_eq!(model(&runner, LIMIT).now, "Running the tests (stops after 10 min)");
        assert_eq!(
            model(&runner, Duration::from_secs(90)).now,
            "Running the tests (stops after 90 s)"
        );
    }

    #[test]
    fn a_loop_back_shows_the_counter_on_the_stage_that_sent_the_work_back() {
        let mut runner = ship();
        runner.start();
        runner.handle(done("built"));
        runner.handle(RunEvent::TestResult { exit_code: 0, output: String::new() });
        runner.handle(blocking("the timer keeps running"));
        runner.handle(done("one blocking finding"));
        let model = model(&runner, LIMIT);
        assert_eq!(model.pills[1].loops, Some((1, 2)));
        assert_eq!(model.pills[0].loops, None);
        assert_eq!(model.loop_text.as_deref(), Some("Loop 1 of 2"));
        assert_eq!(model.now, "Stage 1 of 4 is working");
        assert_eq!(states(&model)[..2], [StageState::Running, StageState::Todo]);
    }

    #[test]
    fn a_stage_that_needs_you_and_one_that_failed_show_as_such() {
        let mut runner = ship();
        runner.start();
        runner.handle(RunEvent::AgentStopped { last_message: None });
        let model = model(&runner, LIMIT);
        assert_eq!(model.pills[0].state, StageState::NeedsYou);
        assert_eq!(model.now, "Waiting for you");
        runner.handle(RunEvent::UserRetry);
        runner.handle(RunEvent::AgentFailed { reason: "exit 1".into() });
        assert_eq!(super::model(&runner, LIMIT).pills[0].state, StageState::Failed);
    }

    #[test]
    fn a_finished_run_has_every_stage_done_and_no_gate() {
        let mut runner = ship();
        runner.start();
        for (summary, tests) in [("a", true), ("b", false), ("c", true), ("d", false)] {
            runner.handle(done(summary));
            if tests {
                runner.handle(RunEvent::TestResult { exit_code: 0, output: String::new() });
            }
        }
        runner.handle(RunEvent::UserApprove);
        let model = model(&runner, LIMIT);
        assert_eq!(states(&model), [StageState::Done; 4]);
        assert_eq!((model.now.as_str(), model.gate), ("Finished", None));
    }

    #[test]
    fn a_stopped_run_says_so() {
        let mut runner = ship();
        runner.start();
        runner.handle(RunEvent::UserStop);
        let model = model(&runner, LIMIT);
        assert_eq!((model.now.as_str(), model.gate), ("Stopped", None));
        assert_eq!(model.pills[0].state, StageState::Failed);
    }

    #[test]
    fn a_pill_tip_names_the_agent_model_level_and_gate() {
        let model = model(&ship(), LIMIT);
        assert_eq!(model.pills[0].tip, "Work - claude opus, Auto edit - gate: Tests pass");
        assert_eq!(model.pills[1].tip, "Review - gemini, Read only - gate: No blocking findings");
        assert_eq!(model.pills[3].tip, "Hand off - claude haiku, Read only - gate: You approve");
    }

    #[test]
    fn a_stage_with_the_approve_flag_says_it_waits_for_you_after_its_gate() {
        let mut ship = Workflows::defaults().workflow.remove("ship").unwrap();
        ship.stages[0].approve = true;
        let runner = Runner::new(
            &ship,
            &BTreeMap::new(),
            RunContext {
                task: String::new(),
                base: "main".into(),
                branch: "b".into(),
                test_command: None,
            },
        )
        .unwrap();
        assert!(
            model(&runner, LIMIT).pills[0].tip.ends_with("gate: Tests pass, then your approval")
        );
    }

    #[test]
    fn a_clicked_stage_shows_its_summary_and_findings() {
        let mut runner = ship();
        runner.start();
        runner.handle(done("Added the refresh timer"));
        runner.handle(RunEvent::TestResult { exit_code: 0, output: String::new() });
        runner.handle(blocking("the timer keeps running after sign-out"));
        runner.handle(RunEvent::Finding {
            file: String::new(),
            line: 0,
            blocking: false,
            text: "rename x".into(),
        });
        let work = detail(&runner, 0).unwrap();
        assert_eq!(work.title, "Stage 1 of 4: Work");
        assert_eq!(work.who, "claude opus, Auto edit");
        assert_eq!(work.gate, "Tests pass");
        assert_eq!(work.summary.as_deref(), Some("Added the refresh timer"));
        assert_eq!(work.findings, Vec::<FindingLine>::new());
        let review = detail(&runner, 1).unwrap();
        assert_eq!(review.summary, None);
        assert_eq!(
            review.findings,
            [
                FindingLine {
                    blocking: true,
                    text: "src/auth.ts:12 the timer keeps running after sign-out".into()
                },
                FindingLine { blocking: false, text: "rename x".into() },
            ]
        );
        assert_eq!(detail(&runner, 9), None);
    }

    #[test]
    fn the_position_names_the_stage_and_its_place() {
        let mut runner = ship();
        assert_eq!(position_label(&runner), "Work 1/4");
        runner.start();
        runner.handle(done("x"));
        runner.handle(RunEvent::TestResult { exit_code: 0, output: String::new() });
        assert_eq!(position_label(&runner), "Review 2/4");
    }
}
