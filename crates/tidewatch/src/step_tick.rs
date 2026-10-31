//! The step an answered question ticks (pure): the question the agent asked with its question tool
//! is matched to the open step of its report that it is about, by the words they share.

use tidewatch_core::panel_state::{AnswerSeen, MarkSeen};
use tidewatch_core::progress::Mark;

use crate::panel_blocks::ticked_by_tidewatch;
use crate::progress_model::ItemRow;

const MIN_WORD: usize = 4;
const NOISE: [&str; 18] = [
    "which", "what", "should", "would", "could", "this", "that", "with", "does", "from", "your",
    "into", "there", "their", "have", "will", "them", "then",
];

fn is_open(mark: Mark) -> bool {
    matches!(mark, Mark::Todo | Mark::Doing | Mark::Blocked)
}

/// The words that tell two lines apart: lower case, four letters or more, no filler. `step 3`
/// is one word, so a numbered walk matches its numbers.
fn words(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut previous = "";
    for token in lower.split(|c: char| !c.is_alphanumeric()).filter(|token| !token.is_empty()) {
        if previous == "step" && token.chars().all(|c| c.is_ascii_digit()) {
            out.push(format!("step{token}"));
        } else if token.chars().count() >= MIN_WORD && !NOISE.contains(&token) {
            out.push(token.to_owned());
        }
        previous = token;
    }
    out.sort();
    out.dedup();
    out
}

fn shared(a: &[String], b: &[String]) -> usize {
    a.iter().filter(|word| b.contains(word)).count()
}

/// The index of the open step `question` is about: the one sharing the most words with it (at
/// least two), else the only step waiting on the owner. A tie between steps names none.
pub(crate) fn matching(question: &str, answer: &str, steps: &[ItemRow<'_>]) -> Option<usize> {
    let asked = words(&format!("{question} {answer}"));
    let scored: Vec<(usize, usize)> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| is_open(step.mark))
        .map(|(index, step)| {
            let own = words(&format!("{} {}", step.text, step.why.unwrap_or_default()));
            (index, shared(&asked, &own))
        })
        .collect();
    let best = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
    let top: Vec<usize> =
        scored.iter().filter(|(_, score)| *score == best).map(|(index, _)| *index).collect();
    match top.as_slice() {
        [only] if best >= 2 => Some(*only),
        _ => {
            let waiting: Vec<usize> = scored
                .iter()
                .filter(|(index, score)| *score >= 1 && steps[*index].mark == Mark::Blocked)
                .map(|(index, _)| *index)
                .collect();
            match waiting.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        }
    }
}

/// The texts of the steps the answers since the report tick, each once, leaving out the steps
/// Tidewatch ticked already.
pub(crate) fn ticks(
    steps: &[ItemRow<'_>],
    answers: &[AnswerSeen],
    marks: &[MarkSeen],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for answer in answers {
        let Some(index) = matching(&answer.question, &answer.answer, steps) else { continue };
        let text = steps[index].text;
        let ticked = ticked_by_tidewatch(marks, text) || out.iter().any(|done| done == text);
        if !ticked {
            out.push(text.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests;
