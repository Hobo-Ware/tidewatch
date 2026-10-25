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

use crate::user_markup::{self, Record};

pub const TOOL_NAME: &str = "ask_user";
pub const MAX_QUESTIONS: usize = 4;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 6;
const MAX_QUESTION_CHARS: usize = 400;
const MAX_HEADER_CHARS: usize = 40;
const MAX_LABEL_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 400;
const MAX_DECISION_CHARS: usize = 200;
const MAX_TRADEOFF_CHARS: usize = 160;
const RECOMMENDED: &str = "(Recommended)";
const LATE_INTRO: &str = "My answers to your earlier questions:";

const QUESTION_KEYS: [&str; 6] =
    ["question", "header", "decision", "multi_select", "multiSelect", "options"];
const OPTION_KEYS: [&str; 4] = ["label", "description", "tradeoff", "recommended"];

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
    /// One line on what choosing it costs or gives up.
    pub tradeoff: Option<String>,
    /// The agent's top pick, set by the `recommended` flag; a trailing "(Recommended)" on the label
    /// counts too.
    pub top_pick: bool,
}

impl QuestionOption {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), description: None, tradeoff: None, top_pick: false }
    }

    /// The label as shown: without a trailing "(Recommended)".
    #[must_use]
    pub fn shown(&self) -> &str {
        split_recommended(&self.label).0
    }

    #[must_use]
    pub fn recommended(&self) -> bool {
        self.top_pick || split_recommended(&self.label).1
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
    /// The choice in one plain sentence, shown on top of the card when the agent gave one.
    pub decision: Option<String>,
    pub select: Select,
    pub options: Vec<QuestionOption>,
}

impl Question {
    /// The options the agent recommends, by index: the first one for a single choice, all of them
    /// for a multiple choice.
    #[must_use]
    pub fn picks(&self) -> Vec<usize> {
        let mut found = (0..self.options.len()).filter(|index| self.options[*index].recommended());
        match self.select {
            Select::Single => found.next().into_iter().collect(),
            Select::Multi => found.collect(),
        }
    }

    /// The answer that goes with the agent's picks, when it has any.
    #[must_use]
    pub fn pick_given(&self) -> Option<Given> {
        let picked = self.picks();
        (!picked.is_empty()).then_some(Given { picked, other: None })
    }

    /// A short name for the question in a list: its header, else its text cut to 60 characters.
    #[must_use]
    pub fn name(&self) -> String {
        const MAX: usize = 60;
        let text = self.header.as_deref().unwrap_or(&self.text);
        if text.chars().count() <= MAX {
            return text.to_owned();
        }
        let cut: String = text.chars().take(MAX - 3).collect();
        format!("{}...", cut.trim_end())
    }
}

/// The answers that take every pick of `questions`, or nothing when one of them has none.
#[must_use]
pub fn all_picks(questions: &[Question]) -> Option<Vec<Given>> {
    questions.iter().map(Question::pick_given).collect()
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
    format!("{LATE_INTRO}\n\n{}", answer_text(questions, given))
}

