//! The comment guard's hooks: `tidewatch --guard comments` after a Write, Edit or MultiEdit
//! (PostToolUse), and `tidewatch --guard comments --stop` at the end of a turn (Stop), both for
//! Claude Code through the tab's `--settings`.
//!
//! The hook reads its JSON payload on stdin. A flag is reported the way Claude Code reads a hook:
//! exit 2 with the message on stderr (checked against the hooks reference of Claude Code 2.1.309: for
//! PostToolUse stderr is shown to Claude although the tool already ran, for Stop it keeps Claude
//! working). Anything else, including a payload that does not parse, is exit 0 and silence.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::comment_flags::{Origin, Record, TabFiles};
use crate::comment_sweep::{self, FileFlags};
use crate::comments::{ADVICE, Flag, Syntax, render, scan};
use crate::guard_run::GuardEnv;
use crate::guards::Rule;
use crate::prefs::Prefs;

const EDIT_LINES: usize = 15;
const SWEEP_FILES_LINES: usize = 6;
const SWEEP_LINES: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCall {
    pub tool_name: String,
    pub tool_input: Value,
    pub cwd: Option<PathBuf>,
    pub stop_hook_active: bool,
}

#[must_use]
pub fn parse_hook(stdin: &str) -> Option<HookCall> {
    let value: Value = serde_json::from_str(stdin).ok()?;
    Some(HookCall {
        tool_name: value.get("tool_name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        tool_input: value.get("tool_input").cloned().unwrap_or(Value::Null),
        cwd: value.get("cwd").and_then(Value::as_str).map(PathBuf::from),
        stop_hook_active: value.get("stop_hook_active").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The text a Write, Edit or MultiEdit put into the file at `path`, each piece with the file line it
/// starts on. `read` gives the file as it is now (the tool already ran), to place an edit; an edit
/// that is not found there starts at line 1.
#[must_use]
pub fn written_pieces(
    call: &HookCall,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<(String, Vec<(usize, String)>)> {
    let input = &call.tool_input;
    let path = input.get("file_path").and_then(Value::as_str)?.to_owned();
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
    let news: Vec<String> = match call.tool_name.as_str() {
        "Write" => return Some((path, vec![(1, text("content")?)])),
        "Edit" => vec![text("new_string")?],
        "MultiEdit" => input
            .get("edits")?
            .as_array()?
            .iter()
            .filter_map(|edit| edit.get("new_string").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        _ => return None,
    };
    let now = read(Path::new(&path)).unwrap_or_default();
    let pieces = news
        .into_iter()
        .map(|new| {
            let line = now.find(&new).map_or(1, |at| now[..at].matches('\n').count() + 1);
            (line, new)
        })
        .collect();
    Some((path, pieces))
}

/// The flags in what a tool call wrote, `None` when it was not a write the guard looks at.
#[must_use]
pub fn flags_of_write(
    call: &HookCall,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<(String, Vec<Flag>)> {
    let (path, pieces) = written_pieces(call, read)?;
    let syntax = Syntax::of_path(&path)?;
    let flags = pieces
        .iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .flat_map(|(line, text)| scan(text, syntax, *line))
        .collect();
    Some((path, flags))
}

fn base_name(path: &str) -> &str {
    Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path)
}

/// The message for flags found in one write.
#[must_use]
pub fn edit_message(path: &str, flags: &[Flag]) -> String {
    let lines = render(flags);
    let more = if lines.len() > EDIT_LINES { "\n  ... and more" } else { "" };
    let shown: Vec<String> =
        lines.iter().take(EDIT_LINES).map(|line| format!("  {line}")).collect();
    format!(
        "COMMENT GUARD - {}: {} suspect comment(s) in what you just wrote.\n{}{more}\n\n{ADVICE}",
        base_name(path),
        flags.len(),
        shown.join("\n"),
    )
}

/// The message for the flags a branch sweep found.
#[must_use]
pub fn sweep_message(files: &[FileFlags]) -> String {
    let mut lines = Vec::new();
    for file in files {
        lines.push(format!("{}: {} suspect", file.path, file.flags.len()));
        lines.extend(
            render(&file.flags).into_iter().take(SWEEP_FILES_LINES).map(|line| format!("  {line}")),
        );
    }
    let more = if lines.len() > SWEEP_LINES { "\n  ... and more" } else { "" };
    lines.truncate(SWEEP_LINES);
    format!(
        "COMMENT GUARD (branch sweep) - noisy comments still in the diff:\n{}{more}\n\n{ADVICE}",
        lines.join("\n"),
    )
}

#[must_use]
pub fn records(origin: Origin, file: &str, flags: &[Flag], at_ms: u64) -> Vec<Record> {
    flags
        .iter()
        .map(|flag| Record {
            at_ms,
            origin,
            file: file.to_owned(),
            line: flag.line,
            kind: flag.kind.tag().to_owned(),
            text: flag.text.clone(),
        })
        .collect()
}

/// A short stable digest of a report, to tell a repeat from a new one.
#[must_use]
pub fn digest(report: &str) -> String {
    let hash = Sha256::digest(report.as_bytes());
    hash.iter().take(6).fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

fn at_ms() -> u64 {
    u64::try_from(crate::clock::now_ms()).unwrap_or(0)
}

/// Runs one hook. Returns the exit code; the report, if any, is printed to stderr.
#[must_use]
pub fn run(stop: bool, stdin: &str, env: &GuardEnv) -> i32 {
    let rules = Prefs::load_file(&env.config).guards.rules_for(&env.repo);
    let files = env.tab.as_deref().map(TabFiles::new);
    if !rules.is_on(Rule::Comments) || files.as_ref().is_some_and(TabFiles::is_off) {
        return 0;
    }
    let Some(call) = parse_hook(stdin) else { return 0 };
    let message = if stop {
        stop_report(&call, files.as_ref(), env)
    } else {
        edit_report(&call, files.as_ref())
    };
    match message {
        Some(text) => {
            eprintln!("{text}");
            2
        }
        None => 0,
    }
}

fn edit_report(call: &HookCall, files: Option<&TabFiles>) -> Option<String> {
    let (path, flags) = flags_of_write(call, |path| std::fs::read_to_string(path).ok())?;
    if flags.is_empty() {
        return None;
    }
    if let Some(files) = files {
        let _ = files.append(&records(Origin::Edit, &path, &flags, at_ms()));
    }
    Some(edit_message(&path, &flags))
}

fn stop_report(call: &HookCall, files: Option<&TabFiles>, env: &GuardEnv) -> Option<String> {
    if call.stop_hook_active {
        return None;
    }
    let dir = call.cwd.clone().unwrap_or_else(|| env.repo.clone());
    let swept = comment_sweep::sweep(&dir)?;
    let report = sweep_message(&swept);
    let digest = digest(&report);
    if swept.is_empty() {
        if let Some(files) = files {
            let _ = files.set_swept(None);
        }
        return None;
    }
    if let Some(files) = files {
        if files.swept().as_deref() == Some(digest.as_str()) {
            return None;
        }
        let _ = files.set_swept(Some(&digest));
        let at = at_ms();
        let all: Vec<Record> = swept
            .iter()
            .flat_map(|file| records(Origin::Sweep, &file.path, &file.flags, at))
            .collect();
        let _ = files.append(&all);
    }
    Some(report)
}

#[cfg(test)]
mod tests;
