//! The trailers guard as a Claude PreToolUse hook on Bash: `tidewatch --guard trailers` reads the
//! command the agent is about to run and refuses a `gh pr|issue` call whose text has a
//! Co-Authored-By trailer or a Generated with footer. The shims in `guard_shim` already refuse the
//! same call when they come first on the PATH; the hook also covers a `gh` the shims never see.
//!
//! The command arrives as one shell string, so this reads it the way `guard_args` reads an argv:
//! heredoc bodies are lifted out, the rest is split into words, and every `gh` word starts a call.
//! A command that does not split (an open quote) is let through. A flag is exit 2 with the reason on
//! stderr, checked against the hooks reference of Claude Code 2.1.310.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::guard_args::{GhCall, Part, Source, TextArg, parse_gh};
use crate::guard_run::{GuardEnv, Machine, check_gh};
use crate::guards::{Rule, Rules};
use crate::prefs::Prefs;

const OPERATORS: [&str; 5] = ["&&", "||", "|", ";", "&"];

struct Heredoc {
    delimiter: String,
    indented: bool,
}

fn heredoc_of(line: &str) -> Option<Heredoc> {
    let at = line
        .match_indices("<<")
        .map(|(at, _)| at)
        .find(|&at| !line[..at].ends_with('<') && !line[at + 2..].starts_with('<'))?;
    let rest = &line[at + 2..];
    let (indented, rest) = rest.strip_prefix('-').map_or((false, rest), |r| (true, r));
    let rest = rest.trim_start().trim_start_matches(['\'', '"']);
    let delimiter: String =
        rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    (!delimiter.is_empty()).then_some(Heredoc { delimiter, indented })
}

/// The command without its heredoc bodies and terminator lines, and the bodies. A heredoc with no
/// terminator is left in place.
fn lift_heredocs(command: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = command.lines().collect();
    let mut kept = Vec::new();
    let mut bodies = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        kept.push(lines[at]);
        let Some(doc) = heredoc_of(lines[at]) else {
            at += 1;
            continue;
        };
        let end = lines[at + 1..].iter().position(|line| {
            let line = if doc.indented { line.trim_start_matches('\t') } else { line };
            *line == doc.delimiter
        });
        match end {
            Some(offset) => {
                bodies.push(lines[at + 1..at + 1 + offset].join("\n"));
                at += offset + 2;
            }
            None => at += 1,
        }
    }
    (kept.join("\n"), bodies)
}

fn is_gh(word: &str) -> bool {
    word == "gh" || word.ends_with("/gh")
}

fn ends_call(word: &str) -> bool {
    OPERATORS.contains(&word) || word.starts_with(['<', '>'])
}

/// Every guarded `gh` call in a shell command, with the text a heredoc feeds it.
#[must_use]
pub fn gh_calls(command: &str) -> Vec<GhCall> {
    let (stripped, bodies) = lift_heredocs(command);
    let Ok(words) = shell_words::split(&stripped) else { return Vec::new() };
    let mut calls = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if !is_gh(word) {
            continue;
        }
        let args: Vec<String> =
            words[at + 1..].iter().take_while(|w| !ends_call(w)).cloned().collect();
        let Some(mut call) = parse_gh(&args) else { continue };
        let reads_stdin = call.texts.iter().any(|t| t.source == Source::Stdin);
        let embeds_heredoc = call
            .texts
            .iter()
            .any(|t| matches!(&t.source, Source::Inline(text) if text.contains("<<")));
        if reads_stdin || embeds_heredoc {
            call.texts.extend(
                bodies
                    .iter()
                    .map(|b| TextArg { part: Part::Body, source: Source::Inline(b.clone()) }),
            );
        }
        calls.push(call);
    }
    calls
}

struct HookMachine {
    cwd: Option<PathBuf>,
    stdin: String,
}

impl Machine for HookMachine {
    fn read(&mut self, _dir: Option<&Path>, source: &Source) -> Option<String> {
        match source {
            Source::Inline(text) => Some(text.clone()),
            Source::File(path) => {
                let full = self.cwd.as_ref().map_or_else(|| path.clone(), |cwd| cwd.join(path));
                std::fs::read(full).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }
            Source::Stdin => Some(self.stdin.clone()),
        }
    }

    fn is_public(&mut self, _dir: Option<&Path>, _repo: Option<&str>) -> Option<bool> {
        None
    }
}

fn trailers_only() -> Rules {
    Rules {
        dashes: false,
        trailers: true,
        private_names: false,
        draft_prs: false,
        rerun_failed: false,
        comments: false,
    }
}

/// The line to show the agent when `command` sends a trailer or footer through `gh`, read from
/// `cwd` for a relative `--body-file`.
#[must_use]
pub fn refusal(command: &str, cwd: Option<&Path>) -> Option<String> {
    gh_calls(command).iter().find_map(|call| {
        let mut machine = HookMachine { cwd: cwd.map(Path::to_path_buf), stdin: String::new() };
        let violation = check_gh(call, trailers_only(), &[], &mut machine)
            .into_iter()
            .find(|v| v.rule == Rule::Trailers)?;
        Some(format!(
            "tidewatch guard [trailers]: {} ({}) Drop that line and run the command again.",
            Rule::Trailers.reason(),
            violation.detail,
        ))
    })
}

/// Runs the hook. Returns the exit code; the reason, if any, is printed to stderr.
#[must_use]
pub fn run(stdin: &str, env: &GuardEnv) -> i32 {
    if !Prefs::load_file(&env.config).guards.rules_for(&env.repo).is_on(Rule::Trailers) {
        return 0;
    }
    let Ok(payload) = serde_json::from_str::<Value>(stdin) else { return 0 };
    if payload.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return 0;
    }
    let Some(command) = payload.pointer("/tool_input/command").and_then(Value::as_str) else {
        return 0;
    };
    let cwd = payload.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    match refusal(command, cwd.as_deref()) {
        Some(reason) => {
            eprintln!("{reason}");
            2
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests;
