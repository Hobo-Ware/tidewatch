//! The one way a choice reaches the agent (pure): every question the owner can answer, whether
//! Claude's menu, a held `ask_user` call or a Needs you line of the progress report, carries a
//! `Route`, and `deliver` turns an answer into what is sent along that route.

use tidewatch_core::question::{Question, Reply, answer_line, late_answer_text};

use crate::reader_keys;
use crate::reader_rows::Origin;

/// What "Let's chat about this" sends in the owner's voice.
pub(crate) const CHAT_TEXT: &str = "Let's chat about this first. Don't give me another set of options until I ask for one; answer in plain text and we'll talk it through.";

/// What the summary card says the owner answered when they chose to chat.
pub(crate) const CHAT_LABEL: &str = "Let's chat about this";

/// The chat message, with the owner's note from the card after a blank line.
pub(crate) fn chat_message(note: &str) -> String {
    let note = note.trim();
    if note.is_empty() { CHAT_TEXT.to_owned() } else { format!("{CHAT_TEXT}\n\n{note}") }
}

/// How an answer to a choice gets to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Claude Code's own question menu: key presses, or Esc and a message.
    ClaudeMenu,
    /// Tidewatch's `ask_user` call, held open until the answer comes back as its result.
    HeldCall,
    /// A question of the progress report: the answer is a typed chat message.
    Chat,
}

impl Route {
    pub(crate) fn of(origin: &Origin) -> Self {
        match origin {
            Origin::Claude => Self::ClaudeMenu,
            Origin::Tidewatch(_) => Self::HeldCall,
        }
    }
}

/// What goes to the agent for one answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Key presses into Claude's menu, one chunk each.
    Keys(Vec<Vec<u8>>),
    /// Esc to close Claude's menu, then this message.
    EscThen(String),
    /// The held call takes `reply`; `fallback` is typed instead when the call already ended.
    Held { reply: Reply, fallback: String },
    /// This message, typed as the owner's.
    Typed(String),
}

/// What `reply` becomes on `route`. `skip_text` is what a skip tells an agent that has no channel.
pub(crate) fn deliver(
    route: Route,
    questions: &[Question],
    reply: Reply,
    skip_text: &str,
) -> Delivery {
    match (route, reply) {
        (Route::ClaudeMenu, Reply::Answered(given)) => {
            match reader_keys::claude_question_keys(questions, &given) {
                Some(keys) => Delivery::Keys(keys),
                None => Delivery::EscThen(late_answer_text(questions, &given)),
            }
        }
        (Route::ClaudeMenu, Reply::Skipped) => Delivery::EscThen(skip_text.to_owned()),
        (Route::ClaudeMenu, Reply::Chat(text)) => Delivery::EscThen(text),
        (Route::HeldCall, Reply::Answered(given)) => {
            let fallback = late_answer_text(questions, &given);
            Delivery::Held { reply: Reply::Answered(given), fallback }
        }
        (Route::HeldCall, Reply::Skipped) => {
            Delivery::Held { reply: Reply::Skipped, fallback: skip_text.to_owned() }
        }
        (Route::HeldCall, Reply::Chat(text)) => {
            Delivery::Held { fallback: text.clone(), reply: Reply::Chat(text) }
        }
        (Route::Chat, reply) => Delivery::Typed(typed_message(questions, reply, skip_text)),
    }
}

/// The message typed to the agent for `reply` to the report's question: the answer as the owner
/// would write it, the skip text, or the chat message.
pub(crate) fn typed_message(questions: &[Question], reply: Reply, skip_text: &str) -> String {
    match reply {
        Reply::Answered(given) => questions
            .iter()
            .zip(&given)
            .map(|(question, given)| answer_line(question, given))
            .collect::<Vec<_>>()
            .join("\n"),
        Reply::Skipped => skip_text.to_owned(),
        Reply::Chat(text) => text,
    }
}

/// The pairs the summary card shows for a chat: every question with the chat label.
pub(crate) fn chat_pairs(questions: &[Question]) -> Vec<(String, String)> {
    questions.iter().map(|question| (question.text.clone(), CHAT_LABEL.to_owned())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::question::{Given, QuestionOption, Select};

    fn asked() -> Vec<Question> {
        vec![Question {
            text: "Where?".into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: ["Stage", "Run"].map(QuestionOption::new).to_vec(),
        }]
    }

    fn run() -> Vec<Given> {
        vec![Given { picked: vec![1], other: None }]
    }

    #[test]
    fn an_answer_goes_along_its_route() {
        let late = late_answer_text(&asked(), &run());
        let cases = [
            (Route::ClaudeMenu, Delivery::Keys(vec![b"2".to_vec()])),
            (Route::HeldCall, Delivery::Held { reply: Reply::Answered(run()), fallback: late }),
            (Route::Chat, Delivery::Typed("Run".into())),
        ];
        for (route, want) in cases {
            assert_eq!(deliver(route, &asked(), Reply::Answered(run()), "skip"), want, "{route:?}");
        }
    }

    #[test]
    fn a_menu_that_cannot_take_keys_is_closed_and_the_answer_typed() {
        let multi = vec![Question { select: Select::Multi, ..asked().remove(0) }];
        let typed = vec![Given { picked: vec![0], other: Some("Somewhere".into()) }];
        let want = Delivery::EscThen(late_answer_text(&multi, &typed));
        assert_eq!(deliver(Route::ClaudeMenu, &multi, Reply::Answered(typed), "skip"), want);
    }

    #[test]
    fn a_skip_goes_along_its_route() {
        let cases = [
            (Route::ClaudeMenu, Delivery::EscThen("skip".into())),
            (Route::HeldCall, Delivery::Held { reply: Reply::Skipped, fallback: "skip".into() }),
            (Route::Chat, Delivery::Typed("skip".into())),
        ];
        for (route, want) in cases {
            assert_eq!(deliver(route, &asked(), Reply::Skipped, "skip"), want, "{route:?}");
        }
    }

    #[test]
    fn the_chat_message_goes_along_every_route() {
        let text = chat_message("");
        assert_eq!(text, CHAT_TEXT);
        let cases = [
            (Route::ClaudeMenu, Delivery::EscThen(text.clone())),
            (
                Route::HeldCall,
                Delivery::Held { reply: Reply::Chat(text.clone()), fallback: text.clone() },
            ),
            (Route::Chat, Delivery::Typed(text.clone())),
        ];
        for (route, want) in cases {
            assert_eq!(deliver(route, &asked(), Reply::Chat(text.clone()), "skip"), want);
        }
    }

    #[test]
    fn a_note_follows_the_chat_text_after_a_blank_line() {
        assert_eq!(
            chat_message("  I want to see the diff first "),
            format!("{CHAT_TEXT}\n\nI want to see the diff first")
        );
        assert_eq!(chat_message("   "), CHAT_TEXT);
    }

    #[test]
    fn a_chat_shows_the_chat_label_as_the_answer() {
        assert_eq!(chat_pairs(&asked()), [("Where?".to_owned(), CHAT_LABEL.to_owned())]);
    }
}
