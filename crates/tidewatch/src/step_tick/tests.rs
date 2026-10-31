use tidewatch_core::panel_event::StepBy;

use super::*;

fn row(mark: Mark, text: &str) -> ItemRow<'_> {
    ItemRow { mark, text, why: None }
}

fn answered(question: &str) -> AnswerSeen {
    AnswerSeen { question: question.to_owned(), answer: "Yes".to_owned(), at_ms: 0 }
}

fn walk() -> Vec<ItemRow<'static>> {
    vec![
        row(Mark::Done, "Step 1: five rules for everything Tidewatch says"),
        row(Mark::Doing, "Step 2: the workflow profile card"),
        row(Mark::Todo, "Step 3: pull request messages before and after"),
        row(Mark::Todo, "Step 4: one way to ask you"),
    ]
}

#[test]
fn a_question_ticks_the_open_step_it_shares_the_most_words_with() {
    let steps = walk();
    let cases = [
        ("Step 3 pull request messages: do the new words hold?", Some(2)),
        ("Does the workflow profile card detect then confirm once per repo?", Some(1)),
        ("Step 4: keep the end block?", Some(3)),
    ];
    for (question, want) in cases {
        assert_eq!(matching(question, "Yes", &steps), want, "{question}");
    }
}

#[test]
fn a_finished_step_is_never_ticked_again() {
    let steps = walk();
    assert_eq!(matching("Step 1: do the five rules for everything hold?", "Yes", &steps), None);
}

#[test]
fn a_question_about_nothing_in_the_report_ticks_nothing() {
    assert_eq!(matching("Should the button be teal?", "Yes", &walk()), None);
}

#[test]
fn two_steps_the_question_fits_equally_tick_neither() {
    let steps = [
        row(Mark::Todo, "Review the panel order"),
        row(Mark::Todo, "Review the panel order again"),
    ];
    assert_eq!(matching("Does the panel order hold?", "Yes", &steps), None);
}

#[test]
fn the_only_step_waiting_on_the_owner_takes_a_question_that_shares_one_word() {
    let steps = [
        row(Mark::Doing, "Write the validator"),
        row(Mark::Blocked, "Decide the accented letters"),
        row(Mark::Todo, "Open the PR"),
    ];
    assert_eq!(matching("Should accented letters become ASCII?", "Keep them", &steps), Some(1));
    let two = [row(Mark::Blocked, "Decide the letters"), row(Mark::Blocked, "Decide the base")];
    assert_eq!(matching("Which letters?", "Keep", &two), Some(0));
    let unclear = [row(Mark::Blocked, "Decide the base"), row(Mark::Blocked, "Decide the base")];
    assert_eq!(matching("Which base?", "Main", &unclear), None);
}

#[test]
fn the_ticks_skip_steps_tidewatch_ticked_and_name_each_step_once() {
    let steps = walk();
    let answers = [
        answered("Does the workflow profile card detect then confirm once?"),
        answered("Is the workflow profile card shown under the strip?"),
        answered("Step 3 pull request messages: do the new words hold?"),
    ];
    let marks = [MarkSeen {
        step: "Step 3: pull request messages before and after".to_owned(),
        by: StepBy::Tidewatch,
        at_ms: 0,
    }];
    assert_eq!(ticks(&steps, &answers, &marks), ["Step 2: the workflow profile card"]);
    assert_eq!(ticks(&steps, &[], &[]), Vec::<String>::new());
}
