//! The subagents of a Claude session, from `<session id>/subagents/agent-<id>.meta.json` and the
//! agent's own `agent-<id>.jsonl` (Claude Code 2.1.316). Only a running agent lacks `finished`
//! in its meta, but older versions never wrote it, so the last assistant `stop_reason` and a
//! silent transcript (`STALE_MS`) decide too. A transcript is read from where the last look
//! stopped, and one that ended long ago is not opened.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::rfc3339::unix_millis;
use crate::text::{clip, squash};
use crate::token_usage::Counter;
use crate::transcript::Source;

/// How long a finished agent stays in the list.
pub const FINISHED_SHOWN_MS: i64 = 60_000;
/// How long an agent that never said it finished may stay silent before it counts as gone.
pub const STALE_MS: i64 = 600_000;
const NOW_CHARS: usize = 100;
const LINE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// The meta file of one agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub description: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub finished: bool,
}

/// `None` for text that is not a JSON object.
#[must_use]
pub fn parse_meta(text: &str) -> Option<Meta> {
    let value: Value = serde_json::from_str(text).ok()?;
    let word = |key: &str| value[key].as_str().map(squash).filter(|text| !text.is_empty());
    value.is_object().then(|| Meta {
        description: word("description").unwrap_or_default(),
        model: word("model"),
        effort: word("effort"),
        finished: value["finished"].as_bool() == Some(true),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// Ended at this unix time in milliseconds.
    Finished(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub id: String,
    pub description: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub started_ms: i64,
    pub phase: Phase,
    /// What it is doing right now (see [`Fold::now`]); its description before it did anything.
    pub now: String,
    pub tokens: u64,
}

impl Agent {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// How long it ran, or has run so far.
    #[must_use]
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        let end = match self.phase {
            Phase::Running => now_ms,
            Phase::Finished(at) => at,
        };
        (end - self.started_ms).max(0)
    }
}

/// Whether the tab's agent has to be looked at again: it works, or a listed agent has not ended.
#[must_use]
pub fn needs_poll(agents: &[Agent], working: bool) -> bool {
    working || agents.iter().any(Agent::is_running)
}

/// The agents that belong in the list at `now_ms`: the running ones and those that ended within
/// [`FINISHED_SHOWN_MS`].
#[must_use]
pub fn shown(agents: &[Agent], now_ms: i64) -> Vec<&Agent> {
    agents
        .iter()
        .filter(|agent| match agent.phase {
            Phase::Running => true,
            Phase::Finished(at) => now_ms - at < FINISHED_SHOWN_MS,
        })
        .collect()
}

/// When the list next changes without a new look: the soonest a finished agent leaves it.
#[must_use]
pub fn next_leave_ms(agents: &[Agent], now_ms: i64) -> Option<i64> {
    agents
        .iter()
        .filter_map(|agent| match agent.phase {
            Phase::Finished(at) if now_ms - at < FINISHED_SHOWN_MS => Some(at + FINISHED_SHOWN_MS),
            _ => None,
        })
        .min()
}

/// The folder of the subagents of the session whose transcript is `transcript`.
#[must_use]
pub fn dir_of(transcript: &Path) -> PathBuf {
    transcript.with_extension("").join("subagents")
}

fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(clip(&squash(line), NOW_CHARS))
}

fn after_cd(command: &str) -> &str {
    command
        .strip_prefix("cd ")
        .and_then(|rest| rest.split_once(" && "))
        .map_or(command, |(_, rest)| rest.trim_start())
}

fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// What one tool call says about itself: its own `description`, else its name and the first path
/// (file name only), pattern, or command line it was given.
fn tool_text(name: &str, input: &Value) -> Option<String> {
    let text = |key: &str| input[key].as_str().map(str::trim).filter(|text| !text.is_empty());
    if let Some(description) = text("description") {
        return Some(clip(&squash(description), NOW_CHARS));
    }
    let path = ["file_path", "path", "notebook_path"].iter().find_map(|key| text(key));
    let argument = if let Some(path) = path {
        base_name(path).to_owned()
    } else {
        let other = ["pattern", "command", "url", "query"].iter().find_map(|key| text(key))?;
        first_line(after_cd(other))?
    };
    Some(clip(&format!("{name} {argument}"), NOW_CHARS))
}

/// What the lines of one agent's transcript read so far add up to.
#[derive(Debug, Default)]
pub struct Fold {
    first_ms: Option<i64>,
    last_ms: Option<i64>,
    ended: bool,
    tool: Option<String>,
    said: Option<String>,
    tokens: Counter,
}

impl Fold {
    pub fn line(&mut self, line: &str) {
        let Ok(value) = serde_json::from_str::<Value>(line) else { return };
        let kind = value["type"].as_str();
        if matches!(kind, Some("user" | "assistant"))
            && let Some(at) = value["timestamp"].as_str().and_then(unix_millis)
        {
            self.first_ms = Some(self.first_ms.map_or(at, |first| first.min(at)));
            self.last_ms = Some(self.last_ms.map_or(at, |last| last.max(at)));
        }
        match kind {
            Some("user") => self.ended = false,
            Some("assistant") => self.assistant(&value),
            _ => {}
        }
    }

    fn assistant(&mut self, value: &Value) {
        let message = &value["message"];
        self.ended = message["stop_reason"].as_str().is_some_and(|reason| reason != "tool_use");
        self.tokens.line(Source::Claude, value);
        for block in message["content"].as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or("tool");
                    if let Some(text) = tool_text(name, &block["input"]) {
                        self.tool = Some(text);
                    }
                }
                Some("text") => {
                    if let Some(text) = block["text"].as_str().and_then(first_line) {
                        self.said = Some(text);
                    }
                }
                _ => {}
            }
        }
    }

    /// The newest tool call's text, else the first line of the newest message.
    #[must_use]
    pub fn now(&self) -> Option<&str> {
        self.tool.as_deref().or(self.said.as_deref())
    }

    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended
    }

    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.tokens.tally().total.total()
    }
}

