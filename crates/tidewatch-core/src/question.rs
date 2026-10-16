//! Multiple-choice questions an agent asks the user, and the answers that go back: the model, the
//! two ways a question arrives (Claude Code's own `AskUserQuestion` tool call, read from its
//! session file, and the Tidewatch MCP tool `ask_user` for every other CLI), the answer text, and
//! the reply channel a held `ask_user` call waits on. Pure; the Reader draws the card.
//!
//! Claude Code 2.1.301 records the call as `"name":"AskUserQuestion","input":{"questions":[{"question",
//! "header","multiSelect","options":[{"label","description"}]}]}` and the result as a `tool_result`
//! whose text is `Your questions have been answered: "Q"="A", "Q2"="A2". ...` (or `The user answered:
//! "Q"="A". ...` when the answer was typed), with the same answers as an object in the line's
//! `toolUseResult.answers`. A refusal is an error result ("The user doesn't want to proceed...").

use std::sync::Arc;
use std::sync::mpsc::Sender;

use serde_json::{Map, Value, json};

pub const TOOL_NAME: &str = "ask_user";
pub const MAX_QUESTIONS: usize = 4;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 6;
const MAX_QUESTION_CHARS: usize = 400;
const MAX_HEADER_CHARS: usize = 40;
const MAX_LABEL_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 400;
const RECOMMENDED: &str = "(Recommended)";

const QUESTION_KEYS: [&str; 5] = ["question", "header", "multi_select", "multiSelect", "options"];
const OPTION_KEYS: [&str; 2] = ["label", "description"];

/// What the tool result says when the user chose to leave the choice to the agent.
pub const SKIPPED_TEXT: &str = "The user skipped your questions and left the choice to you. Use your best judgment and carry on.";
/// What the tool result says when nobody answered in time.
pub const TIMED_OUT_TEXT: &str =
    "The user did not answer in time. Proceed with your best judgment.";
/// What the tool result says when the question went away unanswered (the tab was closed).
pub const CLOSED_TEXT: &str = "The question could not be answered because its tab was closed. Proceed with your best judgment.";
/// What a skip types into an agent that asked with its own tool, after the menu is dismissed.
pub const SKIP_REPLY: &str = "Skip your questions and use your best judgment.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    Single,
    Multi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

impl QuestionOption {
    /// The label as shown: without a trailing "(Recommended)".
    #[must_use]
    pub fn shown(&self) -> &str {
        split_recommended(&self.label).0
    }

    #[must_use]
    pub fn recommended(&self) -> bool {
        split_recommended(&self.label).1
    }
}

/// A label without its trailing "(Recommended)" (any case), and whether it had one.
#[must_use]
pub fn split_recommended(label: &str) -> (&str, bool) {
    let trimmed = label.trim_end();
    let cut = trimmed.len().checked_sub(RECOMMENDED.len());
    match cut.filter(|at| trimmed.is_char_boundary(*at)) {
        Some(at) if trimmed[at..].eq_ignore_ascii_case(RECOMMENDED) => {
            (trimmed[..at].trim_end(), true)
        }
        _ => (trimmed, false),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub text: String,
    pub header: Option<String>,
    pub select: Select,
    pub options: Vec<QuestionOption>,
}

/// What the user gave for one question: the options picked (by index) and a typed answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Given {
    pub picked: Vec<usize>,
    pub other: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Answered(Vec<Given>),
    Skipped,
}

/// The answer of one question as one line: the picked labels, then the typed text.
#[must_use]
pub fn answer_line(question: &Question, given: &Given) -> String {
    let mut parts: Vec<&str> = given
        .picked
        .iter()
        .filter_map(|index| question.options.get(*index))
        .map(QuestionOption::shown)
        .collect();
    if let Some(other) = &given.other {
        parts.push(other);
    }
    parts.join(", ")
}

/// Each question with its answer, for the summary card.
#[must_use]
pub fn pairs(questions: &[Question], given: &[Given]) -> Vec<(String, String)> {
    questions
        .iter()
        .zip(given)
        .map(|(question, given)| (question.text.clone(), answer_line(question, given)))
        .collect()
}

