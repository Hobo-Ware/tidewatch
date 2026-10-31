//! The panel facts one batch of transcript entries and turn marks stands for (pure): the turn
//! boundaries, the agent's latest words, the tool calls that opened and closed and the questions
//! that were asked and answered. Anything the CLI stamped before `since_ms` is history and adds
//! nothing, so reading a long session again at launch does not flood the log.

use crate::panel_event::Fact;
use crate::transcript::{AskPart, Entry, Role, ToolPhase};
use crate::transcript_marks::TurnMark;

/// A fact with the time the CLI wrote the line it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub fact: Fact,
    pub ts_ms: Option<i64>,
}

fn is_history(ts: Option<i64>, since_ms: i64) -> bool {
    ts.is_some_and(|ts| ts < since_ms)
}

/// The facts of `entries` in order with `marks` placed among them. Between two turn boundaries only
/// the last thing the agent said is kept, because that is the one the panel reads.
#[must_use]
pub fn facts_of(entries: &[Entry], marks: &[TurnMark], since_ms: i64) -> Vec<Timed> {
    let mut out = Out::default();
    let mut marks = marks.iter().peekable();
    for (at, entry) in entries.iter().enumerate() {
        while let Some(mark) = marks.next_if(|mark| mark.before <= at) {
            out.mark(mark, since_ms);
        }
        if !is_history(entry.ts, since_ms) {
            out.entry(entry);
        }
    }
    for mark in marks {
        out.mark(mark, since_ms);
    }
    out.facts
}

#[derive(Default)]
struct Out {
    facts: Vec<Timed>,
    said_at: Option<usize>,
}

impl Out {
    fn mark(&mut self, mark: &TurnMark, since_ms: i64) {
        if is_history(mark.ts, since_ms) {
            return;
        }
        self.said_at = None;
        self.facts.push(Timed {
            fact: Fact::Turn { started: mark.started, ts_ms: mark.ts },
            ts_ms: mark.ts,
        });
    }

    fn push(&mut self, fact: Fact, ts_ms: Option<i64>) {
        self.facts.push(Timed { fact, ts_ms });
    }

    fn entry(&mut self, entry: &Entry) {
        let ts = entry.ts;
        match (&entry.role, &entry.tool) {
            (Role::Assistant, None) => {
                let fact = Fact::Said { tail: entry.text.clone() };
                if let Some(at) = self.said_at {
                    self.facts[at] = Timed { fact, ts_ms: ts };
                } else {
                    self.said_at = Some(self.facts.len());
                    self.push(fact, ts);
                }
            }
            (Role::Tool, Some(call)) => {
                let id = call.id.clone().unwrap_or_else(|| call.summary.clone());
                match call.phase {
                    ToolPhase::Call => {
                        self.push(Fact::ToolOpened { id, name: call.name.clone() }, ts);
                        if let Some(AskPart::Asked(questions)) = &call.ask {
                            let summary =
                                questions.first().map(|q| q.text.clone()).unwrap_or_default();
                            let n = u32::try_from(questions.len()).unwrap_or(u32::MAX);
                            self.push(Fact::AskOpened { summary, n }, ts);
                        }
                    }
                    ToolPhase::Result { .. } => {
                        if let Some(AskPart::Answered(pairs)) = &call.ask {
                            for (question, answer) in pairs {
                                let fact = Fact::QuestionAnswered {
                                    question: question.clone(),
                                    answer: answer.clone(),
                                };
                                self.push(fact, ts);
                            }
                        }
                        if call.ask.is_some() {
                            self.push(Fact::AskClosed, ts);
                        }
                        self.push(Fact::ToolClosed { id }, ts);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
