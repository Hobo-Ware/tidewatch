//! The `meeseeks_done` tool of the Tidewatch MCP server: a Meeseeks says its one task is finished.
//! Tidewatch then checks the work and opens the pull request; the agent never ends itself.

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::mcp::problems_error;
use crate::meeseeks::MAX_SUMMARY_CHARS;

pub const DONE_TOOL: &str = "meeseeks_done";
pub const MAX_ANSWER_CHARS: usize = 4000;

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": DONE_TOOL,
        "description": "Only for a Meeseeks with one task: call this once, when you believe you are done or cannot go further. Start the summary with `Blocked:` and the reason if you are blocked. Tidewatch decides what to check and ends the session itself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": format!("What you did, on one line of at most {MAX_SUMMARY_CHARS} characters; it becomes the pull request title. A read-only task puts its whole answer here, up to {MAX_ANSWER_CHARS} characters.") },
            },
            "required": ["summary"],
            "additionalProperties": false,
        },
    })
}

fn refused(problems: &[String]) -> (String, bool) {
    problems_error(&format!("Not recorded. Fix this and call {DONE_TOOL} again:"), problems)
}

pub fn done(args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let mut problems: Vec<String> = args
        .keys()
        .filter(|key| key.as_str() != "summary")
        .map(|key| format!("{key} is not an argument; the only one is summary"))
        .collect();
    let summary = if let Some(Value::String(text)) = args.get("summary") {
        text.trim()
    } else {
        problems.push("summary must be a string".to_owned());
        ""
    };
    if summary.is_empty() && !problems.iter().any(|p| p.starts_with("summary")) {
        problems.push("summary must say what you did".to_owned());
    }
    if summary.chars().count() > MAX_ANSWER_CHARS {
        problems.push(format!("summary must be at most {MAX_ANSWER_CHARS} characters"));
    }
    if !problems.is_empty() {
        return refused(&problems);
    }
    emit(TabEvent::MeeseeksDone { summary: summary.to_owned() });
    (
        "Recorded. Tidewatch is checking your work now. Wait: it will paste what to fix, or end this session."
            .to_owned(),
        false,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn call(value: &Value) -> ((String, bool), Vec<TabEvent>) {
        let seen = RefCell::new(Vec::new());
        let result = done(&args(value), &|event| seen.borrow_mut().push(event));
        (result, seen.into_inner())
    }

    #[test]
    fn a_good_call_emits_the_trimmed_summary_once() {
        let ((text, is_error), events) = call(&json!({"summary": "  Renamed the helper  "}));
        assert!(!is_error, "{text}");
        assert_eq!(events, [TabEvent::MeeseeksDone { summary: "Renamed the helper".into() }]);
    }

    #[test]
    fn every_bad_call_is_a_tool_error_that_says_what_to_fix_and_emits_nothing() {
        let long = "x".repeat(MAX_ANSWER_CHARS + 1);
        let cases = [
            (json!({}), "summary must be a string"),
            (json!({"summary": 4}), "summary must be a string"),
            (json!({"summary": "   "}), "summary must say what you did"),
            (json!({"summary": long}), "at most 4000 characters"),
            (json!({"summary": "ok", "pr": true}), "pr is not an argument"),
        ];
        for (value, want) in cases {
            let ((text, is_error), events) = call(&value);
            assert!(is_error && text.contains(want), "{value}: {text}");
            assert!(text.contains("call meeseeks_done again"), "{value}: {text}");
            assert_eq!(events, [], "{value}");
        }
    }

    #[test]
    fn a_long_multi_line_answer_goes_through_and_the_edit_cap_is_the_machines_to_hold() {
        let answer = format!("First line\n{}", "x".repeat(MAX_ANSWER_CHARS - 11));
        let ((_, is_error), events) = call(&json!({"summary": answer}));
        assert!(!is_error);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn the_schema_requires_the_summary_and_allows_nothing_else() {
        let definition = tool_definition();
        assert_eq!(definition["name"], "meeseeks_done");
        assert_eq!(definition["inputSchema"]["required"], json!(["summary"]));
        assert_eq!(definition["inputSchema"]["additionalProperties"], json!(false));
    }
}
