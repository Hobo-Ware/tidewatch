//! The workflow runner: a pure state machine from events to commands.
//!
//! The app feeds [`RunEvent`]s in (what the agent reported over MCP, what the hooks and the process
//! said, the test command's result, what the user clicked) and carries out the [`Command`] each one
//! returns. The runner starts no process and reads no file: every gate is decided here from facts
//! the app hands over, never from the agent's word.
//!
//! ```text
//! start -> StartStage -> stage_done -> gate check -> next StartStage ... -> Finished
//!                            |              |
//!                   stopped, no stage_done  fails -> on_fail and loops left -> StartStage (earlier)
//!                            |              fails -> no loops left, or no on_fail -> AskUser
//!                         AskUser
//! ```
//!
//! Events that do not fit the current phase are ignored and return `None`; [`RunEvent::UserStop`]
//! stops from any phase that is still running. A stage that was started by `StartStage` is
//! running from then on, so there is no separate "stage started" event. The app must drop events
//! of a previous stage's agent once the next stage has started.
//!
//! The state serializes to `stages.json` ([`Runner::save`], [`Runner::load`]); after a restart,
//! [`Runner::resume`] repeats the command the run was waiting on.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::events::TabEvent;
use crate::fsutil::write_atomic;
use crate::profiles::Profile;
use crate::prompt::{self, Vars};
use crate::status::HookKind;
use crate::workflows::{Action, Gate, Stage, Workflow};

/// How much of a failing test run reaches a prompt: the end, where the errors are.
pub const TEST_OUTPUT_TAIL_CHARS: usize = 4_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageState {
    #[default]
    Todo,
    Running,
    NeedsYou,
    Done,
    Failed,
}

/// What the run is about, fixed when the tab opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunContext {
    pub task: String,
    pub base: String,
    pub branch: String,
    /// The repo's test command (see `test_command`). `None` makes the test gates ask the user.
    pub test_command: Option<String>,
}

/// A finding an agent reported while a stage ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The stage that was running.
    pub stage: usize,
    pub file: String,
    pub line: u32,
    pub blocking: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// The agent called `stage_done`.
    StageDone { summary: String },
    /// The agent called `report_finding`.
    Finding { file: String, line: u32, blocking: bool, text: String },
    /// The agent's turn ended and no `stage_done` came (the Stop hook, with its last message).
    AgentStopped { last_message: Option<String> },
    /// The agent process exited non-zero or reported an error.
    AgentFailed { reason: String },
    /// The test command finished; `output` is its combined output, the runner keeps the tail.
    TestResult { exit_code: i32, output: String },
    /// Approve the gate. When the run asked about something else this means "pass": accept the stage.
    UserApprove,
    /// Reject an approval, with the reason. The reason reaches the stage the work goes back to.
    UserReject { feedback: String },
    /// Run the stage again, or hand a failed gate another round of loops.
    UserRetry,
    /// Stop the workflow.
    UserStop,
}

impl RunEvent {
    /// What a listener message means to the runner: `stage_done`, `report_finding`, a finished turn
    /// (the safety net) and a failed turn. Everything else is the status line's business.
    #[must_use]
    pub fn from_tab_event(event: &TabEvent) -> Option<Self> {
        match event {
            TabEvent::StageDone { summary } => Some(Self::StageDone { summary: summary.clone() }),
            TabEvent::Finding { file, line, blocking, text } => Some(Self::Finding {
                file: file.clone(),
                line: *line,
                blocking: *blocking,
                text: text.clone(),
            }),
            TabEvent::Hook(hook) => match &hook.kind {
                HookKind::TurnDone => {
                    Some(Self::AgentStopped { last_message: hook.message.clone() })
                }
                HookKind::TurnFailed(reason) => Some(Self::AgentFailed { reason: reason.clone() }),
                _ => None,
            },
            TabEvent::NoteReply { .. }
            | TabEvent::Proposal { .. }
            | TabEvent::Progress { .. }
            | TabEvent::Job { .. }
            | TabEvent::Ask { .. } => None,
        }
    }
}

