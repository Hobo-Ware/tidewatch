//! The `set_avatar` tool of the Tidewatch MCP server: the agent draws a small icon for its task.
//! The SVG is cleaned (see `avatar.rs`), saved beside the tab's notes and shown on the tab.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::avatar::{self, MAX_SVG_BYTES};
use crate::avatar_check;
use crate::avatar_store;
use crate::events::TabEvent;
use crate::paths::avatar_file_beside;

pub const SET_TOOL: &str = "set_avatar";

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": SET_TOOL,
        "description": "Once you understand the task, call this once to give the tab an icon: a round icon, a 64x64 flat SVG about the task, bright colors, nothing near the edge. Tidewatch draws the dark circular plate and its ring, so draw no ring, frame, border or background tile of your own: only the art itself, which is scaled to fill the plate. Keep the drawing inside the middle circle of the viewBox and use only colors that are at least 3:1 against #222937. 2 to 3 colors, simple shapes (rect, circle, ellipse, line, polyline, polygon, path). Anything else in the SVG is dropped. An icon that is too dark or runs past the circle is refused with what to fix. Call it again only when asked to redraw.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "svg": { "type": "string", "description": format!("The SVG, viewBox 0 0 64 64, at most {} KB.", MAX_SVG_BYTES / 1024) },
                "reason": { "type": "string", "description": "One line on why this icon fits the task. The user sees it on hover." },
            },
            "required": ["svg", "reason"],
            "additionalProperties": false,
        },
    })
}

fn not_set(problems: &[String]) -> (String, bool) {
    (
        format!(
            "The avatar was not set. Fix these and call {SET_TOOL} again:\n- {}",
            problems.join("\n- ")
        ),
        true,
    )
}

/// Cleans the SVG, saves it next to `notes_file` and tells the tab. A bad call is a tool error
/// with what to fix.
pub fn set(
    args: &Map<String, Value>,
    notes_file: &Path,
    emit: &dyn Fn(TabEvent),
) -> (String, bool) {
    let mut problems = Vec::new();
    let mut text = |key: &str| {
        let found = args.get(key).and_then(Value::as_str).map(str::to_owned);
        if found.is_none() {
            problems.push(format!("{key} must be a string"));
        }
        found
    };
    let (svg, reason) = (text("svg"), text("reason"));
    let (Some(svg), Some(reason)) = (svg, reason) else { return not_set(&problems) };
    let avatar = match avatar::clean(&svg, &reason) {
        Ok(avatar) => avatar,
        Err(e) => return not_set(&[e.to_string()]),
    };
    if let Err(unreadable) = avatar_check::check(&avatar.svg) {
        let problems: Vec<String> = unreadable.iter().map(ToString::to_string).collect();
        return not_set(&problems);
    }
    if let Err(e) = avatar_store::save(&avatar_file_beside(notes_file), &avatar) {
        return (format!("The avatar could not be saved ({e}). Call {SET_TOOL} again."), true);
    }
    emit(TabEvent::Avatar { avatar });
    ("Avatar set. Tidewatch shows it on this tab.".to_owned(), false)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const SVG: &str = "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2dd4bf\"/><script>1</script></svg>";

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_good_call_saves_the_cleaned_svg_and_emits_it() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let seen = RefCell::new(Vec::new());
        let (text, is_error) =
            set(&args(&json!({"svg": SVG, "reason": "a teal dot"})), &notes, &|event| {
                seen.borrow_mut().push(event);
            });
        assert!(!is_error, "{text}");
        let events = seen.borrow();
        let [TabEvent::Avatar { avatar }] = events.as_slice() else { panic!("{events:?}") };
        assert!(!avatar.svg.contains("script"));
        assert_eq!(avatar.reason, "a teal dot");
        assert_eq!(avatar_store::load(&dir.path().join("avatar.json")), Some(avatar.clone()));
    }

    #[test]
    fn a_bad_call_is_a_tool_error_that_says_what_to_fix_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let cases = [
            (json!({}), "svg must be a string"),
            (json!({"svg": SVG}), "reason must be a string"),
            (json!({"svg": 5, "reason": "x"}), "svg must be a string"),
            (json!({"svg": "<svg/>", "reason": "x"}), "no allowed shape"),
            (json!({"svg": "not svg", "reason": "x"}), "does not parse"),
            (json!({"svg": SVG, "reason": "  "}), "reason must say"),
        ];
        for (call, want) in cases {
            let seen = RefCell::new(0);
            let (text, is_error) = set(&args(&call), &notes, &|_| *seen.borrow_mut() += 1);
            assert!(is_error, "{call}");
            assert!(
                text.contains(want) && text.contains("call set_avatar again"),
                "{call}: {text}"
            );
            assert_eq!(*seen.borrow(), 0);
        }
        assert!(!dir.path().join("avatar.json").exists());
    }

    #[test]
    fn an_icon_that_cannot_be_read_is_refused_with_a_fix_and_saved_nowhere() {
        let dark = "<svg viewBox=\"0 0 24 24\"><path d=\"M7 11V8a5 5 0 0 1 10 0v3h1v10H6V11z\" fill=\"#2a2f3a\"/></svg>";
        let square =
            "<svg viewBox=\"0 0 64 64\"><rect width=\"64\" height=\"64\" fill=\"#2dd4bf\"/></svg>";
        let both = "<svg viewBox=\"0 0 64 64\"><rect width=\"64\" height=\"64\" rx=\"16\" fill=\"#123b38\"/></svg>";
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        for (svg, wants) in [
            (dark, vec!["too dark to read on its plate (#222937)", "at least 3:1"]),
            (square, vec!["outside the circle", "middle circle of the viewBox"]),
            (both, vec!["too dark", "outside the circle"]),
        ] {
            let seen = RefCell::new(0);
            let (text, is_error) = set(&args(&json!({"svg": svg, "reason": "x"})), &notes, &|_| {
                *seen.borrow_mut() += 1;
            });
            assert!(is_error, "{svg}");
            for want in wants {
                assert!(
                    text.contains(want) && text.contains("call set_avatar again"),
                    "{want}: {text}"
                );
            }
            assert_eq!(*seen.borrow(), 0);
        }
        assert!(!dir.path().join("avatar.json").exists());
    }

    #[test]
    fn the_tool_text_asks_for_a_round_bright_icon_with_nothing_near_the_edge() {
        let text = tool_definition()["description"].as_str().unwrap().to_owned();
        for want in [
            "round icon",
            "bright colors",
            "nothing near the edge",
            "no ring, frame, border or background tile",
            "3:1",
        ] {
            assert!(text.contains(want), "{want}");
        }
        assert!(text.contains(&crate::avatar_check::hex(crate::avatar_check::PLATE)));
    }

    #[test]
    fn the_schema_requires_both_arguments() {
        let definition = tool_definition();
        assert_eq!(definition["inputSchema"]["required"], json!(["svg", "reason"]));
    }
}
