//! "Go with your picks": answering every waiting question of a tab that has a recommended option in
//! one click. A question row is answered only when each of its questions has a pick; the others
//! stay open and are named. Pure; `reader_glue` carries the answers out.

use tidewatch_core::question::{self, Given};

use crate::reader_rows::{Kind, Rows};

const SHOWN_ANSWERS: usize = 3;

/// One waiting row that the picks answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    pub(crate) seq: u64,
    pub(crate) given: Vec<Given>,
    /// The answer of each question as a line, in order.
    pub(crate) lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bulk {
    pub(crate) answers: Vec<Answer>,
    /// The names of the waiting questions that have no pick.
    pub(crate) left_open: Vec<String>,
}

impl Bulk {
    /// How many waiting rows the picks answer.
    pub(crate) fn count(&self) -> usize {
        self.answers.len()
    }
}

pub(crate) fn plan(rows: &Rows) -> Bulk {
    let mut bulk = Bulk::default();
    for row in rows.pending_questions() {
        let Kind::Question(asked) = &row.kind else { continue };
        if let Some(given) = question::all_picks(&asked.questions).filter(|given| !given.is_empty())
        {
            let lines = asked
                .questions
                .iter()
                .zip(&given)
                .map(|(question, given)| question::answer_line(question, given))
                .collect();
            bulk.answers.push(Answer { seq: row.seq, given, lines });
        } else {
            let names = asked.questions.iter().filter(|question| question.picks().is_empty());
            bulk.left_open.extend(names.map(question::Question::name));
        }
    }
    bulk
}

/// What the toast says after the picks went in: how many, a short list of the answers, and the
/// questions left open.
pub(crate) fn toast(bulk: &Bulk) -> String {
    let lines: Vec<&str> =
        bulk.answers.iter().flat_map(|answer| answer.lines.iter().map(String::as_str)).collect();
    let mut text = match lines.len() {
        0 => "No waiting question has a pick.".to_owned(),
        n => {
            let shown = lines.iter().take(SHOWN_ANSWERS).copied().collect::<Vec<_>>().join(", ");
            let more = n.saturating_sub(SHOWN_ANSWERS);
            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
            format!("Went with the picks for {n}: {shown}{tail}.")
        }
    };
    if !bulk.left_open.is_empty() {
        text = format!("{text} Still open: {}.", bulk.left_open.join(", "));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use tidewatch_core::question::{AskReply, Question, QuestionOption, Select};

    fn question(text: &str, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        }
    }

    fn rows_with(asks: Vec<Vec<Question>>) -> Rows {
        let mut rows = Rows::default();
        for questions in asks {
            let (sender, _receiver) = channel();
            rows.push_ask(questions, AskReply::new(sender));
        }
        rows
    }

    #[test]
    fn every_waiting_row_with_picks_is_answered_with_them() {
        let rows = rows_with(vec![
            vec![question("Where?", &["Stage (Recommended)", "Run"])],
            vec![
                question("Which?", &["Unit", "Lints (Recommended)"]),
                question("How?", &["Fast", "Slow (Recommended)"]),
            ],
        ]);
        let bulk = plan(&rows);
        assert_eq!(bulk.count(), 2);
        assert_eq!(bulk.answers[0].given, [Given { picked: vec![0], other: None }]);
        assert_eq!(bulk.answers[0].lines, ["Stage"]);
        assert_eq!(bulk.answers[1].lines, ["Lints", "Slow"]);
        assert_eq!(bulk.left_open.len(), 0);
    }

    #[test]
    fn a_row_with_a_question_that_has_no_pick_stays_open_and_is_named() {
        let mut headed = question("Which database should hold the sessions?", &["A", "B"]);
        headed.header = Some("Database".into());
        let rows = rows_with(vec![
            vec![question("Where?", &["Stage (Recommended)", "Run"])],
            vec![question("Which?", &["Unit", "Lints (Recommended)"]), headed],
            vec![question("Anything else?", &["No", "Yes"])],
        ]);
        let bulk = plan(&rows);
        assert_eq!(bulk.count(), 1);
        assert_eq!(bulk.answers[0].lines, ["Stage"]);
        assert_eq!(bulk.left_open, ["Database", "Anything else?"]);
    }

    #[test]
    fn no_waiting_question_means_an_empty_plan() {
        let bulk = plan(&Rows::default());
        assert_eq!(bulk, Bulk::default());
        assert_eq!(toast(&bulk), "No waiting question has a pick.");
    }

    #[test]
    fn answered_rows_are_not_asked_again() {
        let mut rows = rows_with(vec![
            vec![question("Where?", &["Stage (Recommended)", "Run"])],
            vec![question("Which?", &["Unit (Recommended)", "Lints"])],
        ]);
        let first = rows.pending_question().unwrap().seq;
        rows.set_asked(first, crate::reader_rows::Asked::Declined);
        let bulk = plan(&rows);
        assert_eq!(bulk.count(), 1);
        assert_ne!(bulk.answers[0].seq, first);
    }

    #[test]
    fn the_toast_lists_a_few_answers_and_names_what_is_left() {
        let answers = (0..5)
            .map(|n| Answer { seq: n, given: Vec::new(), lines: vec![format!("pick {n}")] })
            .collect();
        let bulk = Bulk { answers, left_open: vec!["Database".into(), "Naming".into()] };
        assert_eq!(
            toast(&bulk),
            "Went with the picks for 5: pick 0, pick 1, pick 2 and 2 more. Still open: Database, Naming."
        );
        let one = Bulk {
            answers: vec![Answer { seq: 1, given: Vec::new(), lines: vec!["Stage".into()] }],
            left_open: Vec::new(),
        };
        assert_eq!(toast(&one), "Went with the picks for 1: Stage.");
    }
}
