//! What the comment guard says on an agent tab, as plain data: the chip text, the lines it lists,
//! the note pasted to a CLI with no hook of its own, and when a turn has just ended (pure).

use tidewatch_core::comment_flags::Record;
use tidewatch_core::comment_sweep::FileFlags;
use tidewatch_core::comments::{ADVICE, Kind};
use tidewatch_core::status::AgentStatus;

const NOTE_LINES: usize = 8;

/// `Comment guard: 3 flags`, or `Comment guard: off for this tab`.
pub(crate) fn chip_text(flags: usize, off: bool) -> String {
    if off {
        return "Comment guard: off for this tab".to_owned();
    }
    let noun = if flags == 1 { "flag" } else { "flags" };
    format!("Comment guard: {flags} {noun}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) place: String,
    pub(crate) kind: String,
    pub(crate) text: String,
}

/// The newest `limit` records as lines, newest first. A flag on the whole file has no line number.
pub(crate) fn lines(records: &[Record], limit: usize) -> Vec<Line> {
    records
        .iter()
        .rev()
        .take(limit)
        .map(|record| Line {
            place: if record.line == 0 {
                record.file.clone()
            } else {
                format!("{}:{}", record.file, record.line)
            },
            kind: record.kind.clone(),
            text: record.text.clone(),
        })
        .collect()
}

/// The short note pasted to an agent that has no hook for it, after its turn.
pub(crate) fn paste_note(files: &[FileFlags]) -> String {
    let total: usize = files.iter().map(|file| file.flags.len()).sum();
    let mut lines = Vec::new();
    for file in files {
        for flag in &file.flags {
            let place = if flag.line == 0 {
                file.path.clone()
            } else {
                format!("{}:{}", file.path, flag.line)
            };
            lines.push(
                format!("- {place} [{}] {}", flag.kind.tag(), flag.text).trim_end().to_owned(),
            );
        }
    }
    let more = lines.len().saturating_sub(NOTE_LINES);
    lines.truncate(NOTE_LINES);
    if more > 0 {
        lines.push(format!("- and {more} more"));
    }
    format!(
        "Comment guard: {total} comment(s) you added fail the keep-list.\n{}\n{ADVICE}",
        lines.join("\n")
    )
}

/// Tracks whether the agent was working, to see a turn end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Turn {
    working: bool,
}

impl Turn {
    /// Sees the status of this frame. True once, when a working agent becomes idle (`Done`).
    pub(crate) fn ended(&mut self, status: &AgentStatus) -> bool {
        let was = self.working;
        self.working = matches!(status, AgentStatus::Working);
        was && matches!(status, AgentStatus::Done)
    }
}

/// Claude Code runs the hooks itself; the other CLIs are swept when their turn ends.
pub(crate) fn needs_idle_sweep(agent_id: &str) -> bool {
    agent_id != "claude"
}

pub(crate) fn kind_tone(kind: &str) -> Tone {
    if kind == Kind::DeadCode.tag() || kind == Kind::Alibi.tag() || kind == Kind::Suppression.tag()
    {
        Tone::Loud
    } else {
        Tone::Quiet
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Loud,
    Quiet,
}

#[cfg(test)]
mod tests;
