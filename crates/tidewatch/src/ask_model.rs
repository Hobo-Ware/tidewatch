//! Which of the ledger's waiting questions the Reader's Needs you card shows (pure).

use tidewatch_core::link_board::Link;
use tidewatch_core::progress::Entry;
use tidewatch_core::question::{Given, Question, Select, answer_text, needs_you_entries};

use crate::reader_rows::{Kind, Rows};

/// The question on the card and what it needs to be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) entry: Entry,
    /// Which question this is, from zero, and how many wait.
    pub(crate) index: usize,
    pub(crate) total: usize,
    /// The links sent for this question.
    pub(crate) read: Vec<Link>,
    /// Whether the digit keys may pick: false while a dialog holds the keyboard.
    pub(crate) keys: bool,
}

/// The Needs you lines for every question the agent has open with its question tool, oldest first:
/// Claude's own `AskUserQuestion` call read from its session file, or a held `ask_user` call.
pub(crate) fn open_lines(rows: &Rows) -> Vec<Entry> {
    rows.pending_questions()
        .into_iter()
        .flat_map(|row| match &row.kind {
            Kind::Question(asked) => needs_you_entries(&asked.questions),
            _ => Vec::new(),
        })
        .collect()
}

/// The option of a lone open question that `text` picks, or the typed answer when none matches.
pub(crate) fn given_for(questions: &[Question], text: &str) -> Option<Vec<Given>> {
    let [question] = questions else { return None };
    let picked = question.options.iter().position(|option| option.shown() == text);
    Some(vec![match picked {
        Some(index) => Given { picked: vec![index], other: None },
        None => Given { picked: Vec::new(), other: Some(text.to_owned()) },
    }])
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
    use tidewatch_core::question::{AskReply, QuestionOption};

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

    fn claude_call(questions: Vec<Question>) -> tidewatch_core::transcript::Entry {
        use tidewatch_core::transcript::{AskPart, Role, ToolCall, ToolPhase};
        tidewatch_core::transcript::Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: "AskUserQuestion".into(),
                summary: String::new(),
                id: Some("q1".into()),
                phase: ToolPhase::Call,
                ask: Some(AskPart::Asked(questions)),
            }),
            ts: None,
            images: Vec::new(),
        }
    }

    fn asked(text: &str, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        }
    }

    #[test]
    fn a_claude_ask_user_question_call_fills_the_open_lines_with_question_options_and_pick() {
        let mut rows = Rows::default();
        assert_eq!(open_lines(&rows), Vec::<Entry>::new());
        rows.push(claude_call(vec![asked("Where?", &["Stage", "Run (Recommended)"])]));
        let lines = open_lines(&rows);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Where?");
        let options: Vec<(&str, bool)> =
            lines[0].options.iter().map(|c| (c.text.as_str(), c.recommended)).collect();
        assert_eq!(options, [("Stage", false), ("Run", true)]);
    }

    #[test]
    fn answering_the_question_empties_the_open_lines_again() {
        use crate::reader_rows::Asked;
        let mut rows = Rows::default();
        rows.push(claude_call(vec![asked("Where?", &["Stage", "Run"])]));
        assert_eq!(open_lines(&rows).len(), 1);
        assert!(rows.set_asked(0, Asked::Sent(vec![("Where?".into(), "Run".into())])));
        assert_eq!(open_lines(&rows), Vec::<Entry>::new());
    }

    #[test]
    fn a_held_ask_user_call_and_a_claude_menu_both_count_and_a_menu_behind_a_newer_row_does_not() {
        let (sender, _keep) = std::sync::mpsc::channel();
        let mut rows = Rows::default();
        rows.push_ask(
            vec![asked("One?", &["A", "B"]), asked("Two?", &["C", "D"])],
            AskReply::new(sender),
        );
        let texts =
            |rows: &Rows| open_lines(rows).into_iter().map(|line| line.text).collect::<Vec<_>>();
        assert_eq!(texts(&rows), ["One?", "Two?"]);
        rows.push(claude_call(vec![asked("Three?", &["E", "F"])]));
        assert_eq!(texts(&rows), ["One?", "Two?", "Three?"]);
        rows.push(tidewatch_core::transcript::Entry {
            role: tidewatch_core::transcript::Role::Assistant,
            text: "carrying on".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        });
        assert_eq!(texts(&rows), ["One?", "Two?"]);
    }

    #[test]
    fn a_chip_picks_the_option_with_that_label_else_it_is_typed_and_only_a_lone_question_is_answered()
     {
        let lone = [asked("Where?", &["Stage", "Run (Recommended)"])];
        assert_eq!(given_for(&lone, "Run"), Some(vec![Given { picked: vec![1], other: None }]));
        assert_eq!(
            given_for(&lone, "Somewhere else"),
            Some(vec![Given { picked: Vec::new(), other: Some("Somewhere else".into()) }])
        );
        let two = [asked("One?", &["A", "B"]), asked("Two?", &["C", "D"])];
        assert_eq!(given_for(&two, "A"), None);
        assert_eq!(given_for(&[], "A"), None);
    }
}
