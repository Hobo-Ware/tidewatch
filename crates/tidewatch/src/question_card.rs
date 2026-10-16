//! The state of a live question card (pure; `question_view` draws it). A multiple-choice question
//! has no typed answer: Claude's menu does not take one there (PLAN.md, findings 2026-10-16).

use tidewatch_core::question::{Given, Question, Select};

/// The numbers `1` to `9` pick; an option past the ninth has no key.
pub(crate) const MAX_KEYED: usize = 9;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Other {
    Off,
    On,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    picked: Vec<usize>,
    other: Other,
    /// The text typed into "Something else"; kept when the user picks an option and comes back.
    typed: String,
}

/// The rows of one question: its options, then "Something else" for a single-choice question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Option(usize),
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Nothing,
    Send(Vec<Given>),
    Skip,
    /// The text field of "Something else" should take the keyboard.
    TypeOther,
}

/// The keys the card reads while it has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardKey {
    Digit(u8),
    Space,
    Enter,
    CmdEnter,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    questions: Vec<Question>,
    slots: Vec<Slot>,
    step: usize,
    cursor: usize,
}

impl Card {
    pub(crate) fn new(questions: Vec<Question>) -> Self {
        let slots = questions
            .iter()
            .map(|_| Slot { picked: Vec::new(), other: Other::Off, typed: String::new() })
            .collect();
        Self { questions, slots, step: 0, cursor: 0 }
    }

    pub(crate) fn step(&self) -> usize {
        self.step
    }

    pub(crate) fn count(&self) -> usize {
        self.questions.len()
    }

    pub(crate) fn is_last(&self) -> bool {
        self.step + 1 == self.questions.len()
    }

    pub(crate) fn question(&self) -> &Question {
        &self.questions[self.step]
    }

