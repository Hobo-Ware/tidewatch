//! One workflow tab's run in the app: the `Runner`, where it is saved, which agent may report and
//! what the user is typing into the ask card. Everything the runner decides comes back as an
//! [`Exec`] for the app to carry out (launch the next stage's agent, run the tests); the state is
//! saved to `stages.json` after every event. No threads, no terminal: the rules are unit tests.
//!
//! Each launch of a stage's agent gets a new generation number, which the agent's token stamps on
//! everything it sends. The number moves the moment the runner starts a stage, so what the previous
//! stage's agent still says (its Stop hook right after `stage_done`, for one) is dropped.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tidewatch_core::events::TabMessage;
use tidewatch_core::profiles::Profile;
use tidewatch_core::runner::{AskReason, Command, Phase, RunEvent, Runner, StopReason};
use tidewatch_core::status::{AgentStatus, Reason};
use tidewatch_core::workflows::Action;

use crate::stepper;
use crate::workflow_ask::{self, Ask, Choice};

/// Launch the stage's agent in the tab, in place of the previous stage's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StageLaunch {
    pub(crate) index: usize,
    pub(crate) attempt: u8,
    pub(crate) generation: u64,
    pub(crate) profile: Profile,
    pub(crate) prompt: String,
    /// The line shown where the stage starts, in the Reader and in the terminal.
    pub(crate) divider: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Exec {
    Launch(StageLaunch),
    RunTests { command: String, timeout: Duration },
}

/// What the status of a workflow tab says while the runner, not the agent, holds the next move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hold {
    pub(crate) status: AgentStatus,
    pub(crate) detail: Option<String>,
}

/// What the user typed or opened on the tab's cards.
#[derive(Debug, Default)]
pub(crate) struct RunUi {
    /// The stage whose summary and findings are open under the stepper.
    pub(crate) open_stage: Option<usize>,
    pub(crate) feedback: String,
    pub(crate) rejecting: bool,
    pub(crate) test_command: String,
    /// The field that was just shown takes the keyboard, once.
    pub(crate) focus_field: bool,
    pub(crate) task: String,
    pub(crate) focus_task: bool,
}

pub(crate) struct TabRun {
    runner: Runner,
    stages_file: Option<PathBuf>,
    test_timeout: Duration,
    test_why: Option<String>,
    generation: u64,
    launching: bool,
    pub(crate) ui: RunUi,
    pub(crate) save_error: Option<String>,
    blurb: String,
}

fn stop_text(runner: &Runner, reason: StopReason) -> String {
    match reason {
        StopReason::User => "Workflow stopped by you".to_owned(),
        StopReason::Rejected { stage } => {
            let name = runner.stage(stage).map_or("", |stage| stage.action.label());
            format!(
                "Stage {} ({name}) was rejected and has no earlier stage to go back to",
                stage + 1
            )
        }
    }
}

/// `--- Stage 2: Review (gemini, Read only) ---`, with the round when the stage runs again.
pub(crate) fn divider(index: usize, action: Action, profile: &Profile, attempt: u8) -> String {
    let model = profile.model.as_deref().map(|model| format!(" {model}")).unwrap_or_default();
    let again = if attempt > 1 { format!(", round {attempt}") } else { String::new() };
    format!(
        "--- Stage {}: {} ({}{model}, {}{again}) ---",
        index + 1,
        action.label(),
        profile.agent,
        profile.level.label(),
    )
}

impl TabRun {
    pub(crate) fn new(
        runner: Runner,
        stages_file: Option<PathBuf>,
        test_timeout: Duration,
    ) -> Self {
        let waiting = *runner.phase() == Phase::Idle && runner.context().task.trim().is_empty();
        let ui = RunUi { focus_task: waiting, ..RunUi::default() };
        Self {
            runner,
            stages_file,
            test_timeout,
            test_why: None,
            generation: 0,
            launching: false,
            ui,
            save_error: None,
            blurb: String::new(),
        }
    }

    pub(crate) fn with_blurb(mut self, blurb: String) -> Self {
        self.blurb = blurb;
        self
    }

