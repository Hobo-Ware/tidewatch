//! The two report tools of the Tidewatch MCP server: `update_progress` checks the typed report an
//! agent sends, saves it and shows it in the tab, `get_progress` gives the saved report back after
//! a context reset. The old markdown `ledger` argument is a tool error that names the typed fields.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::link_board::{Link, MAX_LINKS_PER_CALL, MAX_NOTE_CHARS, MAX_TITLE_CHARS};
use crate::link_store;
use crate::mcp::problems_error;
use crate::paths::links_file_beside;
use crate::progress::{MAX_LEDGER_BYTES, State};
use crate::progress_report::{
    Checked, MAX_NOW_CHARS, MAX_OPTIONS, MAX_STAGES, MIN_OPTIONS, MIN_STAGES, ProgressReport,
    StageKind, StageStatus, StepStatus,
};
use crate::progress_store;

pub const UPDATE_TOOL: &str = "update_progress";
pub const GET_TOOL: &str = "get_progress";
const LEDGER_KEY: &str = "ledger";
const LEDGER_GONE: &str = "the `ledger` field is gone: send the typed fields, state, now and stages first, then steps, needs_you, waiting_on and scheduled when they apply";

fn words<T: Copy>(all: &[T], word: fn(T) -> &'static str) -> Vec<&'static str> {
    all.iter().copied().map(word).collect()
}

fn note_schema(what: &str) -> Value {
    json!({
        "type": "array",
        "description": what,
        "items": {
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "why": { "type": "string" },
            },
            "required": ["text"],
        },
    })
}

fn links_schema(what: &str, with_for: bool) -> Value {
    let mut properties = json!({
        "url": { "type": "string", "description": "An http or https address." },
        "title": { "type": "string", "maxLength": MAX_TITLE_CHARS, "description": format!("What the link is, in one line of at most {MAX_TITLE_CHARS} characters.") },
    });
    if with_for {
        properties["for"] = json!({ "type": "string", "maxLength": MAX_NOTE_CHARS, "description": "A short note on what the link is for." });
    }
    json!({
        "type": "array",
        "maxItems": MAX_LINKS_PER_CALL,
        "description": what,
        "items": {
            "type": "object",
            "properties": properties,
            "required": ["url", "title"],
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "goal": { "type": "string", "description": "The task in one line. Required on the first report; later reports may leave it out." },
            "title": { "type": "string", "description": "A short heading for the task, when the goal is long." },
            "state": {
                "type": "string",
                "enum": words(&State::ALL, State::word),
                "description": "working: you are on it. needs_you: only the user can unblock it, and needs_you holds the question. waiting: a job, PR, date or event you named in waiting_on has to happen first. blocked: something outside your control stops you. done: the whole task is finished, nothing is open.",
            },
            "now": { "type": "string", "maxLength": MAX_NOW_CHARS, "description": format!("One plain sentence on what is happening right now, at most {MAX_NOW_CHARS} characters.") },
            "kind": { "type": "string", "enum": ["read-only", "edit"], "description": "Only to say the task is a read-only question or investigation, or that it now edits files." },
            "stages": {
                "type": "array",
                "minItems": MIN_STAGES,
                "maxItems": MAX_STAGES,
                "description": format!("{MIN_STAGES} to {MAX_STAGES} stages, named to fit this task, in order. Work after a merge, such as a deploy, a backfill or a live check, is its own stage."),
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "kind": { "type": "string", "enum": words(StageKind::ALL, StageKind::word), "description": "What the stage is for, whatever you called it." },
                        "status": { "type": "string", "enum": words(StageStatus::ALL, StageStatus::word) },
                    },
                    "required": ["name", "kind", "status"],
                },
            },
            "steps": {
                "type": "array",
                "description": "The concrete steps of this task, each under one of your stages.",
                "items": {
                    "type": "object",
                    "properties": {
                        "stage": { "type": "string", "description": "The name of one of your stages." },
                        "text": { "type": "string" },
                        "status": { "type": "string", "enum": words(StepStatus::ALL, StepStatus::word) },
                        "evidence": { "type": "string", "description": "For a done step: what shows it is done." },
                        "reason": { "type": "string", "description": "For a blocked or skipped step: why." },
                    },
                    "required": ["stage", "text", "status"],
                },
            },
            "needs_you": {
                "type": "object",
                "description": "A question only the user can answer, for state needs_you and only then. A question you ask with your question tool is filled in by Tidewatch; use this one for a question that waits on something slower.",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "minItems": MIN_OPTIONS, "maxItems": MAX_OPTIONS, "items": { "type": "string" }, "description": "Short answers the user can click." },
                    "recommended": { "type": "integer", "minimum": 0, "description": "The index of the option you recommend." },
                    "why": { "type": "string", "description": "Why you need the answer." },
                    "links": links_schema("Links the user should read before answering, such as an artifact or a pull request. They stay tied to this question.", false),
                },
                "required": ["question", "options", "recommended"],
            },
            "links": links_schema(&format!("Links you share with the user: artifacts, pull requests, docs. Tidewatch keeps every link for the life of the task, so send each one once, with a title and what it is for. At most {MAX_LINKS_PER_CALL} per call."), true),
            "waiting_on": {
                "type": "object",
                "description": "What you wait for, exactly one of job, pr, date or event. Required when state is waiting.",
                "properties": {
                    "job": { "type": "string", "description": "A job name, as you report it with report_job." },
                    "pr": { "type": "integer", "minimum": 1, "description": "A pull request number." },
                    "date": { "type": "string", "description": "YYYY-MM-DD." },
                    "event": { "type": "string" },
                },
            },
            "scheduled": {
                "type": "array",
                "description": "Later work that is not for this turn. Everything you tell the user about later belongs here.",
                "items": {
                    "type": "object",
                    "properties": {
                        "when": {
                            "type": "object",
                            "description": "Exactly one of date (YYYY-MM-DD), about (YYYY-MM-DD, a guess), event (waits for something to happen), parked (set aside until what) or you (only the user can decide it, written as {\"you\": true}).",
                            "properties": {
                                "date": { "type": "string" },
                                "about": { "type": "string" },
                                "event": { "type": "string" },
                                "parked": { "type": "string" },
                                "you": { "type": "boolean" },
                            },
                        },
                        "text": { "type": "string" },
                        "why": { "type": "string" },
                    },
                    "required": ["when", "text"],
                },
            },
            "verified": note_schema("What you checked, with the command and its result as why."),
            "not_verified": note_schema("What you could not check, with the blocker as why."),
            "decisions": note_schema("Decisions made, with the reason as why."),
        },
        "required": ["state", "now", "stages"],
        "additionalProperties": false,
    })
}