    pub(crate) fn rows(&self) -> Vec<Row> {
        let question = self.question();
        let mut rows: Vec<Row> = (0..question.options.len()).map(Row::Option).collect();
        if question.select == Select::Single {
            rows.push(Row::Other);
        }
        rows
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn is_picked(&self, option: usize) -> bool {
        self.slots[self.step].picked.contains(&option)
    }

    pub(crate) fn other_is_on(&self) -> bool {
        self.slots[self.step].other == Other::On
    }

    /// The text of "Something else" for the shown question, to edit in place.
    pub(crate) fn typed_mut(&mut self) -> &mut String {
        &mut self.slots[self.step].typed
    }

    /// Takes a click on, or the key of, a row.
    pub(crate) fn choose(&mut self, row: Row) -> Effect {
        let slot = &mut self.slots[self.step];
        match (self.questions[self.step].select, row) {
            (Select::Single, Row::Option(index)) => {
                slot.picked = vec![index];
                slot.other = Other::Off;
                Effect::Nothing
            }
            (Select::Multi, Row::Option(index)) => {
                if let Some(at) = slot.picked.iter().position(|picked| *picked == index) {
                    slot.picked.remove(at);
                } else {
                    slot.picked.push(index);
                    slot.picked.sort_unstable();
                }
                Effect::Nothing
            }
            (Select::Single, Row::Other) => {
                slot.picked.clear();
                slot.other = Other::On;
                Effect::TypeOther
            }
            (Select::Multi, Row::Other) => Effect::Nothing,
        }
    }

    /// Whether the shown question has an answer: an option, or typed text.
    pub(crate) fn answered(&self) -> bool {
        self.slot_answered(self.step)
    }

    fn slot_answered(&self, index: usize) -> bool {
        let slot = &self.slots[index];
        !slot.picked.is_empty() || slot.other == Other::On && !slot.typed.trim().is_empty()
    }

    pub(crate) fn can_send(&self) -> bool {
        (0..self.slots.len()).all(|index| self.slot_answered(index))
    }

    pub(crate) fn next(&mut self) {
        if self.answered() && !self.is_last() {
            self.step += 1;
            self.cursor = 0;
        }
    }

    pub(crate) fn back(&mut self) {
        if self.step > 0 {
            self.step -= 1;
            self.cursor = 0;
        }
    }

    pub(crate) fn send(&self) -> Effect {
        if !self.can_send() {
            return Effect::Nothing;
        }
        Effect::Send(
            self.slots
                .iter()
                .map(|slot| Given {
                    picked: slot.picked.clone(),
                    other: (slot.other == Other::On && !slot.typed.trim().is_empty())
                        .then(|| slot.typed.trim().to_owned()),
                })
                .collect(),
        )
    }

    pub(crate) fn key(&mut self, key: CardKey) -> Effect {
        let rows = self.rows();
        match key {
            CardKey::Digit(number) => {
                let index = usize::from(number).saturating_sub(1);
                if number == 0 || index >= MAX_KEYED {
                    return Effect::Nothing;
                }
                match rows.get(index) {
                    Some(row) => {
                        self.cursor = index;
                        self.choose(*row)
                    }
                    None => Effect::Nothing,
                }
            }
            CardKey::Space => {
                rows.get(self.cursor).map_or(Effect::Nothing, |row| self.choose(*row))
            }
            CardKey::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Effect::Nothing
            }
            CardKey::Down => {
                self.cursor = (self.cursor + 1).min(rows.len().saturating_sub(1));
                Effect::Nothing
            }
            CardKey::Enter => {
                self.next();
                Effect::Nothing
            }
            CardKey::CmdEnter => self.send(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::question::QuestionOption;

    fn question(text: &str, select: Select, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            select,
            options: labels
                .iter()
                .map(|label| QuestionOption { label: (*label).into(), description: None })
                .collect(),
        }
    }

    fn card() -> Card {
        Card::new(vec![
            question("Where?", Select::Single, &["Stage", "Run"]),
            question("Which?", Select::Multi, &["Unit", "Integration", "Lints"]),
        ])
    }

    #[test]
    fn a_single_choice_card_has_its_options_then_something_else() {
        let rows = card().rows();
        assert_eq!(rows, [Row::Option(0), Row::Option(1), Row::Other]);
    }

    #[test]
    fn a_multiple_choice_card_has_no_something_else() {
        let mut card = card();
        card.choose(Row::Option(0));
        card.next();
        assert_eq!(card.rows(), [Row::Option(0), Row::Option(1), Row::Option(2)]);
        assert_eq!(card.choose(Row::Other), Effect::Nothing);
    }

    #[test]
    fn picking_an_option_of_a_single_choice_replaces_the_pick_and_the_typed_answer() {
        let mut card = card();
        assert_eq!(card.choose(Row::Other), Effect::TypeOther);
        assert!(card.other_is_on());
        card.choose(Row::Option(1));
        assert!(!card.other_is_on());
        assert!(card.is_picked(1));
        card.choose(Row::Option(0));
        assert!(card.is_picked(0) && !card.is_picked(1));
    }

    #[test]
    fn something_else_clears_the_pick_and_needs_text_to_count_as_an_answer() {
        let mut card = card();
        card.choose(Row::Option(0));
        card.choose(Row::Other);
        assert!(!card.is_picked(0));
        assert!(!card.answered());
        card.typed_mut().push_str("   ");
        assert!(!card.answered());
        card.typed_mut().push_str("only on CI");
        assert!(card.answered());
    }

    #[test]
    fn multiple_choice_ticks_toggle_and_stay_in_option_order() {
        let mut card = card();
        card.choose(Row::Option(0));
        card.next();
        for index in [2, 0, 1] {
            card.choose(Row::Option(index));
        }
        card.choose(Row::Option(0));
        assert!(!card.is_picked(0) && card.is_picked(1) && card.is_picked(2));
        card.choose(Row::Option(0));
        let Effect::Send(given) = card.send() else { panic!("{:?}", card.send()) };
        assert_eq!(given[1].picked, [0, 1, 2]);
    }

    #[test]
    fn next_needs_an_answer_and_stops_at_the_last_question() {
        let mut card = card();
        card.next();
        assert_eq!(card.step(), 0);
        card.choose(Row::Option(0));
        card.next();
        assert_eq!(card.step(), 1);
        assert!(card.is_last());
        card.next();
        assert_eq!(card.step(), 1);
        card.back();
        assert_eq!(card.step(), 0);
        card.back();
        assert_eq!(card.step(), 0);
    }

    #[test]
    fn send_needs_every_question_answered_and_trims_the_typed_answer() {
        let mut card = card();
        card.choose(Row::Other);
        card.typed_mut().push_str("  both  ");
        assert_eq!(card.send(), Effect::Nothing);
        card.next();
        card.choose(Row::Option(1));
        assert_eq!(
            card.send(),
            Effect::Send(vec![
                Given { picked: vec![], other: Some("both".into()) },
                Given { picked: vec![1], other: None },
            ])
        );
    }

    #[test]
    fn the_number_keys_pick_rows_and_a_number_past_the_rows_does_nothing() {
        let mut card = card();
        assert_eq!(card.key(CardKey::Digit(2)), Effect::Nothing);
        assert!(card.is_picked(1));
        assert_eq!(card.key(CardKey::Digit(3)), Effect::TypeOther);
        assert!(card.other_is_on());
        for number in [0, 4, 9] {
            assert_eq!(card.key(CardKey::Digit(number)), Effect::Nothing, "{number}");
        }
        assert!(card.other_is_on());
    }

    #[test]
    fn space_ticks_the_row_under_the_cursor_and_the_arrows_move_it() {
        let mut card = card();
        card.choose(Row::Option(0));
        card.next();
        card.key(CardKey::Space);
        assert!(card.is_picked(0));
        card.key(CardKey::Down);
        card.key(CardKey::Down);
        card.key(CardKey::Down);
        assert_eq!(card.cursor(), 2);
        card.key(CardKey::Space);
        assert!(card.is_picked(2));
        card.key(CardKey::Up);
        card.key(CardKey::Up);
        card.key(CardKey::Up);
        assert_eq!(card.cursor(), 0);
        card.key(CardKey::Space);
        assert!(!card.is_picked(0));
    }

    #[test]
    fn enter_goes_next_and_command_enter_sends() {
        let mut card = card();
        card.key(CardKey::Enter);
        assert_eq!(card.step(), 0);
        card.key(CardKey::Digit(1));
        card.key(CardKey::Enter);
        assert_eq!(card.step(), 1);
        assert_eq!(card.key(CardKey::CmdEnter), Effect::Nothing);
        card.key(CardKey::Digit(1));
        card.key(CardKey::Enter);
        assert_eq!(card.step(), 1);
        assert!(matches!(card.key(CardKey::CmdEnter), Effect::Send(_)));
    }

    #[test]
    fn moving_between_questions_keeps_each_answer() {
        let mut card = card();
        card.choose(Row::Option(1));
        card.next();
        card.choose(Row::Option(2));
        card.back();
        assert!(card.is_picked(1));
        card.next();
        assert!(card.is_picked(2));
    }
}