/// Why the run waits for the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskReason {
    /// An approval gate, or a stage with `approve`: approve, or reject with a reason.
    Approve,
    /// The agent stopped without `stage_done`. Pass runs the gate on what is there, retry runs the
    /// stage again.
    NoStageDone {
        last_message: Option<String>,
    },
    /// The gate failed and there was no `on_fail`, or its loops are used up.
    GateFailed {
        gate: Gate,
        detail: String,
        loops_used: u8,
    },
    AgentFailed {
        reason: String,
    },
    /// A test gate with no test command. Set one and retry, or pass.
    NoTestCommand,
}

impl AskReason {
    fn stage_state(&self) -> StageState {
        match self {
            Self::Approve | Self::NoStageDone { .. } | Self::NoTestCommand => StageState::NeedsYou,
            Self::GateFailed { .. } | Self::AgentFailed { .. } => StageState::Failed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    User,
    /// An approval was rejected and the stage has no `on_fail` to go back to.
    Rejected {
        stage: usize,
    },
}

/// What the app does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Launch the stage's agent in the tab's pane (with a divider), with `prompt` as its task.
    /// `attempt` counts from 1; above 1 means the stage runs again after a loop or a retry.
    StartStage {
        index: usize,
        attempt: u8,
        action: Action,
        profile: Profile,
        prompt: String,
    },
    /// Run the test command in the worktree and answer with [`RunEvent::TestResult`].
    RunTests {
        command: String,
    },
    /// Show the tab as needing the user and answer with the event the reason names.
    AskUser {
        index: usize,
        reason: AskReason,
    },
    Finished,
    Stopped {
        reason: StopReason,
    },
}

/// Where the run is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Built, not started.
    Idle,
    /// The stage's agent works.
    Working,
    /// The test command runs.
    Testing,
    Asking(AskReason),
    Finished,
    Stopped(StopReason),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PlannedStage {
    stage: Stage,
    profile: Profile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    workflow_name: String,
    stages: Vec<PlannedStage>,
    ctx: RunContext,
    position: usize,
    phase: Phase,
    states: Vec<StageState>,
    loops: Vec<u8>,
    attempts: Vec<u8>,
    summaries: Vec<Option<String>>,
    latest_summary: Option<usize>,
    findings: Vec<Finding>,
    test_output: Option<String>,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn tail(text: &str, max_chars: usize) -> String {
    let text = text.trim_end();
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_owned();
    }
    let start = text.char_indices().nth(total - max_chars).map_or(0, |(offset, _)| offset);
    format!("...\n{}", &text[start..])
}

fn format_findings<'a>(findings: impl Iterator<Item = &'a Finding>) -> String {
    let mut out = String::new();
    for finding in findings {
        let kind = if finding.blocking { "blocking" } else { "nit" };
        let place = match (finding.file.as_str(), finding.line) {
            ("", _) => String::new(),
            (file, 0) => format!(" {file}"),
            (file, line) => format!(" {file}:{line}"),
        };
        let _ = writeln!(out, "- [{kind}]{place} {}", finding.text);
    }
    out.trim_end().to_owned()
}