/// The tools' entries for `tools/list`.
#[must_use]
pub fn tool_definitions() -> [Value; 2] {
    [
        json!({
            "name": UPDATE_TOOL,
            "description": "Report the task's state to the user. Each call replaces the last. Send it once work starts and before your final reply.",
            "inputSchema": input_schema(),
        }),
        json!({
            "name": GET_TOOL,
            "description": "Read back the progress report last saved for this task. Call it first after a context reset or when you resume a session, then carry on from it.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            },
        }),
    ]
}

fn not_saved(problems: &[String]) -> (String, bool) {
    problems_error(
        &format!("The report was not saved. Fix these and call {UPDATE_TOOL} again:"),
        problems,
    )
}

fn save_failed(e: &std::io::Error) -> (String, bool) {
    (format!("The report could not be saved ({e}). Call {UPDATE_TOOL} again."), true)
}

/// Saves what an agent sent and tells the tab. A report that breaks a rule is a tool error that
/// lists each fix and saves nothing. The reply is a one-line summary, then up to three warning lines.
/// The old markdown `ledger` argument is one of those rules.
pub fn update(args: &Map<String, Value>, file: &Path, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let mut args = args.clone();
    let mut problems = Vec::new();
    if args.remove(LEDGER_KEY).is_some() {
        problems.push(LEDGER_GONE.to_owned());
    }
    let size = Value::Object(args.clone()).to_string().len();
    if size > MAX_LEDGER_BYTES {
        problems.push(format!(
            "the report is {size} bytes, over the {} KB limit: shorten the entries and drop what is finished and no longer needed",
            MAX_LEDGER_BYTES / 1024
        ));
    }
    let saved = progress_store::load_report(file);
    let checked = match ProgressReport::check_with_links(&args, saved.as_ref()) {
        Ok(checked) => Some(checked),
        Err(found) => {
            problems.extend(found);
            None
        }
    };
    let Some(Checked { report, links }) = checked.filter(|_| problems.is_empty()) else {
        return not_saved(&problems);
    };
    if let Err(e) = progress_store::save_report(file, &report) {
        return save_failed(&e);
    }
    let links: Vec<Link> =
        links.into_iter().map(|link| link.into_link(crate::clock::now_ms())).collect();
    if !links.is_empty() {
        if let Err(e) = link_store::merge(&links_file_beside(file), &links) {
            return save_failed(&e);
        }
        emit(TabEvent::Links { links });
    }
    let ledger = report.to_ledger();
    let mut reply = ledger.summary();
    for warning in report.warnings() {
        reply.push('\n');
        reply.push_str(&warning);
    }
    emit(TabEvent::Progress { ledger: Box::new(ledger) });
    (reply, false)
}

