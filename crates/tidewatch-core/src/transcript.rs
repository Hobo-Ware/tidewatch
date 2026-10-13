//! Agent transcripts for the Reader view (PLAN.md, D12): each CLI's own session file parsed into one
//! common [`Entry`] model, and a [`Tail`] that reads only what was appended since the last look.
//!
//! Formats (versions checked 2026-10-13, real samples under `tests/fixtures/`):
//! - Claude Code 2.1.296: `~/.claude/projects/<cwd>/<session>.jsonl`. One content block per line;
//!   `user`, `assistant` lines make entries, everything else (attachments, queue operations, hook
//!   summaries, sidechains, meta messages) is skipped.
//! - Gemini CLI 0.46.0: `~/.gemini/tmp/<slug>/chats/session-*-<id8>.jsonl`. A message id can repeat
//!   with more fields (tool calls attached later); text and each tool call are emitted once.
//! - Copilot CLI 1.0.91: `~/.copilot/session-state/<id>/events.jsonl`, one event per line.
//! - Codex 0.162.1: `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, checked on a real
//!   interactive run (`codex_live.jsonl`). The model's tools arrive as `custom_tool_call` items
//!   (a code string in `input`) with a list of text parts as `output`; `function_call` items are
//!   still read, from a hand-written sample (`codex_function_call.jsonl`).
//!
//! A malformed line is skipped, never a panic. [`Tail::poll`] reads the file and blocks on disk: call
//! it off the UI thread. The parsers are pure.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::rfc3339;