/// The answers as the tool result an agent reads: `Q: ...` and `A: ...` per question.
#[must_use]
pub fn answer_text(questions: &[Question], given: &[Given]) -> String {
    pairs(questions, given)
        .iter()
        .map(|(question, answer)| format!("Q: {question}\nA: {answer}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The answers as a message typed to an agent whose `ask_user` call already ended (it timed out or
/// its client gave up), so a late answer still reaches it.
#[must_use]
pub fn late_answer_text(questions: &[Question], given: &[Given]) -> String {
    format!("My answers to your earlier questions:\n\n{}", answer_text(questions, given))
}

/// The reply channel of one held `ask_user` call. Sending fails once the call has ended.
#[derive(Clone)]
pub struct AskReply(Arc<Sender<Reply>>);

impl AskReply {
    #[must_use]
    pub fn new(sender: Sender<Reply>) -> Self {
        Self(Arc::new(sender))
    }

    /// Whether the held call took the reply.
    #[must_use]
    pub fn send(&self, reply: Reply) -> bool {
        self.0.send(reply).is_ok()
    }
}

impl std::fmt::Debug for AskReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AskReply")
    }
}

impl PartialEq for AskReply {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for AskReply {}

fn clean(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The questions of a Claude Code `AskUserQuestion` call's input. Claude validates its own calls,
/// so this only drops what cannot be shown: a question with no text or no options.
#[must_use]
pub fn from_claude_input(input: &Value) -> Option<Vec<Question>> {
    let questions: Vec<Question> = input["questions"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            let text = clean(item["question"].as_str()?)?;
            let options: Vec<QuestionOption> = item["options"]
                .as_array()?
                .iter()
                .filter_map(|option| {
                    Some(QuestionOption {
                        label: clean(option["label"].as_str()?)?,
                        description: option["description"].as_str().and_then(clean),
                    })
                })
                .collect();
            (!options.is_empty()).then(|| Question {
                text,
                header: item["header"].as_str().and_then(clean),
                select: if item["multiSelect"].as_bool() == Some(true) {
                    Select::Multi
                } else {
                    Select::Single
                },
                options,
            })
        })
        .collect();
    (!questions.is_empty()).then_some(questions)
}

/// Each answered question and its answer from a Claude `AskUserQuestion` result: the line's
/// `toolUseResult.answers` (in the order of its `questions`) when it has them, else the pairs in the
/// result text.
#[must_use]
pub fn claude_answers(structured: &Value, content: &str) -> Vec<(String, String)> {
    if let Some(answers) = structured["answers"].as_object() {
        let mut ordered: Vec<(String, String)> = Vec::new();
        for question in structured["questions"].as_array().into_iter().flatten() {
            let Some(text) = question["question"].as_str() else { continue };
            if let Some(answer) = answers.get(text).and_then(Value::as_str) {
                ordered.push((text.to_owned(), answer.to_owned()));
            }
        }
        for (text, answer) in answers {
            if let Some(answer) = answer.as_str()
                && !ordered.iter().any(|(seen, _)| seen == text)
            {
                ordered.push((text.clone(), answer.to_owned()));
            }
        }
        if !ordered.is_empty() {
            return ordered;
        }
    }
    pairs_in_text(content)
}

/// The `"Q"="A"` pairs of a result text such as `Your questions have been answered: "Q"="A", "Q2"="B".
/// You can now continue...`.
fn pairs_in_text(content: &str) -> Vec<(String, String)> {
    let Some((_, rest)) = content.split_once(": \"") else { return Vec::new() };
    let body = format!("\"{rest}");
    let mut found = Vec::new();
    let mut remaining = body.as_str();
    while let Some(after) = remaining.strip_prefix('"') {
        let Some((question, tail)) = after.split_once("\"=\"") else { break };
        let Some(end) =
            tail.find("\", \"").or_else(|| tail.find("\". ")).or_else(|| tail.rfind('"'))
        else {
            break;
        };
        found.push((question.to_owned(), tail[..end].to_owned()));
        remaining = tail[end + 1..].trim_start_matches([',', ' ']);
    }
    found
}

/// The `ask_user` entry for `tools/list`.
#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Ask the user one to four multiple-choice questions and wait for the answers. Tidewatch shows them as radio buttons (one answer) or checkboxes (several) with a field for another answer. Use it instead of guessing when the user's choice changes what you do. The result lists each question with the answer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_QUESTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The question, ending with a question mark." },
                            "header": { "type": "string", "description": "A word or two naming the topic." },
                            "multi_select": { "type": "boolean", "description": "True when several options may be picked. Default false." },
                            "options": {
                                "type": "array",
                                "minItems": MIN_OPTIONS,
                                "maxItems": MAX_OPTIONS,
                                "description": "The choices. Put the one you recommend first and end its label with (Recommended). Tidewatch adds a Something else field itself.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "description": "A short name for the choice." },
                                        "description": { "type": "string", "description": "What choosing it means." },
                                    },
                                    "required": ["label"],
                                    "additionalProperties": false,
                                },
                            },
                        },
                        "required": ["question", "options"],
                        "additionalProperties": false,
                    },
                },
            },
            "required": ["questions"],
            "additionalProperties": false,
        },
    })
}

