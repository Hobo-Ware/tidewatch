//! The `set_avatar` tool of the Tidewatch MCP server: the agent gives its tab an icon, either one
//! of the bundled Lucide icons in a color from a fixed list or an SVG it draws when none fits. The
//! SVG is cleaned (see `avatar.rs`), saved beside the tab's notes and shown on the tab.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::avatar::{self, MAX_SVG_BYTES};
use crate::avatar_check;
use crate::avatar_icons;
use crate::avatar_store;
use crate::events::TabEvent;
use crate::mcp::problems_error;
use crate::paths::avatar_file_beside;

pub const SET_TOOL: &str = "set_avatar";

#[must_use]
pub fn tool_definition() -> Value {
    let colors = avatar_icons::color_names();
    json!({
        "name": SET_TOOL,
        "description": format!("Once you know the goal, call this once to give the tab an icon. Prefer a bundled icon: send `icon`, a Lucide name such as music, shield, refresh-cw, database, bug, rocket or book-open, and `color`, one of {}. Choose the one object the goal is about and name it in `reason`. An unknown name is refused with close names. Only when no icon fits, send `svg` instead: a round icon, a 64x64 flat SVG, bright colors, nothing near the edge. Tidewatch draws the dark circular plate and its ring, so draw no ring, frame, border or background tile of your own: only the art itself, which is scaled to fill the plate. Keep the drawing inside the middle circle of the viewBox and use only colors that are at least 3:1 against {}. 2 to 3 colors, simple shapes (rect, circle, ellipse, line, polyline, polygon, path). Anything else in the SVG is dropped. Send icon or svg, never both. An icon that is too dark or runs past the circle is refused with what to fix. Call it again only when asked to redraw.", colors.join(", "), avatar_check::hex(avatar_check::PLATE)),
        "inputSchema": {
            "type": "object",
            "properties": {
                "icon": { "type": "string", "description": "A Lucide icon name, such as music, shield or refresh-cw." },
                "color": { "type": "string", "enum": colors, "description": "The icon's color. Required with icon." },
                "svg": { "type": "string", "description": format!("Only when no icon fits: the SVG, viewBox 0 0 64 64, at most {} KB.", MAX_SVG_BYTES / 1024) },
                "reason": { "type": "string", "description": "One line on why this icon fits the task. The user sees it on hover." },
            },
            "required": ["reason"],
            "additionalProperties": false,
        },
    })
}

fn not_set(problems: &[String]) -> (String, bool) {
    problems_error(
        &format!("The avatar was not set. Fix these and call {SET_TOOL} again:"),
        problems,
    )
}

fn text(args: &Map<String, Value>, key: &str, problems: &mut Vec<String>) -> Option<String> {
    match args.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(found)) => Some(found.clone()),
        Some(_) => {
            problems.push(format!("{key} must be a string"));
            None
        }
    }
}

fn icon_svg(icon: &str, color: Option<&str>, problems: &mut Vec<String>) -> Option<String> {
    let names = avatar_icons::color_names().join(", ");
    let ink = match color {
        None => {
            problems.push(format!("color is required with icon: one of {names}"));
            None
        }
        Some(color) => {
            let ink = avatar_icons::color_hex(color);
            if ink.is_none() {
                problems.push(format!("color \"{color}\" is not one of {names}"));
            }
            ink
        }
    };
    let ink = ink?;
    let drawn = avatar_icons::svg(icon, &ink);
    if drawn.is_none() {
        let close = avatar_icons::suggest(icon);
        problems.push(if close.is_empty() {
            format!("icon \"{icon}\" is not in the set and no name is close: try one plain word for the object, such as database or shield")
        } else {
            format!("icon \"{icon}\" is not in the set; close names: {}", close.join(", "))
        });
    }
    drawn
}

