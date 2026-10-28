//! What the comment guard says on an agent tab, as plain data: the chip text, the lines it lists
//! with each file shown relative to the tab's folders, the note pasted to a CLI with no hook of its
//! own, and when a turn has just ended (pure).

use std::path::{Path, PathBuf};

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

/// The folders a flagged file is shown relative to: the tab's worktree, then its repo root, then
/// the home folder as `~`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PathRoots {
    bases: Vec<PathBuf>,
    home: Option<PathBuf>,
}

impl PathRoots {
    pub(crate) fn new(tree: &Path, repo: &Path, home: Option<&Path>) -> Self {
        let mut bases = vec![tree.to_path_buf()];
        if repo != tree {
            bases.push(repo.to_path_buf());
        }
        Self { bases, home: home.map(Path::to_path_buf) }
    }
}

/// `file` relative to the first root that holds it, `~/...` under the home folder, else as given.
/// A path that is already relative stays as it is.
pub(crate) fn display_path(file: &str, roots: &PathRoots) -> String {
    let path = Path::new(file);
    if !path.is_absolute() {
        return file.to_owned();
    }
    for base in &roots.bases {
        if let Some(rest) = relative_under(path, base) {
            return rest;
        }
    }
    match roots.home.as_deref().and_then(|home| relative_under(path, home)) {
        Some(rest) => format!("~/{rest}"),
        None => file.to_owned(),
    }
}

fn relative_under(path: &Path, base: &Path) -> Option<String> {
    let rest = path.strip_prefix(base).ok()?;
    let text = rest.to_string_lossy();
    (!text.is_empty()).then(|| text.into_owned())
}

/// The newest `limit` records as lines, newest first. A flag on the whole file has no line number.
pub(crate) fn lines(records: &[Record], limit: usize, roots: &PathRoots) -> Vec<Line> {
    records
        .iter()
        .rev()
        .take(limit)
        .map(|record| {
            let file = display_path(&record.file, roots);
            Line {
                place: if record.line == 0 { file } else { format!("{file}:{}", record.line) },
                kind: record.kind.clone(),
                text: record.text.clone(),
            }
        })
        .collect()
}

/// A rule's tag as the words the strip shows: `dead-code` reads `commented-out code`.
pub(crate) fn kind_label(kind: &str) -> String {
    if kind == Kind::DeadCode.tag() {
        "commented-out code".to_owned()
    } else {
        kind.replace('-', " ")
    }
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
