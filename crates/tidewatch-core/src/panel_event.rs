//! The numbered facts one tab's panel is built from (pure). `seq` is the only order; `at_ms` only
//! feeds ages and the on-time count, so a clock that jumps backward reorders nothing.

use serde::{Deserialize, Serialize};

use crate::job;
use crate::link_board::Link;
use crate::panel_rules::ReportDigest;
use crate::panel_state::PanelState;
use crate::status::{HookEvent, TerminalSignal};

/// The most a clipped text keeps, in bytes.
pub const CLIP_BYTES: usize = 2048;
const SUMMARY_BYTES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Hook,
    Transcript,
    Process,
    Terminal,
    Report,
    Job,
    Link,
    Git,
    Pr,
    Tidewatch,
    App,
}

impl Source {
    pub const ALL: [Self; 11] = [
        Self::Hook,
        Self::Transcript,
        Self::Process,
        Self::Terminal,
        Self::Report,
        Self::Job,
        Self::Link,
        Self::Git,
        Self::Pr,
        Self::Tidewatch,
        Self::App,
    ];

    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Transcript => "transcript",
            Self::Process => "process",
            Self::Terminal => "terminal",
            Self::Report => "report",
            Self::Job => "job",
            Self::Link => "link",
            Self::Git => "git",
            Self::Pr => "pr",
            Self::Tidewatch => "tidewatch",
            Self::App => "app",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    Question,
    Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepBy {
    Tidewatch,
    Agent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFacts {
    pub changed: u32,
    pub staged: u32,
    pub ahead: u32,
    pub behind: u32,
    pub head: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrWord {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksWord {
    Running,
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFacts {
    pub number: u64,
    pub state: PrWord,
    pub checks: Option<ChecksWord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fact {
    /// The agent process began; `hooked` says it reports by hooks.
    Started {
        hooked: bool,
    },
    /// The window restarted or the listener had a gap: what hooks last said no longer holds.
    Restart,
    Hook(HookEvent),
    /// A turn boundary read from the transcript; `ts_ms` is the time the CLI wrote there.
    Turn {
        started: bool,
        ts_ms: Option<i64>,
    },
    /// The agent's latest message, as the transcript has it (its tail).
    Said {
        tail: String,
    },
    ToolOpened {
        id: String,
        name: String,
    },
    ToolClosed {
        id: String,
    },
    AskOpened {
        summary: String,
        n: u32,
    },
    AskClosed,
    PermissionOpened {
        tool: String,
    },
    PermissionClosed,
    Process {
        alive: bool,
        exit_code: Option<i32>,
    },
    Terminal(TerminalSignal),
    /// The terminal printed. The app records at most one a second.
    Output,
    /// The agent saved a report; its body stays in `progress.json`.
    ReportSaved {
        saved_ms: i64,
        digest: ReportDigest,
    },
    Job(job::Report),
    Link(Link),
    Git(GitFacts),
    Pr(PrFacts),
    Check {
        name: String,
        passed: bool,
    },
    /// The owner answered what the panel showed as asked.
    Answered {
        kind: AskKind,
    },
    StepMark {
        step: String,
        by: StepBy,
    },
    /// The whole state up to this event's `seq`, written when the log is trimmed.
    Checkpoint(Box<PanelState>),
}

impl Fact {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Started { .. } => "started",
            Self::Restart => "restart",
            Self::Hook(_) => "hook",
            Self::Turn { started: true, .. } => "turn_started",
            Self::Turn { started: false, .. } => "turn_ended",
            Self::Said { .. } => "said",
            Self::ToolOpened { .. } => "tool_opened",
            Self::ToolClosed { .. } => "tool_closed",
            Self::AskOpened { .. } => "ask_opened",
            Self::AskClosed => "ask_closed",
            Self::PermissionOpened { .. } => "permission_opened",
            Self::PermissionClosed => "permission_closed",
            Self::Process { .. } => "process",
            Self::Terminal(_) => "terminal",
            Self::Output => "output",
            Self::ReportSaved { .. } => "report_saved",
            Self::Job(_) => "job",
            Self::Link(_) => "link",
            Self::Git(_) => "git",
            Self::Pr(_) => "pr",
            Self::Check { .. } => "check",
            Self::Answered { .. } => "answered",
            Self::StepMark { .. } => "step_mark",
            Self::Checkpoint(_) => "checkpoint",
        }
    }

    /// The fact with its long texts cut to what the log keeps: the tail of a message (the end block
    /// sits there), the head of a summary.
    #[must_use]
    pub fn clipped(self) -> Self {
        match self {
            Self::Hook(mut hook) => {
                hook.message = hook.message.map(|message| clip_tail(&message, CLIP_BYTES));
                Self::Hook(hook)
            }
            Self::Said { tail } => Self::Said { tail: clip_tail(&tail, CLIP_BYTES) },
            Self::AskOpened { summary, n } => {
                Self::AskOpened { summary: clip_head(&summary, SUMMARY_BYTES), n }
            }
            Self::PermissionOpened { tool } => {
                Self::PermissionOpened { tool: clip_head(&tool, SUMMARY_BYTES) }
            }
            Self::ToolOpened { id, name } => Self::ToolOpened {
                id: clip_head(&id, SUMMARY_BYTES),
                name: clip_head(&name, SUMMARY_BYTES),
            },
            Self::ToolClosed { id } => Self::ToolClosed { id: clip_head(&id, SUMMARY_BYTES) },
            other => other,
        }
    }
}

/// One numbered fact of one tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at_ms: i64,
    pub source: Source,
    pub fact: Fact,
}

fn clip_tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut start = text.len() - max.saturating_sub(3);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("...{}", &text[start..])
}

fn clip_head(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max.saturating_sub(3);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::HookKind;

    fn hook_with(message: &str) -> Fact {
        Fact::Hook(HookEvent {
            kind: HookKind::TurnDone,
            session_id: None,
            transcript_path: None,
            message: Some(message.to_owned()),
        })
    }

    fn message_of(fact: &Fact) -> String {
        match fact {
            Fact::Hook(hook) => hook.message.clone().unwrap_or_default(),
            other => panic!("not a hook: {other:?}"),
        }
    }

    #[test]
    fn a_long_hook_message_keeps_its_tail_within_two_kilobytes() {
        let long = format!("{}Done\n- the end block", "x".repeat(5000));
        let clipped = message_of(&hook_with(&long).clipped());
        assert_eq!(clipped.len(), CLIP_BYTES);
        assert!(clipped.starts_with("..."));
        assert!(clipped.ends_with("Done\n- the end block"));
    }

    #[test]
    fn clipping_never_splits_a_character_and_leaves_short_text_alone() {
        assert_eq!(message_of(&hook_with("short").clipped()), "short");
        let wide = "\u{e9}".repeat(3000);
        let clipped = message_of(&hook_with(&wide).clipped());
        assert!(clipped.len() <= CLIP_BYTES);
        assert!(clipped.starts_with("..."));
        assert!(clipped[3..].chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn a_long_summary_keeps_its_head() {
        let fact = Fact::AskOpened { summary: format!("Which one?{}", "y".repeat(900)), n: 1 };
        match fact.clipped() {
            Fact::AskOpened { summary, n } => {
                assert_eq!(n, 1);
                assert_eq!(summary.len(), SUMMARY_BYTES);
                assert!(summary.starts_with("Which one?"));
                assert!(summary.ends_with("..."));
            }
            other => panic!("changed kind: {other:?}"),
        }
    }

    #[test]
    fn every_source_has_one_word_and_the_list_names_each_once() {
        let words: Vec<&str> = Source::ALL.iter().map(|source| source.word()).collect();
        let mut unique = words.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), words.len());
        assert_eq!(words.len(), 11);
    }

    #[test]
    fn an_event_survives_a_round_trip_through_a_json_line() {
        let event = Event {
            seq: 7,
            at_ms: 1_793_017_688_981,
            source: Source::Transcript,
            fact: Fact::Turn { started: false, ts_ms: Some(1_793_017_688_000) },
        };
        let line = serde_json::to_string(&event).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
    }
}
