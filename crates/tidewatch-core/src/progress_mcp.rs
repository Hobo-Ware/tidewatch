//! The two ledger tools of the Tidewatch MCP server: `update_progress` saves the ledger an agent
//! sends and shows it in the tab, `get_progress` gives the saved text back after a context reset.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::progress::{Ledger, MAX_LEDGER_BYTES};
use crate::progress_store;

pub const UPDATE_TOOL: &str = "update_progress";
pub const GET_TOOL: &str = "get_progress";
const LEDGER_KEY: &str = "ledger";

/// The tools' entries for `tools/list`.
#[must_use]
pub fn tool_definitions() -> [Value; 2] {
    [
        json!({
            "name": UPDATE_TOOL,
            "description": "Save the progress ledger of this task. Tidewatch shows it to the user live. Send the whole ledger, in the format the house style gives, first thing and again at every milestone. Returns what to fix when the ledger is not usable, so call it again with the fix.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    LEDGER_KEY: {
                        "type": "string",
                        "description": format!("The whole ledger as markdown, at most {} KB.", MAX_LEDGER_BYTES / 1024),
                    },
                },
                "required": [LEDGER_KEY],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": GET_TOOL,
            "description": "Read back the progress ledger last saved for this task. Call it first after a context reset or when you resume a session, then carry on from it.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            },
        }),
    ]
}

fn not_saved(problems: &[String]) -> (String, bool) {
    (
        format!(
            "The ledger was not saved. Fix these and call {UPDATE_TOOL} again:\n- {}",
            problems.join("\n- ")
        ),
        true,
    )
}

/// Saves the ledger to `file` and tells the tab. A ledger without a goal is a tool error with what
/// to fix, so the agent calls again; anything else is saved. The reply is a one-line summary.
pub fn update(args: &Map<String, Value>, file: &Path, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let mut problems: Vec<String> = args
        .keys()
        .filter(|key| key.as_str() != LEDGER_KEY)
        .map(|key| format!("unknown argument \"{key}\", the only argument is {LEDGER_KEY}"))
        .collect();
    let Some(text) = args.get(LEDGER_KEY).and_then(Value::as_str) else {
        problems
            .push(format!("{LEDGER_KEY} must be a string holding the whole ledger as markdown"));
        return not_saved(&problems);
    };
    if text.len() > MAX_LEDGER_BYTES {
        problems.push(format!(
            "the ledger is {} bytes, over the {} KB limit: shorten the entries and drop what is finished and no longer needed",
            text.len(),
            MAX_LEDGER_BYTES / 1024
        ));
    }
    let ledger = Ledger::parse(text);
    problems.extend(ledger.problems());
    if !problems.is_empty() {
        return not_saved(&problems);
    }
    if let Err(e) = progress_store::save(file, &ledger) {
        return (format!("The ledger could not be saved ({e}). Call {UPDATE_TOOL} again."), true);
    }
    let summary = ledger.summary();
    emit(TabEvent::Progress { ledger: Box::new(ledger) });
    (summary, false)
}

/// The saved text, or a line saying there is none yet.
#[must_use]
pub fn get(file: &Path) -> (String, bool) {
    let text = progress_store::read_text(file).unwrap_or_else(|| {
        format!("No ledger has been saved for this task yet. Write one with {UPDATE_TOOL}.")
    });
    (text, false)
}