/// The questions and answers of a message `answer_text` or `late_answer_text` built, as
/// `pairs` gave them, or `None` when the text is anything else. The paste wrapper Claude Code puts
/// around a pasted message and the intro line are optional; the rest must be `Q: ...` / `A: ...`
/// blocks from the first character, so a message that only mentions `Q:` in prose is not one.
#[must_use]
pub fn parse_answers(text: &str) -> Option<Vec<(String, String)>> {
    let Record::Text(text) = user_markup::classify(text) else { return None };
    let body = match text.strip_prefix(LATE_INTRO) {
        Some(rest) => rest.strip_prefix("\n\n")?,
        None => text.as_str(),
    };
    let blocks = body.strip_prefix("Q: ")?.split("\n\nQ: ");
    blocks
        .map(|block| {
            let (question, answer) = block.split_once("\nA:")?;
            let answer = answer.strip_prefix(' ').unwrap_or(answer);
            (!question.trim().is_empty()).then(|| (question.to_owned(), answer.to_owned()))
        })
        .collect()
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
                        description: option["description"].as_str().and_then(clean),
                        ..QuestionOption::new(clean(option["label"].as_str()?)?)
                    })
                })
                .collect();
            (!options.is_empty()).then(|| Question {
                text,
                header: item["header"].as_str().and_then(clean),
                decision: None,
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
        "description": "Ask the user one to four multiple-choice questions and wait for the answers. Tidewatch shows them as radio buttons (one answer) or checkboxes (several) with a field for another answer. Use it instead of guessing when the user's choice changes what you do. Write for someone who has not followed your work: plain words, no jargon, no code names. Give two or three options, and mark your top pick with recommended: true on one option. Give each question a one-sentence decision and each option a one-line tradeoff. If an option deletes or overwrites anything, its label names the exact items and how many. The user can answer every open question with your picks in one click, so only mark an option you would stand behind. The result lists each question with the answer.",
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
                            "decision": { "type": "string", "description": "The choice in one plain sentence, for example: Pick how the retry count is stored. Shown on top of the card." },
                            "multi_select": { "type": "boolean", "description": "True when several options may be picked. Default false." },
                            "options": {
                                "type": "array",
                                "minItems": MIN_OPTIONS,
                                "maxItems": MAX_OPTIONS,
                                "description": "The choices: two or three, in plain words. Mark at most one with recommended: true. Tidewatch adds a Something else field itself.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "description": "A short name for the choice." },
                                        "description": { "type": "string", "description": "What choosing it means." },
                                        "tradeoff": { "type": "string", "description": "One line on what this costs or gives up compared with the others." },
                                        "recommended": { "type": "boolean", "description": "True on your top pick. At most one option per question." },
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
    let top_pick = match object.get("recommended") {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => {
            problems.push(format!("{context}recommended must be true or false"));
            false
        }
    };
    Some(QuestionOption {
        description: text_field(object, "description", MAX_DESCRIPTION_CHARS, context, problems),
        tradeoff: text_field(object, "tradeoff", MAX_TRADEOFF_CHARS, context, problems),
        top_pick,
        ..QuestionOption::new(label)
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
    let decision = text_field(object, "decision", MAX_DECISION_CHARS, &context, problems);
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
    if options.iter().filter(|option| option.recommended()).count() > 1 {
        problems.push(format!("{context}mark at most one option as recommended"));
    }
    Question { text, header, decision, select, options }
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
        QuestionOption::new(label)
    }

    fn question(text: &str, select: Select, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: None,
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

    type Case<'a> = (&'a str, &'a [&'a str], &'a [usize], Option<&'a str>);

    fn answered(texts: &[Case]) -> (Vec<Question>, Vec<Given>) {
        let questions = texts
            .iter()
            .map(|(text, labels, _, _)| question(text, Select::Multi, labels))
            .collect();
        let given = texts
            .iter()
            .map(|(_, _, picked, other)| Given {
                picked: picked.to_vec(),
                other: other.map(str::to_owned),
            })
            .collect();
        (questions, given)
    }

    #[test]
    fn what_answer_text_built_reads_back_as_its_pairs() {
        let cases = [
            vec![("Where?", &["Stage", "Run"][..], &[0][..], None)],
            vec![
                ("Where should the retry count live?", &["Stage", "Run"][..], &[1][..], None),
                ("Which tests?", &["Unit", "Lints"][..], &[0, 1][..], Some("doc tests")),
                ("Anything else?", &["No", "Yes"][..], &[][..], Some("see https://example.com/x")),
            ],
            vec![("Multi\nline question?", &["A"][..], &[0][..], None)],
            vec![("Nothing picked?", &["A"][..], &[][..], None)],
            vec![
                ("Nothing picked?", &["A"][..], &[][..], None),
                ("Why: because", &["A: b"][..], &[0][..], None),
            ],
        ];
        for case in cases {
            let (questions, given) = answered(&case);
            let expected = pairs(&questions, &given);
            assert_eq!(parse_answers(&answer_text(&questions, &given)), Some(expected.clone()));
            assert_eq!(parse_answers(&late_answer_text(&questions, &given)), Some(expected));
        }
    }

    #[test]
    fn the_paste_wrapper_around_the_message_does_not_matter() {
        let (questions, given) =
            answered(&[("Where?", &["Stage", "Run"], &[0], None), ("How?", &["Fast"], &[0], None)]);
        let expected = Some(pairs(&questions, &given));
        let late = late_answer_text(&questions, &given);
        for wrapped in [
            format!("<pasted_content id=\"7\">{late}</pasted_content id=\"7\">"),
            format!("<pasted_content id=\"7\">\n{late}\n</pasted_content id=\"7\">\n"),
            format!("  \n{late}\n\n"),
        ] {
            assert_eq!(parse_answers(&wrapped), expected, "{wrapped}");
        }
    }

    #[test]
    fn a_message_that_only_mentions_q_in_prose_is_not_answers() {
        for text in [
            "",
            "Q: only a question",
            "Q:no space\nA: x",
            "I have a Q: what is this?\nA: no idea",
            "Please look at this.\n\nQ: Where?\nA: Stage",
            "My answers to your earlier questions:",
            "My answers to your earlier questions:\n\nthey are all fine",
            "My answers to your earlier questions: Q: Where?\nA: Stage",
            "Q: Where?\nA: Stage\n\nQ: no answer here",
            "<command-name>/clear</command-name>",
            "<system-reminder>Q: Where?\nA: Stage</system-reminder>",
        ] {
            assert_eq!(parse_answers(text), None, "{text}");
        }
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
    fn the_old_payload_still_parses_with_nothing_new_in_it() {
        let questions = from_tool_args(&args(&json!({"questions": [
            {"question": "Q?", "options": [{"label": "A (Recommended)", "description": "first"}, {"label": "B"}]},
        ]})))
        .unwrap();
        assert_eq!(questions[0].decision, None);
        assert_eq!(questions[0].options[0].tradeoff, None);
        assert!(questions[0].options[0].recommended());
        assert!(!questions[0].options[1].recommended());
        assert_eq!(questions[0].picks(), [0]);
    }

    #[test]
    fn the_new_payload_carries_a_decision_tradeoffs_and_a_pick() {
        let questions = from_tool_args(&args(&json!({"questions": [
            {"question": "Which storage?", "decision": "  Pick where the retry count is kept.  ",
             "options": [
                {"label": "On the run", "tradeoff": "Simple, but every stage shares one count."},
                {"label": "On the stage", "recommended": true, "tradeoff": "A bit more code, each stage counts alone."},
                {"label": "In a file", "recommended": false},
            ]},
        ]})))
        .unwrap();
        let question = &questions[0];
        assert_eq!(question.decision.as_deref(), Some("Pick where the retry count is kept."));
        assert_eq!(
            question.options[0].tradeoff.as_deref(),
            Some("Simple, but every stage shares one count.")
        );
        assert_eq!(question.options[2].tradeoff, None);
        assert_eq!(question.picks(), [1]);
        assert_eq!(question.pick_given(), Some(Given { picked: vec![1], other: None }));
    }

    #[test]
    fn a_pick_is_one_option_and_the_flag_or_the_suffix_both_count() {
        let cases: Vec<(Value, &str)> = vec![
            (
                json!({"question": "Q?", "options": [{"label": "A", "recommended": true}, {"label": "B", "recommended": true}]}),
                "mark at most one option as recommended",
            ),
            (
                json!({"question": "Q?", "options": [{"label": "A", "recommended": true}, {"label": "B (Recommended)"}]}),
                "mark at most one option as recommended",
            ),
            (
                json!({"question": "Q?", "options": [{"label": "A", "recommended": "yes"}, {"label": "B"}]}),
                "option 1: recommended must be true or false",
            ),
            (
                json!({"question": "Q?", "decision": "x".repeat(201), "options": [{"label": "A"}, {"label": "B"}]}),
                "decision is longer than 200 characters",
            ),
            (
                json!({"question": "Q?", "options": [{"label": "A", "tradeoff": "x".repeat(161)}, {"label": "B"}]}),
                "tradeoff is longer than 160 characters",
            ),
        ];
        for (question, want) in cases {
            let problems =
                from_tool_args(&args(&json!({"questions": [question.clone()]}))).unwrap_err();
            assert!(
                problems.iter().any(|problem| problem.contains(want)),
                "{question}: {problems:?} lacks {want:?}"
            );
        }
    }

    #[test]
    fn picks_are_the_first_recommended_option_of_a_single_choice_and_all_of_a_multiple_choice() {
        let single = question("Q?", Select::Single, &["A", "B (Recommended)", "C (Recommended)"]);
        assert_eq!(single.picks(), [1]);
        let multi = question("Q?", Select::Multi, &["A (Recommended)", "B", "C (Recommended)"]);
        assert_eq!(multi.picks(), [0, 2]);
        assert_eq!(question("Q?", Select::Single, &["A", "B"]).pick_given(), None);
    }

    #[test]
    fn all_picks_answers_every_question_or_none_of_them() {
        let with = question("A?", Select::Single, &["x", "y (Recommended)"]);
        let without = question("B?", Select::Single, &["x", "y"]);
        assert_eq!(
            all_picks(&[with.clone(), with.clone()]),
            Some(vec![
                Given { picked: vec![1], other: None },
                Given { picked: vec![1], other: None }
            ])
        );
        assert_eq!(all_picks(&[with, without]), None);
        assert_eq!(all_picks(&[]), Some(Vec::new()));
    }

    #[test]
    fn a_question_is_named_by_its_header_or_its_cut_text() {
        let mut q = question("Where should the retry count live?", Select::Single, &["a", "b"]);
        assert_eq!(q.name(), "Where should the retry count live?");
        q.header = Some("Retries".into());
        assert_eq!(q.name(), "Retries");
        q.header = None;
        q.text = "w".repeat(80);
        assert_eq!(q.name().chars().count(), 60);
        assert!(q.name().ends_with("..."));
    }

    #[test]
    fn the_tool_description_tells_agents_how_to_write_a_decision() {
        let schema = tool_definition();
        let description = schema["description"].as_str().unwrap();
        for needle in
            ["plain words", "no jargon", "two or three options", "top pick", "exact items"]
        {
            assert!(description.contains(needle), "missing {needle}");
        }
        let option = &schema["inputSchema"]["properties"]["questions"]["items"]["properties"]["options"]
            ["items"]["properties"];
        assert_eq!(option["recommended"]["type"], "boolean");
        assert_eq!(option["tradeoff"]["type"], "string");
        let question = &schema["inputSchema"]["properties"]["questions"]["items"]["properties"];
        assert_eq!(question["decision"]["type"], "string");
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
