//! Where a turn begins and ends in each CLI's transcript (pure). The parser keeps only chat and
//! tool entries, so these marks are what lets Tidewatch tell a finished turn from a hook it never
//! received.
//!
//! What marks a turn, per CLI (checked on the redacted fixtures and the installed versions):
//! - Claude Code 2.1.316: a real user prompt starts it; an assistant record with
//!   `stop_reason: "end_turn"` or a `system` record of subtype `stop_hook_summary` ends it.
//! - Codex 0.162.1: `event_msg` records `task_started` and `task_complete` (or `turn_aborted`, a
//!   name the binary carries but no fixture holds).
//! - Copilot 1.0.103: `assistant.turn_start` opens the first round of an interaction and
//!   `assistant.turn_end` closes the last one. A round whose message asked for tools is followed
//!   by another `turn_start` in the same millisecond, so it is no end.
//! - Gemini 0.46.0: no turn records. A user message starts a turn and an assistant message with
//!   text and no tool calls ends it, which is the weakest of the four.

use serde_json::Value;

use crate::transcript::{Entry, Role, Source};

/// A turn boundary read from one transcript line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnMark {
    pub started: bool,
    /// Unix milliseconds the CLI wrote on the line.
    pub ts: Option<i64>,
    /// How many entries the parser had emitted before this mark.
    pub before: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Marker {
    marks: Vec<TurnMark>,
    last: Option<bool>,
    interaction: Option<String>,
    round_asked_tools: bool,
}

impl Marker {
    pub(crate) fn take(&mut self) -> Vec<TurnMark> {
        std::mem::take(&mut self.marks)
    }

    /// Marks for one line. `seen` is the entry count before the line, `entries` what it produced.
    pub(crate) fn line(
        &mut self,
        source: Source,
        line: &Value,
        entries: &[Entry],
        ts: Option<i64>,
        seen: usize,
    ) {
        let after = seen + entries.len();
        let wrote = |role: Role| entries.iter().any(|entry| entry.role == role);
        match source {
            Source::Claude => self.claude(line, wrote(Role::User), ts, seen, after),
            Source::Gemini => {
                self.gemini(line, wrote(Role::User), wrote(Role::Assistant), ts, seen, after);
            }
            Source::Copilot => self.copilot(line, ts, seen, after),
            Source::Codex => self.codex(line, ts, seen, after),
        }
    }

    /// A start after a start is dropped; an end after an end moves the earlier one to the later
    /// line, so a turn that writes several closing records ends after the last of them.
    fn note(&mut self, started: bool, ts: Option<i64>, before: usize) {
        if self.last == Some(started) {
            if !started && let Some(end) = self.marks.last_mut() {
                end.ts = ts.or(end.ts);
                end.before = before;
            }
            return;
        }
        self.last = Some(started);
        self.marks.push(TurnMark { started, ts, before });
    }

    fn claude(&mut self, line: &Value, user: bool, ts: Option<i64>, seen: usize, after: usize) {
        if line["isSidechain"].as_bool() == Some(true) || line["isMeta"].as_bool() == Some(true) {
            return;
        }
        match line["type"].as_str() {
            Some("user") if user => self.note(true, ts, seen),
            Some("assistant") if line["message"]["stop_reason"].as_str() == Some("end_turn") => {
                self.note(false, ts, after);
            }
            Some("system") if line["subtype"].as_str() == Some("stop_hook_summary") => {
                self.note(false, ts, after);
            }
            _ => {}
        }
    }

    fn gemini(
        &mut self,
        line: &Value,
        user: bool,
        assistant: bool,
        ts: Option<i64>,
        seen: usize,
        after: usize,
    ) {
        let calls = line["toolCalls"].as_array().is_some_and(|calls| !calls.is_empty());
        match line["type"].as_str() {
            Some("user") if user => self.note(true, ts, seen),
            Some("gemini") if assistant && !calls => self.note(false, ts, after),
            _ => {}
        }
    }

    fn copilot(&mut self, line: &Value, ts: Option<i64>, seen: usize, after: usize) {
        let data = &line["data"];
        match line["type"].as_str() {
            Some("assistant.turn_start") => {
                let interaction = data["interactionId"].as_str().map(str::to_owned);
                if interaction.is_none() || interaction != self.interaction {
                    self.note(true, ts, seen);
                }
                self.interaction = interaction;
                self.round_asked_tools = false;
            }
            Some("assistant.message") => {
                self.round_asked_tools =
                    data["toolRequests"].as_array().is_some_and(|tools| !tools.is_empty());
            }
            Some("assistant.turn_end") if !self.round_asked_tools => self.note(false, ts, after),
            Some("abort") => {
                self.round_asked_tools = false;
                self.note(false, ts, after);
            }
            _ => {}
        }
    }

    fn codex(&mut self, line: &Value, ts: Option<i64>, seen: usize, after: usize) {
        if line["type"].as_str() != Some("event_msg") {
            return;
        }
        match line["payload"]["type"].as_str() {
            Some("task_started") => self.note(true, ts, seen),
            Some("task_complete" | "turn_aborted") => self.note(false, ts, after),
            _ => {}
        }
    }
}
