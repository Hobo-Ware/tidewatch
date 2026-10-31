//! What a Meeseeks card says, as plain data: the title and meta line, the four stage bars, the
//! status box and the struggling card (pure).

use tidewatch_core::kind::Kind;
use tidewatch_core::meeseeks::{Meeseeks, Phase, StageMark, Step, elapsed_text};

use crate::progress_model::{Bar, BarKind, StageShape};

pub(crate) const STRUGGLE_BODY: &str = "It can't finish alone. It won't poof until you decide.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Done,
    Active,
    Pending,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) text: String,
    pub(crate) strong: bool,
}

fn plain(text: &str) -> Span {
    Span { text: text.to_owned(), strong: false }
}

fn strong(text: &str) -> Span {
    Span { text: text.to_owned(), strong: true }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Struggling {
    pub(crate) title: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) title: String,
    pub(crate) meta: String,
    pub(crate) bars: Vec<(&'static str, Look)>,
    pub(crate) status: Vec<Span>,
    pub(crate) struggling: Option<Struggling>,
    pub(crate) shape: StageShape,
}

fn look(mark: StageMark) -> Look {
    match mark {
        StageMark::Done => Look::Done,
        StageMark::Active => Look::Active,
        StageMark::Pending => Look::Pending,
        StageMark::Skipped => Look::Skipped,
    }
}

fn checking(step: Step, meeseeks: &Meeseeks, tests: Option<&str>) -> Span {
    strong(match step {
        Step::Change => "commit and clean tree",
        Step::Tests => tests.unwrap_or("the tests"),
        Step::Proof => "clean tree after the checks",
        Step::HandOff if meeseeks.criteria.pr => match meeseeks.target().pull_request() {
            Some(pr) => return strong(&format!("{} update", pr.label())),
            None => "pull request",
        },
        Step::HandOff => "hand off",
    })
}

fn status(meeseeks: &Meeseeks, struggling: bool, tests: Option<&str>) -> Vec<Span> {
    match meeseeks.phase {
        Phase::Working if struggling => {
            vec![plain("Not checking any more. Waiting for you to decide.")]
        }
        Phase::Working => match (&meeseeks.note, meeseeks.kind) {
            (Some(note), _) => vec![plain(note), plain(". Waiting for the fix.")],
            (None, Kind::ReadOnly) => vec![plain(
                "Read-only. It poofs with its answer when it says it is done. A change to the worktree makes it an edit.",
            )],
            (None, Kind::Edit) => {
                vec![plain("Working. Tidewatch checks the work when it says it is done.")]
            }
        },
        Phase::Checking(step) => vec![
            plain("Says it's done. Tidewatch is checking: "),
            checking(step, meeseeks, tests),
            plain(" running..."),
        ],
        Phase::Poofing if meeseeks.blocked_reason().is_some() => vec![
            strong("Blocked."),
            plain(&format!(" {}. Poofing...", meeseeks.blocked_reason().unwrap_or_default())),
        ],
        Phase::Poofing if meeseeks.kind == Kind::ReadOnly => {
            vec![strong("Answered."), plain(" Poofing...")]
        }
        Phase::Poofing if meeseeks.nothing_to_ship => {
            vec![strong("Nothing changed."), plain(" No checks to run. Poofing...")]
        }
        Phase::Poofing => vec![strong("Checks passed."), plain(" Poofing...")],
    }
}

impl Card {
    /// The checks as one bar of the panel's stage row: filled by the checks done, with the one
    /// running counted as half. None until a check has begun, and for a read-only Meeseeks.
    pub(crate) fn checks_bar(&self) -> Option<Bar> {
        if self.bars.is_empty() {
            return None;
        }
        let count = |look| self.bars.iter().filter(|(_, bar)| *bar == look).count();
        let (done, active, skipped) =
            (count(Look::Done), count(Look::Active), count(Look::Skipped));
        let total = self.bars.len() - skipped;
        let fill = if total == 0 {
            0.0
        } else {
            ((done as f32 + 0.5 * active as f32) / total as f32).clamp(0.0, 1.0)
        };
        Some(Bar {
            label: "Checks".to_owned(),
            fill,
            kind: if total == 0 { BarKind::Skipped } else { BarKind::Plain },
            current: active > 0,
            done,
            total,
        })
    }
}

pub(crate) fn card(meeseeks: &Meeseeks, name: &str, now_ms: i64, tests: Option<&str>) -> Card {
    let struggle = meeseeks.struggle(now_ms);
    let marks = meeseeks.stages();
    let bars = if meeseeks.phase == Phase::Working || meeseeks.kind == Kind::ReadOnly {
        Vec::new()
    } else {
        let target = meeseeks.target();
        Step::ALL
            .iter()
            .zip(marks)
            .map(|(step, mark)| (step.label_for(&target), look(mark)))
            .collect()
    };
    Card {
        title: meeseeks.task.clone(),
        meta: format!(
            "{} \u{b7} {}",
            meeseeks.kind.label(),
            elapsed_text(meeseeks.elapsed_ms(now_ms))
        ),
        bars,
        status: status(meeseeks, struggle.is_some(), tests),
        struggling: struggle
            .map(|why| Struggling { title: format!("{name} is struggling"), reason: why.reason() }),
        shape: StageShape { kind: meeseeks.kind, no_diff: meeseeks.nothing_to_ship },
    }
}

#[cfg(test)]
mod tests;
