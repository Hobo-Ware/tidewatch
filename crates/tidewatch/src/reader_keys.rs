//! The keys that answer a CLI's permission prompt, so the Reader's buttons can type them into the
//! pty. Only answers verified against a real prompt are listed; for any other agent the Reader
//! offers "Switch to terminal" instead of guessing.
//!
//! Claude Code 2.1.296 at Ask level, checked 2026-10-13 in real tabs (raw grid read, then each key sent
//! from the Reader): the prompt is a numbered list, "1. Yes", "2. Yes, and ..." (seen for a file
//! write; other tools word it differently), "3. No", under "Esc to cancel". "1" allowed the write
//! (the file was created, the tab went back to Working). Esc denied it (no file, the transcript got
//! "[Request interrupted by user for tool use]"). Deny is Esc, not "3", because the options can differ per tool.
//! A denied prompt sends no hook and the status stays "needs you", so the Reader hides its buttons
//! after one answer until the next hook.
//!
//! Claude Code 2.1.301 `AskUserQuestion` menu, checked 2026-10-16 in a pty (haiku, a throwaway repo):
//! the menu is a tab per question plus a Submit tab. A number key picks that option of a single-choice
//! question and moves to the next tab; the same number on a multiple-choice question ticks it and
//! stays; Right moves to the next tab; the row after the last option ("Type something.", number
//! options + 1) takes typed text and Enter, then moves on; on the Submit tab `1` is "Submit
//! answers". A lone single-choice question submits at once, with no Submit tab. Esc cancels the
//! whole menu ("User declined to answer questions") and ends the turn. Keys must reach the pty as
//! separate writes: `213` and a Right arrow in one write were not taken as three presses, so every
//! press is its own chunk. A multiple-choice "Type something" row did not behave: it is not used.

use tidewatch_core::question::{Given, Question, Select};
use tidewatch_core::transcript::Source;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Allow,
    Deny,
}

impl Answer {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Allow => "Allow",
            Self::Deny => "Deny",
        }
    }
}

const CLAUDE: &[(Answer, &[u8])] = &[(Answer::Allow, b"1"), (Answer::Deny, b"\x1b")];

/// The answers a CLI's permission prompt takes and the bytes that give each, or none when they are
/// not verified.
pub(crate) fn answers(source: Option<Source>) -> &'static [(Answer, &'static [u8])] {
    match source {
        Some(Source::Claude) => CLAUDE,
        _ => &[],
    }
}

pub(crate) fn bytes(source: Option<Source>, answer: Answer) -> Option<&'static [u8]> {
    answers(source).iter().find(|(a, _)| *a == answer).map(|(_, keys)| *keys)
}

const ENTER: &[u8] = b"\r";
const RIGHT: &[u8] = b"\x1b[C";
const SUBMIT: &[u8] = b"1";
/// Dismisses Claude's question menu without answering.
pub(crate) const CLAUDE_MENU_ESCAPE: &[u8] = b"\x1b";

/// Esc, which ends the turn of every agent CLI Tidewatch runs.
pub(crate) const INTERRUPT: &[u8] = b"\x1b";

fn digit(number: usize) -> Option<Vec<u8>> {
    u8::try_from(number).ok().filter(|n| (1..=9).contains(n)).map(|n| vec![b'0' + n])
}

fn typed_chunk(text: &str) -> Option<Vec<u8>> {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    (!clean.is_empty()).then(|| clean.as_bytes().to_vec())
}

