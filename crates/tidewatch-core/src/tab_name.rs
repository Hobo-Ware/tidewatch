//! The `set_name` tool of the Tidewatch MCP server: an agent names its tab with a short kebab-case
//! name for its task. The name is what every tab label shows; the worktree and branch keep theirs.

use serde_json::{Map, Value, json};

use crate::branch_name;
use crate::events::TabEvent;
use crate::mcp::problems_error;

pub const SET_TOOL: &str = "set_name";
pub const MAX_CHARS: usize = 30;
pub const MIN_WORDS: usize = 2;
pub const MAX_WORDS: usize = 4;

const RULE: &str = "2 to 4 lowercase words joined by single hyphens (letters and digits only), at most 30 characters, for example `fix-https-validation` or `my-super-feature`";

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": SET_TOOL,
        "description": "Call this once, with your first progress report, to name your tab after your task, and again only if the task changes. The name is what Tidewatch shows on your tab.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": format!("A short kebab-case name for your task: {RULE}.") },
            },
            "required": ["name"],
            "additionalProperties": false,
        },
    })
}

/// The problems with `name` as a tab name, empty when it is one.
#[must_use]
pub fn problems(name: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if name.is_empty() {
        return vec!["name must not be empty".to_owned()];
    }
    if name.chars().count() > MAX_CHARS {
        problems.push(format!("name must be at most {MAX_CHARS} characters"));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        problems.push("name may only have lowercase ascii letters, digits and hyphens".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        problems.push("name must not start or end with a hyphen or have two in a row".to_owned());
    } else {
        let words = name.split('-').count();
        if !(MIN_WORDS..=MAX_WORDS).contains(&words) {
            problems.push(format!("name must be {MIN_WORDS} to {MAX_WORDS} words, not {words}"));
        }
    }
    if let Err(problem) = branch_name::check(name)
        && problems.is_empty()
    {
        problems.push(problem.message());
    }
    problems
}

fn refused(problems: &[String]) -> (String, bool) {
    problems_error(
        &format!("Not recorded. Fix this and call {SET_TOOL} again. The rule: {RULE}."),
        problems,
    )
}

pub fn set(args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let mut found: Vec<String> = args
        .keys()
        .filter(|key| key.as_str() != "name")
        .map(|key| format!("{key} is not an argument; the only one is name"))
        .collect();
    let name = if let Some(Value::String(text)) = args.get("name") {
        let name = text.trim();
        found.extend(problems(name));
        name
    } else {
        found.push("name must be a string".to_owned());
        ""
    };
    if !found.is_empty() {
        return refused(&found);
    }
    emit(TabEvent::TabName { name: name.to_owned() });
    (format!("Recorded. This tab is now called {name}."), false)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn call(value: &Value) -> ((String, bool), Vec<TabEvent>) {
        let seen = RefCell::new(Vec::new());
        let result = set(value.as_object().unwrap(), &|event| seen.borrow_mut().push(event));
        (result, seen.into_inner())
    }

    #[test]
    fn a_good_name_emits_once_and_a_trimmed_one_is_kept_trimmed() {
        let good = [
            "fix-https-validation",
            "my-super-feature",
            "retry-sync",
            "bump-vite-6-3",
            "a1-b2",
            "abcdefghijklmnopqrstuvwxyz-abc",
        ];
        for name in good {
            let ((text, is_error), events) = call(&json!({"name": format!("  {name} ")}));
            assert!(!is_error, "{name}: {text}");
            assert_eq!(events, [TabEvent::TabName { name: name.into() }], "{name}");
        }
    }

    #[test]
    fn every_bad_name_is_a_tool_error_that_lists_the_rule_and_emits_nothing() {
        let cases = [
            (json!({}), "name must be a string"),
            (json!({"name": 4}), "name must be a string"),
            (json!({"name": "   "}), "name must not be empty"),
            (json!({"name": "feature"}), "2 to 4 words, not 1"),
            (json!({"name": "one-two-three-four-five"}), "2 to 4 words, not 5"),
            (json!({"name": "Fix-Login"}), "lowercase ascii letters, digits and hyphens"),
            (json!({"name": "fix login"}), "lowercase ascii letters, digits and hyphens"),
            (json!({"name": "fix_login"}), "lowercase ascii letters, digits and hyphens"),
            (json!({"name": "fix/login"}), "lowercase ascii letters, digits and hyphens"),
            (json!({"name": "caf\u{e9}-login"}), "lowercase ascii letters, digits and hyphens"),
            (json!({"name": "-fix-login"}), "start or end with a hyphen"),
            (json!({"name": "fix-login-"}), "start or end with a hyphen"),
            (json!({"name": "fix--login"}), "two in a row"),
            (json!({"name": "abcdefghijklmnopqrstuvwxyz-abcd"}), "at most 30 characters"),
            (json!({"name": "fix-login", "why": "x"}), "why is not an argument"),
        ];
        for (value, want) in cases {
            let ((text, is_error), events) = call(&value);
            assert!(is_error && text.contains(want), "{value}: {text}");
            assert!(text.contains("call set_name again"), "{value}: {text}");
            assert!(text.contains("`fix-https-validation`"), "{value}: {text}");
            assert_eq!(events, [], "{value}");
        }
    }

    #[test]
    fn a_name_that_passes_is_always_a_valid_branch_segment() {
        for name in ["ab-cd", "a-b", "x1-y2-z3-w4", "fix-lock"] {
            assert_eq!(problems(name), Vec::<String>::new(), "{name}");
            assert_eq!(branch_name::check(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn the_schema_requires_the_name_and_allows_nothing_else() {
        let definition = tool_definition();
        assert_eq!(definition["name"], "set_name");
        assert_eq!(definition["inputSchema"]["required"], json!(["name"]));
        assert_eq!(definition["inputSchema"]["additionalProperties"], json!(false));
    }
}
