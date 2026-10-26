//! The two ledger tools of the Tidewatch MCP server: `update_progress` saves the ledger an agent
//! sends and shows it in the tab, `get_progress` gives the saved text back after a context reset.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::progress::{Ledger, MAX_LEDGER_BYTES, real};
use crate::progress_store;

pub const UPDATE_TOOL: &str = "update_progress";
pub const GET_TOOL: &str = "get_progress";
const LEDGER_KEY: &str = "ledger";
const MAX_WARNINGS: usize = 3;
const WARNING_TEXT_CHARS: usize = 60;
const MOVE_TO_SCHEDULED: &str = "Next holds only what you do in this same turn. Move later work from Next to Scheduled with a `when`, such as `- [ ] on: the pull request merges -- Watch the deploy -- it ships after the merge`, and call update_progress again.";

fn shortened(text: &str) -> String {
    match text.char_indices().nth(WARNING_TEXT_CHARS) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

#[must_use]
pub fn warnings(ledger: &Ledger) -> Vec<String> {
    let state = match (&ledger.status.unknown_state, ledger.status.state) {
        (Some(word), _) => Some(format!(
            "Warning: state `{}` is not one of working, waiting, blocked or done.",
            shortened(word)
        )),
        (None, None) => Some(
            "Warning: Status has no `state:` line. Set it to working, waiting, blocked or done."
                .to_owned(),
        ),
        (None, Some(_)) => None,
    };
    let empty_checklist = ledger.checklist.iter().all(|phase| phase.items.is_empty()).then(|| {
        "Warning: the Checklist has no steps. List your steps under the four stages.".to_owned()
    });
    let repeated = real(&ledger.needs_you)
        .find(|ask| {
            real(&ledger.next).any(|next| next.text.trim().eq_ignore_ascii_case(ask.text.trim()))
        })
        .map(|ask| {
            format!(
                "Warning: `{}` is in both Needs you and Next. Keep it in Needs you only.",
                shortened(&ask.text)
            )
        });
    let no_options = real(&ledger.needs_you).any(|ask| ask.options.is_empty()).then(|| {
        "Warning: a Needs you line has no options. End each with `-- options: A | B* | C`."
            .to_owned()
    });
    [state, empty_checklist, repeated, no_options]
        .into_iter()
        .flatten()
        .take(MAX_WARNINGS)
        .collect()
}

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
/// to fix, so the agent calls again; anything else is saved. The reply is a one-line summary, then up
/// to three warning lines, then a line when the ledger stopped and left its later work in Next.
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
    let mut reply = ledger.summary();
    for warning in warnings(&ledger) {
        reply.push('\n');
        reply.push_str(&warning);
    }
    if !ledger.stranded_next().is_empty() {
        reply.push('\n');
        reply.push_str(MOVE_TO_SCHEDULED);
    }
    emit(TabEvent::Progress { ledger: Box::new(ledger) });
    (reply, false)
}

/// The saved text, or a line saying there is none yet.
#[must_use]
pub fn get(file: &Path) -> (String, bool) {
    let text = progress_store::read_text(file).unwrap_or_else(|| {
        format!("No ledger has been saved for this task yet. Write one with {UPDATE_TOOL}.")
    });
    (text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: &str = "## Checklist\n### 1. Understand\n- [ ] read\n";

    fn warned(text: &str) -> Vec<String> {
        warnings(&Ledger::parse(text))
    }

    #[test]
    fn a_complete_ledger_has_no_warnings() {
        let text = format!(
            "# Goal\ng\n\n## Status\nstate: working\n\n{STEPS}\n## Needs you\n- Pick -- why -- options: A* | B\n"
        );
        assert_eq!(warned(&text), Vec::<String>::new());
        let nothing =
            format!("# Goal\ng\n\n## Status\nstate: done\n\n{STEPS}\n## Needs you\n- nothing\n");
        assert_eq!(warned(&nothing), Vec::<String>::new());
    }

    #[test]
    fn each_soft_problem_gets_one_short_line() {
        let cases = [
            ("no state", format!("# Goal\ng\n\n{STEPS}"), "no `state:` line"),
            (
                "no state value",
                format!("# Goal\ng\n\n## Status\nstate:\n\n{STEPS}"),
                "no `state:` line",
            ),
            (
                "an unknown state word",
                format!("# Goal\ng\n\n## Status\nstate: asleep\n\n{STEPS}"),
                "state `asleep` is not one of working, waiting, blocked or done",
            ),
            (
                "an empty checklist",
                "# Goal\ng\n\n## Status\nstate: working\n".to_owned(),
                "the Checklist has no steps",
            ),
            (
                "headings only",
                "# Goal\ng\n\n## Status\nstate: working\n\n## Checklist\n### 1. Build\n".to_owned(),
                "the Checklist has no steps",
            ),
            (
                "a question that is also a next step",
                format!(
                    "# Goal\ng\n\n## Status\nstate: working\n\n{STEPS}\n## Next\n- Pick a fix\n\n## Needs you\n- pick a fix -- options: A* | B\n"
                ),
                "`pick a fix` is in both Needs you and Next",
            ),
            (
                "a question with no options",
                format!(
                    "# Goal\ng\n\n## Status\nstate: waiting\n\n{STEPS}\n## Needs you\n- Post the Slack follow-up\n"
                ),
                "a Needs you line has no options",
            ),
        ];
        for (name, text, want) in cases {
            let lines = warned(&text);
            assert_eq!(lines.len(), 1, "{name}: {lines:?}");
            assert!(
                lines[0].starts_with("Warning: ") && lines[0].contains(want),
                "{name}: {lines:?}"
            );
            assert!(lines[0].is_ascii() && !lines[0].contains('\n'), "{name}");
        }
    }

    #[test]
    fn at_most_three_warnings_come_back_most_useful_first() {
        let lines = warned("# Goal\ng\n\n## Next\n- Ship\n\n## Needs you\n- Ship\n");
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("state"), "{lines:?}");
        assert!(lines[1].contains("Checklist"), "{lines:?}");
        assert!(lines[2].contains("both Needs you and Next"), "{lines:?}");
    }

    #[test]
    fn a_long_unknown_word_or_question_is_cut_in_the_warning() {
        let long = "x".repeat(200);
        let lines = warned(&format!("# Goal\ng\n\n## Status\nstate: {long}\n\n{STEPS}"));
        assert!(lines[0].len() < 140, "{lines:?}");
        assert!(lines[0].contains("..."));
    }

    #[test]
    fn the_save_reply_carries_the_warnings_after_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let args = serde_json::json!({ "ledger": "# Goal\ng\n" });
        let (reply, is_error) = update(args.as_object().unwrap(), &file, &|_| {});
        assert!(!is_error);
        let lines: Vec<&str> = reply.lines().collect();
        assert_eq!(lines[0], "saved");
        assert_eq!(lines.len(), 3, "{reply}");
        assert!(file.exists(), "a warning never stops the save");
    }
}