    pub(crate) fn with_test_why(mut self, why: Option<String>) -> Self {
        self.test_why = why;
        self
    }

    pub(crate) fn test_why(&self) -> Option<&str> {
        self.test_why.as_deref()
    }

    /// A run taken over from another process continues at the stage generation its agent holds.
    pub(crate) fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// A stage is on its way or its tests are running: work in this process that no other can
    /// finish.
    pub(crate) fn busy(&self) -> bool {
        self.launching || *self.runner.phase() == Phase::Testing
    }

    pub(crate) fn blurb(&self) -> &str {
        &self.blurb
    }

    pub(crate) fn awaiting_task(&self) -> bool {
        *self.runner.phase() == Phase::Idle && self.runner.context().task.trim().is_empty()
    }

    /// Saves the run as it is, so a tab that is closed before it starts comes back asking for its task.
    pub(crate) fn persist(&mut self) {
        self.save();
    }

    /// Start run: the task goes to the runner and stage 1 starts. Nothing happens for a blank task or
    /// a run that already started.
    pub(crate) fn start_with(&mut self, task: &str) -> Vec<Exec> {
        if !self.awaiting_task() || task.trim().is_empty() {
            return Vec::new();
        }
        self.runner.set_task(task);
        self.ui.task.clear();
        self.start()
    }

    pub(crate) fn runner(&self) -> &Runner {
        &self.runner
    }

