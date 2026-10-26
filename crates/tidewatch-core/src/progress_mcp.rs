//! The two report tools of the Tidewatch MCP server: `update_progress` checks the typed report an
//! agent sends, saves it and shows it in the tab, `get_progress` gives the saved report back after
//! a context reset. The old markdown `ledger` argument is still taken and mapped onto the report.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::progress::{Ledger, MAX_LEDGER_BYTES, Mark, State, When, real};
use crate::progress_report::{
    MAX_NOW_CHARS, MAX_OPTIONS, MAX_STAGES, MIN_OPTIONS, MIN_STAGES, ProgressReport, StageKind,
    StageStatus, StepStatus,
};
use crate::progress_store;

pub const UPDATE_TOOL: &str = "update_progress";
pub const GET_TOOL: &str = "get_progress";
const LEDGER_KEY: &str = "ledger";
const TYPED_KEYS: [&str; 13] = [
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
];
const PREFER_TYPED: &str = "Typed fields are preferred: next time call update_progress with state, now and stages (and steps, needs_you, waiting_on and scheduled when they apply) instead of a markdown ledger.";
const MAX_WARNINGS: usize = 3;
const MAX_WHEN_FIXES: usize = 5;
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
            "Warning: state `{}` is not one of working, needs_you, waiting, blocked or done.",
            shortened(word)
        )),
        (None, None) => Some(
            "Warning: Status has no `state:` line. Set it to working, needs_you, waiting, blocked or done."
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

/// One line for each open Scheduled item whose `<when>` is a date written after `on:` or cannot be
/// read, read from the text as written so it holds whatever the parser makes of it.
#[must_use]
pub fn when_fixes(ledger: &Ledger) -> Vec<String> {
    let mut fixes: Vec<String> = ledger
        .scheduled
        .iter()
        .filter(|item| !matches!(item.mark, Mark::Done | Mark::Skipped))
        .filter_map(|item| {
            let written = item.when_text.trim();
            let what = shortened(&item.what);
            let after_on = written
                .get(..3)
                .filter(|head| head.eq_ignore_ascii_case("on:"))
                .map(|_| written[3..].trim())
                .filter(|date| When::parse(date).date().is_some());
            if let Some(date) = after_on {
                return Some(format!(
                    "fix next save: \"{}\" is a date, write \"{date}\" -- {what}",
                    shortened(written)
                ));
            }
            matches!(item.when, When::Unknown(_)).then(|| {
                format!(
                    "fix next save: \"{}\" is not a when; use YYYY-MM-DD, ~YYYY-MM-DD, on: <event>, parked: <until what> or you -- {what}",
                    shortened(written)
                )
            })
        })
        .collect();
    if fixes.len() > MAX_WHEN_FIXES {
        let more = fixes.len() - MAX_WHEN_FIXES;
        fixes.truncate(MAX_WHEN_FIXES);
        fixes.push(format!("fix next save: {more} more Scheduled items have a when to fix"));
    }
    fixes
}

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
                "description": "A question only the user can answer. Required when state is needs_you, and only then. A question that sits only in your chat message is never seen here.",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "minItems": MIN_OPTIONS, "maxItems": MAX_OPTIONS, "items": { "type": "string" }, "description": "Short answers the user can click." },
                    "recommended": { "type": "integer", "minimum": 0, "description": "The index of the option you recommend." },
                    "why": { "type": "string", "description": "Why you need the answer." },
                },
                "required": ["question", "options", "recommended"],
            },
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
            LEDGER_KEY: { "type": "string", "description": format!("The old format, still accepted: the whole ledger as markdown, at most {} KB. Prefer the typed fields; send one or the other.", MAX_LEDGER_BYTES / 1024) },
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
            "description": "Report the progress of this task. Tidewatch shows it to the user live and maps your state, stages, question and waits onto its screens. Send the whole report first thing and again at every milestone: each call replaces the last. Name 2 to 5 stages that fit this task. Work after a merge, such as a deploy or a backfill, is its own stage. Report done only when the whole task is finished and no step is open. A question for the user goes in needs_you, never only in chat. A job, PR, date or event you wait on goes in waiting_on. If a rule is broken the report is not saved and the reply lists each fix: call again with them.",
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
    (
        format!(
            "The report was not saved. Fix these and call {UPDATE_TOOL} again:\n- {}",
            problems.join("\n- ")
        ),
        true,
    )
}

fn save_failed(e: &std::io::Error) -> (String, bool) {
    (format!("The report could not be saved ({e}). Call {UPDATE_TOOL} again."), true)
}

/// Saves what an agent sent and tells the tab. A report that breaks a rule is a tool error that
/// lists each fix and saves nothing. The reply is a one-line summary, then up to three warning lines.
/// The old markdown `ledger` argument is still taken: it is mapped onto the report and saved the same
/// way, and its reply also carries the old warnings and a line that the typed fields are preferred.
pub fn update(args: &Map<String, Value>, file: &Path, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let typed = TYPED_KEYS.iter().any(|key| args.contains_key(*key));
    if args.contains_key(LEDGER_KEY) && !typed {
        return update_markdown(args, file, emit);
    }
    update_typed(args, file, emit)
}

