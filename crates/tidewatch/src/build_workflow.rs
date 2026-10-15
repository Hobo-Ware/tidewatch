//! "Build workflow from this session" on the app's side, apart from drawing: when the command may
//! run on a tab, the wait for the agent's answer, and the fallback that reads a `toml` block out of
//! the Reader's rows when the agent answered in text instead of calling `propose_workflow`.
//! Pure data: the app pastes the prompt and opens the draft.

use tidewatch_core::propose;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::workflows::Workflow;

use crate::agent_status_line::is_ready;
use crate::pane::{Body, Pane};
use crate::reader_rows::{Kind, Row};
use crate::status_view;

/// How long the agent has to start working on the prompt before the wait is dropped.
const START_PATIENCE_S: f64 = 15.0;
/// Seconds after the turn ended at which the Reader's rows are read: its file lags the hook.
const SCAN_AT_S: [f64; 3] = [1.5, 4.0, 8.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blocked {
    NoAgent,
    InWorkflow,
    NotRunning,
    Starting,
    Working,
    WaitingOnYou,
    Asked,
}

impl Blocked {
    /// Short enough for the palette row; it is also its tooltip.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::NoAgent => "needs an agent tab",
            Self::InWorkflow => "not in a workflow run",
            Self::NotRunning => "agent is not running",
            Self::Starting => "agent is starting",
            Self::Working => "agent is working",
            Self::WaitingOnYou => "agent is waiting on you",
            Self::Asked => "already asked",
        }
    }
}

/// Whether the command may run on `pane`: an agent tab that is not a workflow's, whose agent is
/// idle (Done, or Ready for its first message), with no request already out.
pub(crate) fn availability<T>(pane: &Pane<T>, asked: bool) -> Result<(), Blocked> {
    let Some(agent) = &pane.agent else { return Err(Blocked::NoAgent) };
    if agent.workflow.is_some() {
        return Err(Blocked::InWorkflow);
    }
    if !matches!(pane.body, Body::Shell(_)) {
        return Err(Blocked::NotRunning);
    }
    if asked {
        return Err(Blocked::Asked);
    }
    let status = agent.run.status();
    if is_ready(agent.first_prompt, status_view::tone(status)) {
        return Ok(());
    }
    match status {
        AgentStatus::Done => Ok(()),
        AgentStatus::Working => Err(Blocked::Working),
        AgentStatus::NeedsYou(_) => Err(Blocked::WaitingOnYou),
        AgentStatus::SettingUp => Err(Blocked::Starting),
        AgentStatus::Failed(_) | AgentStatus::Shell => Err(Blocked::NotRunning),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Starting,
    Working,
    Settling { since: f64, scans: usize },
}

/// What the app does next for an agent it asked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    /// Nothing yet; look again at this time, or on the next event when `None`.
    Wait(Option<f64>),
    /// Read the Reader's rows now and report back with [`Asked::scanned_empty`] when nothing is there.
    Scan,
    Dropped(Dropped),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dropped {
    NeverStarted,
    Failed,
    NoWorkflow,
}

impl Dropped {
    pub(crate) fn text(self, tab: &str) -> String {
        match self {
            Self::NeverStarted => format!("{tab} did not start on the request"),
            Self::Failed => format!("{tab} stopped before it sent a workflow"),
            Self::NoWorkflow => format!("{tab} finished without sending a workflow"),
        }
    }
}

/// One request that is out: the prompt went into a tab, and its answer is awaited.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asked {
    pub(crate) tab: u64,
    /// The newest Reader row when the prompt was sent: only later rows count as the answer.
    mark: Option<u64>,
    sent_at: f64,
    phase: Phase,
}

impl Asked {
    pub(crate) fn new(tab: u64, mark: Option<u64>, now: f64) -> Self {
        Self { tab, mark, sent_at: now, phase: Phase::Starting }
    }

    pub(crate) fn mark(&self) -> Option<u64> {
        self.mark
    }

    /// Call every frame with the tab's status. The turn the prompt started is over once the agent
    /// was seen working and is Done again.
    pub(crate) fn step(&mut self, status: &AgentStatus, now: f64) -> Step {
        if matches!(status, AgentStatus::Failed(_) | AgentStatus::Shell) {
            return Step::Dropped(Dropped::Failed);
        }
        let busy = matches!(status, AgentStatus::Working | AgentStatus::NeedsYou(_));
        match self.phase {
            Phase::Starting | Phase::Settling { .. } if busy => {
                self.phase = Phase::Working;
                Step::Wait(None)
            }
            Phase::Starting if now - self.sent_at >= START_PATIENCE_S => {
                Step::Dropped(Dropped::NeverStarted)
            }
            Phase::Starting => Step::Wait(Some(self.sent_at + START_PATIENCE_S)),
            Phase::Working if *status == AgentStatus::Done => {
                self.phase = Phase::Settling { since: now, scans: 0 };
                Step::Wait(Some(now + SCAN_AT_S[0]))
            }
            Phase::Working => Step::Wait(None),
            Phase::Settling { since, scans } => match SCAN_AT_S.get(scans) {
                Some(after) if now >= since + after => Step::Scan,
                Some(after) => Step::Wait(Some(since + after)),
                None => Step::Dropped(Dropped::NoWorkflow),
            },
        }
    }

    /// The rows held no workflow yet: the next look is a little later, and the last one drops the wait.
    pub(crate) fn scanned_empty(&mut self) -> Step {
        let Phase::Settling { since, scans } = &mut self.phase else { return Step::Wait(None) };
        *scans += 1;
        match SCAN_AT_S.get(*scans) {
            Some(after) => Step::Wait(Some(*since + after)),
            None => Step::Dropped(Dropped::NoWorkflow),
        }
    }
}

/// The last `toml` block of the agent's messages that came after the prompt.
pub(crate) fn toml_after<'a>(
    rows: impl Iterator<Item = &'a Row>,
    mark: Option<u64>,
) -> Option<String> {
    rows.filter(|row| row.kind == Kind::Assistant && mark.is_none_or(|mark| row.seq > mark))
        .filter_map(|row| propose::last_toml_block(&row.text))
        .last()
}

/// A workflow out of a `toml` block, with the stages' agents filled in and the editor's rules
/// applied, or the first thing wrong with it.
pub(crate) fn workflow_from_toml(text: &str, agent: &str) -> Result<Workflow, String> {
    let mut workflow = propose::parse_toml(text).map_err(first)?;
    propose::with_agent(&mut workflow, agent);
    let found = propose::problems(&workflow);
    if found.is_empty() { Ok(workflow) } else { Err(first(found)) }
}

/// The first problem of a proposal, for a toast.
pub(crate) fn first(problems: Vec<String>) -> String {
    problems.into_iter().next().unwrap_or_else(|| "the workflow is not valid".to_owned())
}

#[cfg(test)]
mod tests;
