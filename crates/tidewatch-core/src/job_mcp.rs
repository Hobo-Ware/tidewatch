//! The `report_job` tool of the Tidewatch MCP server: an agent reports a wait it knows about (CI
//! checks, a deploy, a loop) and the tab shows it as a job.

use serde_json::{Map, Value, json};

use crate::events::TabEvent;
use crate::job::{JobState, Report};
use crate::mcp::problems_error;

pub const REPORT_TOOL: &str = "report_job";

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": REPORT_TOOL,
        "description": "Report a job you drive turn by turn (CI checks you poll, a deploy you watch) so Tidewatch shows its progress. Call it when the job starts, when it moves and when it ends. Not for a script you run in the background: that script appends JSON lines to $TIDEWATCH_PROGRESS itself. Jobs merge by name, so send the same name each time and only the fields that changed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "A short stable name, for example \"PR checks\"." },
                "done": { "type": "number", "minimum": 0, "description": "How much is finished." },
                "total": { "type": "number", "minimum": 0, "description": "How much there is in all, when known." },
                "unit": { "type": "string", "description": "What done and total count, for example \"checks\" or \"rows\"." },
                "phase": { "type": "string", "description": "What it is doing now, for example \"macOS tests\"." },
                "state": { "type": "string", "enum": ["running", "done", "failed"], "description": "Set done or failed when it ends." },
                "eta_s": { "type": "integer", "minimum": 0, "description": "Seconds left, when you know better than the rate does." },
            },
            "required": ["name"],
            "additionalProperties": false,
        },
    })
}

fn number(args: &Map<String, Value>, key: &str, problems: &mut Vec<String>) -> Option<f64> {
    match args.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(text)) => {
            let parsed = text.trim().replace(',', "").parse::<f64>().ok();
            if parsed.is_none() {
                problems.push(format!("{key} must be a number"));
            }
            parsed
        }
        Some(_) => {
            problems.push(format!("{key} must be a number"));
            None
        }
    }
}

fn word(args: &Map<String, Value>, key: &str, problems: &mut Vec<String>) -> Option<String> {
    match args.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            problems.push(format!("{key} must be a string"));
            None
        }
    }
}

pub fn parse_args(args: &Map<String, Value>) -> Result<Report, Vec<String>> {
    let mut problems = Vec::new();
    let name = word(args, "name", &mut problems).unwrap_or_default();
    let state = word(args, "state", &mut problems).and_then(|word| {
        let state = JobState::parse(&word);
        if state.is_none() {
            problems.push("state must be running, done or failed".to_owned());
        }
        state
    });
    let eta = number(args, "eta_s", &mut problems);
    let report = Report {
        name,
        done: number(args, "done", &mut problems),
        total: number(args, "total", &mut problems),
        unit: word(args, "unit", &mut problems),
        phase: word(args, "phase", &mut problems),
        state,
        eta_s: eta.filter(|secs| secs.is_finite() && *secs >= 0.0).map(|secs| secs as u64),
    };
    let cleaned = report.cleaned();
    if cleaned.is_none() {
        problems.push("name must say what the job is".to_owned());
    }
    match cleaned {
        Some(report) if problems.is_empty() => Ok(report),
        _ => Err(problems),
    }
}

pub fn report(args: &Map<String, Value>, emit: &dyn Fn(TabEvent)) -> (String, bool) {
    match parse_args(args) {
        Ok(report) => {
            let text = format!("Recorded {}.", report.name);
            emit(TabEvent::Job { report });
            (text, false)
        }
        Err(problems) => problems_error(
            &format!("The job was not recorded. Fix these and call {REPORT_TOOL} again:"),
            &problems,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_full_call_becomes_one_report() {
        let call = json!({
            "name": " PR checks ", "done": 3, "total": 5, "unit": "checks",
            "phase": "macOS", "state": "running", "eta_s": 180,
        });
        assert_eq!(
            parse_args(&args(&call)),
            Ok(Report {
                name: "PR checks".into(),
                done: Some(3.0),
                total: Some(5.0),
                unit: Some("checks".into()),
                phase: Some("macOS".into()),
                state: Some(JobState::Running),
                eta_s: Some(180),
            })
        );
    }

    #[test]
    fn only_the_name_is_required() {
        assert_eq!(parse_args(&args(&json!({"name": "deploy"}))), Ok(Report::named("deploy")));
    }

    #[test]
    fn every_problem_is_listed_for_the_agent_to_fix() {
        let cases = [
            (json!({}), vec!["name must say what the job is"]),
            (json!({"name": "  "}), vec!["name must say what the job is"]),
            (json!({"name": 5}), vec!["name must be a string", "name must say what the job is"]),
            (json!({"name": "x", "done": true}), vec!["done must be a number"]),
            (json!({"name": "x", "done": "many"}), vec!["done must be a number"]),
            (
                json!({"name": "x", "state": "sleeping"}),
                vec!["state must be running, done or failed"],
            ),
            (
                json!({"name": "x", "total": [], "unit": 3}),
                vec!["unit must be a string", "total must be a number"],
            ),
        ];
        for (call, want) in cases {
            let mut got = parse_args(&args(&call)).unwrap_err();
            let mut want: Vec<String> = want.into_iter().map(str::to_owned).collect();
            got.sort();
            want.sort();
            assert_eq!(got, want, "{call}");
        }
    }

    #[test]
    fn nonsense_numbers_are_cleaned_not_refused() {
        let report =
            parse_args(&args(&json!({"name": "x", "done": 373, "total": 0, "eta_s": -5}))).unwrap();
        assert_eq!((report.done, report.total, report.eta_s), (Some(373.0), None, None));
    }

    #[test]
    fn a_good_call_emits_the_report_and_a_bad_one_emits_nothing() {
        let seen = RefCell::new(Vec::new());
        let emit = |event| seen.borrow_mut().push(event);
        let (text, is_error) = report(&args(&json!({"name": "deploy", "done": 1})), &emit);
        assert_eq!((text.as_str(), is_error), ("Recorded deploy.", false));
        let (text, is_error) = report(&args(&json!({"done": 1})), &emit);
        assert!(is_error);
        assert!(text.contains("call report_job again"));
        assert_eq!(seen.borrow().len(), 1);
    }

    #[test]
    fn the_schema_names_every_argument_the_parser_reads() {
        let definition = tool_definition();
        let properties = definition["inputSchema"]["properties"].as_object().unwrap();
        for key in ["name", "done", "total", "unit", "phase", "state", "eta_s"] {
            assert!(properties.contains_key(key), "{key}");
        }
        assert_eq!(definition["inputSchema"]["required"], json!(["name"]));
    }
}
