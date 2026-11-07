//! The ledger's waiting questions as the Reader's card shows them (pure): which one shows, and how
//! it becomes a `Question` for the one card.

use tidewatch_core::link_board::Link;
use tidewatch_core::progress::{Choice, Entry};
use tidewatch_core::question::{
    Given, Question, QuestionOption, Reply, Select, answer_text, needs_you_entries,
};
use tidewatch_core::question_words;

use crate::choice;
use crate::progress_model::{Ask as CardAsk, Look};

/// The question on the card and what it needs to be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) entry: Entry,
    /// Which question this is, from zero, and how many wait.
    pub(crate) index: usize,
    pub(crate) total: usize,
    /// The links sent for this question.
    pub(crate) read: Vec<Link>,
}

const FALLBACK: [&str; 2] = ["Done", "Not now"];

/// The agent's options for a question, or Done and Not now when it gave none.
pub(crate) fn choices(options: &[Choice]) -> Vec<Choice> {
    if options.is_empty() {
        FALLBACK
            .iter()
            .map(|text| Choice { text: (*text).to_owned(), recommended: false })
            .collect()
    } else {
        options.to_vec()
    }
}

/// The agent's why line as shown: its first letter in capitals, the rest as written.
pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.trim().chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A Needs you line as the single-choice question the one card draws.
pub(crate) fn question_of(entry: &Entry) -> Question {
    Question {
        text: entry.text.trim().to_owned(),
        header: None,
        decision: None,
        select: Select::Single,
        options: choices(&entry.options)
            .into_iter()
            .map(|choice| QuestionOption {
                top_pick: choice.recommended,
                ..QuestionOption::new(choice.text)
            })
            .collect(),
    }
}

/// The ledger's save time and how many of its questions were answered one by one.
pub(crate) type Answered = (Option<i64>, usize);

/// The question to show out of `total`: answering one of several moves on even while the agent works.
pub(crate) fn shown(
    total: usize,
    stale: bool,
    given: Option<Answered>,
    saved: Option<i64>,
) -> Option<usize> {
    let step = given.filter(|(at, _)| *at == saved).map_or(0, |(_, count)| count);
    (step < total && (!stale || step > 0)).then_some(step)
}

/// Puts the questions the agent has open with its question tool on an Overview card's `look`, read
/// from the Reader's rows and not copied into the ledger: they count with the report's own (one the
/// report repeats is counted once), and a lone one is the question the card's chips answer.
pub(crate) fn add_tool_questions(look: &mut Look, tool: &[Question], own: &[&Entry]) {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let [first, ..] = tool else { return };
    let extra = own.iter().filter(|entry| !tool.iter().any(|q| same(&q.text, &entry.text))).count();
    look.questions = tool.len() + extra;
    look.ask = match (look.questions, extra) {
        (1, 0) => {
            needs_you_entries(std::slice::from_ref(first)).into_iter().next().map(|entry| CardAsk {
                text: entry.text.trim().to_owned(),
                why: entry.why.filter(|why| !why.trim().is_empty()),
                options: entry.options,
            })
        }
        _ => None,
    };
}

/// What the owner's reply to the report's question types to the agent, and whether it closes every
/// question at once (a skip or a chat does; an answer moves on to the next one).
pub(crate) fn ledger_message(question: &Question, total: usize, reply: Reply) -> (String, bool) {
    let closes = !matches!(reply, Reply::Answered(_));
    let text =
        choice::typed_message(std::slice::from_ref(question), reply, question_words::SKIP_REPLY);
    if closes { (text, true) } else { (reply_text(&question.text, total, &text), false) }
}

/// What `given` becomes once question `step` is answered, or `None` after the last one.
pub(crate) fn after_answer(total: usize, step: usize, saved: Option<i64>) -> Option<Answered> {
    (step + 1 < total).then_some((saved, step + 1))
}

