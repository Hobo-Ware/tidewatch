//! The Tidewatch MCP server (PLAN.md, D6): JSON-RPC 2.0 over streamable HTTP, stateless, one reply
//! per POST.
//!
//! [`handle_post`] takes the raw request body of an authenticated `/mcp` POST and returns the reply.
//! It speaks `initialize`, `ping`, `tools/list` and `tools/call` and swallows notifications. The
//! tools are `stage_done`, `report_finding`, `list_notes`, `reply_to_note` and `propose_workflow`. Calls that matter to the
//! app become [`TabEvent`]s through `emit`; the note tools read and write the tab's `notes.json`
//! through `notes.rs`.
//!
//! Errors follow JSON-RPC: unparseable body -32700, not a request -32600, unknown method -32601,
//! unknown tool or bad arguments -32602, a failed save -32603. Nothing here panics on input.
//!
//! The wire shape was checked against claude 2.1.296 (which also probes `server/discover`, answered
//! with -32601), gemini 0.46.0 and copilot 1.0.91.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use kelp_core::review::Comment;
use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::launch::MCP_SERVER_NAME;
use crate::notes::Notes;
use crate::propose;

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

/// What one tab's tools act on.
pub struct ToolCtx<'a> {
    /// The agent id (`claude`, `gemini`, ...). It signs replies in the notes.
    pub agent: &'a str,
    pub notes_file: &'a Path,
    /// Serializes read-modify-write of `notes_file` between concurrent calls.
    pub notes_lock: &'a Mutex<()>,
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
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
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
        "instructions": "Tidewatch tools for the task in this worktree: report that a stage is done, report review findings, and read and answer the notes the user left on your changes.",
    })
}