fn update_typed(args: &Map<String, Value>, file: &Path, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    let mut args = args.clone();
    let both = args.remove(LEDGER_KEY).is_some();
    let size = Value::Object(args.clone()).to_string().len();
    let mut problems = Vec::new();
    if both {
        problems.push(format!("send the typed fields or {LEDGER_KEY}, not both"));
    }
    if size > MAX_LEDGER_BYTES {
        problems.push(format!(
            "the report is {size} bytes, over the {} KB limit: shorten the entries and drop what is finished and no longer needed",
            MAX_LEDGER_BYTES / 1024
        ));
    }
    let saved = progress_store::load_report(file);
    let report = match ProgressReport::check(&args, saved.as_ref()) {
        Ok(report) => Some(report),
        Err(found) => {
            problems.extend(found);
            None
        }
    };
    let Some(report) = report.filter(|_| problems.is_empty()) else {
        return not_saved(&problems);
    };
    if let Err(e) = progress_store::save_report(file, &report) {
        return save_failed(&e);
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

fn update_markdown(
    args: &Map<String, Value>,
    file: &Path,
    emit: &dyn Fn(TabEvent),
) -> (String, bool) {
    let mut problems: Vec<String> = args
        .keys()
        .filter(|key| key.as_str() != LEDGER_KEY)
        .map(|key| format!("unknown argument \"{key}\""))
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
        return save_failed(&e);
    }
    let mut reply = ledger.summary();
    for warning in warnings(&ledger) {
        reply.push('\n');
        reply.push_str(&warning);
    }
    for fix in when_fixes(&ledger) {
        reply.push('\n');
        reply.push_str(&fix);
    }
    if !ledger.stranded_next().is_empty() {
        reply.push('\n');
        reply.push_str(MOVE_TO_SCHEDULED);
    }
    if !progress_store::markdown_was_told(file) && progress_store::mark_markdown_told(file).is_ok()
    {
        reply.push('\n');
        reply.push_str(PREFER_TYPED);
    }
    emit(TabEvent::Progress { ledger: Box::new(ProgressReport::from_ledger(&ledger).to_ledger()) });
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
                "state `asleep` is not one of working, needs_you, waiting, blocked or done",
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
        assert!(lines[0].len() < 150, "{lines:?}");
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
        assert_eq!(lines.len(), 4, "{reply}");
        assert!(lines[3].starts_with("Typed fields are preferred"), "{reply}");
        assert!(file.exists(), "a warning never stops the save");
    }

    fn saved_with(scheduled: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let ledger =
            format!("# Goal\ng\n\n## Status\nstate: working\n\n{STEPS}\n## Scheduled\n{scheduled}");
        let args = serde_json::json!({ "ledger": ledger });
        let (reply, is_error) = update(args.as_object().unwrap(), &file, &|_| {});
        assert!(!is_error);
        assert!(file.exists(), "a bad when never stops the save");
        reply.lines().filter(|line| *line != PREFER_TYPED).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_save_reply_names_a_date_after_on_and_a_when_it_cannot_read() {
        let reply = saved_with(
            "- [ ] on: 2026-11-01 -- Drop beta tracking\n- [ ] On: ~2026-11-07 -- Count old apps\n- [ ] someday -- Tidy up\n- [ ] 2026-13-45 -- Fix the date\n",
        );
        let lines: Vec<&str> = reply.lines().collect();
        assert_eq!(lines[0], "saved: phase 1 Understand, 0/1 done, 4 scheduled", "{reply}");
        assert_eq!(
            lines[1..],
            [
                "fix next save: \"on: 2026-11-01\" is a date, write \"2026-11-01\" -- Drop beta tracking",
                "fix next save: \"On: ~2026-11-07\" is a date, write \"~2026-11-07\" -- Count old apps",
                "fix next save: \"someday\" is not a when; use YYYY-MM-DD, ~YYYY-MM-DD, on: <event>, parked: <until what> or you -- Tidy up",
                "fix next save: \"2026-13-45\" is not a when; use YYYY-MM-DD, ~YYYY-MM-DD, on: <event>, parked: <until what> or you -- Fix the date",
            ]
        );
        assert!(reply.is_ascii());
    }

    #[test]
    fn a_clean_scheduled_section_gets_only_the_saved_line() {
        let reply = saved_with(
            "- [ ] 2026-11-01 -- a\n- [ ] ~2026-11-07 -- b\n- [ ] on: 2026-11-01 ships -- c\n- [ ] on: the partner replies -- d\n- [ ] parked: until the rewrite -- e\n- [ ] you -- f\n- [x] someday -- g\n",
        );
        assert_eq!(reply.lines().count(), 1, "{reply}");
    }

    #[test]
    fn many_bad_whens_are_cut_to_five_and_a_count() {
        let items = "- [ ] soon -- an item\n".repeat(8);
        let lines: Vec<String> = saved_with(&items).lines().map(str::to_owned).collect();
        assert_eq!(lines.len(), 1 + MAX_WHEN_FIXES + 1);
        assert_eq!(lines[6], "fix next save: 3 more Scheduled items have a when to fix");
    }
}
