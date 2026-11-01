//! Stand-in panel facts for captures: `TIDEWATCH_FAKE_PANEL=fresh|outdated|quiet|since|answers|lastturn` gives every
//! agent tab one fixed `PanelView` (nothing is read, logged or written), and
//! `TIDEWATCH_FAKE_ONTIME=<percent>` fills the About card's `Panel on time` row.

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::panel_event::{Source, StepBy};
use tidewatch_core::panel_state::MarkSeen;
use tidewatch_core::panel_view::{Age, Ages, PanelView, Quiet, ReportMark, Revs, Tone, Turn, Why};
use tidewatch_core::progress::State;
use tidewatch_core::status::AgentStatus;

const MINUTE_MS: i64 = 60_000;

/// The step `since` shows as ticked by Tidewatch; a ledger with a done step of this text shows it.
pub(crate) const TICKED_STEP: &str = "Run the tests";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Fresh,
    Outdated,
    Quiet,
    Since,
    Answers,
    LastTurn,
}

impl Fake {
    pub(crate) fn from_text(text: Option<&str>) -> Option<Self> {
        match text?.trim() {
            "fresh" => Some(Self::Fresh),
            "outdated" => Some(Self::Outdated),
            "quiet" => Some(Self::Quiet),
            "since" => Some(Self::Since),
            "answers" => Some(Self::Answers),
            "lastturn" => Some(Self::LastTurn),
            _ => None,
        }
    }

    pub(crate) fn from_env() -> Option<Self> {
        Self::from_text(std::env::var("TIDEWATCH_FAKE_PANEL").ok().as_deref())
    }

    pub(crate) fn view(self, now_ms: i64) -> PanelView {
        let ago = |minutes: i64| {
            let at_ms = now_ms - minutes * MINUTE_MS;
            Some(Age { at_ms })
        };
        let working = !matches!(self, Self::Outdated);
        let (reported, tone, status) = if working {
            (State::Working, Tone::Working, AgentStatus::Working)
        } else {
            (State::Done, Tone::Done, AgentStatus::Done)
        };
        let mark = match self {
            Self::Outdated => ReportMark::OutOfDate(Why::WorkingButTurnEnded),
            Self::LastTurn => ReportMark::OutOfDate(Why::DoneButTurnRunning),
            _ => ReportMark::Fresh,
        };
        let report_minutes = match self {
            Self::Fresh => 2,
            Self::Outdated => 14,
            Self::Quiet => 8,
            Self::Since | Self::Answers => 6,
            Self::LastTurn => 40,
        };
        let quiet = match self {
            Self::Quiet => vec![Quiet { source: Source::Hook, since_ms: now_ms - 11 * MINUTE_MS }],
            _ => Vec::new(),
        };
        PanelView {
            tone,
            status,
            reported: Some(reported),
            report: Some(mark),
            ages: Ages {
                report: ago(report_minutes),
                turn: if working { ago(0) } else { ago(11) },
                jobs: ago(1),
                links: ago(5),
                ..Ages::default()
            },
            quiet,
            since: if self == Self::Since { 3 } else { 0 },
            answers: if self == Self::Answers { 3 } else { 0 },
            turn: Turn {
                running: working,
                started_ms: Some(now_ms - 20 * MINUTE_MS),
                ended_ms: (!working).then_some(now_ms - 11 * MINUTE_MS),
            },
            asking: None,
            end_block: Some(EndBlock {
                done: vec!["Added the helper".to_owned()],
                needs_you: Vec::new(),
                next: vec!["Review the diff".to_owned()],
            }),
            revs: Revs::default(),
            shown_seq: 0,
        }
    }

    pub(crate) fn marks(self, now_ms: i64) -> Vec<MarkSeen> {
        match self {
            Self::Since | Self::Answers => vec![MarkSeen {
                step: TICKED_STEP.to_owned(),
                by: StepBy::Tidewatch,
                at_ms: now_ms - 3 * MINUTE_MS,
            }],
            _ => Vec::new(),
        }
    }
}

pub(crate) fn ontime_from_env() -> Option<f64> {
    std::env::var("TIDEWATCH_FAKE_ONTIME")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|p| p.is_finite())
}

#[cfg(test)]
mod tests;