fn unknown_keys(object: &Map<String, Value>, known: &[&str], context: &str) -> Vec<String> {
    object
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .map(|key| format!("{context}unknown field \"{key}\""))
        .collect()
}

fn text_field(
    object: &Map<String, Value>,
    key: &str,
    max: usize,
    context: &str,
    problems: &mut Vec<String>,
) -> Option<String> {
    match object.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let text = clean(text)?;
            if text.chars().count() > max {
                problems.push(format!("{context}{key} is longer than {max} characters"));
                return None;
            }
            Some(text)
        }
        Some(_) => {
            problems.push(format!("{context}{key} must be a string"));
            None
        }
    }
}

fn option_from(value: &Value, context: &str, problems: &mut Vec<String>) -> Option<QuestionOption> {
    let Some(object) = value.as_object() else {
        problems.push(format!("{context}must be an object with a label"));
        return None;
    };
    problems.extend(unknown_keys(object, &OPTION_KEYS, context));
    let Some(label) = text_field(object, "label", MAX_LABEL_CHARS, context, problems) else {
        problems.push(format!("{context}needs a label"));
        return None;
    };
    Some(QuestionOption {
        label,
        description: text_field(object, "description", MAX_DESCRIPTION_CHARS, context, problems),
    })
}

fn question_from(
    object: &Map<String, Value>,
    number: usize,
    problems: &mut Vec<String>,
) -> Question {
    let context = format!("question {number}: ");
    problems.extend(unknown_keys(object, &QUESTION_KEYS, &context));
    let text = text_field(object, "question", MAX_QUESTION_CHARS, &context, problems)
        .unwrap_or_else(|| {
            problems.push(format!("{context}needs question text"));
            String::new()
        });
    let header = text_field(object, "header", MAX_HEADER_CHARS, &context, problems);
    let select = match object.get("multi_select").or_else(|| object.get("multiSelect")) {
        None | Some(Value::Null | Value::Bool(false)) => Select::Single,
        Some(Value::Bool(true)) => Select::Multi,
        Some(_) => {
            problems.push(format!("{context}multi_select must be true or false"));
            Select::Single
        }
    };
    let mut options = Vec::new();
    match object.get("options") {
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let at = format!("{context}option {}: ", index + 1);
                options.extend(option_from(item, &at, problems));
            }
        }
        _ => problems.push(format!("{context}options must be a list")),
    }
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()) {
        problems.push(format!(
            "{context}give {MIN_OPTIONS} to {MAX_OPTIONS} options, not {}",
            options.len()
        ));
    }
    for (index, option) in options.iter().enumerate() {
        let shown = option.shown().to_lowercase();
        if options[..index].iter().any(|earlier| earlier.shown().to_lowercase() == shown) {
            problems.push(format!("{context}the option \"{}\" appears twice", option.shown()));
        }
    }
    Question { text, header, select, options }
}