/// Draws the icon or cleans the SVG, saves it next to `notes_file` and tells the tab. A bad call
/// is a tool error with what to fix.
pub fn set(
    args: &Map<String, Value>,
    notes_file: &Path,
    emit: &dyn Fn(TabEvent),
) -> (String, bool) {
    let mut problems = Vec::new();
    let reason = text(args, "reason", &mut problems);
    if reason.is_none() && problems.is_empty() {
        problems.push("reason must be a string".to_owned());
    }
    let (svg, icon, color) = (
        text(args, "svg", &mut problems),
        text(args, "icon", &mut problems),
        text(args, "color", &mut problems),
    );
    let drawn = match (svg, icon) {
        (Some(_), Some(_)) => {
            problems.push("send icon with color, or svg, not both".to_owned());
            None
        }
        (Some(svg), None) => {
            if color.is_some() {
                problems.push("color goes with icon: an svg carries its own colors".to_owned());
            }
            Some(svg)
        }
        (None, Some(icon)) => icon_svg(&icon, color.as_deref(), &mut problems),
        (None, None) => {
            if !args.contains_key("svg") && !args.contains_key("icon") {
                problems.push("send icon with color, or svg".to_owned());
            }
            None
        }
    };
    let (Some(drawn), Some(reason)) = (drawn, reason) else { return not_set(&problems) };
    if !problems.is_empty() {
        return not_set(&problems);
    }
    let avatar = match avatar::clean(&drawn, &reason) {
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
            (json!({}), vec!["reason must be a string", "send icon with color, or svg"]),
            (json!({"svg": SVG}), vec!["reason must be a string"]),
            (json!({"svg": 5, "reason": "x"}), vec!["svg must be a string"]),
            (json!({"icon": 5, "color": "teal", "reason": "x"}), vec!["icon must be a string"]),
            (json!({"svg": "<svg/>", "reason": "x"}), vec!["no allowed shape"]),
            (json!({"svg": "not svg", "reason": "x"}), vec!["does not parse"]),
            (json!({"svg": SVG, "reason": "  "}), vec!["reason must say"]),
            (json!({"icon": "music", "color": "teal", "reason": "  "}), vec!["reason must say"]),
        ];
        for (call, wants) in cases {
            let seen = RefCell::new(0);
            let (text, is_error) = set(&args(&call), &notes, &|_| *seen.borrow_mut() += 1);
            assert!(is_error, "{call}");
            assert!(text.contains("call set_avatar again"), "{call}: {text}");
            for want in wants {
                assert!(text.contains(want), "{call}: {text}");
            }
            assert_eq!(*seen.borrow(), 0);
        }
        assert!(!dir.path().join("avatar.json").exists());
    }

    #[test]
    fn an_icon_and_a_color_save_a_picture_that_passes_the_check_and_emit_it() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let seen = RefCell::new(Vec::new());
        let call =
            json!({"icon": "music", "color": "purple", "reason": "Syncing the playlists: a note"});
        let (text, is_error) = set(&args(&call), &notes, &|event| seen.borrow_mut().push(event));
        assert!(!is_error, "{text}");
        let events = seen.borrow();
        let [TabEvent::Avatar { avatar }] = events.as_slice() else { panic!("{events:?}") };
        assert!(avatar.svg.contains("stroke=\"#c4b5fd\"") && avatar.svg.contains("<path"));
        assert_eq!(avatar_check::check(&avatar.svg), Ok(()));
        assert_eq!(avatar.reason, "Syncing the playlists: a note");
        assert_eq!(avatar_store::load(&dir.path().join("avatar.json")), Some(avatar.clone()));
    }

    #[test]
    fn every_listed_color_saves_an_icon() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        for color in avatar_icons::color_names() {
            let call = json!({"icon": "shield", "color": color, "reason": "x"});
            let (text, is_error) = set(&args(&call), &notes, &|_| {});
            assert!(!is_error, "{color}: {text}");
        }
    }

    #[test]
    fn an_unknown_icon_is_refused_with_up_to_eight_close_names() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let call = json!({"icon": "music-note", "color": "teal", "reason": "x"});
        let (text, is_error) = set(&args(&call), &notes, &|_| panic!("nothing is emitted"));
        assert!(is_error);
        assert!(
            text.contains("icon \"music-note\" is not in the set; close names: music"),
            "{text}"
        );
        let names = text.split("close names: ").nth(1).unwrap();
        assert_eq!(names.split(", ").count(), 8, "{text}");
        assert!(!dir.path().join("avatar.json").exists());

        let call = json!({"icon": "zzzzzz", "color": "teal", "reason": "x"});
        let (text, is_error) = set(&args(&call), &notes, &|_| {});
        assert!(is_error && text.contains("no name is close"), "{text}");
    }

    #[test]
    fn an_icon_needs_a_listed_color() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let list = avatar_icons::color_names().join(", ");
        let cases = [
            (
                json!({"icon": "music", "reason": "x"}),
                format!("color is required with icon: one of {list}"),
            ),
            (
                json!({"icon": "music", "color": "chartreuse", "reason": "x"}),
                format!("color \"chartreuse\" is not one of {list}"),
            ),
        ];
        for (call, want) in cases {
            let (text, is_error) = set(&args(&call), &notes, &|_| panic!("nothing is emitted"));
            assert!(is_error && text.contains(&want), "{call}: {text}");
        }
    }

    #[test]
    fn an_icon_and_an_svg_together_are_refused_and_so_is_a_color_with_an_svg() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.json");
        let cases = [
            (
                json!({"icon": "music", "color": "teal", "svg": SVG, "reason": "x"}),
                "send icon with color, or svg, not both",
            ),
            (
                json!({"color": "teal", "svg": SVG, "reason": "x"}),
                "color goes with icon: an svg carries its own colors",
            ),
        ];
        for (call, want) in cases {
            let (text, is_error) = set(&args(&call), &notes, &|_| panic!("nothing is emitted"));
            assert!(is_error && text.contains(want), "{call}: {text}");
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
    fn the_tool_text_prefers_an_icon_and_keeps_the_rules_for_a_drawn_one() {
        let text = tool_definition()["description"].as_str().unwrap().to_owned();
        for want in [
            "Prefer a bundled icon",
            "Lucide",
            "the one object the goal is about",
            "name it in `reason`",
            "Only when no icon fits",
            "never both",
            "round icon",
            "bright colors",
            "nothing near the edge",
            "no ring, frame, border or background tile",
            "3:1",
        ] {
            assert!(text.contains(want), "{want}");
        }
        for color in avatar_icons::color_names() {
            assert!(text.contains(color), "{color}");
        }
        assert!(text.contains(&crate::avatar_check::hex(crate::avatar_check::PLATE)));
    }

    #[test]
    fn the_schema_requires_the_reason_and_lists_the_colors() {
        let definition = tool_definition();
        let schema = &definition["inputSchema"];
        assert_eq!(schema["required"], json!(["reason"]));
        assert_eq!(schema["properties"]["color"]["enum"], json!(avatar_icons::color_names()));
        for key in ["icon", "color", "svg", "reason"] {
            assert_eq!(schema["properties"][key]["type"], "string", "{key}");
        }
    }
}
