//! What a poofed Meeseeks' summary tab says, as plain data: the header, the checks with their
//! results (none, and one quiet line instead, when nothing changed), the agent's own summary, the pull request, the branch it left, the last turn and the
//! ledger's final stages (pure, table-tested). The branch facts come from a git look at the branch,
//! never from the removed folder, so they may still be on their way.

use jiff::tz::TimeZone;
use tidewatch_core::kind::Kind;
use tidewatch_core::meeseeks::{CheckLine, Step, Verdict, blocked_reason_of, elapsed_text};
use tidewatch_core::meeseeks_branch::{Branch, Changed};
use tidewatch_core::meeseeks_log::Poofed;

use crate::job_model::clock_label;
use crate::meeseeks_model::{Card, Look, Span};
use crate::progress_model::StageShape;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CheckRow {
    pub(crate) label: &'static str,
    pub(crate) look: Look,
    pub(crate) result: &'static str,
    pub(crate) detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrLink {
    pub(crate) label: String,
    pub(crate) url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchLine {
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) files: Option<FilesLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FilesLine {
    pub(crate) text: String,
    pub(crate) added: String,
    pub(crate) removed: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageRow {
    pub(crate) name: String,
    pub(crate) done: u32,
    pub(crate) total: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Quiet {
    pub(crate) text: String,
    pub(crate) branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) prompt: Option<String>,
    pub(crate) meta: String,
    pub(crate) card: Card,
    pub(crate) checks: Vec<CheckRow>,
    pub(crate) read_only: bool,
    pub(crate) quiet: Option<Quiet>,
    pub(crate) said: Option<String>,
    pub(crate) pr: Option<PrLink>,
    pub(crate) branch: Option<BranchLine>,
    pub(crate) done: Vec<String>,
    pub(crate) next: Vec<String>,
    pub(crate) stages: Vec<StageRow>,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn look_of(verdict: Verdict) -> Look {
    match verdict {
        Verdict::Passed => Look::Done,
        Verdict::Skipped => Look::Skipped,
    }
}

fn result_of(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Passed => "Passed",
        Verdict::Skipped => "Skipped",
    }
}

/// The checks as saved, or four passes without a line for a poof logged before checks were kept.
fn lines_of(record: &Poofed) -> Vec<CheckLine> {
    if !record.checks.is_empty() {
        return record.checks.clone();
    }
    Step::ALL
        .iter()
        .map(|&step| CheckLine { step, verdict: Verdict::Passed, detail: String::new() })
        .collect()
}

fn meta_of(record: &Poofed, zone: &TimeZone) -> String {
    let took = elapsed_text(i64::try_from(record.duration_secs).unwrap_or(0).saturating_mul(1000));
    let mut parts = vec!["Poofed".to_owned(), took];
    if record.ended_ms > 0 {
        parts.extend(clock_label(record.ended_ms, zone));
    }
    parts.join(" \u{b7} ")
}

fn files_line(changed: Changed) -> FilesLine {
    let text = match changed.files {
        0 => "No files changed".to_owned(),
        n => format!("{} changed", plural(n, "file", "files")),
    };
    FilesLine {
        text,
        added: format!("+{}", changed.added),
        removed: format!("-{}", changed.removed),
    }
}

/// `None` when the poof was logged without its branch. `facts` is `None` while git is being asked.
fn touched(facts: Option<&Branch>) -> bool {
    matches!(facts, Some(Branch::Found(found)) if found.ahead > 0 || found.changed.files > 0)
}

fn quiet_text(record: &Poofed) -> &'static str {
    if record.kind == Kind::ReadOnly {
        "Read-only \u{b7} nothing changed \u{b7} branch kept"
    } else if blocked_reason_of(&record.summary).is_some() {
        "Blocked \u{b7} nothing changed \u{b7} branch kept"
    } else {
        "Nothing changed \u{b7} branch kept"
    }
}

fn branch_line(record: &Poofed, facts: Option<&Branch>) -> Option<BranchLine> {
    if record.branch.is_empty() || record.repo.as_os_str().is_empty() {
        return None;
    }
    let name = record.branch.clone();
    let (text, files) = match facts {
        None => ("Reading the branch...".to_owned(), None),
        Some(Branch::Missing) => ("The branch is gone, deleted since it poofed".to_owned(), None),
        Some(Branch::Unreadable(why)) => (format!("Cannot read the branch: {why}"), None),
        Some(Branch::Found(found)) => (
            format!("{} ahead of {}", plural(found.ahead, "commit", "commits"), found.base),
            Some(files_line(found.changed)),
        ),
    };
    Some(BranchLine { name, text, files })
}

pub(crate) fn summary(record: &Poofed, facts: Option<&Branch>, zone: &TimeZone) -> Summary {
    let lines = lines_of(record);
    let checks: Vec<CheckRow> = lines
        .iter()
        .map(|line| CheckRow {
            label: line.step.label(),
            look: look_of(line.verdict),
            result: result_of(line.verdict),
            detail: line.detail.clone(),
        })
        .collect();
    let passed = lines.iter().filter(|line| line.verdict == Verdict::Passed).count();
    let none_ran = lines.iter().all(|line| line.verdict == Verdict::Skipped);
    let title = if record.kind == Kind::ReadOnly {
        "Answered"
    } else if blocked_reason_of(&record.summary).is_some() {
        "Blocked"
    } else if none_ran {
        "Nothing to check"
    } else {
        "Checks passed"
    };
    let card = Card {
        title: title.to_owned(),
        meta: if none_ran {
            "no checks".to_owned()
        } else {
            format!("{passed} of {}", lines.len())
        },
        bars: checks.iter().map(|check| (check.label, check.look)).collect(),
        status: vec![
            Span { text: "Poofed.".to_owned(), strong: true },
            Span { text: " The worktree is gone and the branch stays.".to_owned(), strong: false },
        ],
        struggling: None,
        shape: StageShape { kind: record.kind, no_diff: false },
    };
    let quiet = none_ran && record.pr_url.is_none() && !touched(facts);
    let prompt = Some(record.task.trim().to_owned()).filter(|task| !task.is_empty());
    let said = Some(record.summary.trim().to_owned()).filter(|text| !text.is_empty());
    let pr = record.pr_url.as_ref().map(|url| PrLink {
        label: record.pr_label().unwrap_or_else(|| url.clone()),
        url: url.clone(),
    });
    Summary {
        prompt,
        meta: meta_of(record, zone),
        card,
        checks: if quiet { Vec::new() } else { checks },
        read_only: record.kind == Kind::ReadOnly,
        quiet: quiet
            .then(|| Quiet { text: quiet_text(record).to_owned(), branch: record.branch.clone() }),
        said,
        pr,
        branch: branch_line(record, facts).filter(|_| !quiet),
        done: record.done.clone(),
        next: record.next.clone(),
        stages: record
            .stages
            .iter()
            .filter(|stage| stage.total > 0)
            .map(|stage| StageRow {
                name: stage.name.clone(),
                done: stage.done,
                total: stage.total,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