/// One transcript read from where the last look stopped.
#[derive(Debug, Default)]
struct Reading {
    fold: Fold,
    offset: u64,
    partial: Vec<u8>,
}

impl Reading {
    fn read(&mut self, path: &Path) {
        let Ok(mut file) = File::open(path) else { return };
        let Ok(len) = file.metadata().map(|meta| meta.len()) else { return };
        if len < self.offset {
            *self = Self::default();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut fresh = Vec::new();
        if file.read_to_end(&mut fresh).is_err() {
            return;
        }
        self.offset += fresh.len() as u64;
        let mut data = std::mem::take(&mut self.partial);
        data.extend(fresh);
        let mut rest: &[u8] = &data;
        while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
            if let Ok(line) = std::str::from_utf8(&rest[..end]) {
                self.fold.line(line);
            }
            rest = &rest[end + 1..];
        }
        if rest.len() <= LINE_MAX_BYTES {
            self.partial = rest.to_vec();
        }
    }
}

fn modified_ms(path: &Path) -> Option<i64> {
    let since = std::fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// Reads one session's subagents, look after look, keeping what it read of each transcript.
#[derive(Debug, Default)]
pub struct Watch {
    readings: HashMap<String, Reading>,
}

impl Watch {
    /// Every agent of `dir` that runs or ended within [`FINISHED_SHOWN_MS`] of `now_ms`: the
    /// running ones, oldest first, then the finished ones, newest first. A missing folder holds
    /// none. Blocks on disk.
    pub fn look(&mut self, dir: &Path, now_ms: i64) -> Vec<Agent> {
        let mut agents = Vec::new();
        let mut present = HashSet::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            self.readings.clear();
            return agents;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".meta.json")) else {
                continue;
            };
            let Some(meta) =
                std::fs::read_to_string(entry.path()).ok().as_deref().and_then(parse_meta)
            else {
                continue;
            };
            if let Some(agent) = self.agent(dir, id, &meta, now_ms) {
                present.insert(id.to_owned());
                agents.push(agent);
            }
        }
        self.readings.retain(|id, _| present.contains(id));
        agents.sort_by_key(|agent| match agent.phase {
            Phase::Running => (0, agent.started_ms),
            Phase::Finished(at) => (1, -at),
        });
        agents
    }

    fn agent(&mut self, dir: &Path, id: &str, meta: &Meta, now_ms: i64) -> Option<Agent> {
        let file = dir.join(format!("{id}.jsonl"));
        let written = modified_ms(&file);
        if meta.finished && written.is_none_or(|at| now_ms - at >= FINISHED_SHOWN_MS) {
            return None;
        }
        let reading = self.readings.entry(id.to_owned()).or_default();
        reading.read(&file);
        let fold = &reading.fold;
        let last = fold.last_ms.max(written);
        let phase = if meta.finished || fold.ended() {
            Phase::Finished(fold.last_ms.or(written).unwrap_or(now_ms))
        } else {
            Phase::Running
        };
        let alive = match phase {
            Phase::Running => last.is_some_and(|at| now_ms - at < STALE_MS),
            Phase::Finished(at) => now_ms - at < FINISHED_SHOWN_MS,
        };
        alive.then(|| Agent {
            id: id.to_owned(),
            description: meta.description.clone(),
            model: meta.model.clone(),
            effort: meta.effort.clone(),
            started_ms: fold.first_ms.or(last).unwrap_or(now_ms),
            phase,
            now: fold.now().map_or_else(|| meta.description.clone(), str::to_owned),
            tokens: fold.tokens(),
        })
    }
}

#[cfg(test)]
mod tests;