/// The five tools with their JSON Schemas, as `tools/list` returns them.
#[must_use]
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "stage_done",
            "description": "Call once when you have finished the task you were given. Summarize what you did and anything the user should check.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "What you did and what to check." }
                },
                "required": ["summary"],
                "additionalProperties": false,
            },
        },
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
        propose::tool_definition(),
    ])
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
    if name == propose::TOOL_NAME {
        let (text, is_error) = propose_workflow(args, ctx, emit);
        return Ok(tool_result(&text, is_error));
    }
    let text = match name {
        "stage_done" => stage_done(args, emit)?,
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

/// A workflow that fails the rules is a tool error with the problems listed, not a protocol error,
/// so the agent reads what to fix and calls again.
fn propose_workflow(
    args: &Map<String, Value>,
    ctx: &ToolCtx<'_>,
    emit: &dyn Fn(TabEvent),
) -> (String, bool) {
    let checked = propose::parse_args(args).and_then(|mut workflow| {
        propose::with_agent(&mut workflow, ctx.agent);
        let found = propose::problems(&workflow);
        if found.is_empty() { Ok(workflow) } else { Err(found) }
    });
    match checked {
        Ok(workflow) => {
            emit(TabEvent::Proposal { workflow });
            (
                "Opened in Tidewatch for review. The user decides whether to save it; there is nothing more to do."
                    .to_owned(),
                false,
            )
        }
        Err(found) => (
            format!(
                "The workflow was not opened. Fix these and call {} again:\n- {}",
                propose::TOOL_NAME,
                found.join("\n- ")
            ),
            true,
        ),
    }
}

fn stage_done(args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> Result<String, RpcError> {
    let summary = text_arg(args, "summary")?;
    emit(TabEvent::StageDone { summary });
    Ok("Recorded. Tidewatch has your summary.".into())
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
    use kelp_core::review::{Anchor, Side};
    use std::cell::RefCell;

    struct Harness {
        dir: tempfile::TempDir,
        lock: Mutex<()>,
        events: RefCell<Vec<TabEvent>>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                lock: Mutex::new(()),
                events: RefCell::new(vec![]),
            }
        }

        fn notes_file(&self) -> std::path::PathBuf {
            self.dir.path().join("repo-abc123").join("tab").join("notes.json")
        }

        fn post(&self, body: &str) -> McpReply {
            let notes_file = self.notes_file();
            let ctx = ToolCtx { agent: "claude", notes_file: &notes_file, notes_lock: &self.lock };
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
    fn tools_list_has_five_tools_with_object_schemas_and_required_fields() {
        let reply =
            Harness::new().json(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
        let tools = reply["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["stage_done", "report_finding", "list_notes", "reply_to_note", "propose_workflow"]
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
                vec!["summary"],
                vec!["file", "line", "blocking", "text"],
                vec![],
                vec!["id", "text"],
                vec!["name", "stages"],
            ]
        );
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
            assert!(tool["description"].as_str().unwrap().len() > 20, "{}", tool["name"]);
        }
    }

    fn proposal() -> Value {
        json!({
            "name": "Change with a check",
            "description": "Do it, then prove it.",
            "stages": [
                { "action": "work", "prompt": "{task}" },
                { "action": "review", "prompt": "Review {base}..HEAD.", "on_fail": 1 },
            ],
        })
    }

    #[test]
    fn propose_workflow_opens_a_valid_workflow_for_review_on_the_callers_agent() {
        let h = Harness::new();
        let reply = h.call("propose_workflow", &proposal());
        assert_eq!(reply["result"]["isError"], false);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Opened in Tidewatch for review."), "{text}");
        let events = h.events.borrow();
        let [TabEvent::Proposal { workflow }] = events.as_slice() else {
            panic!("expected one proposal, got {events:?}");
        };
        assert_eq!(workflow.name, "Change with a check");
        assert_eq!(workflow.stages[1].on_fail, Some(0));
        assert!(workflow.stages.iter().all(|s| s.run.agent.as_deref() == Some("claude")));
    }

    #[test]
    fn propose_workflow_answers_the_problems_as_a_tool_error_and_opens_nothing() {
        let h = Harness::new();
        let mut bad = proposal();
        bad["stages"][1]["on_fail"] = json!(2);
        bad["stages"][0]["permissions"] = json!("yolo");
        bad["name"] = json!("");
        let reply = h.call("propose_workflow", &bad);
        assert!(reply.get("error").is_none(), "a rule failure is not a protocol error: {reply}");
        assert_eq!(reply["result"]["isError"], true);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with(
                "The workflow was not opened. Fix these and call propose_workflow again:\n- "
            ),
            "{text}"
        );
        assert!(text.contains("name must be a non-empty string"), "{text}");
        assert!(
            text.contains("stage 1: permissions must be one of: plan, ask, edits, auto"),
            "{text}"
        );
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
        let stage_two =
            json!({ "name": "n", "stages": [{ "action": "work", "prompt": "p", "on_fail": 1 }] });
        let reply = h.call("propose_workflow", &stage_two);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("stage 1 (Work): on_fail must name an earlier stage, not stage 1"),
            "{text}"
        );
        assert_eq!(*h.events.borrow(), Vec::<TabEvent>::new());
    }

    #[test]
    fn stage_done_emits_the_summary_and_returns_text_content() {
        let h = Harness::new();
        let reply = h.call("stage_done", &json!({"summary": "  hello  "}));
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(reply["result"]["content"][0]["type"], "text");
        assert_eq!(h.events.borrow().clone(), [TabEvent::StageDone { summary: "hello".into() }]);
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
        let h = Harness::new();
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let cases: Vec<(&str, Value)> = vec![
            ("stage_done", json!({})),
            ("stage_done", json!({"summary": ""})),
            ("stage_done", json!({"summary": "   "})),
            ("stage_done", json!({"summary": 5})),
            ("stage_done", json!({"summary": long})),
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
            json!({"name": "stage_done", "arguments": "x"}),
            json!({"name": "stage_done", "arguments": [1]}),
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
            &ToolCtx { agent: "a", notes_file: Path::new("/none"), notes_lock: &h.lock },
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
                    let ctx = ToolCtx { agent: "claude", notes_file, notes_lock: lock };
                    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": {"name": "reply_to_note", "arguments": {"id": id, "text": format!("reply {n}")}}});
                    handle_post(body.to_string().as_bytes(), &ctx, &|_| {});
                });
            }
        });
        assert_eq!(Notes::load(&notes_file).threads[0].comments.len(), 9);
    }
}