/// What a pick sends: the bare answer for a lone question, `Q: ...` and `A: ...` when several wait.
pub(crate) fn reply_text(question: &str, total: usize, pick: &str) -> String {
    if total < 2 {
        return pick.to_owned();
    }
    let question = Question {
        text: question.trim().to_owned(),
        header: None,
        decision: None,
        select: Select::Single,
        options: Vec::new(),
    };
    answer_text(&[question], &[Given { picked: Vec::new(), other: Some(pick.to_owned()) }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_question_shows_until_the_ledger_goes_stale() {
        assert_eq!(shown(3, false, None, Some(5)), Some(0));
        assert_eq!(shown(3, true, None, Some(5)), None);
        assert_eq!(shown(0, false, None, Some(5)), None);
    }

    #[test]
    fn an_answered_question_moves_the_card_to_the_next_one_even_when_the_agent_works_on() {
        assert_eq!(shown(3, true, Some((Some(5), 1)), Some(5)), Some(1));
        assert_eq!(shown(3, true, Some((Some(5), 2)), Some(5)), Some(2));
        assert_eq!(shown(3, true, Some((Some(5), 3)), Some(5)), None);
    }

    #[test]
    fn a_ledger_saved_since_starts_over_at_the_first_question() {
        assert_eq!(shown(2, false, Some((Some(5), 1)), Some(9)), Some(0));
    }

    #[test]
    fn the_last_answer_ends_the_run_and_the_others_count_on() {
        assert_eq!(after_answer(1, 0, Some(5)), None);
        assert_eq!(after_answer(3, 0, Some(5)), Some((Some(5), 1)));
        assert_eq!(after_answer(3, 2, Some(5)), None);
    }

    #[test]
    fn a_lone_question_gets_the_bare_answer() {
        assert_eq!(reply_text("Apply it now?", 1, "Applied it"), "Applied it");
    }

    #[test]
    fn with_several_questions_each_answer_names_its_question() {
        assert_eq!(reply_text("Apply it now?", 2, "Applied it"), "Q: Apply it now?\nA: Applied it");
        assert_eq!(
            reply_text(" Post the follow-up? ", 3, "Draft only"),
            "Q: Post the follow-up?\nA: Draft only"
        );
    }

    #[test]
    fn a_needs_you_line_becomes_a_single_choice_question_with_the_agents_pick() {
        let entry = Entry {
            text: " Where? ".into(),
            why: Some("it changes the cost".into()),
            options: vec![
                Choice { text: "Stage".into(), recommended: false },
                Choice { text: "Run".into(), recommended: true },
            ],
        };
        let question = question_of(&entry);
        assert_eq!(question.text, "Where?");
        assert_eq!(question.select, Select::Single);
        let labels: Vec<&str> =
            question.options.iter().map(|option| option.label.as_str()).collect();
        assert_eq!(labels, ["Stage", "Run"]);
        assert_eq!(question.picks(), [1]);
    }

    #[test]
    fn a_question_with_no_options_offers_done_and_not_now() {
        let entry = Entry { text: "Ship it?".into(), why: None, options: Vec::new() };
        let labels: Vec<String> =
            question_of(&entry).options.into_iter().map(|o| o.label).collect();
        assert_eq!(labels, ["Done", "Not now"]);
    }

    fn tool_question(text: &str) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: Some("it changes the cost".into()),
            select: Select::Single,
            options: vec![QuestionOption::new("Stage"), QuestionOption::new("Run (Recommended)")],
        }
    }

    fn look_of(report: &str) -> Look {
        crate::progress_model::look(&tidewatch_core::progress::Ledger::parse(report), false)
    }

    #[test]
    fn a_lone_tool_question_is_the_overview_cards_ask_with_its_options_and_pick() {
        let mut look = look_of("# Goal\nx\n");
        add_tool_questions(&mut look, &[tool_question("Where?")], &[]);
        assert_eq!(look.questions, 1);
        let ask = look.ask.expect("a lone question can be answered from the card");
        assert_eq!(
            (ask.text.as_str(), ask.why.as_deref()),
            ("Where?", Some("it changes the cost"))
        );
        let options: Vec<(&str, bool)> =
            ask.options.iter().map(|c| (c.text.as_str(), c.recommended)).collect();
        assert_eq!(options, [("Stage", false), ("Run", true)]);
    }

    #[test]
    fn tool_questions_count_with_the_reports_and_a_repeat_counts_once() {
        let report = "# Goal\nx\n\n## Needs you\n- where?\n- Pick a name\n";
        let mut look = look_of(report);
        let own = tidewatch_core::progress::Ledger::parse(report).needs_you;
        let own: Vec<&Entry> = own.iter().collect();
        add_tool_questions(&mut look, &[tool_question("Where?")], &own);
        assert_eq!(look.questions, 2);
        assert!(look.ask.is_none(), "two questions wait, so a chip cannot say which it answers");
        let mut same = look_of("# Goal\nx\n\n## Needs you\n- where?\n");
        let lone = tidewatch_core::progress::Ledger::parse("# Goal\nx\n\n## Needs you\n- where?\n");
        add_tool_questions(
            &mut same,
            &[tool_question("Where?")],
            &lone.needs_you.iter().collect::<Vec<_>>(),
        );
        assert_eq!(same.questions, 1);
        assert!(same.ask.is_some());
    }

    #[test]
    fn no_tool_question_leaves_the_look_alone() {
        let mut look = look_of("# Goal\nx\n\n## Needs you\n- Pick a name\n");
        let before = look.clone();
        add_tool_questions(&mut look, &[], &[]);
        assert_eq!(look, before);
    }

    #[test]
    fn the_message_an_answer_to_the_report_types_depends_on_the_reply_and_how_many_wait() {
        let question = question_of(&Entry {
            text: "Where?".into(),
            why: None,
            options: vec![
                Choice { text: "Stage".into(), recommended: false },
                Choice { text: "Run".into(), recommended: true },
            ],
        });
        let run = || Reply::Answered(vec![Given { picked: vec![1], other: None }]);
        let typed = Reply::Answered(vec![Given { picked: Vec::new(), other: Some("Both".into()) }]);
        let chat = choice::chat_message("later");
        let cases = [
            ("a lone answer", 1, run(), ("Run".to_owned(), false)),
            ("one of two", 2, run(), ("Q: Where?\nA: Run".to_owned(), false)),
            ("a typed answer", 1, typed, ("Both".to_owned(), false)),
            ("a skip", 3, Reply::Skipped, (question_words::SKIP_REPLY.to_owned(), true)),
            ("a chat", 3, Reply::Chat(chat.clone()), (chat, true)),
        ];
        for (name, total, reply, want) in cases {
            assert_eq!(ledger_message(&question, total, reply), want, "{name}");
        }
    }

    #[test]
    fn the_why_line_starts_with_a_capital() {
        assert_eq!(sentence(" it changes the cost"), "It changes the cost");
        assert_eq!(sentence(""), "");
    }
}
