//! What Tidewatch tells an agent when the user did not pick an answer to its questions: they were
//! skipped, or the wait ran out and Tidewatch answered. Pure.

use crate::question::{Given, Question, answer_line, answer_text};

/// What the tool result says when the user chose to leave the choice to the agent.
pub const SKIPPED: &str = "The user skipped your questions. Go with your recommended answer and say which one you picked in your final reply.";
/// What a skip types into an agent that asked with its own tool, after the menu is dismissed.
pub const SKIP_REPLY: &str = "Skip your questions. Go with your recommended answer and say which one you picked in your final reply.";

/// The answers Tidewatch picked after the wait, as a message to an agent whose question already
/// ended: what was picked, that it is a stand-in, and the question each answer belongs to.
#[must_use]
pub fn picked_for_you(questions: &[Question], given: &[Given], minutes: u64) -> String {
    let after = if minutes == 1 { "1 minute".to_owned() } else { format!("{minutes} minutes") };
    let lead = match (questions, given) {
        ([question], [given]) => {
            let answer = answer_line(question, given);
            format!("Tidewatch picked \"{answer}\" for you after {after}. The user may change it.")
        }
        _ => format!(
            "Tidewatch picked these answers for you after {after}. The user may change them."
        ),
    };
    format!("{lead}\n\n{}", answer_text(questions, given))
}

/// What a card says about the answers Tidewatch picked while the owner was away.
#[must_use]
pub fn pick_review_line(questions: &[Question], given: &[Given]) -> String {
    match (questions, given) {
        ([question], [given]) => format!(
            "Tidewatch picked \"{}\" for you. Open the tab to review it",
            answer_line(question, given)
        ),
        _ => format!(
            "Tidewatch picked answers to {} questions for you. Open the tab to review them",
            questions.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::{QuestionOption, Select};

    fn question(text: &str, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        }
    }

    #[test]
    fn a_skip_hands_the_choice_to_the_agent_and_asks_it_to_say_what_it_picked() {
        for text in [SKIPPED, SKIP_REPLY] {
            assert!(text.contains("Go with your recommended answer"), "{text}");
            assert!(text.contains("say which one you picked in your final reply"), "{text}");
            assert!(!text.contains("best judgment"), "{text}");
        }
    }

    #[test]
    fn one_picked_question_names_the_answer_the_wait_and_that_the_user_may_change_it() {
        let questions = [question("Where should it live?", &["On the stage", "On the run"])];
        let given = [Given { picked: vec![1], other: None }];
        assert_eq!(
            picked_for_you(&questions, &given, 5),
            "Tidewatch picked \"On the run\" for you after 5 minutes. The user may change it.\n\n\
             Q: Where should it live?\nA: On the run"
        );
    }

    #[test]
    fn several_questions_are_listed_one_by_one() {
        let questions = [question("First?", &["a", "b"]), question("Second?", &["c", "d"])];
        let given =
            [Given { picked: vec![0], other: None }, Given { picked: vec![1], other: None }];
        assert_eq!(
            picked_for_you(&questions, &given, 1),
            "Tidewatch picked these answers for you after 1 minute. The user may change them.\n\n\
             Q: First?\nA: a\n\nQ: Second?\nA: d"
        );
    }

    #[test]
    fn the_card_line_names_the_pick_or_counts_the_questions() {
        let one = [question("Where?", &["On the stage", "On the run"])];
        let first = [Given { picked: vec![0], other: None }];
        assert_eq!(
            pick_review_line(&one, &first),
            "Tidewatch picked \"On the stage\" for you. Open the tab to review it"
        );
        let two = [question("A?", &["a", "b"]), question("B?", &["c", "d"])];
        let both = [Given { picked: vec![0], other: None }, Given { picked: vec![1], other: None }];
        assert_eq!(
            pick_review_line(&two, &both),
            "Tidewatch picked answers to 2 questions for you. Open the tab to review them"
        );
    }
}
