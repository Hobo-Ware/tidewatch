//! Tidewatch tool names that no longer exist, found in the owner's `house-style.md`.

use std::path::Path;

use crate::fsutil::write_atomic;

/// A tool Tidewatch used to offer, from `git log -S` on the MCP tool names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Removed {
    pub name: &'static str,
    pub since: &'static str,
}

pub const REMOVED: [Removed; 2] = [
    Removed { name: "stage_done", since: "2026-10-27" },
    Removed { name: "propose_workflow", since: "2026-10-27" },
];

const CLIENT_PREFIX: &str = "mcp__tidewatch__";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadLine {
    /// 1-based, as an editor counts.
    pub number: usize,
    pub text: String,
    pub tool: Removed,
}

#[must_use]
pub fn scan(text: &str) -> Vec<DeadLine> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let tool = REMOVED.iter().copied().find(|tool| names(line, tool.name))?;
            Some(DeadLine { number: index + 1, text: line.to_owned(), tool })
        })
        .collect()
}

/// `text` without line `number`, and only when that line still reads `expect`: the file may have
/// been edited since it was scanned.
#[must_use]
pub fn remove_line(text: &str, number: usize, expect: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let at = number.checked_sub(1)?;
    if lines.get(at)?.trim_end_matches(['\n', '\r']) != expect {
        return None;
    }
    Some(lines.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, line)| *line).collect())
}

pub fn remove_from_file(file: &Path, number: usize, expect: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(file)?;
    let Some(changed) = remove_line(&text, number, expect) else { return Ok(false) };
    write_atomic(file, &changed)?;
    Ok(true)
}

fn names(line: &str, tool: &str) -> bool {
    let mut from = 0;
    while let Some(found) = line[from..].find(tool) {
        let at = from + found;
        let before = &line[..at];
        let after = line[at + tool.len()..].chars().next();
        let word_before = before.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        let clean_start = !word_before || before.ends_with(CLIENT_PREFIX);
        if clean_start && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            return true;
        }
        from = at + tool.len();
    }
    false
}

#[cfg(test)]
mod tests;
