//! The repo service tools of the Tidewatch MCP server (`list_services`, `add_service`,
//! `restart_service`, `stop_service`, `service_log`) and the words that tell agents what runs:
//! the house-style block and the line a prompt's answer carries. A call becomes a
//! [`TabEvent::Services`] and is held until the app, which owns the processes, answers it.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::service_supervisor::Word;
use crate::services::Service;

pub const LIST_TOOL: &str = "list_services";
pub const ADD_TOOL: &str = "add_service";
pub const RESTART_TOOL: &str = "restart_service";
pub const STOP_TOOL: &str = "stop_service";
pub const LOG_TOOL: &str = "service_log";
pub const TOOLS: [&str; 5] = [LIST_TOOL, ADD_TOOL, RESTART_TOOL, STOP_TOOL, LOG_TOOL];

pub const DEFAULT_LOG_LINES: usize = 50;
pub const MAX_LOG_LINES: usize = 200;
pub const REPLY_WAIT: Duration = Duration::from_secs(15);

#[must_use]
pub fn is_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    List,
    Add(Service),
    Restart(String),
    Stop(String),
    Log { name: String, lines: usize },
}

/// The answer to a held call: the text for the agent, or the reason it failed.
pub type Answer = Result<String, String>;

#[derive(Clone)]
pub struct ServiceReply(Arc<Sender<Answer>>);

impl ServiceReply {
    #[must_use]
    pub fn new(sender: Sender<Answer>) -> Self {
        Self(Arc::new(sender))
    }

    /// Whether the held call took the answer.
    #[must_use]
    pub fn send(&self, answer: Answer) -> bool {
        self.0.send(answer).is_ok()
    }
}

impl std::fmt::Debug for ServiceReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceReply")
    }
}

impl PartialEq for ServiceReply {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ServiceReply {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceCall {
    pub call: Call,
    pub reply: ServiceReply,
}

#[must_use]
pub fn tool_definitions() -> Vec<Value> {
    let by_name = json!({
        "type": "object",
        "properties": { "name": { "type": "string", "description": "The service's name from list_services." } },
        "required": ["name"],
        "additionalProperties": false,
    });
    vec![
        json!({
            "name": LIST_TOOL,
            "description": "List the repo's services (long-running processes Tidewatch keeps running next to you, such as a database proxy or a dev server) with their command, the port each one printed that it listens on, state, uptime and last exit code.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
        json!({
            "name": ADD_TOOL,
            "description": "Save a new repo service and start it. Use it for a long-running process the work needs (a database proxy, a dev server, a tunnel) instead of running it in your own shell. Tidewatch keeps it running, restarts it after a crash and shows it to the owner.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "A short unique name, at most 40 characters, such as `db` or `api`." },
                    "command": { "type": "string", "description": "The shell command to run in the repo's main checkout." },
                    "autostart": { "type": "boolean", "description": "Start it whenever the repo opens in Tidewatch. Defaults to true." },
                },
                "required": ["name", "command"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": RESTART_TOOL,
            "description": "Stop a repo service if it runs and start it again. Also brings back one that stopped or gave up.",
            "inputSchema": by_name,
        }),
        json!({
            "name": STOP_TOOL,
            "description": "Stop a repo service. Tidewatch does not restart it until you or the owner start it again.",
            "inputSchema": by_name,
        }),
        json!({
            "name": LOG_TOOL,
            "description": "The last lines a repo service printed, without colors or control codes. Read it to see why a service fails.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The service's name from list_services." },
                    "lines": { "type": "integer", "minimum": 1, "maximum": MAX_LOG_LINES, "description": format!("How many lines, {DEFAULT_LOG_LINES} by default.") },
                },
                "required": ["name"],
                "additionalProperties": false,
            },
        }),
    ]
}

fn refused(tool: &str, problems: &[String]) -> (String, bool) {
    (
        format!(
            "Nothing was changed. Fix these and call {tool} again:\n- {}",
            problems.join("\n- ")
        ),
        true,
    )
}

fn unknown_keys(args: &Map<String, Value>, allowed: &[&str], problems: &mut Vec<String>) {
    for key in args.keys().filter(|key| !allowed.contains(&key.as_str())) {
        let allowed = if allowed.is_empty() {
            "it takes no arguments".to_owned()
        } else {
            format!("the arguments are {}", allowed.join(", "))
        };
        problems.push(format!("{key} is not an argument; {allowed}"));
    }
}

fn text_arg(args: &Map<String, Value>, key: &str, problems: &mut Vec<String>) -> String {
    match args.get(key) {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(_) => {
            problems.push(format!("{key} must be a string"));
            String::new()
        }
        None => {
            problems.push(format!("{key} is required"));
            String::new()
        }
    }
}

fn name_arg(args: &Map<String, Value>, problems: &mut Vec<String>) -> String {
    let name = text_arg(args, "name", problems);
    if name.is_empty() && !problems.iter().any(|p| p.starts_with("name")) {
        problems.push("name must say which service, see list_services".to_owned());
    }
    name
}

fn number_arg(
    args: &Map<String, Value>,
    key: &str,
    range: std::ops::RangeInclusive<u64>,
    problems: &mut Vec<String>,
) -> Option<u64> {
    let Value::Number(number) = args.get(key)? else {
        problems.push(format!("{key} must be a whole number"));
        return None;
    };
    let Some(found) = number.as_u64().filter(|n| range.contains(n)) else {
        problems.push(format!(
            "{key} must be a whole number from {} to {}",
            range.start(),
            range.end()
        ));
        return None;
    };
    Some(found)
}

/// The call a tool name and its arguments stand for, or every problem with them.
pub fn parse(tool: &str, args: &Map<String, Value>) -> Result<Call, Vec<String>> {
    let mut problems = Vec::new();
    let call = match tool {
        LIST_TOOL => {
            unknown_keys(args, &[], &mut problems);
            Call::List
        }
        ADD_TOOL => {
            unknown_keys(args, &["name", "command", "port", "autostart"], &mut problems);
            let name = name_arg(args, &mut problems);
            let command = text_arg(args, "command", &mut problems);
            let autostart = match args.get("autostart") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(on)) => *on,
                Some(_) => {
                    problems.push("autostart must be true or false".to_owned());
                    true
                }
            };
            Call::Add(Service::new(&name, &command, autostart))
        }
        RESTART_TOOL | STOP_TOOL => {
            unknown_keys(args, &["name"], &mut problems);
            let name = name_arg(args, &mut problems);
            if tool == RESTART_TOOL { Call::Restart(name) } else { Call::Stop(name) }
        }
        LOG_TOOL => {
            unknown_keys(args, &["name", "lines"], &mut problems);
            let name = name_arg(args, &mut problems);
            let lines = number_arg(args, "lines", 1..=MAX_LOG_LINES as u64, &mut problems)
                .map_or(DEFAULT_LOG_LINES, |lines| lines as usize);
            Call::Log { name, lines }
        }
        other => return Err(vec![format!("{other} is not a service tool")]),
    };
    if problems.is_empty() { Ok(call) } else { Err(problems) }
}

