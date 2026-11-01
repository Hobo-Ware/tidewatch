//! What the panel draws at one instant: the tone, how far the report can be trusted, the age of
//! each block and the sources that went quiet (plain data, built by `PanelState::view`).

use serde::{Deserialize, Serialize};

use crate::end_block::EndBlock;
use crate::panel_event::{AskKind, Source};
use crate::progress::State;
use crate::status::{AgentStatus, Reason};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Setup,
    Working,
    Needs,
    Waiting,
    Blocked,
    Done,
    Failed,
    Stopped,
    Shell,
}

impl Tone {
    /// A quiet terminal asked for nothing, so it reads as `Done`, never as `Needs`.
    #[must_use]
    pub fn of(status: &AgentStatus) -> Self {
        match status {
            AgentStatus::SettingUp => Self::Setup,
            AgentStatus::Working => Self::Working,
            AgentStatus::NeedsYou(Reason::Quiet) | AgentStatus::Done => Self::Done,
            AgentStatus::NeedsYou(_) => Self::Needs,
            AgentStatus::Failed(_) => Self::Failed,
            AgentStatus::Stopped => Self::Stopped,
            AgentStatus::Shell => Self::Shell,
        }
    }

    /// The tone a tab shows: what the report stands for while the agent works or is idle, and the
    /// facts Tidewatch owns when they say more (setting up, a prompt, failed, stopped).
    #[must_use]
    pub fn shown(status: &AgentStatus, reported: Option<State>) -> Self {
        let owned = Self::of(status);
        match (owned, reported) {
            (Self::Working | Self::Done, Some(state)) => match state {
                State::Working => Self::Working,
                State::NeedsYou => Self::Needs,
                State::Waiting => Self::Waiting,
                State::Blocked => Self::Blocked,
                State::Done => Self::Done,
            },
            _ => owned,
        }
    }
}

/// Why a report no longer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Why {
    /// It says the owner is needed and nothing asks.
    NeedsYouNoQuestion,
    /// It says work goes on and the turn ended after it.
    WorkingButTurnEnded,
    /// It waits on something and names nothing, and the turn ended after it.
    WaitingOnNothing,
    /// It says done and a later turn runs.
    DoneButTurnRunning,
    /// The owner answered its question.
    QuestionAnswered,
    /// A turn started after it was saved.
    OlderThanTurn,
}

impl Why {
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::NeedsYouNoQuestion => "says it needs you, nothing asks",
            Self::WorkingButTurnEnded => "says it works, the turn ended",
            Self::WaitingOnNothing => "waits on nothing, the turn ended",
            Self::DoneButTurnRunning => "says done, a turn runs",
            Self::QuestionAnswered => "its question was answered",
            Self::OlderThanTurn => "older than the turn that runs",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportMark {
    Fresh,
    OutOfDate(Why),
}

/// When a block was last updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Age {
    pub at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ages {
    pub report: Option<Age>,
    pub turn: Option<Age>,
    pub jobs: Option<Age>,
    pub links: Option<Age>,
    pub git: Option<Age>,
    pub pr: Option<Age>,
    pub checks: Option<Age>,
}

/// A source that stayed quiet past its limit; `since_ms` is the last word it gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quiet {
    pub source: Source,
    pub since_ms: i64,
}

/// A counter per block that moves whenever the block's content does, so a screen can tell
/// whether to redraw it without comparing the content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revs {
    pub report: u32,
    pub jobs: u32,
    pub links: u32,
    pub git: u32,
    pub pr: u32,
    pub checks: u32,
    pub marks: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub running: bool,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
}

/// A question or a permission prompt open right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asking {
    pub kind: AskKind,
    pub since_ms: i64,
    pub summary: String,
}

impl Asking {
    #[must_use]
    pub fn reason(&self) -> Reason {
        match self.kind {
            AskKind::Question => Reason::Question,
            AskKind::Permission => Reason::Permission,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelView {
    pub tone: Tone,
    pub status: AgentStatus,
    /// The state the report stands for after the facts weighed it; `None` without a report.
    pub reported: Option<State>,
    pub report: Option<ReportMark>,
    pub ages: Ages,
    pub quiet: Vec<Quiet>,
    /// Things that happened since the report was saved: tool calls, jobs, commits, pull request
    /// changes, checks.
    pub since: u32,
    /// Questions the owner answered since the report was saved.
    pub answers: u32,
    pub turn: Turn,
    pub asking: Option<Asking>,
    pub end_block: Option<EndBlock>,
    pub revs: Revs,
    /// The last event folded.
    pub shown_seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_terminal_reads_done_and_a_prompt_reads_needs() {
        let cases = [
            (AgentStatus::SettingUp, Tone::Setup),
            (AgentStatus::Working, Tone::Working),
            (AgentStatus::Done, Tone::Done),
            (AgentStatus::NeedsYou(Reason::Quiet), Tone::Done),
            (AgentStatus::NeedsYou(Reason::Permission), Tone::Needs),
            (AgentStatus::NeedsYou(Reason::Question), Tone::Needs),
            (AgentStatus::Failed("x".into()), Tone::Failed),
            (AgentStatus::Stopped, Tone::Stopped),
            (AgentStatus::Shell, Tone::Shell),
        ];
        for (status, want) in cases {
            assert_eq!(Tone::of(&status), want, "{status:?}");
        }
    }

    #[test]
    fn the_report_speaks_for_a_working_or_idle_agent_and_facts_speak_for_the_rest() {
        let cases = [
            (AgentStatus::Working, Some(State::Done), Tone::Done),
            (AgentStatus::Done, Some(State::Working), Tone::Working),
            (AgentStatus::Done, Some(State::Waiting), Tone::Waiting),
            (AgentStatus::NeedsYou(Reason::Quiet), Some(State::Blocked), Tone::Blocked),
            (AgentStatus::Working, None, Tone::Working),
            (AgentStatus::NeedsYou(Reason::Permission), Some(State::Done), Tone::Needs),
            (AgentStatus::Failed("x".into()), Some(State::Working), Tone::Failed),
            (AgentStatus::Stopped, Some(State::Working), Tone::Stopped),
            (AgentStatus::SettingUp, Some(State::Done), Tone::Setup),
        ];
        for (status, reported, want) in cases {
            assert_eq!(Tone::shown(&status, reported), want, "{status:?} {reported:?}");
        }
    }

    #[test]
    fn a_question_reads_as_the_reason_its_kind_names() {
        let ask = |kind| Asking { kind, since_ms: 0, summary: String::new() };
        assert_eq!(ask(AskKind::Question).reason(), Reason::Question);
        assert_eq!(ask(AskKind::Permission).reason(), Reason::Permission);
    }
}
