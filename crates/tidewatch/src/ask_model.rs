//! Which of the ledger's waiting questions the Reader's Needs you card shows (pure).

use tidewatch_core::link_board::Link;
use tidewatch_core::progress::Entry;
use tidewatch_core::question::{Given, Question, Select, answer_text};

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
}