/// The key presses, one chunk each, that give `given` to Claude Code's question menu. `None` when
/// the menu cannot take it by keys (a number past 9, a multiple-choice question with nothing ticked,
/// a typed answer on a multiple-choice question), so the caller types the answer as a reply instead.
pub(crate) fn claude_question_keys(
    questions: &[Question],
    given: &[Given],
) -> Option<Vec<Vec<u8>>> {
    if questions.is_empty() || questions.len() != given.len() {
        return None;
    }
    let mut keys = Vec::new();
    for (question, given) in questions.iter().zip(given) {
        match (question.select, given.picked.as_slice(), &given.other) {
            (Select::Single, [only], None) => keys.push(digit(*only + 1)?),
            (Select::Single, [], Some(text)) => {
                keys.push(digit(question.options.len() + 1)?);
                keys.push(typed_chunk(text)?);
                keys.push(ENTER.to_vec());
            }
            (Select::Multi, picked @ [_, ..], None) => {
                for option in picked {
                    keys.push(digit(*option + 1)?);
                }
                keys.push(RIGHT.to_vec());
            }
            _ => return None,
        }
    }
    let lone_single = questions.len() == 1 && questions[0].select == Select::Single;
    if !lone_single {
        keys.push(SUBMIT.to_vec());
    }
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_allows_with_one_and_denies_with_escape() {
        assert_eq!(bytes(Some(Source::Claude), Answer::Allow), Some(&b"1"[..]));
        assert_eq!(bytes(Some(Source::Claude), Answer::Deny), Some(&b"\x1b"[..]));
    }

    fn q(select: Select, labels: &[&str]) -> Question {
        Question {
            text: "Q?".into(),
            header: None,
            decision: None,
            select,
            options: labels
                .iter()
                .map(|label| tidewatch_core::question::QuestionOption::new(*label))
                .collect(),
        }
    }

    fn picked(options: &[usize]) -> Given {
        Given { picked: options.to_vec(), other: None }
    }

    fn typed(text: &str) -> Given {
        Given { picked: vec![], other: Some(text.into()) }
    }

    type Keys = Vec<Vec<u8>>;
    type Case = (&'static str, Vec<Question>, Vec<Given>, Option<Keys>);

    fn chunks(keys: &[&[u8]]) -> Keys {
        keys.iter().map(|key| key.to_vec()).collect()
    }

    #[test]
    fn claude_2_1_301_question_menu_key_sequences() {
        let single = q(Select::Single, &["Red", "Green", "Blue"]);
        let multi = q(Select::Multi, &["Salt", "Pepper", "Sugar", "Lemon"]);
        let cases: Vec<Case> = vec![
            (
                "single then multiple, as run: 2, then 1 and 3, Right, Submit",
                vec![single.clone(), multi.clone()],
                vec![picked(&[1]), picked(&[0, 2])],
                Some(chunks(&[b"2", b"1", b"3", b"\x1b[C", b"1"])),
            ),
            (
                "multiple then single, as run: ticks, Right, 2, Submit",
                vec![multi.clone(), single.clone()],
                vec![picked(&[0, 2]), picked(&[1])],
                Some(chunks(&[b"1", b"3", b"\x1b[C", b"2", b"1"])),
            ),
            (
                "a lone single choice submits with its number",
                vec![single.clone()],
                vec![picked(&[1])],
                Some(chunks(&[b"2"])),
            ),
            (
                "a lone multiple choice ticks, goes to Submit and submits",
                vec![multi.clone()],
                vec![picked(&[1])],
                Some(chunks(&[b"2", b"\x1b[C", b"1"])),
            ),
            (
                "a typed answer is the row after the options, the text, Enter",
                vec![single.clone()],
                vec![typed("turbo mode")],
                Some(chunks(&[b"4", b"turbo mode", b"\r"])),
            ),
            (
                "a typed answer first, then a tick, then Submit",
                vec![single.clone(), multi.clone()],
                vec![typed("my lang"), picked(&[0])],
                Some(chunks(&[b"4", b"my lang", b"\r", b"1", b"\x1b[C", b"1"])),
            ),
            (
                "control characters never reach the menu",
                vec![single.clone()],
                vec![typed("a\r\x1bb\n")],
                Some(chunks(&[b"4", b"ab", b"\r"])),
            ),
            ("nothing ticked", vec![multi.clone()], vec![picked(&[])], None),
            ("nothing picked", vec![single.clone()], vec![picked(&[])], None),
            ("blank typed answer", vec![single.clone()], vec![typed("  ")], None),
            ("typed answer on a multiple choice", vec![multi.clone()], vec![typed("x")], None),
            ("two picks on a single choice", vec![single.clone()], vec![picked(&[0, 1])], None),
            (
                "a pick and a typed answer",
                vec![single.clone()],
                vec![Given { picked: vec![0], other: Some("x".into()) }],
                None,
            ),
            (
                "fewer answers than questions",
                vec![single.clone(), multi.clone()],
                vec![picked(&[0])],
                None,
            ),
            ("no questions", vec![], vec![], None),
            (
                "the tenth option has no number key",
                vec![q(Select::Single, &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"])],
                vec![picked(&[9])],
                None,
            ),
        ];
        for (name, questions, given, want) in cases {
            assert_eq!(claude_question_keys(&questions, &given), want, "{name}");
        }
    }

    #[test]
    fn the_menu_escape_is_one_escape_byte() {
        assert_eq!(CLAUDE_MENU_ESCAPE, b"\x1b");
    }

    #[test]
    fn unverified_prompts_have_no_buttons() {
        for source in [Some(Source::Gemini), Some(Source::Copilot), Some(Source::Codex), None] {
            assert_eq!(answers(source).len(), 0, "{source:?}");
            assert_eq!(bytes(source, Answer::Allow), None);
        }
    }

    #[test]
    fn every_listed_answer_has_a_label_and_distinct_keys() {
        let listed = answers(Some(Source::Claude));
        assert_ne!(listed[0].1, listed[1].1);
        assert_eq!(listed[0].0.label(), "Allow");
        assert_eq!(listed[1].0.label(), "Deny");
    }
}