/// The questions the `ask_user` arguments describe, or every way they do not (in words for the
/// agent, which reads them as the tool error and calls again).
pub fn from_tool_args(args: &Map<String, Value>) -> Result<Vec<Question>, Vec<String>> {
    let mut problems = unknown_keys(args, &["questions"], "");
    let mut questions = Vec::new();
    match args.get("questions") {
        Some(Value::Array(items)) if (1..=MAX_QUESTIONS).contains(&items.len()) => {
            for (index, item) in items.iter().enumerate() {
                match item.as_object() {
                    Some(object) => questions.push(question_from(object, index + 1, &mut problems)),
                    None => problems.push(format!("question {}: must be an object", index + 1)),
                }
            }
        }
        Some(Value::Array(items)) => problems
            .push(format!("ask 1 to {MAX_QUESTIONS} questions in one call, not {}", items.len())),
        _ => problems.push("questions must be a list of questions".to_owned()),
    }
    if problems.is_empty() { Ok(questions) } else { Err(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(label: &str) -> QuestionOption {
        QuestionOption { label: label.into(), description: None }
    }

    fn question(text: &str, select: Select, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            select,
            options: labels.iter().map(|label| option(label)).collect(),
        }
    }

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_recommended_suffix_is_split_off_in_any_case() {
        let cases = [
            ("Wavy line (Recommended)", ("Wavy line", true)),
            ("Wavy line (recommended)  ", ("Wavy line", true)),
            ("Wavy line", ("Wavy line", false)),
            ("(Recommended)", ("", true)),
            ("Recommended option", ("Recommended option", false)),
            ("Cafe\u{301} (Recommended)", ("Cafe\u{301}", true)),
            ("h\u{e9}", ("h\u{e9}", false)),
        ];
        for (label, want) in cases {
            assert_eq!(split_recommended(label), want, "{label:?}");
        }
    }

    #[test]
    fn claude_input_becomes_questions() {
        let input = json!({"questions": [
            {"question": "Which color?", "header": "Color", "multiSelect": false,
             "options": [{"label": "Red (Recommended)", "description": "warm"}, {"label": "Blue"}]},
            {"question": "Which toppings?", "multiSelect": true,
             "options": [{"label": "Salt", "description": "  "}, {"label": "Sugar"}]},
        ]});
        let questions = from_claude_input(&input).unwrap();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].header.as_deref(), Some("Color"));
        assert_eq!(questions[0].select, Select::Single);
        assert!(questions[0].options[0].recommended());
        assert_eq!(questions[0].options[0].shown(), "Red");
        assert_eq!(questions[0].options[0].description.as_deref(), Some("warm"));
        assert_eq!(questions[0].options[1].description, None);
        assert_eq!(questions[1].select, Select::Multi);
        assert_eq!(questions[1].options[0].description, None);
    }

    #[test]
    fn claude_input_without_anything_to_show_is_nothing() {
        let cases = [
            json!({}),
            json!({"questions": []}),
            json!({"questions": "no"}),
            json!({"questions": [{"question": "", "options": [{"label": "A"}]}]}),
            json!({"questions": [{"question": "Q?", "options": []}]}),
            json!({"questions": [{"question": "Q?", "options": [{"description": "x"}]}]}),
            json!({"questions": [7]}),
        ];
        for input in cases {
            assert_eq!(from_claude_input(&input), None, "{input}");
        }
    }

    #[test]
    fn answers_read_from_a_claude_result() {
        let structured = json!({
            "questions": [{"question": "Which toppings?"}, {"question": "Which color?"}],
            "answers": {"Which color?": "Green", "Which toppings?": "Salt, Sugar"},
        });
        let want = vec![
            ("Which toppings?".to_owned(), "Salt, Sugar".to_owned()),
            ("Which color?".to_owned(), "Green".to_owned()),
        ];
        assert_eq!(claude_answers(&structured, "ignored"), want);
    }

    #[test]
    fn answers_fall_back_to_the_result_text_in_every_wording() {
        let cases = [
            (
                "Your questions have been answered: \"Which color?\"=\"Green\", \"Which toppings?\"=\"Salt, Sugar\". You can now continue with these answers in mind.",
                vec![("Which color?", "Green"), ("Which toppings?", "Salt, Sugar")],
            ),
            (
                "The user answered: \"Which mode?\"=\"turbo mode\". Read the answers carefully \u{2014} they may request clarification.",
                vec![("Which mode?", "turbo mode")],
            ),
            (
                "User has answered your questions: \"Where?\"=\"On the stage\". You can now continue.",
                vec![("Where?", "On the stage")],
            ),
            ("nothing quoted here", vec![]),
            ("", vec![]),
        ];
        for (content, want) in cases {
            let want: Vec<(String, String)> =
                want.into_iter().map(|(q, a)| (q.to_owned(), a.to_owned())).collect();
            assert_eq!(claude_answers(&Value::Null, content), want, "{content}");
        }
    }

    #[test]
    fn answer_text_lists_each_question_with_its_picks_and_typed_answer() {
        let questions = [
            question(
                "Where should the retry count live?",
                Select::Single,
                &["On the stage (Recommended)", "On the run"],
            ),
            question(
                "Which tests should retry?",
                Select::Multi,
                &["Unit tests", "Integration tests", "Lints"],
            ),
            question("Anything else?", Select::Single, &["No", "Yes"]),
        ];
        let given = [
            Given { picked: vec![0], other: None },
            Given { picked: vec![0, 1], other: Some("doc tests".into()) },
            Given { picked: vec![], other: Some("only on CI".into()) },
        ];
        assert_eq!(
            answer_text(&questions, &given),
            "Q: Where should the retry count live?\nA: On the stage\n\nQ: Which tests should retry?\nA: Unit tests, Integration tests, doc tests\n\nQ: Anything else?\nA: only on CI"
        );
        assert!(
            late_answer_text(&questions, &given)
                .starts_with("My answers to your earlier questions:\n\nQ: Where")
        );
        assert_eq!(pairs(&questions, &given)[1].1, "Unit tests, Integration tests, doc tests");
    }

    #[test]
    fn a_picked_index_out_of_range_is_ignored() {
        let q = question("Q?", Select::Single, &["A", "B"]);
        assert_eq!(answer_line(&q, &Given { picked: vec![5], other: None }), "");
    }

    #[test]
    fn valid_tool_arguments_become_questions() {
        let questions = from_tool_args(&args(&json!({"questions": [
            {"question": "Q1?", "header": "One", "options": [{"label": "A (Recommended)", "description": "first"}, {"label": "B"}]},
            {"question": "Q2?", "multi_select": true, "options": [{"label": "X"}, {"label": "Y"}, {"label": "Z"}]},
            {"question": "Q3?", "multiSelect": true, "options": [{"label": "X"}, {"label": "Y"}]},
        ]})))
        .unwrap();
        assert_eq!(questions.len(), 3);
        assert_eq!(questions[0].select, Select::Single);
        assert_eq!(questions[1].select, Select::Multi);
        assert_eq!(questions[2].select, Select::Multi);
        assert_eq!(questions[0].header.as_deref(), Some("One"));
    }

    #[test]
    fn invalid_tool_arguments_say_what_to_fix() {
        let two = json!([{"label": "A"}, {"label": "B"}]);
        let cases: Vec<(Value, &str)> = vec![
            (json!({}), "questions must be a list"),
            (json!({"questions": []}), "ask 1 to 4 questions"),
            (
                json!({"questions": (0..5).map(|_| json!({"question": "Q?", "options": two})).collect::<Vec<_>>()}),
                "ask 1 to 4 questions in one call, not 5",
            ),
            (json!({"questions": [5]}), "question 1: must be an object"),
            (json!({"questions": [{"options": two}]}), "question 1: needs question text"),
            (
                json!({"questions": [{"question": "Q?", "options": [{"label": "A"}]}]}),
                "question 1: give 2 to 6 options, not 1",
            ),
            (
                json!({"questions": [{"question": "Q?", "options": (0..7).map(|n| json!({"label": format!("o{n}")})).collect::<Vec<_>>()}]}),
                "give 2 to 6 options, not 7",
            ),
            (json!({"questions": [{"question": "Q?"}]}), "question 1: options must be a list"),
            (
                json!({"questions": [{"question": "Q?", "options": [{"label": "A"}, {"description": "x"}]}]}),
                "question 1: option 2: needs a label",
            ),
            (
                json!({"questions": [{"question": "Q?", "options": [{"label": "A"}, "B"]}]}),
                "question 1: option 2: must be an object with a label",
            ),
            (
                json!({"questions": [{"question": "Q?", "options": [{"label": "A"}, {"label": "a (Recommended)"}]}]}),
                "the option \"a\" appears twice",
            ),
            (
                json!({"questions": [{"question": "Q?", "multi_select": "yes", "options": two}]}),
                "question 1: multi_select must be true or false",
            ),
            (
                json!({"questions": [{"question": "Q?", "color": "red", "options": two}]}),
                "question 1: unknown field \"color\"",
            ),
            (json!({"questions": [], "extra": 1}), "unknown field \"extra\""),
            (
                json!({"questions": [{"question": "x".repeat(401), "options": two}]}),
                "question is longer than 400 characters",
            ),
        ];
        for (arguments, want) in cases {
            let problems = from_tool_args(&args(&arguments)).unwrap_err();
            assert!(
                problems.iter().any(|problem| problem.contains(want)),
                "{arguments}: {problems:?} lacks {want:?}"
            );
        }
    }

    #[test]
    fn every_problem_is_listed_not_just_the_first() {
        let problems = from_tool_args(&args(&json!({"questions": [
            {"question": "", "options": [{"label": "A"}]},
            {"question": "Q?", "options": []},
        ]})))
        .unwrap_err();
        assert!(problems.len() >= 3, "{problems:?}");
    }

    #[test]
    fn the_tool_schema_names_its_limits() {
        let schema = tool_definition();
        assert_eq!(schema["name"], TOOL_NAME);
        let items = &schema["inputSchema"]["properties"]["questions"];
        assert_eq!(items["maxItems"], MAX_QUESTIONS);
        assert_eq!(items["items"]["properties"]["options"]["minItems"], MIN_OPTIONS);
        assert_eq!(items["items"]["properties"]["options"]["maxItems"], MAX_OPTIONS);
    }

    #[test]
    fn a_reply_reaches_the_held_call_only_while_it_waits() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let reply = AskReply::new(sender);
        let copy = reply.clone();
        assert_eq!(reply, copy);
        assert!(reply.send(Reply::Skipped));
        assert_eq!(receiver.recv().unwrap(), Reply::Skipped);
        drop(receiver);
        assert!(!copy.send(Reply::Skipped));
    }
}
