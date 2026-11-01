//! The Tidewatch MCP server (PLAN.md, D6): JSON-RPC 2.0 over streamable HTTP, stateless, one reply
//! per POST.
//!
//! [`handle_post`] takes the raw request body of an authenticated `/mcp` POST and returns the reply.
//! It speaks `initialize`, `ping`, `tools/list` and `tools/call` and swallows notifications. The
//! tools are `report_finding`, `list_notes`, `reply_to_note`, `ask_user`, `update_progress`,
//! `get_progress`, `get_workflow`, `report_job`, `set_avatar`, `set_name` and `meeseeks_done`, plus
//! the repo service tools of `services_mcp.rs`. A tab is offered the ones its [`Offer`] allows
//! (`meeseeks_done` only to a Meeseeks, `report_finding` to neither a Meeseeks,
//! `ask_user` only to a CLI without a question tool Tidewatch reads); a call to one it is not
//! offered is answered with a tool error that says where it works. `set_avatar` is listed and works
//! whenever it is called. Calls that matter to the app become [`TabEvent`]s through `emit`; the note tools
//! read and write the tab's `notes.json` through `notes.rs`. `ask_user` holds its reply until the user
//! answers (or [`AskWait::timeout`] runs out, or the client hangs up): the listener serves each
//! connection on its own thread, so a held call blocks nothing else.
//!
//! Errors follow JSON-RPC: unparseable body -32700, not a request -32600, unknown method -32601,
//! unknown tool or bad arguments -32602, a failed save -32603. Nothing here panics on input.
//!
//! The wire shape was checked against claude 2.1.296 (which also probes `server/discover`, answered
//! with -32601), gemini 0.46.0 and copilot 1.0.91.

use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kelp_core::review::Comment;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::agents::asks_by_itself;
use crate::avatar_mcp;
use crate::events::TabEvent;
use crate::job_mcp;
use crate::launch::MCP_SERVER_NAME;
use crate::meeseeks_mcp;
use crate::notes::Notes;
use crate::progress_mcp;
use crate::question::{self, AskReply, Reply};
use crate::question_words;
use crate::services_mcp;
use crate::tab_name;
use crate::workflow_mcp;
use crate::workflow_profile::Workflow;

/// The protocol revision Tidewatch answers with when the client asks for one it does not know.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: [&str; 3] = [PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];
/// Longest text argument accepted, in characters.
pub const MAX_TEXT_CHARS: usize = 20_000;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// How long a held `ask_user` call waits, and how it learns its client is gone.
pub struct AskWait<'a> {
    /// How long the user has to answer before the agent is told to use its judgment.
    pub timeout: Duration,
    /// How often the wait looks at `alive`.
    pub poll: Duration,
    /// Whether the client that made the call still waits for its reply.
    pub alive: &'a dyn Fn() -> bool,
}

/// The Tidewatch tools that only report or look something up: they change nothing in the work, so
/// the Reader does not count them as a break in what the agent says.
pub const REPORT_TOOLS: [&str; 7] = [
    progress_mcp::UPDATE_TOOL,
    progress_mcp::GET_TOOL,
    workflow_mcp::GET_TOOL,
    job_mcp::REPORT_TOOL,
    avatar_mcp::SET_TOOL,
    "report_finding",
    "reply_to_note",
];

/// What a tab is, which decides the tools `tools/list` offers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabRole {
    /// An agent in a tab of its own. A credential an older version saved for a workflow stage reads
    /// back as this.
    #[default]
    #[serde(alias = "Workflow")]
    Agent,
    /// A Meeseeks: it ends with `meeseeks_done`, and nobody reads findings in a tab that poofs.
    Meeseeks,
}

/// What decides the tools `tools/list` offers a tab, and which calls are refused.
#[derive(Clone, Copy, Debug)]
pub struct Offer<'a> {
    pub role: TabRole,
    pub agent: &'a str,
}

impl Offer<'_> {
    fn offers(self, tool: &str) -> bool {
        match tool {
            meeseeks_mcp::DONE_TOOL => self.role == TabRole::Meeseeks,
            "report_finding" => self.role != TabRole::Meeseeks,
            question::TOOL_NAME => !asks_by_itself(self.agent),
            _ => true,
        }
    }

    fn refusal(tool: &str) -> String {
        match tool {
            meeseeks_mcp::DONE_TOOL => "`meeseeks_done` is not offered in this tab: only a Meeseeks can call it. Finish with your end-of-turn block.".to_owned(),
            question::TOOL_NAME => "`ask_user` is not offered to this CLI: it has a question tool of its own. Ask with that.".to_owned(),
            _ => format!("`{tool}` is not offered to a Meeseeks: nobody reads a finding in a tab that ends with its task. Put what you found in the summary of `meeseeks_done`."),
        }
    }
}

/// What one tab's tools act on.
pub struct ToolCtx<'a> {
    /// The agent id (`claude`, `gemini`, ...). It signs replies in the notes.
    pub agent: &'a str,
    pub role: TabRole,
    pub notes_file: &'a Path,
    /// Serializes read-modify-write of `notes_file` between concurrent calls.
    pub notes_lock: &'a Mutex<()>,
    /// The tab's `progress.md`, where `update_progress` saves the ledger.
    pub progress_file: &'a Path,
    pub ask: AskWait<'a>,
    /// The repo's workflow profile, read only when an agent asks for it.
    pub workflow: &'a dyn Fn() -> Workflow,
    /// Lines a saved report's answer carries once: the icon and name asks, due as soon as the
    /// report has a goal, so the tab does not wait for the owner's next prompt.
    pub after_save: &'a dyn Fn() -> Vec<String>,
}

impl ToolCtx<'_> {
    fn offer(&self) -> Offer<'_> {
        Offer { role: self.role, agent: self.agent }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpReply {
    /// A JSON body for `200 OK`.
    Json(String),
    /// `202 Accepted` with no body: the POST held only notifications or responses.
    Accepted,
}

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }
}

/// Handles one POST body: a single message or a batch.
pub fn handle_post(body: &[u8], ctx: &ToolCtx<'_>, emit: &dyn Fn(TabEvent)) -> McpReply {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(e) => {
            return reply(&error_response(&Value::Null, PARSE_ERROR, &format!("parse error: {e}")));
        }
    };
    match message {
        Value::Array(batch) if batch.is_empty() => {
            reply(&error_response(&Value::Null, INVALID_REQUEST, "empty batch"))
        }
        Value::Array(batch) => {
            let replies: Vec<Value> =
                batch.iter().filter_map(|item| handle_message(item, ctx, emit)).collect();
            if replies.is_empty() { McpReply::Accepted } else { reply(&Value::Array(replies)) }
        }
        single => {
            handle_message(&single, ctx, emit).map_or(McpReply::Accepted, |value| reply(&value))
        }
    }
}

fn reply(value: &Value) -> McpReply {
    McpReply::Json(value.to_string())
}