impl Runner {
    /// Plans a run: resolves every stage's launch profile against `profiles`. Fails with the
    /// reasons when the workflow cannot run (see `Workflow::validate`) or a profile is unknown.
    pub fn new(
        workflow: &Workflow,
        profiles: &BTreeMap<String, Profile>,
        ctx: RunContext,
    ) -> Result<Self, Vec<String>> {
        let mut problems = workflow.validate();
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut stages = Vec::with_capacity(workflow.stages.len());
        for (index, stage) in workflow.stages.iter().enumerate() {
            match stage.resolve_profile(&workflow.name, profiles) {
                Ok(profile) => stages.push(PlannedStage { stage: stage.clone(), profile }),
                Err(e) => {
                    problems.push(format!("stage {} ({}): {e}", index + 1, stage.action.label()));
                }
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        let count = stages.len();
        Ok(Self {
            workflow_name: workflow.name.clone(),
            stages,
            ctx,
            position: 0,
            phase: Phase::Idle,
            states: vec![StageState::Todo; count],
            loops: vec![0; count],
            attempts: vec![0; count],
            summaries: vec![None; count],
            latest_summary: None,
            findings: Vec::new(),
            test_output: None,
        })
    }

    #[must_use]
    pub fn workflow_name(&self) -> &str {
        &self.workflow_name
    }

    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    #[must_use]
    pub fn stage(&self, index: usize) -> Option<&Stage> {
        self.stages.get(index).map(|planned| &planned.stage)
    }

    /// The launch profile stage `index` runs with, as resolved when the run was planned.
    #[must_use]
    pub fn profile(&self, index: usize) -> Option<&Profile> {
        self.stages.get(index).map(|planned| &planned.profile)
    }

    #[must_use]
    pub fn context(&self) -> &RunContext {
        &self.ctx
    }

    /// The stage the run is on (the last one once it is over).
    #[must_use]
    pub fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    #[must_use]
    pub fn states(&self) -> &[StageState] {
        &self.states
    }

    /// How many times stage `index`'s gate has sent the work back.
    #[must_use]
    pub fn loops_used(&self, index: usize) -> u8 {
        self.loops.get(index).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn summary(&self, index: usize) -> Option<&str> {
        self.summaries.get(index)?.as_deref()
    }

    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    #[must_use]
    pub fn is_over(&self) -> bool {
        matches!(self.phase, Phase::Finished | Phase::Stopped(_))
    }

    /// Changes the test command, for a run that asked because it had none.
    pub fn set_test_command(&mut self, command: Option<String>) {
        self.ctx.test_command = command;
    }

    /// Sets the task of a run that has not started yet. Ignored once it has.
    pub fn set_task(&mut self, task: &str) {
        if self.phase == Phase::Idle {
            task.trim().clone_into(&mut self.ctx.task);
        }
    }

    /// Starts stage 1. `None` when the run already started.
    pub fn start(&mut self) -> Option<Command> {
        (self.phase == Phase::Idle).then(|| self.begin_stage(0))
    }

    pub fn handle(&mut self, event: RunEvent) -> Option<Command> {
        if matches!(self.phase, Phase::Idle | Phase::Finished | Phase::Stopped(_)) {
            return None;
        }
        if event == RunEvent::UserStop {
            return Some(self.stop(StopReason::User));
        }
        let command = match (self.phase.clone(), event) {
            (
                Phase::Working | Phase::Asking(AskReason::NoStageDone { .. }),
                RunEvent::StageDone { summary },
            ) => {
                self.record_summary(summary);
                self.check_gate()
            }
            (
                Phase::Working | Phase::Asking(AskReason::NoStageDone { .. }),
                RunEvent::Finding { file, line, blocking, text },
            ) => {
                self.findings.push(Finding { stage: self.position, file, line, blocking, text });
                return None;
            }
            (Phase::Working, RunEvent::AgentStopped { last_message }) => {
                self.ask(AskReason::NoStageDone { last_message })
            }
            (Phase::Working, RunEvent::AgentFailed { reason }) => {
                self.fail(AskReason::AgentFailed { reason })
            }
            (Phase::Testing, RunEvent::TestResult { exit_code, output }) => {
                self.test_finished(exit_code, &output)
            }
            (Phase::Asking(reason), RunEvent::UserApprove) => self.approved(reason),
            (Phase::Asking(AskReason::Approve), RunEvent::UserReject { feedback }) => {
                self.rejected(feedback)
            }
            (Phase::Asking(reason), RunEvent::UserRetry) => return self.retry(&reason),
            _ => return None,
        };
        Some(command)
    }

    /// The command the run is waiting on, to repeat after a restart: the stage's agent starts again
    /// with the same prompt, tests run again, a question is asked again.
    #[must_use]
    pub fn resume(&self) -> Option<Command> {
        match &self.phase {
            Phase::Idle => None,
            Phase::Working => Some(self.start_command(self.position)),
            Phase::Testing => {
                self.ctx.test_command.clone().map(|command| Command::RunTests { command })
            }
            Phase::Asking(reason) => {
                Some(Command::AskUser { index: self.position, reason: reason.clone() })
            }
            Phase::Finished => Some(Command::Finished),
            Phase::Stopped(reason) => Some(Command::Stopped { reason: *reason }),
        }
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }

    /// `None` for a missing file, and for one that is unreadable or does not describe a runnable
    /// state (a stderr note says which): a bad state file must never stop the app.
    #[must_use]
    pub fn load(file: &Path) -> Option<Self> {
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); not resuming", file.display());
                return None;
            }
        };
        let runner = serde_json::from_slice::<Self>(&bytes)
            .inspect_err(|e| {
                eprintln!("tidewatch: {} is unreadable ({e}); not resuming", file.display());
            })
            .ok()?;
        if !runner.is_consistent() {
            eprintln!("tidewatch: {} does not match its workflow; not resuming", file.display());
            return None;
        }
        Some(runner)
    }