/// Runs one tool call: the arguments are checked here, the rest is the app's answer.
pub fn call(tool: &str, args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    call_within(tool, args, emit, REPLY_WAIT)
}

pub fn call_within(
    tool: &str,
    args: &Map<String, Value>,
    emit: &dyn Fn(TabEvent),
    wait: Duration,
) -> (String, bool) {
    let parsed = match parse(tool, args) {
        Ok(parsed) => parsed,
        Err(problems) => return refused(tool, &problems),
    };
    let (sender, receiver) = channel();
    emit(TabEvent::Services(ServiceCall { call: parsed, reply: ServiceReply::new(sender) }));
    match receiver.recv_timeout(wait) {
        Ok(Ok(text)) => (text, false),
        Ok(Err(reason)) => (reason, true),
        Err(_) => (
            format!(
                "Tidewatch did not answer {tool} within {} seconds. Its window may be hidden or busy; call it again.",
                wait.as_secs()
            ),
            true,
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Listed {
    pub name: String,
    pub command: String,
    pub port: Option<u16>,
    pub state: &'static str,
    pub uptime_secs: Option<u64>,
    pub last_exit: Option<i32>,
}

impl Listed {
    #[must_use]
    pub fn new(
        service: &Service,
        port: Option<u16>,
        word: Word,
        uptime_secs: Option<f64>,
        last_exit: Option<i32>,
    ) -> Self {
        Self {
            name: service.name.clone(),
            command: service.command.clone(),
            port,
            state: word.text(),
            uptime_secs: uptime_secs.map(|secs| secs as u64),
            last_exit,
        }
    }
}

/// The answer of `list_services`: the services as JSON, or a line that says there are none.
#[must_use]
pub fn list_text(listed: &[Listed]) -> String {
    if listed.is_empty() {
        return "This repo has no services. Use add_service to save and start one.".to_owned();
    }
    serde_json::to_string_pretty(&json!({ "services": listed })).unwrap_or_default()
}

/// The block of an agent's house style, `None` when the repo has no service.
#[must_use]
pub fn house_block(services: &[Service]) -> Option<String> {
    if services.is_empty() {
        return None;
    }
    let mut lines = String::new();
    for service in services {
        let _ = writeln!(lines, "- `{}`: `{}`", service.name, service.command);
    }
    Some(format!(
        "## Repo services\n\nTidewatch keeps these long-running services of this repo running next to you. Use them; do not start a second copy in your own shell.\n\n{lines}\nTools: `{LIST_TOOL}` (state, uptime, last exit), `{LOG_TOOL}` (the last lines a service printed: read it before guessing why one fails), `{RESTART_TOOL}`, `{STOP_TOOL}` and `{ADD_TOOL}` (save and start a long-running process the work needs).\n"
    ))
}

/// One service's facts for the line a prompt carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub name: String,
    pub port: Option<u16>,
    pub word: Word,
}

/// The line the answer to a prompt carries; the same facts give the same line, so a tab is told
/// again only when the list or a state changed.
#[must_use]
pub fn context_line(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let parts: Vec<String> = facts
        .iter()
        .map(|fact| {
            let port = fact.port.map_or_else(String::new, |port| format!(" on port {port}"));
            format!("{}{port} is {}", fact.name, fact.word.text())
        })
        .collect();
    Some(format!(
        "Repo services Tidewatch keeps running: {}. Use {LIST_TOOL} or {LOG_TOOL} for detail.",
        parts.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn the_five_tools_have_object_schemas_and_the_required_fields_exist() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, TOOLS);
        for tool in &tools {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{tool}");
            assert_eq!(schema["additionalProperties"], false);
            for required in schema["required"].as_array().into_iter().flatten() {
                assert!(schema["properties"][required.as_str().unwrap()].is_object(), "{tool}");
            }
        }
    }

    #[test]
    fn arguments_parse_into_calls() {
        let cases = [
            (LIST_TOOL, json!({}), Call::List),
            (
                ADD_TOOL,
                json!({"name": " db ", "command": "pscale connect shop main", "port": 3309}),
                Call::Add(Service::new("db", "pscale connect shop main", true)),
            ),
            (
                ADD_TOOL,
                json!({"name": "api", "command": "npm run dev", "autostart": false}),
                Call::Add(Service::new("api", "npm run dev", false)),
            ),
            (RESTART_TOOL, json!({"name": "db"}), Call::Restart("db".into())),
            (STOP_TOOL, json!({"name": "db"}), Call::Stop("db".into())),
            (LOG_TOOL, json!({"name": "db"}), Call::Log { name: "db".into(), lines: 50 }),
            (
                LOG_TOOL,
                json!({"name": "db", "lines": 200}),
                Call::Log { name: "db".into(), lines: 200 },
            ),
        ];
        for (tool, given, want) in cases {
            assert_eq!(parse(tool, &args(&given)), Ok(want), "{tool} {given}");
        }
    }

    #[test]
    fn a_port_an_older_agent_sends_is_accepted_and_ignored() {
        let add = |port: Value| {
            parse(ADD_TOOL, &args(&json!({"name": "a", "command": "b", "port": port})))
        };
        for port in [json!(3309), json!(0), json!(70000), json!("80")] {
            assert_eq!(add(port.clone()), Ok(Call::Add(Service::new("a", "b", true))), "{port}");
        }
        let tools = tool_definitions();
        let add_tool = tools.iter().find(|tool| tool["name"] == ADD_TOOL).unwrap();
        assert!(add_tool["inputSchema"]["properties"]["port"].is_null());
    }

    #[test]
    fn every_problem_with_the_arguments_is_listed_at_once() {
        let cases = [
            (LIST_TOOL, json!({"x": 1}), 1),
            (ADD_TOOL, json!({}), 2),
            (ADD_TOOL, json!({"name": "a", "command": "b", "autostart": "yes"}), 1),
            (ADD_TOOL, json!({"name": 5, "command": ["x"], "extra": 1}), 3),
            (RESTART_TOOL, json!({}), 1),
            (STOP_TOOL, json!({"name": "  "}), 1),
            (LOG_TOOL, json!({"name": "db", "lines": 0}), 1),
            (LOG_TOOL, json!({"name": "db", "lines": 201}), 1),
            (LOG_TOOL, json!({"lines": -1}), 2),
        ];
        for (tool, given, want) in cases {
            let found = parse(tool, &args(&given)).unwrap_err();
            assert_eq!(found.len(), want, "{tool} {given}: {found:?}");
        }
        assert_eq!(parse("restart", &Map::new()).unwrap_err().len(), 1);
    }

    #[test]
    fn a_call_is_held_until_the_app_answers_and_its_failure_is_a_tool_error() {
        let answer = |reply: Answer| {
            let emit = move |event: TabEvent| {
                let TabEvent::Services(call) = event else { panic!("a service event") };
                assert_eq!(call.call, Call::Restart("db".into()));
                assert!(call.reply.send(reply.clone()));
            };
            call(RESTART_TOOL, &args(&json!({"name": "db"})), &emit)
        };
        assert_eq!(answer(Ok("Restarted db.".into())), ("Restarted db.".to_owned(), false));
        assert_eq!(answer(Err("no such service".into())), ("no such service".to_owned(), true));
    }

    #[test]
    fn bad_arguments_emit_nothing_and_list_the_fixes() {
        let emitted = std::cell::Cell::new(false);
        let emit = |_: TabEvent| emitted.set(true);
        let (text, is_error) = call(ADD_TOOL, &args(&json!({"name": "db"})), &emit);
        assert!(is_error);
        assert!(text.starts_with("Nothing was changed. Fix these and call add_service again:"));
        assert!(text.contains("- command is required"), "{text}");
        assert!(!emitted.get());
    }

    #[test]
    fn an_unanswered_call_gives_up_with_a_reason() {
        let emit = |_: TabEvent| {};
        let (text, is_error) =
            call_within(LIST_TOOL, &Map::new(), &emit, Duration::from_millis(20));
        assert!(is_error);
        assert!(text.contains("did not answer list_services"), "{text}");
    }

    #[test]
    fn a_reply_reaches_the_held_call_only_while_it_waits() {
        let (sender, receiver) = channel();
        let reply = ServiceReply::new(sender);
        let copy = reply.clone();
        assert_eq!(reply, copy);
        assert!(reply.send(Ok("x".into())));
        assert_eq!(receiver.recv().unwrap(), Ok("x".to_owned()));
        drop(receiver);
        assert!(!copy.send(Ok("late".into())));
    }

    #[test]
    fn the_listing_is_json_with_every_field_or_a_line_when_empty() {
        let db = Service::new("db", "pscale connect shop main --port 3309", true);
        let listed = [Listed::new(&db, Some(3309), Word::Running, Some(8040.7), Some(1))];
        let value: Value = serde_json::from_str(&list_text(&listed)).unwrap();
        let first = &value["services"][0];
        assert_eq!(first["name"], "db");
        assert_eq!(first["command"], "pscale connect shop main --port 3309");
        assert_eq!(first["port"], 3309);
        assert_eq!(first["state"], "running");
        assert_eq!(first["uptime_secs"], 8040);
        assert_eq!(first["last_exit"], 1);
        assert!(list_text(&[]).contains("no services"));
    }

    #[test]
    fn the_house_block_names_each_service_and_the_tools_and_is_absent_without_services() {
        assert_eq!(house_block(&[]), None);
        let block = house_block(&[
            Service::new("db", "pscale connect shop main --port 3309", true),
            Service::new("api", "npm run dev", true),
        ])
        .unwrap();
        assert!(block.contains("- `db`: `pscale connect shop main --port 3309`\n"));
        assert!(block.contains("- `api`: `npm run dev`\n"));
        for tool in TOOLS {
            assert!(block.contains(tool), "{tool}");
        }
        assert!(block.is_ascii());
    }

    #[test]
    fn the_prompt_line_changes_only_when_a_name_a_port_or_a_state_does() {
        let fact = |name: &str, port, word| Fact { name: name.into(), port, word };
        assert_eq!(context_line(&[]), None);
        let one = context_line(&[
            fact("db", Some(3309), Word::Running),
            fact("api", None, Word::Crashed),
        ]);
        let same = context_line(&[
            fact("db", Some(3309), Word::Running),
            fact("api", None, Word::Crashed),
        ]);
        let moved = context_line(&[
            fact("db", Some(3309), Word::Running),
            fact("api", None, Word::Restarting),
        ]);
        assert_eq!(one, same);
        assert_ne!(one, moved);
        let text = one.unwrap();
        assert!(text.contains("db on port 3309 is running; api is crashed"), "{text}");
        assert!(text.contains("list_services"));
    }
}