fn handle_message(message: &Value, ctx: &ToolCtx<'_>, emit: &dyn Fn(TabEvent)) -> Option<Value> {
    let Some(object) = message.as_object().filter(|o| o.get("jsonrpc") == Some(&json!("2.0")))
    else {
        return Some(error_response(&Value::Null, INVALID_REQUEST, "not a JSON-RPC 2.0 message"));
    };
    let method = object.get("method").and_then(Value::as_str);
    let id = match object.get("id") {
        None => None,
        Some(id) if id.is_string() || id.is_number() => Some(id),
        Some(_) => {
            return Some(error_response(
                &Value::Null,
                INVALID_REQUEST,
                "id must be a string or number",
            ));
        }
    };
    let Some(method) = method else {
        let is_response = object.contains_key("result") || object.contains_key("error");
        return (!is_response).then(|| {
            error_response(id.unwrap_or(&Value::Null), INVALID_REQUEST, "missing method")
        });
    };
    let id = id?;
    let params = object.get("params");
    let outcome = match method {
        "initialize" => Ok(initialize_result(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions_for(ctx.offer()) })),
        "tools/call" => call_tool(params, ctx, emit),
        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("method not found: {other}"))),
    };
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => error_response(id, e.code, &e.message),
    })
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result(params: Option<&Value>) -> Value {
    let asked = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
    let version = asked.filter(|v| SUPPORTED_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": MCP_SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Tidewatch tools for the task in this worktree: report review findings, read and answer the notes the user left on your changes, report jobs that run over a minute, and read the repo's workflow.",
    })
}

#[must_use]
pub fn tool_definitions_for(offer: Offer<'_>) -> Value {
    let offered: Vec<Value> = tool_definitions()
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["name"].as_str().is_some_and(|name| offer.offers(name)))
        .cloned()
        .collect();
    Value::Array(offered)
}

/// Every tool the server has, for any role.
#[must_use]
pub fn tool_definitions() -> Value {
    let [update_progress, get_progress] = progress_mcp::tool_definitions();
    let mut tools = json!([
        {
            "name": "report_finding",
            "description": "Report one problem you found in the code under review. It appears as a note on that line in the user's diff.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Path relative to the repository root." },
                    "line": { "type": "integer", "minimum": 0, "description": "1-based line number, 0 for the whole file." },
                    "blocking": { "type": "boolean", "description": "True if this must be fixed before the change can merge." },
                    "text": { "type": "string", "description": "What is wrong and how to fix it." },
                },
                "required": ["file", "line", "blocking", "text"],
                "additionalProperties": false,
            },
        },
        {
            "name": "list_notes",
            "description": "List the review notes the user left on your changes, with every comment so far. Only open notes unless include_resolved is true.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_resolved": { "type": "boolean", "description": "Also list resolved notes." }
                },
                "additionalProperties": false,
            },
        },
        {
            "name": "reply_to_note",
            "description": "Answer one review note by its id. The reply appears in the note's thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "minimum": 1, "description": "The note id from list_notes." },
                    "text": { "type": "string", "description": "Your reply." },
                },
                "required": ["id", "text"],
                "additionalProperties": false,
            },
        },
        question::tool_definition(),
        update_progress,
        get_progress,
        workflow_mcp::tool_definition(),
        job_mcp::tool_definition(),
        avatar_mcp::tool_definition(),
        meeseeks_mcp::tool_definition(),
        tab_name::tool_definition(),
    ]);
    if let Some(list) = tools.as_array_mut() {
        list.extend(services_mcp::tool_definitions());
    }
    tools
}

fn call_tool(
    params: Option<&Value>,
    ctx: &ToolCtx<'_>,
    emit: &dyn Fn(TabEvent),
) -> Result<Value, RpcError> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| RpcError::params("tools/call needs an object with a tool name"))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::params("tools/call needs a string name"))?;
    let empty = Map::new();
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(args)) => args,
        Some(_) => return Err(RpcError::params("arguments must be an object")),
    };
    if !ctx.offer().offers(name) {
        return Ok(tool_result(&Offer::refusal(name), true));
    }
    if name == progress_mcp::UPDATE_TOOL {
        let (mut text, is_error) = progress_mcp::update(args, ctx.progress_file, emit);
        if !is_error {
            for line in (ctx.after_save)() {
                text.push('\n');
                text.push_str(&line);
            }
        }
        return Ok(tool_result(&text, is_error));
    }
    if name == progress_mcp::GET_TOOL {
        let (text, is_error) = progress_mcp::get(ctx.progress_file);
        return Ok(tool_result(&text, is_error));
    }
    if name == workflow_mcp::GET_TOOL {
        let (text, is_error) = workflow_mcp::get((ctx.workflow)());
        return Ok(tool_result(&text, is_error));
    }
    if name == avatar_mcp::SET_TOOL {
        let (text, is_error) = avatar_mcp::set(args, ctx.notes_file, emit);
        return Ok(tool_result(&text, is_error));
    }
    if name == meeseeks_mcp::DONE_TOOL {
        let (text, is_error) = meeseeks_mcp::done(args, emit);
        return Ok(tool_result(&text, is_error));
    }
    if name == tab_name::SET_TOOL {
        let (text, is_error) = tab_name::set(args, emit);
        return Ok(tool_result(&text, is_error));
    }
    if name == job_mcp::REPORT_TOOL {
        let (text, is_error) = job_mcp::report(args, emit);
        return Ok(tool_result(&text, is_error));
    }
    if name == question::TOOL_NAME {
        let (text, is_error) = ask_user(args, &ctx.ask, emit);
        return Ok(tool_result(&text, is_error));
    }
    if services_mcp::is_tool(name) {
        let (text, is_error) = services_mcp::call(name, args, emit);
        return Ok(tool_result(&text, is_error));
    }
    let text = match name {
        "report_finding" => report_finding(args, emit)?,
        "list_notes" => list_notes(args, ctx),
        "reply_to_note" => reply_to_note(args, ctx, emit)?,
        other => return Err(RpcError::params(format!("unknown tool: {other}"))),
    };
    Ok(tool_result(&text, false))
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Shows the questions in the tab and holds the call until they are answered. Questions that fail
/// the rules are a tool error with the problems listed, so the agent fixes them and asks again.
fn ask_user(
    args: &Map<String, Value>,
    wait: &AskWait<'_>,
    emit: &dyn Fn(TabEvent),
) -> (String, bool) {
    let questions = match question::from_tool_args(args) {
        Ok(questions) => questions,
        Err(found) => {
            return (
                format!(
                    "The questions were not shown. Fix these and call {} again:\n- {}",
                    question::TOOL_NAME,
                    found.join("\n- ")
                ),
                true,
            );
        }
    };
    let (sender, receiver) = channel();
    let reply = AskReply::new(sender);
    let end = reply.end_mark();
    emit(TabEvent::Ask { questions: questions.clone(), reply });
    let started = Instant::now();
    let text = loop {
        match receiver.recv_timeout(wait.poll) {
            Ok(Reply::Answered(given)) => break question::answer_text(&questions, &given),
            Ok(Reply::PickedForYou { given, minutes }) => {
                break question_words::picked_for_you(&questions, &given, minutes);
            }
            Ok(Reply::Skipped) => break question_words::SKIPPED.to_owned(),
            Err(RecvTimeoutError::Disconnected) => break question::CLOSED_TEXT.to_owned(),
            Err(RecvTimeoutError::Timeout) => {
                if started.elapsed() >= wait.timeout || !(wait.alive)() {
                    break question::TIMED_OUT_TEXT.to_owned();
                }
            }
        }
    };
    end.mark();
    (text, false)
}