/// The saved report as JSON, or a line saying there is none.
#[must_use]
pub fn get(file: &Path) -> (String, bool) {
    let text = progress_store::load_report(file).map_or_else(
        || format!("No report has been saved for this task yet. Write one with {UPDATE_TOOL}."),
        |report| report.to_json(),
    );
    (text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(extra: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        let mut args = serde_json::json!({
            "goal": "Ship it",
            "state": "working",
            "now": "Writing",
            "stages": [
                { "name": "Build", "kind": "build", "status": "active" },
                { "name": "Ship", "kind": "ship", "status": "todo" }
            ],
            "steps": [{ "stage": "Build", "text": "Write", "status": "active" }],
        });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        args.as_object().unwrap().clone()
    }

    fn run(
        args: &serde_json::Map<String, serde_json::Value>,
        file: &Path,
    ) -> (String, bool, usize) {
        let events = std::cell::RefCell::new(Vec::new());
        let (reply, is_error) = update(args, file, &|event| events.borrow_mut().push(event));
        let links = events
            .borrow()
            .iter()
            .map(|event| match event {
                TabEvent::Links { links } => links.len(),
                _ => 0,
            })
            .sum();
        (reply, is_error, links)
    }

    #[test]
    fn reported_links_are_kept_beside_the_report_and_announced_to_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let args = typed(&serde_json::json!({ "links": [
            { "url": "https://claude.ai/artifact/abc", "title": "Board", "for": "the pick" },
        ]}));
        let (reply, is_error, announced) = run(&args, &file);
        assert!(!is_error, "{reply}");
        assert_eq!(announced, 1);
        let board = link_store::load(&links_file_beside(&file)).unwrap();
        assert_eq!(board.links[0].title, "Board");
        assert_eq!(board.links[0].note.as_deref(), Some("the pick"));
        assert!(board.links[0].first_seen_ms > 0);
    }

    #[test]
    fn a_later_report_never_drops_a_link_and_a_report_without_links_touches_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let first = typed(&serde_json::json!({ "links": [
            { "url": "https://a.dev/1", "title": "One" },
            { "url": "https://a.dev/2", "title": "Two" },
        ]}));
        run(&first, &file);
        let (_, _, announced) = run(&typed(&serde_json::json!({})), &file);
        assert_eq!(announced, 0, "no links, no event");
        let again = typed(&serde_json::json!({ "links": [
            { "url": "https://a.dev/1", "title": "One, renamed", "for": "now with a reason" },
            { "url": "https://a.dev/3", "title": "Three" },
        ]}));
        run(&again, &file);
        let board = link_store::load(&links_file_beside(&file)).unwrap();
        let titles: Vec<&str> = board.links.iter().map(|link| link.title.as_str()).collect();
        assert_eq!(titles, ["One, renamed", "Two", "Three"]);
        assert_eq!(board.links[0].note.as_deref(), Some("now with a reason"));
    }

    #[test]
    fn a_bad_link_refuses_the_whole_report_and_saves_no_link() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let args = typed(&serde_json::json!({ "links": [
            { "url": "https://a.dev/1", "title": "Fine" },
            { "url": "ftp://a.dev/2", "title": "Bad" },
        ]}));
        let (reply, is_error, announced) = run(&args, &file);
        assert!(is_error && announced == 0);
        assert!(reply.starts_with("The report was not saved."), "{reply}");
        assert!(
            reply.contains("links[1].url \"ftp://a.dev/2\" must be an http or https link"),
            "{reply}"
        );
        assert!(!links_file_beside(&file).exists());
        assert!(!file.exists());
    }

    #[test]
    fn the_tool_schema_describes_links_with_their_limits_and_the_question_links() {
        let [update, _] = tool_definitions();
        let schema = &update["inputSchema"]["properties"];
        assert_eq!(schema["links"]["maxItems"], 20);
        assert_eq!(schema["links"]["items"]["properties"]["title"]["maxLength"], 120);
        assert_eq!(schema["links"]["items"]["required"], serde_json::json!(["url", "title"]));
        assert!(schema["links"]["items"]["properties"]["for"].is_object());
        let ask = &schema["needs_you"]["properties"]["links"];
        assert_eq!(ask["maxItems"], 20);
        assert!(ask["items"]["properties"]["for"].is_null());
    }

    #[test]
    fn the_old_ledger_field_is_a_tool_error_that_names_the_typed_fields_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let sent = std::cell::Cell::new(0);
        let args = serde_json::json!({ "ledger": "# Goal\ng\n" });
        let (reply, is_error) =
            update(args.as_object().unwrap(), &file, &|_| sent.set(sent.get() + 1));
        assert!(is_error);
        assert!(
            reply.starts_with(
                "The report was not saved. Fix these and call update_progress again:\n- "
            ),
            "{reply}"
        );
        assert!(
            reply.contains(
                "the `ledger` field is gone: send the typed fields, state, now and stages"
            ),
            "{reply}"
        );
        assert_eq!((file.exists(), sent.get()), (false, 0));
    }

    #[test]
    fn a_ledger_beside_good_typed_fields_is_still_refused_and_nothing_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let args = typed(&serde_json::json!({ "ledger": "# Goal\ng\n" }));
        let (reply, is_error, _) = run(&args, &file);
        assert!(is_error, "{reply}");
        assert_eq!(reply.lines().count(), 2, "{reply}");
        assert!(!file.exists());
    }

    #[test]
    fn the_schema_has_no_ledger_field_and_the_description_is_the_short_one() {
        let [update, _] = tool_definitions();
        assert!(update["inputSchema"]["properties"].get("ledger").is_none());
        assert_eq!(
            update["description"],
            "Report the task's state to the user. Each call replaces the last. Send it once work starts and before your final reply."
        );
    }
}