    fn is_consistent(&self) -> bool {
        let count = self.stages.len();
        count > 0
            && self.position < count
            && [self.states.len(), self.loops.len(), self.attempts.len(), self.summaries.len()]
                .iter()
                .all(|len| *len == count)
            && self.latest_summary.is_none_or(|index| index < count)
            && self.findings.iter().all(|finding| finding.stage < count)
            && self
                .stages
                .iter()
                .enumerate()
                .all(|(index, planned)| planned.stage.on_fail.is_none_or(|target| target < index))
    }

    fn stage_at(&self, index: usize) -> &Stage {
        &self.stages[index].stage
    }

    fn start_command(&self, index: usize) -> Command {
        let planned = &self.stages[index];
        let template = planned.stage.prompt_template();
        let others = format_findings(self.findings.iter().filter(|f| f.stage != index));
        let test_output = self.test_output.as_deref().unwrap_or("");
        let previous_summary =
            self.latest_summary.and_then(|i| self.summaries[i].as_deref()).unwrap_or("");
        let vars = Vars {
            task: &self.ctx.task,
            base: &self.ctx.base,
            branch: &self.ctx.branch,
            previous_summary,
            findings: &others,
            test_output,
        };
        let mut prompt = prompt::render(template, &vars);
        let attempt = self.attempts[index];
        if attempt > 1 {
            if !others.is_empty() && !template.contains("{findings}") {
                let _ = write!(prompt, "\n\nOpen findings to address:\n{others}");
            }
            if !test_output.is_empty() && !template.contains("{test_output}") {
                let _ = write!(prompt, "\n\nOutput of the failing test run:\n{test_output}");
            }
        }
        Command::StartStage {
            index,
            attempt,
            action: planned.stage.action,
            profile: planned.profile.clone(),
            prompt,
        }
    }

    fn begin_stage(&mut self, index: usize) -> Command {
        self.position = index;
        self.states[index] = StageState::Running;
        self.attempts[index] = self.attempts[index].saturating_add(1);
        self.findings.retain(|finding| finding.stage != index);
        self.phase = Phase::Working;
        self.start_command(index)
    }

    fn record_summary(&mut self, summary: String) {
        self.summaries[self.position] = Some(summary);
        self.latest_summary = Some(self.position);
    }

    fn ask(&mut self, reason: AskReason) -> Command {
        self.states[self.position] = reason.stage_state();
        self.phase = Phase::Asking(reason.clone());
        Command::AskUser { index: self.position, reason }
    }

    fn stop(&mut self, reason: StopReason) -> Command {
        if self.states[self.position] != StageState::Done {
            self.states[self.position] = StageState::Failed;
        }
        self.phase = Phase::Stopped(reason);
        Command::Stopped { reason }
    }