fn report_finding(args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> Result<String, RpcError> {
    let file = text_arg(args, "file")?;
    let line = args
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| RpcError::params("line must be a non-negative integer"))?;
    let blocking = args
        .get("blocking")
        .and_then(Value::as_bool)
        .ok_or_else(|| RpcError::params("blocking must be true or false"))?;
    let text = text_arg(args, "text")?;
    let message = format!("Recorded finding for {file}:{line}.");
    emit(TabEvent::Finding { file, line, blocking, text });
    Ok(message)
}

fn list_notes(args: &Map<String, Value>, ctx: &ToolCtx<'_>) -> String {
    let include_resolved = args.get("include_resolved").and_then(Value::as_bool).unwrap_or(false);
    let notes = {
        let _guard = ctx.notes_lock.lock().unwrap_or_else(PoisonError::into_inner);
        Notes::load(ctx.notes_file)
    };
    let threads: Vec<Value> = notes
        .threads
        .iter()
        .filter(|t| include_resolved || !t.resolved)
        .map(|t| {
            json!({
                "id": t.id,
                "file": t.path,
                "line": t.anchor.line_hint,
                "resolved": t.resolved,
                "comments": t.comments.iter().map(|c| json!({ "author": c.author, "body": c.body })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(threads).to_string()
}

fn reply_to_note(
    args: &Map<String, Value>,
    ctx: &ToolCtx<'_>,
    emit: &dyn Fn(TabEvent),
) -> Result<String, RpcError> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| RpcError::params("id must be a positive integer"))?;
    let text = text_arg(args, "text")?;
    {
        let _guard = ctx.notes_lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notes = Notes::load(ctx.notes_file);
        if !notes.threads.iter().any(|t| t.id == id) {
            return Err(RpcError::params(format!("no note with id {id}")));
        }
        let comment =
            Comment { author: ctx.agent.to_owned(), time: unix_now(), body: text.clone() };
        notes.reply(id, comment);
        notes
            .save(ctx.notes_file)
            .map_err(|e| RpcError::new(INTERNAL_ERROR, format!("could not save the reply: {e}")))?;
    }
    emit(TabEvent::NoteReply { note_id: id, text });
    Ok(format!("Reply added to note {id}."))
}

fn text_arg(args: &Map<String, Value>, key: &str) -> Result<String, RpcError> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| RpcError::params(format!("{key} must be a non-empty string")))?;
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(RpcError::params(format!("{key} is longer than {MAX_TEXT_CHARS} characters")));
    }
    Ok(text.to_owned())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{self, Ledger};
    use crate::progress_store;
    use kelp_core::review::{Anchor, Side};
    use std::cell::RefCell;

    fn test_wait(alive: &dyn Fn() -> bool) -> AskWait<'_> {
        AskWait { timeout: Duration::from_millis(400), poll: Duration::from_millis(10), alive }
    }

    struct Harness {
        dir: tempfile::TempDir,
        lock: Mutex<()>,
        events: RefCell<Vec<TabEvent>>,
        role: TabRole,
        agent: &'static str,
        workflow: Workflow,
    }

    impl Harness {
        fn new() -> Self {
            Self::of(TabRole::Agent)
        }

        fn of(role: TabRole) -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                lock: Mutex::new(()),
                events: RefCell::new(vec![]),
                role,
                agent: "claude",
                workflow: Workflow::default(),
            }
        }

        fn running(self, agent: &'static str) -> Self {
            Self { agent, ..self }
        }

        fn notes_file(&self) -> std::path::PathBuf {
            self.dir.path().join("repo-abc123").join("tab").join("notes.json")
        }

        fn progress_file(&self) -> std::path::PathBuf {
            self.dir.path().join("repo-abc123").join("tab").join("progress.md")
        }

        fn post(&self, body: &str) -> McpReply {
            let notes_file = self.notes_file();
            let alive = || true;
            let progress_file = self.progress_file();
            let workflow = || self.workflow;
            let ctx = ToolCtx {
                agent: self.agent,
                role: self.role,
                notes_file: &notes_file,
                notes_lock: &self.lock,
                progress_file: &progress_file,
                ask: test_wait(&alive),
                workflow: &workflow,
                after_save: &Vec::new,
            };
            handle_post(body.as_bytes(), &ctx, &|event| self.events.borrow_mut().push(event))
        }

        fn json(&self, body: &Value) -> Value {
            match self.post(&body.to_string()) {
                McpReply::Json(text) => serde_json::from_str(&text).unwrap(),
                McpReply::Accepted => panic!("expected a JSON reply to {body}"),
            }
        }

        fn call(&self, tool: &str, args: &Value) -> Value {
            self.json(&json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                "params": {"name": tool, "arguments": args}}))
        }

        fn seed_note(&self, body: &str) -> u64 {
            let mut notes = Notes::load(&self.notes_file());
            let anchor = Anchor {
                side: Side::New,
                line_hint: 12,
                text: "let x = 1;".into(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            };
            let id = notes.add(
                "src/lib.rs",
                None,
                anchor,
                Comment { author: "You".into(), time: 1, body: body.into() },
            );
            notes.save(&self.notes_file()).unwrap();
            id
        }
    }

    fn error_code(reply: &Value) -> i64 {
        reply["error"]["code"].as_i64().unwrap_or_else(|| panic!("no error in {reply}"))
    }

    #[test]
    fn initialize_echoes_a_supported_version_and_names_the_server() {
        let h = Harness::new();
        for (asked, want) in [
            ("2025-06-18", "2025-06-18"),
            ("2025-03-26", "2025-03-26"),
            ("2024-11-05", "2024-11-05"),
            ("1999-01-01", PROTOCOL_VERSION),
        ] {
            let reply = h.json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": asked, "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}}));
            assert_eq!(reply["id"], 1, "{asked}");
            assert_eq!(reply["result"]["protocolVersion"], want, "{asked}");
            assert_eq!(reply["result"]["serverInfo"]["name"], "tidewatch");
            assert_eq!(reply["result"]["capabilities"]["tools"]["listChanged"], false);
        }
        let bare = h.json(&json!({"jsonrpc": "2.0", "id": "a", "method": "initialize"}));
        assert_eq!(bare["id"], "a");
        assert_eq!(bare["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn notifications_are_accepted_without_a_body() {
        let h = Harness::new();
        for body in [
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
            json!({"jsonrpc": "2.0", "id": 4, "result": {}}),
        ] {
            assert_eq!(h.post(&body.to_string()), McpReply::Accepted, "{body}");
        }
        assert_eq!(h.events.borrow().len(), 0);
    }

    #[test]
    fn ping_answers_with_an_empty_result() {
        let reply = Harness::new().json(&json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}));
        assert_eq!(reply["result"], json!({}));
    }

    #[test]
    fn the_server_has_sixteen_tools_with_object_schemas_and_required_fields() {
        let all = tool_definitions();
        let tools = all.as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "report_finding",
                "list_notes",
                "reply_to_note",
                "ask_user",
                "update_progress",
                "get_progress",
                "get_workflow",
                "report_job",
                "set_avatar",
                "meeseeks_done",
                "set_name",
                "list_services",
                "add_service",
                "restart_service",
                "stop_service",
                "service_log"
            ]
        );
        let required: Vec<Vec<&str>> = tools
            .iter()
            .map(|t| {
                t["inputSchema"]["required"]
                    .as_array()
                    .map(|r| r.iter().map(|v| v.as_str().unwrap()).collect())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(
            required,
            [
                vec!["file", "line", "blocking", "text"],
                vec![],
                vec!["id", "text"],
                vec!["questions"],
                vec!["state", "now", "stages"],
                vec![],
                vec![],
                vec!["name"],
                vec!["reason"],
                vec!["summary"],
                vec!["name"],
                vec![],
                vec!["name", "command"],
                vec!["name"],
                vec!["name"],
                vec!["name"],
            ]
        );
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
            assert!(tool["description"].as_str().unwrap().len() > 20, "{}", tool["name"]);
        }
    }

    fn offered_by(h: &Harness) -> Vec<String> {
        let reply = h.json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
        let tools = reply["result"]["tools"].as_array().unwrap();
        tools.iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect()
    }

    fn offered(role: TabRole) -> Vec<String> {
        offered_by(&Harness::of(role))
    }

    #[test]
    fn a_tab_is_offered_only_the_tools_its_role_can_use() {
        let shared = [
            "list_notes",
            "reply_to_note",
            "update_progress",
            "get_progress",
            "get_workflow",
            "report_job",
            "set_avatar",
            "set_name",
            "list_services",
            "add_service",
            "restart_service",
            "stop_service",
            "service_log",
        ];
        let with = |extra: &[&str]| -> Vec<String> {
            let mut names: Vec<String> = extra.iter().map(|s| (*s).to_owned()).collect();
            names.extend(shared.iter().map(|s| (*s).to_owned()));
            names.sort();
            names
        };
        let sorted = |role| {
            let mut names = offered(role);
            names.sort();
            names
        };
        assert_eq!(sorted(TabRole::Agent), with(&["report_finding"]));
        assert_eq!(sorted(TabRole::Meeseeks), with(&["meeseeks_done"]));
    }

    #[test]
    fn ask_user_is_offered_only_to_a_cli_without_a_question_tool_tidewatch_reads() {
        let cases = [
            ("claude", false),
            ("codex", true),
            ("gemini", true),
            ("copilot", true),
            ("aider", true),
        ];
        for (agent, want) in cases {
            let h = Harness::new().running(agent);
            assert_eq!(offered_by(&h).iter().any(|name| name == "ask_user"), want, "{agent}");
        }
        let h = Harness::new().running("claude");
        let reply = h.call("ask_user", &questions_args());
        assert_eq!(reply["result"]["isError"], true);
        assert!(
            text_of(&reply).contains(
                "`ask_user` is not offered to this CLI: it has a question tool of its own"
            ),
            "{reply}"
        );
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
    }

    #[test]
    fn set_avatar_is_listed_and_a_fresh_tab_with_no_click_can_call_it() {
        let args = json!({"svg": "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2dd4bf\"/></svg>", "reason": "a dot"});
        for role in [TabRole::Agent, TabRole::Meeseeks] {
            assert!(offered(role).iter().any(|name| name == "set_avatar"), "{role:?}");
        }
        let h = Harness::new();
        assert_eq!(h.call("set_avatar", &args)["result"]["isError"], false);
        assert_eq!(h.events.borrow().len(), 1);
    }

    #[test]
    fn get_workflow_gives_back_the_repos_profile_to_every_role() {
        use crate::workflow_profile::{CatchUp, Fixes, PullRequests};
        let workflow = Workflow {
            catch_up: CatchUp::Merge,
            fixes: Fixes::NewCommits,
            pull_requests: PullRequests::Ready,
            ..Workflow::default()
        };
        for role in [TabRole::Agent, TabRole::Meeseeks] {
            let h = Harness { workflow, ..Harness::of(role) };
            let reply = h.call("get_workflow", &json!({}));
            assert_eq!(reply["result"]["isError"], false, "{role:?}");
            let profile: Value = serde_json::from_str(text_of(&reply)).unwrap();
            assert_eq!(
                (&profile["catch_up"], &profile["fixes"], &profile["pull_requests"]),
                (&json!("merge"), &json!("new-commits"), &json!("ready")),
                "{role:?}"
            );
            assert_eq!(profile["rules"], json!(workflow.launch_lines()), "{role:?}");
        }
    }

    #[test]
    fn a_call_to_a_tool_the_tab_is_not_offered_is_a_tool_error_that_says_where_it_works() {
        let cases = [
            (TabRole::Agent, "meeseeks_done", json!({"summary": "x"}), "only a Meeseeks"),
            (
                TabRole::Meeseeks,
                "report_finding",
                json!({"file": "a", "line": 1, "blocking": false, "text": "t"}),
                "not offered to a Meeseeks",
            ),
        ];
        for (role, tool, args, says) in cases {
            let h = Harness::of(role);
            let reply = h.call(tool, &args);
            assert_eq!(reply["result"]["isError"], true, "{role:?} {tool}");
            let text = reply["result"]["content"][0]["text"].as_str().unwrap();
            assert!(
                text.contains(&format!("`{tool}` is not offered")) && text.contains(says),
                "{text}"
            );
            assert_eq!(h.events.borrow().len(), 0, "{role:?} {tool}");
        }
    }

    const LEDGER: &str = "# Goal\nAdd search\n\n## Status\nphase: 2\nstate: working\nnote: wiring\n\n## Checklist\n### 1. Plan\n- [x] a\n### 2. Build\n- [x] b\n- [~] c\n- [ ] d\n\n## Scheduled\n- [ ] you -- Pick a name\n";

    fn text_of(reply: &Value) -> &str {
        reply["result"]["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn update_progress_with_the_old_ledger_field_is_a_tool_error_that_names_the_typed_fields() {
        let h = Harness::new();
        let reply = h.call("update_progress", &json!({ "ledger": LEDGER }));
        assert!(reply.get("error").is_none(), "a rule failure is not a protocol error: {reply}");
        assert_eq!(reply["result"]["isError"], true);
        let text = text_of(&reply);
        assert!(
            text.starts_with(
                "The report was not saved. Fix these and call update_progress again:\n- the `ledger` field is gone: send the typed fields, state, now and stages"
            ),
            "{text}"
        );
        assert!(!h.progress_file().exists());
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
    }

    fn typed() -> Value {
        json!({
            "goal": "Fix the retry",
            "state": "needs_you",
            "now": "Waiting for the cap",
            "stages": [
                { "name": "Build", "kind": "build", "status": "active" },
                { "name": "Ship", "kind": "ship", "status": "todo" }
            ],
            "steps": [{ "stage": "Build", "text": "Add backoff", "status": "active" }],
            "needs_you": { "question": "Cap?", "options": ["30s", "60s"], "recommended": 1 }
        })
    }

    #[test]
    fn update_progress_takes_the_typed_fields_stores_the_report_and_shows_the_ledger() {
        let h = Harness::new();
        let reply = h.call("update_progress", &typed());
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert_eq!(text_of(&reply), "saved: phase 1 Build, 0/1 done");
        let stored = progress_store::load_report(&h.progress_file()).unwrap();
        assert_eq!(stored.state, progress::State::NeedsYou);
        assert!(h.progress_file().with_file_name("progress.json").exists());
        let events = h.events.borrow();
        let [TabEvent::Progress { ledger }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(**ledger, stored.to_ledger());
        assert!(ledger.has_question());
    }

    #[test]
    fn a_typed_report_that_breaks_a_rule_is_refused_with_each_fix_and_saves_nothing() {
        let h = Harness::new();
        let mut bad = typed();
        bad["now"] = json!("x".repeat(130));
        bad["stages"][0]["kind"] = json!("deploy");
        bad["needs_you"]["recommended"] = json!(5);
        let reply = h.call("update_progress", &bad);
        assert!(reply.get("error").is_none(), "a rule failure is not a protocol error: {reply}");
        assert_eq!(reply["result"]["isError"], true);
        let text = text_of(&reply);
        assert!(
            text.starts_with(
                "The report was not saved. Fix these and call update_progress again:\n- "
            ),
            "{text}"
        );
        assert_eq!(text.lines().count(), 4, "{text}");
        assert!(!h.progress_file().exists());
        assert!(!h.progress_file().with_file_name("progress.json").exists());
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
    }

    #[test]
    fn typed_fields_and_a_markdown_ledger_together_are_refused_and_save_nothing() {
        let h = Harness::new();
        let mut both = typed();
        both["ledger"] = json!(LEDGER);
        let reply = h.call("update_progress", &both);
        assert_eq!(reply["result"]["isError"], true);
        assert!(text_of(&reply).contains("the `ledger` field is gone"));
        assert!(!h.progress_file().exists());
    }

    #[test]
    fn the_second_typed_report_may_leave_the_goal_out_and_keeps_it() {
        let h = Harness::new();
        h.call("update_progress", &typed());
        let mut later = typed();
        later.as_object_mut().unwrap().remove("goal");
        later["state"] = json!("working");
        later.as_object_mut().unwrap().remove("needs_you");
        let reply = h.call("update_progress", &later);
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let stored = progress_store::load_report(&h.progress_file()).unwrap();
        assert_eq!(stored.goal, "Fix the retry");
        assert_eq!(stored.state, progress::State::Working);
    }

    #[test]
    fn the_update_progress_schema_lists_every_typed_field_and_the_words_of_each_enum() {
        let [update, _] = progress_mcp::tool_definitions();
        let schema = &update["inputSchema"];
        let props = schema["properties"].as_object().unwrap();
        for key in [
            "goal",
            "title",
            "state",
            "now",
            "kind",
            "stages",
            "steps",
            "needs_you",
            "waiting_on",
            "scheduled",
            "verified",
            "not_verified",
            "decisions",
        ] {
            assert!(props.contains_key(key), "{key}");
        }
        assert!(!props.contains_key("ledger"));
        assert_eq!(schema["required"], json!(["state", "now", "stages"]));
        assert_eq!(
            props["state"]["enum"],
            json!(["working", "needs_you", "waiting", "blocked", "done"])
        );
        let stage = &props["stages"]["items"]["properties"];
        assert_eq!(
            stage["kind"]["enum"],
            json!(["understand", "build", "ship", "verify", "research"])
        );
        assert_eq!(stage["status"]["enum"], json!(["todo", "active", "done", "skipped"]));
        assert_eq!(
            props["steps"]["items"]["properties"]["status"]["enum"],
            json!(["todo", "active", "done", "skipped", "blocked"])
        );
        assert_eq!(props["now"]["maxLength"], 120);
        assert_eq!(props["stages"]["minItems"], 2);
        assert_eq!(props["stages"]["maxItems"], 5);
        assert_eq!(props["needs_you"]["properties"]["options"]["minItems"], 2);
        assert_eq!(props["needs_you"]["properties"]["options"]["maxItems"], 4);
    }

    #[test]
    fn set_avatar_over_rpc_saves_beside_the_notes_and_shows_it_without_the_script() {
        let h = Harness::new();
        let svg = "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2dd4bf\"/><script>1</script></svg>";
        let reply = h.call("set_avatar", &json!({"svg": svg, "reason": "a teal dot"}));
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let events = h.events.borrow();
        let [TabEvent::Avatar { avatar }] = events.as_slice() else { panic!("{events:?}") };
        assert!(avatar.svg.contains("<circle") && !avatar.svg.contains("script"));
        let saved = crate::avatar_store::load(&h.notes_file().with_file_name("avatar.json"));
        assert_eq!(saved.as_ref(), Some(avatar));
        drop(events);
        let bad = h.call("set_avatar", &json!({"svg": "<svg><script/></svg>", "reason": "x"}));
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(h.events.borrow().len(), 1);
    }

    #[test]
    fn a_service_tool_over_rpc_is_answered_by_the_app_and_a_bad_call_never_reaches_it() {
        let h = Harness::new();
        let notes_file = h.notes_file();
        let progress_file = h.progress_file();
        let alive = || true;
        let ctx = ToolCtx {
            agent: "claude",
            role: TabRole::Agent,
            notes_file: &notes_file,
            notes_lock: &h.lock,
            progress_file: &progress_file,
            ask: test_wait(&alive),
            workflow: &Workflow::default,
            after_save: &Vec::new,
        };
        let asked = RefCell::new(Vec::new());
        let emit = |event: TabEvent| {
            let TabEvent::Services(call) = event else { panic!("a service event") };
            asked.borrow_mut().push(call.call.clone());
            assert!(call.reply.send(Ok("Stopped db.".into())));
        };
        let post = |tool: &str, args: Value| {
            let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": tool, "arguments": args}});
            let McpReply::Json(text) = handle_post(body.to_string().as_bytes(), &ctx, &emit) else {
                panic!("a JSON reply")
            };
            serde_json::from_str::<Value>(&text).unwrap()
        };
        let reply = post("stop_service", json!({"name": "db"}));
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert_eq!(text_of(&reply), "Stopped db.");
        let bad = post("stop_service", json!({}));
        assert_eq!(bad["result"]["isError"], true);
        assert!(text_of(&bad).contains("name is required"), "{bad}");
        assert_eq!(*asked.borrow(), [services_mcp::Call::Stop("db".into())]);
    }

    #[test]
    fn meeseeks_done_over_rpc_tells_the_tab_and_a_bad_call_is_a_tool_error() {
        let h = Harness::of(TabRole::Meeseeks);
        let reply = h.call("meeseeks_done", &json!({"summary": "Renamed the helper"}));
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let events = h.events.borrow();
        assert_eq!(*events, [TabEvent::MeeseeksDone { summary: "Renamed the helper".into() }]);
        drop(events);
        for bad in [json!({}), json!({"summary": 4}), json!({"summary": "x", "pr": 1})] {
            let reply = h.call("meeseeks_done", &bad);
            assert_eq!(reply["result"]["isError"], true, "{bad}");
        }
        assert_eq!(h.events.borrow().len(), 1);
    }

    #[test]
    fn set_name_over_rpc_tells_the_tab_of_any_role_and_a_bad_name_is_a_tool_error() {
        for role in [TabRole::Agent, TabRole::Meeseeks] {
            let h = Harness::of(role);
            let reply = h.call("set_name", &json!({"name": "fix-https-validation"}));
            assert_eq!(reply["result"]["isError"], false, "{role:?} {reply}");
            let events = h.events.borrow();
            assert_eq!(*events, [TabEvent::TabName { name: "fix-https-validation".into() }]);
            drop(events);
            for bad in [json!({}), json!({"name": "Fix Login"}), json!({"name": "login"})] {
                let reply = h.call("set_name", &bad);
                assert_eq!(reply["result"]["isError"], true, "{role:?} {bad}");
            }
            assert_eq!(h.events.borrow().len(), 1, "{role:?}");
        }
    }

    #[test]
    fn report_job_over_rpc_shows_the_job_and_a_bad_call_is_a_tool_error() {
        let h = Harness::new();
        let reply = h.call("report_job", &json!({"name": "PR checks", "done": 3, "total": 5}));
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let events = h.events.borrow();
        let [TabEvent::Job { report }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((report.done, report.total), (Some(3.0), Some(5.0)));
        drop(events);
        let bad = h.call("report_job", &json!({"done": 3}));
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(h.events.borrow().len(), 1);
    }

    #[test]
    fn get_progress_gives_back_the_saved_text_and_says_so_when_there_is_none() {
        let h = Harness::new();
        let reply = h.call("get_progress", &json!({}));
        assert_eq!(reply["result"]["isError"], false);
        assert!(text_of(&reply).starts_with("No report has been saved"), "{reply}");
        h.call("update_progress", &typed());
        let reply = h.call("get_progress", &json!({}));
        let back: Value = serde_json::from_str(text_of(&reply)).unwrap();
        assert_eq!(back["state"], "needs_you");
        assert_eq!(back["needs_you"]["recommended"], 1);
    }

    #[test]
    fn get_progress_maps_a_markdown_only_file_from_an_older_version_to_the_typed_report() {
        let h = Harness::new();
        std::fs::create_dir_all(h.progress_file().parent().unwrap()).unwrap();
        std::fs::write(h.progress_file(), LEDGER).unwrap();
        let reply = h.call("get_progress", &json!({}));
        let back: Value = serde_json::from_str(text_of(&reply)).unwrap();
        assert_eq!(
            back["stages"].as_array().map(Vec::len),
            Some(Ledger::parse(LEDGER).checklist.len())
        );
        assert!(back["now"].is_string());
    }

    #[test]
    fn update_progress_rejects_only_what_the_agent_can_fix_and_saves_nothing() {
        let h = Harness::new();
        let big = "y".repeat(progress::MAX_LEDGER_BYTES);
        let cases = [
            (json!({}), "state is required"),
            (json!({ "ledger": 5 }), "the `ledger` field is gone"),
            (json!({ "goal": big }), "over the 32 KB limit"),
            (json!({ "extra": 1 }), "unknown argument \"extra\""),
        ];
        for (args, want) in cases {
            let reply = h.call("update_progress", &args);
            assert!(
                reply.get("error").is_none(),
                "a rule failure is not a protocol error: {reply}"
            );
            assert_eq!(reply["result"]["isError"], true, "{args}");
            let text = text_of(&reply);
            assert!(
                text.starts_with(
                    "The report was not saved. Fix these and call update_progress again:\n- "
                ),
                "{text}"
            );
            assert!(text.contains(want), "{want}: {text}");
        }
        assert!(!h.progress_file().exists());
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
    }

    fn questions_args() -> Value {
        json!({"questions": [
            {"question": "Where should the retry count live?", "header": "Retry",
             "options": [{"label": "On the stage (Recommended)"}, {"label": "On the run"}]},
            {"question": "Which tests should retry?", "multi_select": true,
             "options": [{"label": "Unit"}, {"label": "Integration"}, {"label": "Lints"}]},
        ]})
    }

    /// Calls `ask_user` and lets `respond` use the reply channel the moment the question is shown.
    fn ask_with(
        args: &Value,
        alive: &dyn Fn() -> bool,
        respond: impl Fn(&AskReply),
    ) -> (Value, Vec<TabEvent>) {
        let h = Harness::new();
        let notes_file = h.notes_file();
        let ctx = ToolCtx {
            agent: "codex",
            role: TabRole::Agent,
            notes_file: &notes_file,
            notes_lock: &h.lock,
            progress_file: Path::new("/none/progress.md"),
            ask: test_wait(alive),
            workflow: &Workflow::default,
            after_save: &Vec::new,
        };
        let shown = RefCell::new(Vec::new());
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "ask_user", "arguments": args}});
        let reply = handle_post(body.to_string().as_bytes(), &ctx, &|event| {
            if let TabEvent::Ask { reply, .. } = &event {
                respond(reply);
            }
            shown.borrow_mut().push(event);
        });
        let McpReply::Json(text) = reply else { panic!("no reply") };
        (serde_json::from_str(&text).unwrap(), shown.into_inner())
    }

    fn result_text(reply: &Value) -> &str {
        reply["result"]["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn ask_user_shows_the_questions_and_returns_the_answers_as_text() {
        let given = vec![
            question::Given { picked: vec![0], other: None },
            question::Given { picked: vec![0, 2], other: Some("doc tests".into()) },
        ];
        let (reply, shown) = ask_with(&questions_args(), &|| true, |held| {
            assert!(held.send(Reply::Answered(given.clone())));
        });
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(
            result_text(&reply),
            "Q: Where should the retry count live?\nA: On the stage\n\nQ: Which tests should retry?\nA: Unit, Lints, doc tests"
        );
        let [TabEvent::Ask { questions, .. }] = shown.as_slice() else { panic!("{shown:?}") };
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].header.as_deref(), Some("Retry"));
        assert_eq!(questions[1].select, question::Select::Multi);
    }

    #[test]
    fn ask_user_tells_the_agent_what_tidewatch_picked_for_a_user_who_stayed_away() {
        let (reply, shown) = ask_with(&questions_args(), &|| true, |held| {
            let given = vec![
                question::Given { picked: vec![0], other: None },
                question::Given { picked: vec![1], other: None },
            ];
            assert!(held.send(Reply::PickedForYou { given, minutes: 5 }));
        });
        assert_eq!(reply["result"]["isError"], false);
        assert!(
            result_text(&reply).starts_with(
                "Tidewatch picked these answers for you after 5 minutes. The user may change them."
            ),
            "{}",
            result_text(&reply)
        );
        assert!(
            result_text(&reply).contains("Q: Where should the retry count live?\nA: On the stage")
        );
        assert_eq!(shown.len(), 1);
    }

    #[test]
    fn ask_user_tells_the_agent_when_the_user_skipped() {
        let (reply, _) = ask_with(&questions_args(), &|| true, |held| {
            assert!(held.send(Reply::Skipped));
        });
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(result_text(&reply), question_words::SKIPPED);
    }

    #[test]
    fn ask_user_gives_up_after_the_timeout_and_the_late_answer_has_nowhere_to_go() {
        let kept: RefCell<Option<AskReply>> = RefCell::new(None);
        let started = Instant::now();
        let (reply, _) = ask_with(&questions_args(), &|| true, |held| {
            *kept.borrow_mut() = Some(held.clone());
        });
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(result_text(&reply), question::TIMED_OUT_TEXT);
        assert!(kept.borrow().as_ref().unwrap().has_ended(), "the card learns the call is over");
        assert!(!kept.borrow().as_ref().unwrap().send(Reply::Skipped));
    }

    #[test]
    fn ask_user_stops_waiting_when_the_client_hangs_up() {
        let kept: RefCell<Option<AskReply>> = RefCell::new(None);
        let started = Instant::now();
        let (reply, _) = ask_with(&questions_args(), &|| false, |held| {
            *kept.borrow_mut() = Some(held.clone());
        });
        assert!(started.elapsed() < Duration::from_millis(300));
        assert_eq!(result_text(&reply), question::TIMED_OUT_TEXT);
        assert!(kept.borrow().as_ref().unwrap().has_ended());
    }

    #[test]
    fn ask_user_says_so_when_the_question_is_dropped_unanswered() {
        let h = Harness::new();
        let notes_file = h.notes_file();
        let alive = || true;
        let ctx = ToolCtx {
            agent: "codex",
            role: TabRole::Agent,
            notes_file: &notes_file,
            notes_lock: &h.lock,
            progress_file: Path::new("/none/progress.md"),
            ask: test_wait(&alive),
            workflow: &Workflow::default,
            after_save: &Vec::new,
        };
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "ask_user", "arguments": questions_args()}});
        let McpReply::Json(text) = handle_post(body.to_string().as_bytes(), &ctx, &|_| {}) else {
            panic!("no reply")
        };
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result_text(&reply), question::CLOSED_TEXT);
    }

    #[test]
    fn ask_user_answers_bad_questions_as_a_tool_error_and_shows_nothing() {
        let bad = json!({"questions": [{"question": "Q?", "options": [{"label": "only"}]}]});
        let (reply, shown) = ask_with(&bad, &|| true, |_| panic!("nothing should be shown"));
        assert_eq!(reply["result"]["isError"], true);
        let text = result_text(&reply);
        assert!(
            text.starts_with(
                "The questions were not shown. Fix these and call ask_user again:\n- "
            ),
            "{text}"
        );
        assert!(text.contains("question 1: give 2 to 6 options, not 1"), "{text}");
        assert_eq!(shown, Vec::<TabEvent>::new());
    }

    #[test]
    fn report_finding_emits_every_field() {
        let h = Harness::new();
        let reply = h.call(
            "report_finding",
            &json!({"file": "src/a.rs", "line": 42, "blocking": true, "text": "off by one"}),
        );
        assert_eq!(reply["result"]["content"][0]["text"], "Recorded finding for src/a.rs:42.");
        assert_eq!(
            h.events.borrow().clone(),
            [TabEvent::Finding {
                file: "src/a.rs".into(),
                line: 42,
                blocking: true,
                text: "off by one".into()
            }]
        );
    }

    #[test]
    fn bad_arguments_are_invalid_params_and_emit_nothing() {
        let h = Harness::of(TabRole::Agent);
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let cases: Vec<(&str, Value)> = vec![
            ("report_finding", json!({})),
            ("report_finding", json!({"file": "a", "line": 1, "blocking": true, "text": ""})),
            ("report_finding", json!({"file": "a", "line": 1, "blocking": true, "text": "   "})),
            ("report_finding", json!({"file": "a", "line": 1, "blocking": true, "text": 5})),
            ("report_finding", json!({"file": "a", "line": 1, "blocking": true, "text": long})),
            ("report_finding", json!({"file": "a", "line": -1, "blocking": true, "text": "t"})),
            ("report_finding", json!({"file": "a", "line": 1.5, "blocking": true, "text": "t"})),
            (
                "report_finding",
                json!({"file": "a", "line": 99_999_999_999_u64, "blocking": true, "text": "t"}),
            ),
            ("report_finding", json!({"file": "a", "line": "3", "blocking": true, "text": "t"})),
            ("report_finding", json!({"file": "a", "line": 3, "blocking": "yes", "text": "t"})),
            ("report_finding", json!({"file": "a", "line": 3, "text": "t"})),
            ("report_finding", json!({"file": "", "line": 3, "blocking": false, "text": "t"})),
            ("report_finding", json!({"file": "a", "line": 3, "blocking": false})),
            ("reply_to_note", json!({"id": "1", "text": "t"})),
            ("reply_to_note", json!({"id": 1})),
            ("reply_to_note", json!({"id": 404, "text": "t"})),
            ("no_such_tool", json!({})),
        ];
        for (tool, args) in cases {
            let reply = h.call(tool, &args);
            assert_eq!(error_code(&reply), INVALID_PARAMS, "{tool} {args}");
            assert_eq!(reply["id"], 9, "{tool} {args}");
        }
        assert_eq!(h.events.borrow().len(), 0);
        assert!(!h.notes_file().exists());
    }

    #[test]
    fn tools_call_with_a_broken_envelope_is_invalid_params() {
        let h = Harness::new();
        for params in [
            json!(null),
            json!([]),
            json!({}),
            json!({"name": 4}),
            json!({"name": "list_notes", "arguments": "x"}),
            json!({"name": "list_notes", "arguments": [1]}),
        ] {
            let reply = h.json(
                &json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": params}),
            );
            assert_eq!(error_code(&reply), INVALID_PARAMS, "{params}");
        }
        let missing = h.json(&json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call"}));
        assert_eq!(error_code(&missing), INVALID_PARAMS);
    }

    #[test]
    fn list_notes_is_empty_without_a_file_and_never_creates_one() {
        let h = Harness::new();
        let reply = h.call("list_notes", &json!({}));
        assert_eq!(reply["result"]["content"][0]["text"], "[]");
        assert!(!h.notes_file().exists());
    }

    #[test]
    fn notes_round_trip_through_the_tabs_notes_file() {
        let h = Harness::new();
        let first = h.seed_note("use formatDate");
        let second = h.seed_note("rename this");
        let listed = h.call("list_notes", &json!({}));
        let text = listed["result"]["content"][0]["text"].as_str().unwrap();
        let threads: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            threads,
            json!([
                {"id": first, "file": "src/lib.rs", "line": 12, "resolved": false,
                    "comments": [{"author": "You", "body": "use formatDate"}]},
                {"id": second, "file": "src/lib.rs", "line": 12, "resolved": false,
                    "comments": [{"author": "You", "body": "rename this"}]},
            ])
        );

        let reply = h.call("reply_to_note", &json!({"id": first, "text": "done, see commit"}));
        assert_eq!(reply["result"]["content"][0]["text"], format!("Reply added to note {first}."));
        assert_eq!(
            h.events.borrow().clone(),
            [TabEvent::NoteReply { note_id: first, text: "done, see commit".into() }]
        );

        let on_disk = Notes::load(&h.notes_file());
        let thread = on_disk.threads.iter().find(|t| t.id == first).unwrap();
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[1].author, "claude");
        assert_eq!(thread.comments[1].body, "done, see commit");
        assert!(thread.comments[1].time > 1_700_000_000);
        assert_eq!(on_disk.threads.iter().find(|t| t.id == second).unwrap().comments.len(), 1);
    }

    #[test]
    fn resolved_notes_are_listed_only_on_request_and_a_reply_reopens_them() {
        let h = Harness::new();
        let id = h.seed_note("fix");
        let mut notes = Notes::load(&h.notes_file());
        notes.set_resolved(id, true);
        notes.save(&h.notes_file()).unwrap();
        let ids = |args: Value| -> Vec<u64> {
            let reply = h.call("list_notes", &args);
            let threads: Vec<Value> =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            threads.iter().map(|t| t["id"].as_u64().unwrap()).collect()
        };
        assert_eq!(ids(json!({})), Vec::<u64>::new());
        assert_eq!(ids(json!({"include_resolved": true})), vec![id]);
        h.call("reply_to_note", &json!({"id": id, "text": "actually still open"}));
        assert_eq!(ids(json!({})), vec![id]);
    }

    #[test]
    fn a_corrupt_notes_file_lists_as_empty_and_rejects_replies() {
        let h = Harness::new();
        std::fs::create_dir_all(h.notes_file().parent().unwrap()).unwrap();
        std::fs::write(h.notes_file(), "{ nope").unwrap();
        assert_eq!(h.call("list_notes", &json!({}))["result"]["content"][0]["text"], "[]");
        let reply = h.call("reply_to_note", &json!({"id": 1, "text": "x"}));
        assert_eq!(error_code(&reply), INVALID_PARAMS);
        assert_eq!(std::fs::read_to_string(h.notes_file()).unwrap(), "{ nope");
    }

    #[test]
    fn unknown_methods_are_method_not_found() {
        let h = Harness::new();
        for method in ["server/discover", "resources/list", "prompts/list", "tools/nope", ""] {
            let reply = h.json(&json!({"jsonrpc": "2.0", "id": 8, "method": method}));
            assert_eq!(error_code(&reply), METHOD_NOT_FOUND, "{method:?}");
            assert_eq!(reply["id"], 8);
        }
    }

    #[test]
    fn garbage_bodies_get_json_rpc_errors_and_never_panic() {
        let h = Harness::new();
        let cases: Vec<(&[u8], i64)> = vec![
            (b"", PARSE_ERROR),
            (b"{", PARSE_ERROR),
            (b"not json", PARSE_ERROR),
            (b"\xff\xfe\x00", PARSE_ERROR),
            (b"null", INVALID_REQUEST),
            (b"7", INVALID_REQUEST),
            (b"\"tools/list\"", INVALID_REQUEST),
            (b"[]", INVALID_REQUEST),
            (b"{}", INVALID_REQUEST),
            (br#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, INVALID_REQUEST),
            (br#"{"jsonrpc":"2.0","id":1}"#, INVALID_REQUEST),
            (br#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#, INVALID_REQUEST),
            (br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, INVALID_REQUEST),
            (br#"{"jsonrpc":"2.0","id":1,"method":7}"#, INVALID_REQUEST),
        ];
        for (body, want) in cases {
            match h.post(&String::from_utf8_lossy(body)) {
                McpReply::Json(text) => {
                    let reply: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(error_code(&reply), want, "{:?}", String::from_utf8_lossy(body));
                }
                McpReply::Accepted => panic!("{:?} was accepted", String::from_utf8_lossy(body)),
            }
        }
        let raw = handle_post(
            b"\xff\xfe\x00",
            &ToolCtx {
                agent: "a",
                role: TabRole::Agent,
                notes_file: Path::new("/none"),
                notes_lock: &h.lock,
                progress_file: Path::new("/none/progress.md"),
                ask: test_wait(&|| true),
                workflow: &Workflow::default,
                after_save: &Vec::new,
            },
            &|_| {},
        );
        assert!(matches!(raw, McpReply::Json(_)));
    }

    #[test]
    fn a_batch_answers_requests_and_skips_notifications() {
        let h = Harness::new();
        let body = json!([
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 1, "method": "ping"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            7,
        ]);
        let McpReply::Json(text) = h.post(&body.to_string()) else { panic!("batch was accepted") };
        let replies: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[1]["id"], 2);
        assert_eq!(error_code(&replies[2]), INVALID_REQUEST);
        let only_notifications = json!([{"jsonrpc": "2.0", "method": "notifications/initialized"}]);
        assert_eq!(h.post(&only_notifications.to_string()), McpReply::Accepted);
    }

    #[test]
    fn concurrent_replies_to_one_note_all_land() {
        let h = Harness::new();
        let id = h.seed_note("start");
        let notes_file = h.notes_file();
        std::thread::scope(|scope| {
            for n in 0..8 {
                let (lock, notes_file) = (&h.lock, &notes_file);
                scope.spawn(move || {
                    let alive = || true;
                    let ctx = ToolCtx {
                        agent: "claude",
                        role: TabRole::Agent,
                        notes_file,
                        notes_lock: lock,
                        progress_file: Path::new("/none/progress.md"),
                        ask: test_wait(&alive),
                        workflow: &Workflow::default,
                        after_save: &Vec::new,
                    };
                    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": {"name": "reply_to_note", "arguments": {"id": id, "text": format!("reply {n}")}}});
                    handle_post(body.to_string().as_bytes(), &ctx, &|_| {});
                });
            }
        });
        assert_eq!(Notes::load(&notes_file).threads[0].comments.len(), 9);
    }
}