    pub(crate) fn stages_file(&self) -> Option<&Path> {
        self.stages_file.as_deref()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn test_timeout(&self) -> Duration {
        self.test_timeout
    }

    /// A launch was asked for and its agent is not running yet.
    #[cfg(test)]
    pub(crate) fn launching(&self) -> bool {
        self.launching
    }

    pub(crate) fn launched(&mut self) {
        self.launching = false;
    }

    /// Whether a message stamped `generation` comes from the agent that runs now.
    pub(crate) fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub(crate) fn start(&mut self) -> Vec<Exec> {
        let command = self.runner.start();
        self.save();
        command.map(|command| self.execs(command)).unwrap_or_default()
    }

    /// After a restart: the command the saved run was waiting on.
    pub(crate) fn resume(&mut self) -> Vec<Exec> {
        self.runner.resume().map(|command| self.execs(command)).unwrap_or_default()
    }

    pub(crate) fn event(&mut self, event: RunEvent) -> Vec<Exec> {
        let command = self.runner.handle(event);
        self.save();
        command.map(|command| self.execs(command)).unwrap_or_default()
    }

    /// A listener message of the current agent. Anything else is the previous stage's.
    pub(crate) fn message(&mut self, message: &TabMessage) -> Vec<Exec> {
        if !self.accepts(message.generation) {
            return Vec::new();
        }
        RunEvent::from_tab_event(&message.event).map(|event| self.event(event)).unwrap_or_default()
    }

    /// The agent could not be started, or its process failed while the stage ran. Ignored while a
    /// launch is on its way: the process that fails then is the previous stage's.
    pub(crate) fn agent_failed(&mut self, reason: String) -> Vec<Exec> {
        if self.launching || *self.runner.phase() != Phase::Working {
            return Vec::new();
        }
        self.event(RunEvent::AgentFailed { reason })
    }

    /// The agent of the launch could not be made ready.
    pub(crate) fn launch_failed(&mut self, reason: String) -> Vec<Exec> {
        self.launching = false;
        self.event(RunEvent::AgentFailed { reason })
    }

    /// A click on one of the ask card's buttons.
    pub(crate) fn choose(&mut self, choice: Choice) -> Vec<Exec> {
        if choice == Choice::Reject && !self.ui.rejecting {
            self.ui.rejecting = true;
            self.ui.focus_field = true;
            return Vec::new();
        }
        if choice == Choice::Retry && !self.ui.test_command.trim().is_empty() {
            self.runner.set_test_command(Some(self.ui.test_command.trim().to_owned()));
        }
        match workflow_ask::event_of(choice, &self.ui.feedback) {
            Some(event) => self.event(event),
            None => Vec::new(),
        }
    }

    /// Back out of writing a rejection.
    pub(crate) fn cancel_reject(&mut self) {
        self.ui.rejecting = false;
        self.ui.feedback.clear();
    }

    /// A click on a stage in the stepper opens its summary, or closes it when it was open.
    pub(crate) fn toggle_stage(&mut self, index: usize) {
        self.ui.open_stage = (self.ui.open_stage != Some(index)).then_some(index);
    }

    fn save(&mut self) {
        let Some(file) = &self.stages_file else { return };
        self.save_error = self.runner.save(file).err().map(|e| format!("Cannot save the run: {e}"));
    }

    fn execs(&mut self, command: Command) -> Vec<Exec> {
        match command {
            Command::StartStage { index, attempt, action, profile, prompt } => {
                self.generation += 1;
                self.launching = true;
                let divider = divider(index, action, &profile, attempt);
                vec![Exec::Launch(StageLaunch {
                    index,
                    attempt,
                    generation: self.generation,
                    profile,
                    prompt,
                    divider,
                })]
            }
            Command::RunTests { command } => {
                vec![Exec::RunTests { command, timeout: self.test_timeout }]
            }
            Command::AskUser { reason, .. } => {
                self.ui = RunUi {
                    open_stage: self.ui.open_stage,
                    focus_field: reason == AskReason::NoTestCommand,
                    ..RunUi::default()
                };
                Vec::new()
            }
            Command::Finished | Command::Stopped { .. } => {
                self.ui = RunUi { open_stage: self.ui.open_stage, ..RunUi::default() };
                Vec::new()
            }
        }
    }

    pub(crate) fn finished(&self) -> bool {
        *self.runner.phase() == Phase::Finished
    }

    pub(crate) fn position(&self) -> String {
        match self.runner.phase() {
            Phase::Finished => format!("{}: finished", self.runner.workflow_name()),
            Phase::Stopped(_) => format!("{}: stopped", self.runner.workflow_name()),
            _ => format!(
                "{}: {}",
                self.runner.workflow_name(),
                stepper::position_label(&self.runner)
            ),
        }
    }

    /// The question on the card, while the run waits for the user.
    pub(crate) fn ask(&self) -> Option<Ask> {
        let Phase::Asking(reason) = self.runner.phase() else { return None };
        let summary = self.runner.summary(self.runner.position());
        Some(workflow_ask::ask_of(reason, &stepper::position_label(&self.runner), summary))
    }

    /// Why the run stopped, once it did.
    pub(crate) fn stopped(&self) -> Option<String> {
        match self.runner.phase() {
            Phase::Stopped(reason) => Some(stop_text(&self.runner, *reason)),
            _ => None,
        }
    }

    /// The status the tab shows while the runner holds the next move; `None` while the agent of a
    /// stage works and its own status is the truth.
    pub(crate) fn hold(&self) -> Option<Hold> {
        let hold = |status, detail: &str| Some(Hold { status, detail: Some(detail.to_owned()) });
        if self.launching && *self.runner.phase() == Phase::Working {
            let stage = stepper::position_label(&self.runner);
            return hold(AgentStatus::SettingUp, &format!("Starting {stage}"));
        }
        match self.runner.phase() {
            Phase::Idle if self.awaiting_task() => hold(AgentStatus::Shell, "Waiting for a task"),
            Phase::Idle | Phase::Working => None,
            Phase::Testing => hold(AgentStatus::Working, "Running the tests"),
            Phase::Asking(reason) => Some(Hold {
                status: AgentStatus::NeedsYou(Reason::Input),
                detail: Some(
                    workflow_ask::ask_of(reason, &stepper::position_label(&self.runner), None)
                        .title,
                ),
            }),
            Phase::Finished => hold(AgentStatus::Done, "Workflow finished"),
            Phase::Stopped(reason) => Some(Hold {
                status: AgentStatus::Failed(stop_text(&self.runner, *reason)),
                detail: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests;