    fn check_gate(&mut self) -> Command {
        let gate = self.stage_at(self.position).gate;
        match gate {
            Gate::Approve => self.ask(AskReason::Approve),
            Gate::NoBlocking => {
                let blocking =
                    self.findings.iter().filter(|f| f.stage == self.position && f.blocking).count();
                if blocking == 0 {
                    self.gate_passed()
                } else {
                    let detail = plural(blocking, "blocking finding", "blocking findings");
                    self.fail(AskReason::GateFailed { gate, detail, loops_used: 0 })
                }
            }
            Gate::Tests | Gate::TestFails | Gate::TestPasses | Gate::SameBehavior => {
                match self.ctx.test_command.clone() {
                    None => self.ask(AskReason::NoTestCommand),
                    Some(command) => {
                        self.states[self.position] = StageState::Running;
                        self.phase = Phase::Testing;
                        Command::RunTests { command }
                    }
                }
            }
        }
    }

    fn test_finished(&mut self, exit_code: i32, output: &str) -> Command {
        let gate = self.stage_at(self.position).gate;
        let passed = exit_code == 0;
        self.test_output = (!passed).then(|| tail(output, TEST_OUTPUT_TAIL_CHARS));
        if passed != (gate == Gate::TestFails) {
            return self.gate_passed();
        }
        let detail = if gate == Gate::TestFails {
            "the tests passed, but a failing test was expected".to_owned()
        } else {
            format!("tests failed (exit {exit_code})")
        };
        self.fail(AskReason::GateFailed { gate, detail, loops_used: 0 })
    }

    fn gate_passed(&mut self) -> Command {
        if self.stage_at(self.position).approve
            && self.stage_at(self.position).gate != Gate::Approve
        {
            return self.ask(AskReason::Approve);
        }
        self.advance()
    }

    fn advance(&mut self) -> Command {
        self.states[self.position] = StageState::Done;
        if self.position + 1 == self.stages.len() {
            self.phase = Phase::Finished;
            return Command::Finished;
        }
        self.begin_stage(self.position + 1)
    }

    fn fail(&mut self, mut reason: AskReason) -> Command {
        let index = self.position;
        let stage = self.stage_at(index);
        if let Some(target) = stage.on_fail
            && self.loops[index] < stage.max_loops
        {
            return self.loop_back(index, target);
        }
        if let AskReason::GateFailed { loops_used, .. } = &mut reason {
            *loops_used = self.loops[index];
        }
        self.ask(reason)
    }

    fn loop_back(&mut self, from: usize, target: usize) -> Command {
        self.loops[from] = self.loops[from].saturating_add(1);
        for state in &mut self.states[target..=from] {
            *state = StageState::Todo;
        }
        self.begin_stage(target)
    }

    fn approved(&mut self, reason: AskReason) -> Command {
        match reason {
            AskReason::NoStageDone { last_message } => {
                if let Some(message) = last_message
                    && self.summaries[self.position].is_none()
                {
                    self.record_summary(message);
                }
                self.check_gate()
            }
            AskReason::Approve
            | AskReason::GateFailed { .. }
            | AskReason::AgentFailed { .. }
            | AskReason::NoTestCommand => self.advance(),
        }
    }

    fn rejected(&mut self, feedback: String) -> Command {
        let index = self.position;
        let stage = self.stage_at(index);
        let (gate, on_fail) = (stage.gate, stage.on_fail);
        if on_fail.is_none() {
            return self.stop(StopReason::Rejected { stage: index });
        }
        let detail = format!("rejected: {feedback}");
        self.findings.push(Finding {
            stage: index,
            file: String::new(),
            line: 0,
            blocking: true,
            text: feedback,
        });
        self.fail(AskReason::GateFailed { gate, detail, loops_used: 0 })
    }

    fn retry(&mut self, reason: &AskReason) -> Option<Command> {
        let index = self.position;
        let command = match reason {
            AskReason::Approve => return None,
            AskReason::NoTestCommand => self.check_gate(),
            AskReason::GateFailed { .. } => match self.stage_at(index).on_fail {
                Some(target) => {
                    self.loops[index] = 0;
                    self.loop_back(index, target)
                }
                None => self.begin_stage(index),
            },
            AskReason::NoStageDone { .. } | AskReason::AgentFailed { .. } => {
                self.begin_stage(index)
            }
        };
        Some(command)
    }
}

#[cfg(test)]
mod tests;