const SUMMARY_MAX_CHARS: usize = 120;
const TOOL_RESULT_MAX_CHARS: usize = 4000;
const HEAD_BYTES: usize = 64;
const PARTIAL_MAX_BYTES: usize = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPhase {
    /// The agent asked for the tool; `Entry::text` is empty.
    Call,
    /// The tool answered; `Entry::text` is its output, shortened to a few thousand characters.
    Result { error: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    /// The telling argument on one line: the command, the file, the pattern.
    pub summary: String,
    /// Pairs a result with its call, when the CLI gives ids.
    pub id: Option<String>,
    pub phase: ToolPhase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub role: Role,
    /// Markdown for `User` and `Assistant`; plain text for the rest.
    pub text: String,
    pub tool: Option<ToolCall>,
    /// Unix milliseconds.
    pub ts: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Claude,
    Gemini,
    Copilot,
    Codex,
}

impl Source {
    /// The source for an agent id of the agent table (`claude`, `gemini`, `copilot`, `codex`). A
    /// custom agent has no transcript reader.
    #[must_use]
    pub fn from_agent_id(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            "copilot" => Some(Self::Copilot),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Turns the lines of one transcript, in order, into entries. It remembers what it needs across
/// lines (tool names for results, what Gemini already showed), so one `Parser` serves one file.
#[derive(Debug)]
pub struct Parser {
    source: Source,
    tools: HashMap<String, (String, String)>,
    emitted: HashSet<String>,
}

impl Parser {
    #[must_use]
    pub fn new(source: Source) -> Self {
        Self { source, tools: HashMap::new(), emitted: HashSet::new() }
    }

    /// The entries one line adds. A line that is not a JSON object, or that holds nothing worth
    /// showing, adds none.
    pub fn feed_line(&mut self, line: &str) -> Vec<Entry> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        if !value.is_object() {
            return Vec::new();
        }
        match self.source {
            Source::Claude => self.claude(&value),
            Source::Gemini => self.gemini(&value),
            Source::Copilot => self.copilot(&value),
            Source::Codex => self.codex(&value),
        }
    }

    fn remember(&mut self, id: &str, name: &str, summary: &str) {
        self.tools.insert(id.to_owned(), (name.to_owned(), summary.to_owned()));
    }

    fn recall(&self, id: &str) -> (String, String) {
        self.tools.get(id).cloned().unwrap_or_else(|| ("tool".to_owned(), String::new()))
    }

    fn first_time(&mut self, key: String) -> bool {
        self.emitted.insert(key)
    }

    fn claude(&mut self, line: &Value) -> Vec<Entry> {
        if line["isSidechain"].as_bool() == Some(true) || line["isMeta"].as_bool() == Some(true) {
            return Vec::new();
        }
        let ts = timestamp(&line["timestamp"]);
        let content = &line["message"]["content"];
        match line["type"].as_str() {
            Some("user") => {
                let mut entries = Vec::new();
                for block in blocks(content) {
                    match block["type"].as_str() {
                        Some("text") => {
                            entries.extend(claude_user_text(block["text"].as_str(), ts));
                        }
                        Some("tool_result") => {
                            let id = block["tool_use_id"].as_str().unwrap_or_default();
                            let error = block["is_error"].as_bool() == Some(true);
                            let (name, summary) = self.recall(id);
                            entries.push(tool_result(
                                name,
                                summary,
                                Some(id),
                                &result_text(&block["content"]),
                                error,
                                ts,
                            ));
                        }
                        _ => {}
                    }
                }
                if let Some(text) = content.as_str() {
                    entries.extend(claude_user_text(Some(text), ts));
                }
                entries
            }
            Some("assistant") => {
                let api_error = line["isApiErrorMessage"].as_bool() == Some(true);
                let mut entries = Vec::new();
                for block in blocks(content) {
                    match block["type"].as_str() {
                        Some("text") if api_error => {
                            entries.extend(entry(Role::System, block["text"].as_str(), ts));
                        }
                        Some("text") => {
                            entries.extend(entry(Role::Assistant, block["text"].as_str(), ts));
                        }
                        Some("tool_use") => {
                            let id = block["id"].as_str().unwrap_or_default();
                            let name = block["name"].as_str().unwrap_or("tool");
                            let summary = summary_of(&block["input"]);
                            self.remember(id, name, &summary);
                            entries.push(tool_call(name, summary, Some(id), ts));
                        }
                        _ => {}
                    }
                }
                entries
            }
            _ => Vec::new(),
        }
    }

    fn gemini(&mut self, line: &Value) -> Vec<Entry> {
        let Some(id) = line["id"].as_str() else {
            return Vec::new();
        };
        let ts = timestamp(&line["timestamp"]);
        match line["type"].as_str() {
            Some("user") => {
                if !self.first_time(format!("{id}:text")) {
                    return Vec::new();
                }
                let text = gemini_user_text(&line["content"]);
                entry(Role::User, Some(&text), ts)
                    .filter(|e| !gemini_injected(&e.text))
                    .into_iter()
                    .collect()
            }
            Some("gemini") => {
                let mut entries = Vec::new();
                let text = line["content"].as_str();
                if text.is_some_and(|t| !t.trim().is_empty())
                    && self.first_time(format!("{id}:text"))
                {
                    entries.extend(entry(Role::Assistant, text, ts));
                }
                for call in line["toolCalls"].as_array().into_iter().flatten() {
                    entries.extend(self.gemini_tool_call(call, ts));
                }
                entries
            }
            Some("error" | "warning") => {
                if self.first_time(format!("{id}:text")) {
                    entry(Role::System, line["content"].as_str(), ts).into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn gemini_tool_call(&mut self, call: &Value, ts: Option<i64>) -> Vec<Entry> {
        let Some(id) = call["id"].as_str() else {
            return Vec::new();
        };
        let name = call["name"].as_str().unwrap_or("tool");
        let summary = summary_of(&call["args"]);
        let mut entries = Vec::new();
        if self.first_time(format!("{id}:call")) {
            self.remember(id, name, &summary);
            entries.push(tool_call(name, summary.clone(), Some(id), ts));
        }
        let status = call["status"].as_str().unwrap_or_default();
        let finished =
            !call["result"].is_null() || matches!(status, "success" | "error" | "cancelled");
        if finished && self.first_time(format!("{id}:result")) {
            let response = &call["result"][0]["functionResponse"]["response"];
            let text = response["output"]
                .as_str()
                .or_else(|| response["error"].as_str())
                .or_else(|| call["resultDisplay"].as_str())
                .unwrap_or_default();
            let ts = timestamp(&call["timestamp"]).or(ts);
            entries.push(tool_result(
                name.to_owned(),
                summary,
                Some(id),
                text,
                status != "success",
                ts,
            ));
        }
        entries
    }

    fn copilot(&mut self, line: &Value) -> Vec<Entry> {
        let ts = timestamp(&line["timestamp"]);
        let data = &line["data"];
        match line["type"].as_str() {
            Some("user.message") => {
                entry(Role::User, data["content"].as_str(), ts).into_iter().collect()
            }
            Some("assistant.message") => {
                entry(Role::Assistant, data["content"].as_str(), ts).into_iter().collect()
            }
            Some("tool.execution_start") => {
                let id = data["toolCallId"].as_str().unwrap_or_default();
                let name = data["toolName"].as_str().unwrap_or("tool");
                let summary = summary_of(&data["arguments"]);
                self.remember(id, name, &summary);
                if name == "report_intent" {
                    return Vec::new();
                }
                vec![tool_call(name, summary, Some(id), ts)]
            }
            Some("tool.execution_complete") => {
                let id = data["toolCallId"].as_str().unwrap_or_default();
                let (name, summary) = self.recall(id);
                if name == "report_intent" {
                    return Vec::new();
                }
                let text = data["result"]["content"]
                    .as_str()
                    .or_else(|| data["error"]["message"].as_str())
                    .unwrap_or_default();
                let error = data["success"].as_bool() == Some(false);
                vec![tool_result(name, summary, Some(id), text, error, ts)]
            }
            Some("session.error") => {
                entry(Role::System, data["message"].as_str(), ts).into_iter().collect()
            }
            Some("abort") => {
                let reason = data["reason"].as_str().unwrap_or("abort");
                entry(Role::System, Some(&format!("Aborted ({reason})")), ts).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    fn codex(&mut self, line: &Value) -> Vec<Entry> {
        if line["type"].as_str() != Some("response_item") {
            return Vec::new();
        }
        let ts = timestamp(&line["timestamp"]);
        let item = &line["payload"];
        match item["type"].as_str() {
            Some("message") => {
                let text = blocks(&item["content"])
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                match item["role"].as_str() {
                    Some("user") if !codex_injected(&text) => {
                        entry(Role::User, Some(&text), ts).into_iter().collect()
                    }
                    Some("assistant") => {
                        entry(Role::Assistant, Some(&text), ts).into_iter().collect()
                    }
                    _ => Vec::new(),
                }
            }
            Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                let id = item["call_id"].as_str().unwrap_or_default();
                let name = item["name"].as_str().unwrap_or("shell");
                let args = item["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .unwrap_or_else(|| {
                        if item["action"].is_object() {
                            item["action"].clone()
                        } else {
                            item["input"].clone()
                        }
                    });
                let summary = summary_of(&args);
                self.remember(id, name, &summary);
                vec![tool_call(name, summary, Some(id), ts)]
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let id = item["call_id"].as_str().unwrap_or_default();
                let (name, summary) = self.recall(id);
                let output = &item["output"];
                let error = output["success"].as_bool() == Some(false);
                let text = codex_output_text(output);
                vec![tool_result(name, summary, Some(id), &text, error, ts)]
            }
            _ => Vec::new(),
        }
    }
}

/// Every entry of a whole transcript. For a file that grows, use [`Tail`].
#[must_use]
pub fn parse_all(source: Source, text: &str) -> Vec<Entry> {
    let mut parser = Parser::new(source);
    text.lines().flat_map(|line| parser.feed_line(line)).collect()
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

fn timestamp(value: &Value) -> Option<i64> {
    value.as_str().and_then(rfc3339::unix_millis)
}

fn entry(role: Role, text: Option<&str>, ts: Option<i64>) -> Option<Entry> {
    let text = text?.trim();
    (!text.is_empty()).then(|| Entry { role, text: text.to_owned(), tool: None, ts })
}

fn tool_call(name: &str, summary: String, id: Option<&str>, ts: Option<i64>) -> Entry {
    Entry {
        role: Role::Tool,
        text: String::new(),
        tool: Some(ToolCall {
            name: name.to_owned(),
            summary,
            id: id.map(str::to_owned),
            phase: ToolPhase::Call,
        }),
        ts,
    }
}

fn tool_result(
    name: String,
    summary: String,
    id: Option<&str>,
    text: &str,
    error: bool,
    ts: Option<i64>,
) -> Entry {
    Entry {
        role: Role::Tool,
        text: clip(text.trim(), TOOL_RESULT_MAX_CHARS),
        tool: Some(ToolCall {
            name,
            summary,
            id: id.map(str::to_owned),
            phase: ToolPhase::Result { error },
        }),
        ts,
    }
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => {
            format!("{}\n... ({} more characters)", &text[..end], text[end..].chars().count())
        }
        None => text.to_owned(),
    }
}

fn result_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    blocks(content)
        .map(|b| match b["type"].as_str() {
            Some("text") => b["text"].as_str().unwrap_or_default().to_owned(),
            Some(other) => format!("[{other}]"),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn claude_user_text(text: Option<&str>, ts: Option<i64>) -> Option<Entry> {
    let trimmed = text?.trim();
    if trimmed.starts_with("<local-command-caveat>") || trimmed.starts_with("<system-reminder>") {
        return None;
    }
    let system = trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("[Request interrupted");
    entry(if system { Role::System } else { Role::User }, Some(trimmed), ts)
}

fn gemini_user_text(content: &Value) -> String {
    match content.as_str() {
        Some(text) => text.to_owned(),
        None => blocks(content).filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
    }
}

/// Gemini writes its own workspace summary as a user message; it is not something the user typed.
fn gemini_injected(text: &str) -> bool {
    text.starts_with("<session_context>") || text.starts_with("- **Workspace Directories:**")
}

fn codex_output_text(output: &Value) -> String {
    match output {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect(),
        other => other["content"].as_str().unwrap_or_default().to_owned(),
    }
}

fn codex_injected(text: &str) -> bool {
    let text = text.trim_start();
    [
        "<environment_context>",
        "<user_instructions>",
        "# AGENTS.md instructions",
        "<user_shell_command>",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

/// The argument that says what a tool call does, on one line.
fn summary_of(args: &Value) -> String {
    const KEYS: [&str; 11] = [
        "command",
        "cmd",
        "file_path",
        "path",
        "dir_path",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
        "name",
    ];
    let named = KEYS.iter().find_map(|key| match &args[*key] {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(parts) => {
            let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            (!words.is_empty()).then(|| words.join(" "))
        }
        _ => None,
    });
    let text = named.or_else(|| args.as_str().map(str::to_owned));
    let line = text.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(SUMMARY_MAX_CHARS) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line,
    }
}

/// Reads a transcript file that grows, and returns only what is new on each [`poll`](Tail::poll).
///
/// It keeps a byte offset. A last line the writer has not finished is held back until its newline
/// arrives (or until it is a complete JSON object). When the file was replaced or cut short, the
/// tail starts again from the top of the new content.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    parser: Parser,
    offset: u64,
    partial: Vec<u8>,
    head: Vec<u8>,
    identity: (u64, u64),
}

impl Tail {
    #[must_use]
    pub fn new(source: Source, path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            parser: Parser::new(source),
            offset: 0,
            partial: Vec::new(),
            head: Vec::new(),
            identity: (0, 0),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Entries added since the last poll. A missing or unreadable file adds none and is tried again
    /// next time. Blocks on disk.
    pub fn poll(&mut self) -> Vec<Entry> {
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        let Ok(meta) = file.metadata() else {
            return Vec::new();
        };
        let identity = file_identity(&meta);
        if self.offset > 0
            && (meta.len() < self.offset
                || identity != self.identity
                || !head_matches(&mut file, &self.head))
        {
            self.restart();
        }
        self.identity = identity;
        let mut fresh = Vec::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_end(&mut fresh).is_err()
        {
            return Vec::new();
        }
        self.offset += fresh.len() as u64;
        self.head = read_prefix(&mut file, HEAD_BYTES.min(self.offset as usize));
        self.take_lines(fresh)
    }

    fn restart(&mut self) {
        self.parser = Parser::new(self.parser.source);
        self.offset = 0;
        self.partial.clear();
        self.head.clear();
    }

    fn take_lines(&mut self, fresh: Vec<u8>) -> Vec<Entry> {
        let mut data = std::mem::take(&mut self.partial);
        data.extend(fresh);
        let mut entries = Vec::new();
        let mut rest: &[u8] = &data;
        while let Some(end) = rest.iter().position(|b| *b == b'\n') {
            entries.extend(self.parse_bytes(&rest[..end]));
            rest = &rest[end + 1..];
        }
        if is_complete_object(rest) {
            entries.extend(self.parse_bytes(rest));
        } else if rest.len() <= PARTIAL_MAX_BYTES {
            self.partial = rest.to_vec();
        }
        entries
    }

    fn parse_bytes(&mut self, line: &[u8]) -> Vec<Entry> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return Vec::new();
        }
        self.parser.feed_line(&String::from_utf8_lossy(line))
    }
}

fn is_complete_object(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'{')
        && serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v.is_object())
}

fn read_prefix(file: &mut std::fs::File, len: usize) -> Vec<u8> {
    let mut buf = vec![0; len];
    if file.seek(SeekFrom::Start(0)).is_err() || file.read_exact(&mut buf).is_err() {
        return Vec::new();
    }
    buf
}

fn head_matches(file: &mut std::fs::File, head: &[u8]) -> bool {
    head.is_empty() || read_prefix(file, head.len()) == head
}

#[cfg(unix)]
fn file_identity(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn file_identity(_: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const CLAUDE: &str = include_str!("../tests/fixtures/claude.jsonl");
    const GEMINI: &str = include_str!("../tests/fixtures/gemini.jsonl");
    const COPILOT: &str = include_str!("../tests/fixtures/copilot.jsonl");
    const CODEX_FUNCTION_CALL: &str = include_str!("../tests/fixtures/codex_function_call.jsonl");
    const CODEX_LIVE: &str = include_str!("../tests/fixtures/codex_live.jsonl");

    type ToolShape = (String, String, ToolPhase);
    type Shaped = (Role, String, Option<ToolShape>);

    fn shape(entries: &[Entry]) -> Vec<Shaped> {
        entries
            .iter()
            .map(|e| {
                let tool = e.tool.as_ref().map(|t| (t.name.clone(), t.summary.clone(), t.phase));
                (e.role, e.text.clone(), tool)
            })
            .collect()
    }

    fn call(name: &str, summary: &str) -> ToolShape {
        (name.into(), summary.into(), ToolPhase::Call)
    }

    fn result(name: &str, summary: &str, error: bool) -> ToolShape {
        (name.into(), summary.into(), ToolPhase::Result { error })
    }

    #[test]
    fn claude_fixture_gives_prompt_tool_call_result_and_answer() {
        let entries = parse_all(Source::Claude, CLAUDE);
        assert_eq!(
            shape(&entries),
            [
                (
                    Role::User,
                    "Run the shell command 'cat notes.txt' then reply with one word.".into(),
                    None
                ),
                (Role::Tool, String::new(), Some(call("Bash", "cat notes.txt"))),
                (Role::Tool, "hello".into(), Some(result("Bash", "cat notes.txt", false))),
                (Role::Assistant, "hello".into(), None),
            ]
        );
        assert_eq!(entries[0].ts, Some(1_791_846_235_696));
        assert_eq!(
            entries[1].tool.as_ref().and_then(|t| t.id.as_deref()),
            Some("toolu_01CJPLN1Vrsb8mQ2XYzmCoXm")
        );
    }

    #[test]
    fn geminis_own_workspace_summary_is_not_a_user_turn() {
        let line = r#"{"id":"u1","timestamp":"2026-10-12T23:04:25.176Z","type":"user","content":[{"text":"- **Workspace Directories:**\n  - /work"}]}"#;
        assert_eq!(Parser::new(Source::Gemini).feed_line(line), Vec::new());
    }

    #[test]
    fn gemini_fixture_gives_prompt_tool_call_result_and_answer_once_each() {
        let entries = parse_all(Source::Gemini, GEMINI);
        assert_eq!(
            shape(&entries),
            [
                (
                    Role::User,
                    "Read the file notes.txt and reply with only its content.".into(),
                    None
                ),
                (Role::Tool, String::new(), Some(call("read_file", "notes.txt"))),
                (Role::Tool, "hello".into(), Some(result("read_file", "notes.txt", false))),
                (Role::Assistant, "hello".into(), None),
            ]
        );
    }

    #[test]
    fn copilot_fixture_gives_prompt_tool_call_result_and_answer() {
        let entries = parse_all(Source::Copilot, COPILOT);
        assert_eq!(
            shape(&entries),
            [
                (
                    Role::User,
                    "Run the shell command 'cat notes.txt' then reply with one word.".into(),
                    None
                ),
                (Role::Tool, String::new(), Some(call("bash", "cat notes.txt"))),
                (
                    Role::Tool,
                    "hello\n<shellId: 0 completed with exit code 0>".into(),
                    Some(result("bash", "cat notes.txt", false))
                ),
                (Role::Assistant, "Hello".into(), None),
            ]
        );
    }

    #[test]
    fn codex_sample_gives_prompt_tool_call_result_and_answer() {
        let entries = parse_all(Source::Codex, CODEX_FUNCTION_CALL);
        assert_eq!(
            shape(&entries),
            [
                (Role::User, "Run cat notes.txt then reply with one word.".into(), None),
                (Role::Tool, String::new(), Some(call("shell", "bash -lc cat notes.txt"))),
                (
                    Role::Tool,
                    "{\"output\":\"hello\\n\",\"metadata\":{\"exit_code\":0}}".into(),
                    Some(result("shell", "bash -lc cat notes.txt", false))
                ),
                (Role::Assistant, "hello".into(), None),
            ]
        );
    }

    #[test]
    fn codex_live_rollout_gives_prompts_code_tool_calls_and_answers() {
        let entries = parse_all(Source::Codex, CODEX_LIVE);
        let roles: Vec<Role> = entries.iter().map(|e| e.role).collect();
        assert_eq!(
            roles,
            [
                Role::User,
                Role::Assistant,
                Role::Tool,
                Role::Tool,
                Role::Tool,
                Role::Tool,
                Role::Assistant,
                Role::User,
                Role::Assistant,
            ]
        );
        let shaped = shape(&entries);
        let stage_done =
            "const r = await tools.mcp__tidewatch__stage_done({summary:\"done\"}); text(r);";
        assert_eq!(shaped[4].2, Some(call("exec", stage_done)));
        assert_eq!(shaped[5].2, Some(result("exec", stage_done, false)));
        assert_eq!(
            shaped[5].1,
            "Script completed\nWall time 0.0 seconds\nOutput:\n{\"content\":[{\"type\":\"text\",\"text\":\"Recorded. Tidewatch has your summary.\"}],\"isError\":false}"
        );
        assert!(shaped[3].1.contains("\nhi\n"));
        assert_eq!(shaped[8].1, "kiwi");
        assert!(entries.iter().all(|e| e.ts.is_some()));
    }

    #[test]
    fn agent_ids_map_to_sources() {
        assert_eq!(Source::from_agent_id("claude"), Some(Source::Claude));
        assert_eq!(Source::from_agent_id("copilot"), Some(Source::Copilot));
        assert_eq!(Source::from_agent_id("my-agent"), None);
    }

    #[test]
    fn claude_interactive_shapes() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Fix the bug"}]},"timestamp":"2026-10-13T10:00:00Z"}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Stop hook feedback: x"}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"sub agent prompt"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: x</local-command-caveat>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"assistant","isApiErrorMessage":true,"message":{"role":"assistant","content":[{"type":"text","text":"API Error: overloaded"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/work/a.rs"}},{"type":"text","text":"Reading."}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"no such file"},{"type":"image"}]}]}}"#,
        ];
        let mut parser = Parser::new(Source::Claude);
        let entries: Vec<Entry> = lines.iter().flat_map(|l| parser.feed_line(l)).collect();
        assert_eq!(
            shape(&entries),
            [
                (Role::User, "Fix the bug".into(), None),
                (Role::System, "<command-name>/clear</command-name>".into(), None),
                (Role::System, "[Request interrupted by user]".into(), None),
                (Role::System, "API Error: overloaded".into(), None),
                (Role::Tool, String::new(), Some(call("Read", "/work/a.rs"))),
                (Role::Assistant, "Reading.".into(), None),
                (
                    Role::Tool,
                    "no such file\n[image]".into(),
                    Some(result("Read", "/work/a.rs", true))
                ),
            ]
        );
    }

    #[test]
    fn gemini_repeated_ids_do_not_repeat_entries() {
        let lines = [
            r#"{"id":"m1","type":"gemini","content":"","thoughts":[{"subject":"x"}]}"#,
            r#"{"id":"m1","type":"gemini","content":"","toolCalls":[{"id":"c1","name":"run_shell_command","args":{"command":"ls"},"status":"executing"}]}"#,
            r#"{"id":"m1","type":"gemini","content":"","toolCalls":[{"id":"c1","name":"run_shell_command","args":{"command":"ls"},"status":"error","result":[{"functionResponse":{"response":{"error":"denied"}}}]}]}"#,
            r#"{"id":"m1","type":"gemini","content":"Done.","toolCalls":[{"id":"c1","name":"run_shell_command","args":{"command":"ls"},"status":"error"}]}"#,
            r#"{"id":"m1","type":"gemini","content":"Done."}"#,
            r#"{"id":"e1","type":"error","content":"quota exceeded"}"#,
            r#"{"id":"i1","type":"info","content":"MCP issues detected."}"#,
            r#"{"$set":{"lastUpdated":"2026-10-13T10:00:00Z"}}"#,
        ];
        let mut parser = Parser::new(Source::Gemini);
        let entries: Vec<Entry> = lines.iter().flat_map(|l| parser.feed_line(l)).collect();
        assert_eq!(
            shape(&entries),
            [
                (Role::Tool, String::new(), Some(call("run_shell_command", "ls"))),
                (Role::Tool, "denied".into(), Some(result("run_shell_command", "ls", true))),
                (Role::Assistant, "Done.".into(), None),
                (Role::System, "quota exceeded".into(), None),
            ]
        );
    }

    #[test]
    fn copilot_errors_aborts_and_intents() {
        let lines = [
            r#"{"type":"tool.execution_start","data":{"toolCallId":"i","toolName":"report_intent","arguments":{"intent":"Exploring"}}}"#,
            r#"{"type":"tool.execution_complete","data":{"toolCallId":"i","success":true,"result":{"content":"Intent logged"}}}"#,
            r#"{"type":"tool.execution_start","data":{"toolCallId":"v","toolName":"view","arguments":{"path":"/work/a.rs"}}}"#,
            r#"{"type":"tool.execution_complete","data":{"toolCallId":"v","success":false,"error":{"message":"not found"}}}"#,
            r#"{"type":"session.error","data":{"message":"Not authenticated"}}"#,
            r#"{"type":"abort","data":{"reason":"user_abort"}}"#,
            r#"{"type":"permission.requested","data":{}}"#,
        ];
        let mut parser = Parser::new(Source::Copilot);
        let entries: Vec<Entry> = lines.iter().flat_map(|l| parser.feed_line(l)).collect();
        assert_eq!(
            shape(&entries),
            [
                (Role::Tool, String::new(), Some(call("view", "/work/a.rs"))),
                (Role::Tool, "not found".into(), Some(result("view", "/work/a.rs", true))),
                (Role::System, "Not authenticated".into(), None),
                (Role::System, "Aborted (user_abort)".into(), None),
            ]
        );
    }

    #[test]
    fn malformed_lines_are_skipped_for_every_source() {
        let junk = [
            "",
            "   ",
            "not json",
            "[1,2]",
            "42",
            "null",
            "{",
            r#"{"type":5}"#,
            r#"{"type":"user","message":7}"#,
            "\u{0}\u{1}",
        ];
        for source in [Source::Claude, Source::Gemini, Source::Copilot, Source::Codex] {
            let mut parser = Parser::new(source);
            for line in junk {
                assert_eq!(parser.feed_line(line), [], "{source:?} {line:?}");
            }
        }
    }

    #[test]
    fn long_tool_output_and_summaries_are_shortened() {
        let big = "x".repeat(10_000);
        let line = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"z","content":"{big}"}}]}}}}"#
        );
        let entries = Parser::new(Source::Claude).feed_line(&line);
        assert!(entries[0].text.ends_with("... (6000 more characters)"));
        assert_eq!(
            entries[0].text.chars().count(),
            4000 + "\n... (6000 more characters)".chars().count()
        );
        let long_command = format!("echo {}", "ab ".repeat(100));
        assert_eq!(
            summary_of(&serde_json::json!({ "command": long_command })).chars().count(),
            123
        );
        assert_eq!(summary_of(&serde_json::json!({ "command": "a\n  b\tc" })), "a b c");
        assert_eq!(summary_of(&serde_json::json!({ "other": 1 })), "");
    }

    fn write_file(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn texts(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| e.text.clone()).collect()
    }

    fn user_line(text: &str) -> String {
        format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{text}\"}}}}\n")
    }

    #[test]
    fn tail_returns_only_new_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, &user_line("one"));
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(texts(&tail.poll()), ["one"]);
        assert_eq!(tail.poll(), []);
        append(&file, user_line("two").as_bytes());
        append(&file, user_line("three").as_bytes());
        assert_eq!(texts(&tail.poll()), ["two", "three"]);
        assert_eq!(tail.poll(), []);
    }

    #[test]
    fn tail_waits_for_the_end_of_a_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let line = user_line("héllo wörld");
        let (first, second) = line.as_bytes().split_at(line.find('ö').unwrap() + 1);
        write_file(&file, "");
        let mut tail = Tail::new(Source::Claude, &file);
        append(&file, first);
        assert_eq!(tail.poll(), []);
        append(&file, second);
        assert_eq!(texts(&tail.poll()), ["héllo wörld"]);
        assert_eq!(tail.poll(), []);
    }

    #[test]
    fn tail_accepts_a_finished_last_line_without_a_newline_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, user_line("one").trim_end());
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(texts(&tail.poll()), ["one"]);
        append(&file, b"\n");
        assert_eq!(tail.poll(), []);
        append(&file, user_line("two").as_bytes());
        assert_eq!(texts(&tail.poll()), ["two"]);
    }

    #[test]
    fn tail_starts_over_after_a_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, &(user_line("first long line one") + &user_line("first long line two")));
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(tail.poll().len(), 2);
        write_file(&file, &user_line("new"));
        assert_eq!(texts(&tail.poll()), ["new"]);
        write_file(&file, "");
        assert_eq!(tail.poll(), []);
        append(&file, user_line("again").as_bytes());
        assert_eq!(texts(&tail.poll()), ["again"]);
    }

    #[test]
    fn tail_starts_over_when_the_file_was_replaced_by_a_longer_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, &user_line("aaa"));
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(texts(&tail.poll()), ["aaa"]);
        let other = dir.path().join("other.jsonl");
        write_file(&other, &(user_line("bbb") + &user_line("ccc") + &user_line("ddd")));
        std::fs::rename(&other, &file).unwrap();
        assert_eq!(texts(&tail.poll()), ["bbb", "ccc", "ddd"]);
    }

    #[test]
    fn tail_notices_a_rewrite_in_place_that_grew() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, &user_line("aaa"));
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(texts(&tail.poll()), ["aaa"]);
        write_file(&file, &(user_line("zzz") + &user_line("yyy") + &user_line("xxx")));
        assert_eq!(texts(&tail.poll()), ["zzz", "yyy", "xxx"]);
    }

    #[test]
    fn tail_of_a_file_that_does_not_exist_yet_adds_nothing_then_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("later.jsonl");
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(tail.poll(), []);
        write_file(&file, &user_line("hi"));
        assert_eq!(texts(&tail.poll()), ["hi"]);
        assert_eq!(tail.path(), file);
    }

    #[test]
    fn tail_skips_garbage_between_good_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_file(&file, &format!("{}garbage\n\r\n{{\"half\n{}", user_line("a"), user_line("b")));
        let mut tail = Tail::new(Source::Claude, &file);
        assert_eq!(texts(&tail.poll()), ["a", "b"]);
    }

    #[test]
    fn feeding_a_file_in_two_appends_equals_feeding_it_whole_at_every_cut() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (Source::Claude, CLAUDE),
            (Source::Gemini, GEMINI),
            (Source::Copilot, COPILOT),
            (Source::Codex, CODEX_FUNCTION_CALL),
            (Source::Codex, CODEX_LIVE),
        ];
        for (source, text) in cases {
            let whole = parse_all(source, text);
            assert_ne!(whole, []);
            let bytes = text.as_bytes();
            let near_a_line_end =
                |cut: usize| cut > 0 && bytes[cut - 1] == b'\n' || bytes.get(cut) == Some(&b'\n');
            let cuts = (0..=bytes.len())
                .filter(|cut| bytes.len() <= 3000 || cut % 7 == 0 || near_a_line_end(*cut));
            for cut in cuts {
                let file = dir.path().join("split.jsonl");
                write_file(&file, "");
                let mut tail = Tail::new(source, &file);
                append(&file, &bytes[..cut]);
                let mut split = tail.poll();
                append(&file, &bytes[cut..]);
                split.extend(tail.poll());
                assert_eq!(split, whole, "{source:?} cut at {cut}");
            }
        }
    }
}
