//! A Meeseeks is an agent with one task that ends itself once its work checks out. This is the pure
//! model: what "done" means, the checks Tidewatch runs in order, when it is struggling and the stage row.

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

pub const MAX_TURNS: u32 = 40;
pub const MAX_FAILURES: u32 = 3;
pub const MAX_MINUTES: i64 = 60;
const TAIL_LINES: usize = 12;
const LINE_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Criteria {
    #[serde(default = "on")]
    pub tests: bool,
    #[serde(default = "on")]
    pub committed: bool,
    #[serde(default = "on")]
    pub pr: bool,
}

fn on() -> bool {
    true
}

impl Criteria {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Default for Criteria {
    fn default() -> Self {
        Self { tests: true, committed: true, pr: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Change,
    Tests,
    Proof,
    HandOff,
}

impl Step {
    pub const ALL: [Self; 4] = [Self::Change, Self::Tests, Self::Proof, Self::HandOff];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Change => "Change",
            Self::Tests => "Tests",
            Self::Proof => "Proof",
            Self::HandOff => "Hand off",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Working,
    Checking(Step),
    Poofing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Skipped,
}

/// One check as the poofed summary lists it: how it ended and the one line it said.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckLine {
    pub step: Step,
    pub verdict: Verdict,
    #[serde(default)]
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageMark {
    Done,
    Active,
    Pending,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Struggle {
    FailedChecks,
    Turns,
    Time,
}

impl Struggle {
    #[must_use]
    pub fn reason(self) -> String {
        match self {
            Self::FailedChecks => format!("{MAX_FAILURES} failed checks"),
            Self::Turns => format!("{MAX_TURNS} turns"),
            Self::Time => format!("{MAX_MINUTES} minutes"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Said(String),
    Passed { step: Step, pr_url: Option<String>, detail: String },
    Failed { step: Step, detail: String },
    PoofRefused(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Nothing,
    Check(Step),
    Paste(String),
    Poof { summary: String, pr_url: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeseeks {
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub criteria: Criteria,
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub started_ms: i64,
    #[serde(skip)]
    pub phase: Phase,
    #[serde(skip)]
    pub summary: Option<String>,
    #[serde(skip)]
    pub pr_url: Option<String>,
    #[serde(skip)]
    pub note: Option<String>,
    #[serde(skip)]
    pub details: Vec<(Step, String)>,
}

impl Hash for Meeseeks {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (&self.task, self.criteria, self.turns, self.failures, self.started_ms).hash(state);
    }
}

impl Meeseeks {
    #[must_use]
    pub fn summon(task: &str, criteria: Criteria, now_ms: i64) -> Self {
        Self { task: task.trim().to_owned(), criteria, started_ms: now_ms, ..Self::default() }
    }

    pub fn turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
    }

    #[must_use]
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.started_ms).max(0)
    }

    #[must_use]
    pub fn struggle(&self, now_ms: i64) -> Option<Struggle> {
        if self.failures >= MAX_FAILURES {
            Some(Struggle::FailedChecks)
        } else if self.turns >= MAX_TURNS {
            Some(Struggle::Turns)
        } else if self.elapsed_ms(now_ms) >= MAX_MINUTES * 60_000 {
            Some(Struggle::Time)
        } else {
            None
        }
    }

    fn runs(&self, step: Step) -> bool {
        match step {
            Step::Change => self.criteria.committed,
            Step::Tests => self.criteria.tests,
            Step::Proof | Step::HandOff => true,
        }
    }

    #[must_use]
    pub fn stages(&self) -> [StageMark; 4] {
        Step::ALL.map(|step| {
            let current = match self.phase {
                Phase::Working => Some(Step::Change),
                Phase::Checking(at) => Some(at),
                Phase::Poofing => Some(Step::HandOff),
            };
            let at = current.map_or(0, Step::index);
            if !self.runs(step) {
                StageMark::Skipped
            } else if step.index() < at {
                StageMark::Done
            } else if step.index() == at {
                StageMark::Active
            } else {
                StageMark::Pending
            }
        })
    }

    /// The four checks as they ended: the ones asked for passed (a Meeseeks only poofs after they
    /// do) with the line each said, the others were skipped.
    #[must_use]
    pub fn check_lines(&self) -> Vec<CheckLine> {
        Step::ALL
            .iter()
            .map(|&step| {
                let asked = self.runs(step) && (step != Step::HandOff || self.criteria.pr);
                let detail = if asked {
                    let said = self.details.iter().find(|(done, _)| *done == step);
                    said.map(|(_, line)| line.clone()).unwrap_or_default()
                } else {
                    "Not asked for".to_owned()
                };
                let verdict = if asked { Verdict::Passed } else { Verdict::Skipped };
                CheckLine { step, verdict, detail }
            })
            .collect()
    }

    fn advance(&mut self, after: Option<Step>) -> Command {
        let from = after.map_or(0, |step| step.index() + 1);
        let next = Step::ALL[from..].iter().copied().find(|step| self.runs(*step));
        match next {
            Some(Step::HandOff) if !self.criteria.pr => self.poof(),
            Some(step) => {
                self.phase = Phase::Checking(step);
                Command::Check(step)
            }
            None => self.poof(),
        }
    }

    fn poof(&mut self) -> Command {
        self.phase = Phase::Poofing;
        Command::Poof {
            summary: self.summary.clone().unwrap_or_default(),
            pr_url: self.pr_url.clone(),
        }
    }

    fn failed(&mut self, note: String, paste: String) -> Command {
        self.failures += 1;
        self.phase = Phase::Working;
        self.note = Some(note);
        Command::Paste(paste)
    }

    pub fn apply(&mut self, event: Event, now_ms: i64) -> Command {
        match (event, self.phase) {
            (Event::Said(_), Phase::Working) if self.struggle(now_ms).is_some() => {
                self.note = Some("Says it is done. Waiting for you to decide.".to_owned());
                Command::Nothing
            }
            (Event::Said(summary), Phase::Working) => {
                self.summary = Some(summary);
                self.note = None;
                self.advance(None)
            }
            (Event::Passed { step, pr_url, detail }, Phase::Checking(at)) if step == at => {
                self.details.retain(|(done, _)| *done != step);
                self.details.push((step, detail));
                self.pr_url = pr_url.or_else(|| self.pr_url.take());
                self.advance(Some(step))
            }
            (Event::Failed { step, detail }, Phase::Checking(at)) if step == at => {
                let note = format!("{} failed: {}", step.label(), first_line(&detail));
                self.failed(note, fix_note(step, &detail))
            }
            (Event::PoofRefused(detail), Phase::Poofing) => {
                let note = format!("Cannot poof: {}", first_line(&detail));
                self.failed(note, fix_note(Step::Proof, &detail))
            }
            _ => Command::Nothing,
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("no details")
}

#[must_use]
pub fn fix_note(step: Step, detail: &str) -> String {
    let again = "then call meeseeks_done again";
    match step {
        Step::Change => format!(
            "Tidewatch checked your work and it is not committed yet: {}. Commit everything on this branch, {again}.",
            first_line(detail)
        ),
        Step::Tests => format!(
            "Tidewatch ran the tests and they failed. Fix them, commit, {again}. The end of the output:\n```\n{}\n```",
            tail_of(detail)
        ),
        Step::Proof => format!(
            "Tidewatch cannot finish while the worktree has loose changes: {}. Commit or remove them, {again}.",
            first_line(detail)
        ),
        Step::HandOff => format!(
            "Tidewatch could not open the pull request: {}. If the code is the cause, fix it and commit, {again}. Otherwise say so and wait.",
            first_line(detail)
        ),
    }
}

#[must_use]
pub fn tail_of(output: &str) -> String {
    let lines: Vec<String> = output
        .lines()
        .map(|line| {
            let clean: String = line.chars().filter(|c| *c == '\t' || !c.is_control()).collect();
            let cut: String = clean.trim_end().chars().take(LINE_CHARS).collect();
            cut.replace("```", "'''")
        })
        .collect();
    let end = lines.iter().rposition(|line| !line.is_empty()).map_or(0, |last| last + 1);
    let start = end.saturating_sub(TAIL_LINES);
    lines[start..end].join("\n")
}

#[must_use]
pub fn tasks_of(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

#[must_use]
pub fn elapsed_text(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    }
}

#[cfg(test)]
mod tests;
