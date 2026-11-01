//! The done check: four checks on an agent that says it is done (committed, pushed, pull request,
//! ledger), each a line with a status, and one outcome over them. Blocks on git and gh.

use std::collections::BTreeSet;
use std::path::Path;

use crate::civil_date::Date;
use crate::delivery::{compare_ref, detect_base};
use crate::done_proof::{self, Findings, TestsRun, Verified};
use crate::limited;
use crate::pr_gh::unpushed;
use crate::pr_status::{
    FIELDS, Outcome as CheckRun, Phase, Snapshot, State, parse_snapshot, phase,
};
use crate::progress::{Ledger, Mark, State as LedgerState, real};
use crate::progress_report::WaitingOn;
use crate::test_command::{self, Gate};
use crate::text::plural;
use crate::tools::{GhError, Tools, run_gh};
use crate::workflow_profile::{Checks, Workflow};

const REMOTE: &str = "origin";
const NAMED_CHECKS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Committed,
    Pushed,
    PullRequest,
    Tests,
    FixTest,
    Verified,
    Scope,
    Unverified,
}

impl Kind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Committed => "Committed",
            Self::Pushed => "Pushed",
            Self::PullRequest => "Pull request",
            Self::Tests => "Tests",
            Self::FixTest => "Fix test",
            Self::Verified => "Verified",
            Self::Scope => "Scope",
            Self::Unverified => "Not verified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    /// The check could not run (no `gh`, no network, checks still running): it says nothing.
    Unknown,
    /// The check does not apply here (no pull request): shown as not checked, counts for nothing.
    Skipped,
    /// Everything left open is due on a later date: nothing can change before then.
    Waiting,
    /// Something to look at that changes nothing: it never blocks and nothing is sent for it.
    Noted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub status: Status,
    pub detail: String,
}

impl Line {
    fn new(kind: Kind, (status, detail): (Status, String)) -> Self {
        Self { kind, status, detail }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Blocked,
    NotVerified,
    Waiting,
}

/// Blocked when any check failed, else Not verified when any could not run, else Waiting when
/// something is due on a later date, else Done.
#[must_use]
pub fn decide(lines: &[Line]) -> Outcome {
    let any = |status| lines.iter().any(|line| line.status == status);
    if any(Status::Fail) {
        Outcome::Blocked
    } else if any(Status::Unknown) {
        Outcome::NotVerified
    } else if any(Status::Waiting) {
        Outcome::Waiting
    } else {
        Outcome::Done
    }
}

/// One item the agent's ledger lists under Not verified, with the later date it waits for when the
/// ledger schedules it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unverified {
    pub text: String,
    pub until: Option<Date>,
}

impl Unverified {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), until: None }
    }

    #[must_use]
    pub fn until(text: impl Into<String>, date: Date) -> Self {
        Self { text: text.into(), until: Some(date) }
    }

    /// The date this item waits for while `today` is before it.
    #[must_use]
    pub fn waiting_for(&self, today: Date) -> Option<Date> {
        self.until.filter(|date| *date > today)
    }
}

const MIN_WORD: usize = 4;
const NOT_A_TOPIC: [&str; 44] = [
    "that",
    "this",
    "from",
    "into",
    "onto",
    "when",
    "what",
    "then",
    "than",
    "there",
    "their",
    "them",
    "have",
    "been",
    "will",
    "would",
    "could",
    "should",
    "were",
    "your",
    "with",
    "across",
    "about",
    "after",
    "before",
    "again",
    "over",
    "also",
    "only",
    "just",
    "check",
    "verify",
    "verified",
    "confirm",
    "test",
    "tested",
    "make",
    "sure",
    "tomorrow",
    "today",
    "yesterday",
    "tonight",
    "next",
    "later",
];
const MONTHS: [&str; 9] =
    ["march", "april", "june", "july", "august", "september", "october", "november", "december"];

fn topic_words(text: &str) -> BTreeSet<String> {
    text.replace('\u{2019}', "'")
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|word| word.trim_matches('\'').to_lowercase())
        .map(|word| word.strip_suffix("'s").map_or(word.clone(), str::to_owned))
        .map(|word| match word.strip_suffix('s') {
            Some(root) if !word.ends_with("ss") && root.len() >= MIN_WORD => root.to_owned(),
            _ => word,
        })
        .filter(|word| {
            word.chars().count() >= MIN_WORD
                && !NOT_A_TOPIC.contains(&word.as_str())
                && !MONTHS.contains(&word.as_str())
                && !word.chars().all(|c| c.is_ascii_digit())
        })
        .collect()
}

fn scheduled_for(item: &str, ledger: &Ledger) -> Option<Date> {
    let wanted = topic_words(item);
    ledger
        .scheduled
        .iter()
        .filter(|entry| !matches!(entry.mark, Mark::Done | Mark::Skipped))
        .filter_map(|entry| {
            let date = entry.when.date()?;
            let steps = entry.steps.iter().map(|step| step.text.as_str());
            let text = std::iter::once(entry.what.as_str()).chain(steps).collect::<Vec<_>>();
            let shared = topic_words(&text.join(" ")).intersection(&wanted).count();
            (shared > 0).then_some((shared, date))
        })
        .max_by_key(|(shared, date)| (*shared, std::cmp::Reverse(*date)))
        .map(|(_, date)| date)
}

/// The ledger's Not verified items. An item waits for the date of the open dated Scheduled item
/// that shares a topic word with it, else for the date the ledger waits on while its state is
/// `waiting`.
#[must_use]
pub fn unverified_of(ledger: &Ledger) -> Vec<Unverified> {
    let waiting = match (ledger.status.state, &ledger.waiting_on) {
        (Some(LedgerState::Waiting), Some(WaitingOn::Date(date))) => Some(*date),
        _ => None,
    };
    real(&ledger.not_verified)
        .map(|entry| Unverified {
            text: entry.text.clone(),
            until: scheduled_for(&entry.text, ledger).or(waiting),
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The commit the checks looked at.
    pub head: String,
    pub lines: Vec<Line>,
    /// The items the agent's ledger lists under Not verified.
    pub unverified: Vec<Unverified>,
    /// What Tidewatch ran or worked out itself.
    pub found: Findings,
}

impl Report {
    #[must_use]
    pub fn outcome(&self) -> Outcome {
        decide(&self.lines)
    }

    /// The report as it reads on `today`: an item whose date has come is no longer waiting.
    #[must_use]
    pub fn on(&self, today: Date) -> Self {
        let mut report = self.clone();
        if !self.unverified.is_empty() {
            let own = |line: &&mut Line| {
                line.kind == Kind::Unverified
                    && matches!(line.status, Status::Unknown | Status::Waiting)
            };
            for line in report.lines.iter_mut().filter(own) {
                (line.status, line.detail) = unverified_status(Some(&self.unverified), today);
            }
        }
        report
    }

    #[must_use]
    pub fn outcome_on(&self, today: Date) -> Outcome {
        self.on(today).outcome()
    }

    /// The short note that asks the agent to fix what failed, with the wording of the repo's
    /// workflow. `None` when nothing failed.
    #[must_use]
    pub fn fix_note(&self, workflow: Workflow) -> Option<String> {
        let failing =
            |kind| self.lines.iter().any(|line| line.kind == kind && line.status == Status::Fail);
        let mut parts: Vec<String> = Vec::new();
        let tests = self.found.tests.as_ref().filter(|_| failing(Kind::Tests));
        let others: Vec<String> = self
            .lines
            .iter()
            .filter(|line| line.status == Status::Fail)
            .filter(|line| !matches!(line.kind, Kind::Tests | Kind::FixTest))
            .map(|line| line.detail.clone())
            .collect();
        if !others.is_empty() {
            parts.push(format!(
                "The done check found: {}. Please fix that and tell me when it is done.",
                others.join("; ")
            ));
        }
        if failing(Kind::FixTest) {
            parts.push(done_proof::FIX_WITHOUT_TEST.to_owned());
        }
        if let Some(run) = tests {
            parts.push(run.failure_note(workflow));
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }
}

/// What the tab knows of its pull request when the check starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrFact {
    /// A lookup said there is none: no GitHub call is made.
    None,
    /// The lookup has not run: ask GitHub for the branch's pull request.
    Unknown,
    Number(u64),
}

pub struct Input<'a> {
    pub tools: &'a Tools,
    pub dir: &'a Path,
    pub branch: &'a str,
    pub pr: PrFact,
    /// The ledger's Not verified items; `None` when the tab keeps no ledger.
    pub unverified: Option<Vec<Unverified>>,
    pub today: Date,
    /// The repo's switches for the checks Tidewatch runs itself.
    pub checks: Checks,
    /// The repo's main checkout, where `.tidewatch.toml` is read when the worktree has none.
    pub repo: &'a Path,
    /// The goal and the steps of the agent's report, which name the files it planned to change;
    /// `None` when the tab keeps no ledger.
    pub plan: Option<Vec<String>>,
    /// The ledger's Verified items; `None` when the tab keeps no ledger.
    pub verified: Option<Vec<Verified>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remote {
    Head(String),
    Missing,
    Unreachable(String),
}

enum Look {
    Skipped,
    Absent,
    Found(Box<Snapshot>),
    Failed(GhError),
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

#[must_use]
pub fn committed_status(
    dirty: usize,
    ahead: Option<(String, usize)>,
    merged: bool,
) -> (Status, String) {
    if dirty > 0 {
        let files = plural(dirty, "file", "files");
        return (Status::Fail, format!("{files} with uncommitted changes"));
    }
    if merged {
        return (Status::Pass, "nothing left uncommitted".to_owned());
    }
    match ahead {
        None => {
            (Status::Unknown, "cannot tell which branch this one is measured against".to_owned())
        }
        Some((base, 0)) => (Status::Fail, format!("no commit ahead of {base}")),
        Some((base, n)) => (
            Status::Pass,
            format!("clean tree, {} ahead of {base}", plural(n, "commit", "commits")),
        ),
    }
}

#[must_use]
pub fn pushed_status(
    local: &str,
    remote: &Remote,
    unpushed: Option<usize>,
    merged: bool,
) -> (Status, String) {
    if merged {
        return (Status::Pass, "the pull request is merged".to_owned());
    }
    match remote {
        Remote::Unreachable(why) => (Status::Unknown, format!("cannot reach {REMOTE}: {why}")),
        Remote::Missing => (Status::Fail, format!("the branch is not on {REMOTE} yet")),
        Remote::Head(head) if head == local => {
            (Status::Pass, format!("{REMOTE} has {}", short(local)))
        }
        Remote::Head(_) => match unpushed {
            Some(n) if n > 0 => {
                (Status::Fail, format!("{} not pushed", plural(n, "commit", "commits")))
            }
            _ => (Status::Fail, format!("{REMOTE} is at a different commit than the branch")),
        },
    }
}

fn failed_names(snapshot: &Snapshot) -> String {
    let mut names: Vec<&str> = snapshot
        .checks
        .iter()
        .filter(|check| check.outcome == CheckRun::Failed)
        .map(|check| check.name.as_str())
        .collect();
    names.dedup();
    let more = names.len().saturating_sub(NAMED_CHECKS);
    let shown = names.iter().take(NAMED_CHECKS).copied().collect::<Vec<_>>().join(", ");
    if more > 0 { format!("{shown} and {more} more") } else { shown }
}

#[must_use]
pub fn pull_request_status(snapshot: &Snapshot, local_head: &str) -> (Status, String) {
    let number = snapshot.number;
    match snapshot.state {
        State::Merged => return (Status::Pass, format!("PR #{number} merged")),
        State::Closed => {
            return (Status::Fail, format!("PR #{number} was closed without merging"));
        }
        State::Open => {}
    }
    if snapshot.conflicting {
        return (Status::Fail, format!("PR #{number} has merge conflicts"));
    }
    if !local_head.is_empty() && snapshot.head != local_head {
        return (
            Status::Unknown,
            format!(
                "PR #{number} shows {}, not the latest commit {}",
                short(&snapshot.head),
                short(local_head)
            ),
        );
    }
    match phase(&snapshot.checks) {
        Phase::Failed => {
            (Status::Fail, format!("PR #{number} checks failing: {}", failed_names(snapshot)))
        }
        Phase::Running { done, total } => {
            (Status::Unknown, format!("PR #{number} checks still running ({done} of {total})"))
        }
        Phase::NoChecks => (Status::Pass, format!("PR #{number} open, no checks reported")),
        Phase::Passed => (Status::Pass, format!("PR #{number} open, checks passing")),
    }
}

#[must_use]
pub fn unverified_status(items: Option<&[Unverified]>, today: Date) -> (Status, String) {
    match items {
        None => (Status::Skipped, "the agent keeps no progress report".to_owned()),
        Some([]) => (Status::Pass, "nothing left unverified".to_owned()),
        Some(items) => {
            let open = items.iter().filter(|item| item.waiting_for(today).is_none()).count();
            if open > 0 {
                return (
                    Status::Unknown,
                    format!("{} the agent did not verify", plural(open, "item", "items")),
                );
            }
            let first = items.iter().filter_map(|item| item.waiting_for(today)).min();
            let when = first.map_or_else(String::new, |date| format!(" until {date}"));
            (Status::Waiting, format!("{} waiting{when}", plural(items.len(), "item", "items")))
        }
    }
}

fn git_stdout(tools: &Tools, dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tools.git(dir);
    command.args(args);
    let out = tools.finish(command).map_err(|e| limited::failure("git", &e))?;
    if out.success {
        Ok(out.stdout.trim().to_owned())
    } else {
        Err(out.stderr.lines().next().unwrap_or_default().trim().to_owned())
    }
}

fn dirty_files(tools: &Tools, dir: &Path) -> Result<usize, String> {
    let status = git_stdout(tools, dir, &["status", "--porcelain"])?;
    Ok(status.lines().filter(|line| !line.trim().is_empty()).count())
}

fn commits_ahead(tools: &Tools, dir: &Path) -> Option<(String, usize)> {
    let base = detect_base(tools, dir)?;
    let against = compare_ref(tools, dir, &base);
    let range = format!("{against}..HEAD");
    let count = git_stdout(tools, dir, &["rev-list", "--count", &range]).ok()?.parse().ok()?;
    Some((base, count))
}

fn remote_head(tools: &Tools, dir: &Path, branch: &str) -> Remote {
    let reference = format!("refs/heads/{branch}");
    match git_stdout(tools, dir, &["ls-remote", REMOTE, &reference]) {
        Err(why) => Remote::Unreachable(why),
        Ok(text) => text
            .split_whitespace()
            .next()
            .map_or(Remote::Missing, |sha| Remote::Head(sha.to_owned())),
    }
}

fn look(input: &Input<'_>) -> Look {
    let target = match &input.pr {
        PrFact::None => return Look::Skipped,
        PrFact::Number(number) => number.to_string(),
        PrFact::Unknown if input.branch.is_empty() || input.branch.starts_with('-') => {
            return Look::Failed(GhError::Failed(format!(
                "{:?} is not a usable branch name",
                input.branch
            )));
        }
        PrFact::Unknown => input.branch.to_owned(),
    };
    match run_gh(input.tools, input.dir, &["pr", "view", &target, "--json", FIELDS]) {
        Ok(json) => parse_snapshot(&json).map_or_else(
            |message| Look::Failed(GhError::Failed(message)),
            |snapshot| Look::Found(Box::new(snapshot)),
        ),
        Err(GhError::Failed(message)) if message.contains("no pull requests found") => Look::Absent,
        Err(error) => Look::Failed(error),
    }
}

fn pull_request_line(look: &Look, local_head: &str) -> (Status, String) {
    match look {
        Look::Skipped | Look::Absent => {
            (Status::Skipped, "no pull request for this branch".to_owned())
        }
        Look::Failed(error) => (Status::Unknown, error.to_string()),
        Look::Found(snapshot) => pull_request_status(snapshot, local_head),
    }
}

fn changed_files(tools: &Tools, dir: &Path) -> Option<(String, Vec<String>)> {
    let base = detect_base(tools, dir)?;
    let range = format!("{}...HEAD", compare_ref(tools, dir, &base));
    let names = git_stdout(tools, dir, &["diff", "--name-only", &range]).ok()?;
    Some((range, names.lines().map(str::to_owned).filter(|name| !name.is_empty()).collect()))
}

fn commit_subjects(tools: &Tools, dir: &Path, range: &str) -> Vec<String> {
    let range = range.replace("...", "..");
    git_stdout(tools, dir, &["log", "--format=%s", &range])
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn tests_line(input: &Input<'_>) -> (Line, Option<TestsRun>) {
    let kind = Kind::Tests;
    match test_command::resolve(input.dir, input.repo) {
        Gate::Run(detected) => {
            let started = std::time::Instant::now();
            let timeout = test_command::timeout_in(input.dir, input.repo);
            let run = test_command::run_in_timeout(input.dir, &detected.command, timeout);
            let ran = TestsRun::of(&detected.command, &run, started.elapsed());
            (Line::new(kind, ran.status()), Some(ran))
        }
        Gate::Skip(why) => (Line::new(kind, (Status::Skipped, why)), None),
        Gate::None => (
            Line::new(kind, (Status::Skipped, "no test command found for this repo".to_owned())),
            None,
        ),
    }
}

fn fix_test_line(tools: &Tools, input: &Input<'_>, range: &str, files: &[String]) -> Option<Line> {
    if !done_proof::is_fix(&commit_subjects(tools, input.dir, range), input.branch) {
        return None;
    }
    let has_test = done_proof::touches_test_file(files)
        || git_stdout(tools, input.dir, &["diff", "-U0", range])
            .is_ok_and(|patch| done_proof::adds_test(&patch));
    let status = if has_test {
        (Status::Pass, "the fix has a test".to_owned())
    } else {
        (Status::Fail, "the branch fixes something but adds no test".to_owned())
    };
    Some(Line::new(Kind::FixTest, status))
}

/// The lines for the switches that are on, and what Tidewatch left behind making them.
fn proof_lines(input: &Input<'_>) -> (Vec<Line>, Findings) {
    let (tools, checks) = (input.tools, input.checks);
    let mut lines = Vec::new();
    let mut found = Findings::default();
    if checks.tests {
        let (line, ran) = tests_line(input);
        lines.push(line);
        found.tests = ran;
    }
    let changed =
        (checks.fix_test || checks.scope).then(|| changed_files(tools, input.dir)).flatten();
    if checks.fix_test
        && let Some((range, files)) = &changed
        && let Some(line) = fix_test_line(tools, input, range, files)
    {
        lines.push(line);
    }
    if checks.verified
        && let Some(items) = &input.verified
        && let Some((status, detail, claimed)) =
            done_proof::verified_status(items, found.tests.as_ref())
    {
        lines.push(Line::new(Kind::Verified, (status, detail)));
        found.claimed = claimed;
    }
    if checks.scope {
        let files = changed.map(|(_, files)| files).unwrap_or_default();
        let (status, detail, outside) = done_proof::scope_status(&files, input.plan.as_deref());
        lines.push(Line::new(Kind::Scope, (status, detail)));
        found.outside = outside;
    }
    (lines, found)
}

/// Runs the four checks in `input.dir`, then the ones the repo's switches turn on.
#[must_use]
pub fn run(input: &Input<'_>) -> Report {
    let (tools, dir) = (input.tools, input.dir);
    let head = git_stdout(tools, dir, &["rev-parse", "HEAD"]).unwrap_or_default();
    let pr = look(input);
    let merged = matches!(&pr, Look::Found(snapshot) if snapshot.state == State::Merged);
    let committed = match dirty_files(tools, dir) {
        Ok(dirty) => committed_status(dirty, commits_ahead(tools, dir), merged),
        Err(why) => (Status::Unknown, format!("cannot read the work tree: {why}")),
    };
    let pushed = if head.is_empty() {
        (Status::Unknown, "cannot read the branch head".to_owned())
    } else {
        let remote = if merged { Remote::Missing } else { remote_head(tools, dir, input.branch) };
        let behind = match &remote {
            Remote::Head(sha) if *sha != head => unpushed(tools, dir, sha).map(|(_, n)| n),
            _ => None,
        };
        pushed_status(&head, &remote, behind, merged)
    };
    let (proof, found) =
        if merged { (Vec::new(), Findings::default()) } else { proof_lines(input) };
    let unverified = input.unverified.clone().unwrap_or_default();
    let mut lines = vec![
        Line::new(Kind::Committed, committed),
        Line::new(Kind::Pushed, pushed),
        Line::new(Kind::PullRequest, pull_request_line(&pr, &head)),
    ];
    lines.extend(proof);
    lines.push(Line::new(
        Kind::Unverified,
        unverified_status(input.unverified.as_deref(), input.today),
    ));
    Report { head, lines, unverified, found }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(status: Status) -> Line {
        Line { kind: Kind::Pushed, status, detail: String::new() }
    }

    #[test]
    fn the_outcome_is_blocked_over_not_verified_over_done() {
        use Status::{Fail, Pass, Skipped, Unknown};
        let cases: [(&[Status], Outcome); 9] = [
            (&[], Outcome::Done),
            (&[Pass, Pass, Pass, Pass], Outcome::Done),
            (&[Pass, Pass, Skipped, Pass], Outcome::Done),
            (&[Skipped, Skipped], Outcome::Done),
            (&[Pass, Unknown], Outcome::NotVerified),
            (&[Skipped, Unknown, Pass], Outcome::NotVerified),
            (&[Pass, Fail], Outcome::Blocked),
            (&[Unknown, Fail], Outcome::Blocked),
            (&[Fail, Fail, Skipped], Outcome::Blocked),
        ];
        for (statuses, want) in cases {
            let lines: Vec<Line> = statuses.iter().copied().map(line).collect();
            assert_eq!(decide(&lines), want, "{statuses:?}");
        }
    }

    #[test]
    fn committed_names_what_is_loose_or_missing() {
        let main = || Some(("main".to_owned(), 2));
        let cases = [
            (1, main(), false, Status::Fail, "1 file with uncommitted changes"),
            (3, main(), false, Status::Fail, "3 files with uncommitted changes"),
            (0, Some(("main".to_owned(), 0)), false, Status::Fail, "no commit ahead of main"),
            (0, main(), false, Status::Pass, "clean tree, 2 commits ahead of main"),
            (
                0,
                Some(("main".to_owned(), 1)),
                false,
                Status::Pass,
                "clean tree, 1 commit ahead of main",
            ),
            (
                0,
                None,
                false,
                Status::Unknown,
                "cannot tell which branch this one is measured against",
            ),
            (0, Some(("main".to_owned(), 0)), true, Status::Pass, "nothing left uncommitted"),
            (2, None, true, Status::Fail, "2 files with uncommitted changes"),
        ];
        for (dirty, ahead, merged, status, detail) in cases {
            assert_eq!(
                committed_status(dirty, ahead.clone(), merged),
                (status, detail.to_owned()),
                "dirty {dirty} ahead {ahead:?} merged {merged}"
            );
        }
    }

    #[test]
    fn pushed_compares_the_remote_branch_head_with_the_local_one() {
        let sha = "1234567890abcdef";
        let other = Remote::Head("fedcba0987654321".to_owned());
        let cases = [
            (Remote::Head(sha.to_owned()), None, false, Status::Pass, "origin has 1234567"),
            (other.clone(), Some(2), false, Status::Fail, "2 commits not pushed"),
            (other.clone(), Some(1), false, Status::Fail, "1 commit not pushed"),
            (
                other.clone(),
                Some(0),
                false,
                Status::Fail,
                "origin is at a different commit than the branch",
            ),
            (other, None, false, Status::Fail, "origin is at a different commit than the branch"),
            (Remote::Missing, None, false, Status::Fail, "the branch is not on origin yet"),
            (
                Remote::Unreachable("no route".to_owned()),
                None,
                false,
                Status::Unknown,
                "cannot reach origin: no route",
            ),
            (Remote::Missing, None, true, Status::Pass, "the pull request is merged"),
        ];
        for (remote, behind, merged, status, detail) in cases {
            assert_eq!(
                pushed_status(sha, &remote, behind, merged),
                (status, detail.to_owned()),
                "{remote:?} {behind:?} {merged}"
            );
        }
    }

    fn snapshot(state: &str, checks: &str, mergeable: &str, head: &str) -> Snapshot {
        let json = format!(
            r#"{{"number":42,"title":"t","url":"https://github.com/acme/web/pull/42","state":"{state}","mergeable":"{mergeable}","mergeStateStatus":"CLEAN","reviewDecision":"","headRefOid":"{head}","baseRefName":"main","headRefName":"agent/x","author":{{"login":"a"}},"createdAt":"2026-01-01T00:00:00Z","statusCheckRollup":[{checks}],"reviews":[],"comments":[]}}"#
        );
        parse_snapshot(&json).unwrap()
    }

    fn run_of(name: &str, status: &str, conclusion: &str) -> String {
        format!(
            r#"{{"__typename":"CheckRun","name":"{name}","workflowName":"CI","status":"{status}","conclusion":"{conclusion}","detailsUrl":"https://github.com/acme/web/actions/runs/1/job/2","startedAt":"2026-01-01T00:00:00Z","completedAt":"2026-01-01T00:01:00Z"}}"#
        )
    }

    #[test]
    fn the_pull_request_line_reads_state_conflicts_head_and_checks() {
        let pass = run_of("test", "COMPLETED", "SUCCESS");
        let fail = |name: &str| run_of(name, "COMPLETED", "FAILURE");
        let running = run_of("lint", "IN_PROGRESS", "");
        let head = "abcdef0123456789";
        let cases = [
            (snapshot("MERGED", "", "UNKNOWN", head), Status::Pass, "PR #42 merged"),
            (
                snapshot("CLOSED", "", "UNKNOWN", head),
                Status::Fail,
                "PR #42 was closed without merging",
            ),
            (
                snapshot("OPEN", &pass, "CONFLICTING", head),
                Status::Fail,
                "PR #42 has merge conflicts",
            ),
            (
                snapshot("OPEN", &pass, "MERGEABLE", "0000000aaaa"),
                Status::Unknown,
                "PR #42 shows 0000000, not the latest commit abcdef0",
            ),
            (
                snapshot("OPEN", &format!("{pass},{}", fail("build")), "MERGEABLE", head),
                Status::Fail,
                "PR #42 checks failing: build",
            ),
            (
                snapshot(
                    "OPEN",
                    &format!("{},{},{},{}", fail("a"), fail("b"), fail("c"), fail("d")),
                    "MERGEABLE",
                    head,
                ),
                Status::Fail,
                "PR #42 checks failing: a, b, c and 1 more",
            ),
            (
                snapshot("OPEN", &format!("{pass},{running}"), "MERGEABLE", head),
                Status::Unknown,
                "PR #42 checks still running (1 of 2)",
            ),
            (
                snapshot("OPEN", "", "MERGEABLE", head),
                Status::Pass,
                "PR #42 open, no checks reported",
            ),
            (
                snapshot("OPEN", &pass, "MERGEABLE", head),
                Status::Pass,
                "PR #42 open, checks passing",
            ),
        ];
        for (snap, status, detail) in cases {
            assert_eq!(pull_request_status(&snap, head), (status, detail.to_owned()), "{detail}");
        }
    }

    #[test]
    fn an_unreadable_local_head_does_not_flag_the_pull_request_head() {
        let snap = snapshot("OPEN", "", "MERGEABLE", "abcdef0123");
        assert_eq!(pull_request_status(&snap, "").0, Status::Pass);
    }

    fn day(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    #[test]
    fn the_ledger_line_lists_unverified_items_and_skips_a_tab_without_a_ledger() {
        let today = day("2026-10-27");
        let items = vec![Unverified::new("the migration on staging")];
        let two = vec![Unverified::new("a"), Unverified::new("b")];
        assert_eq!(unverified_status(None, today).0, Status::Skipped);
        assert_eq!(
            unverified_status(Some(&[]), today),
            (Status::Pass, "nothing left unverified".to_owned())
        );
        assert_eq!(
            unverified_status(Some(&items), today),
            (Status::Unknown, "1 item the agent did not verify".to_owned())
        );
        assert_eq!(
            unverified_status(Some(&two), today),
            (Status::Unknown, "2 items the agent did not verify".to_owned())
        );
    }

    #[test]
    fn items_due_on_a_later_date_wait_and_only_the_rest_are_unverified() {
        let today = day("2026-10-27");
        let tomorrow = Unverified::until("live list", day("2026-10-28"));
        let cases = [
            (vec![tomorrow.clone()], Status::Waiting, "1 item waiting until 2026-10-28"),
            (
                vec![tomorrow.clone(), Unverified::until("two", day("2026-11-02"))],
                Status::Waiting,
                "2 items waiting until 2026-10-28",
            ),
            (
                vec![tomorrow.clone(), Unverified::new("safari")],
                Status::Unknown,
                "1 item the agent did not verify",
            ),
            (
                vec![Unverified::until("due today", today)],
                Status::Unknown,
                "1 item the agent did not verify",
            ),
            (
                vec![Unverified::until("overdue", day("2026-10-01")), tomorrow],
                Status::Unknown,
                "1 item the agent did not verify",
            ),
        ];
        for (items, status, detail) in cases {
            assert_eq!(
                unverified_status(Some(&items), today),
                (status, detail.to_owned()),
                "{items:?}"
            );
        }
    }

    #[test]
    fn the_outcome_waits_only_when_nothing_else_is_open() {
        use Status::{Fail, Pass, Unknown, Waiting};
        let cases: [(&[Status], Outcome); 5] = [
            (&[Pass, Waiting], Outcome::Waiting),
            (&[Waiting, Waiting], Outcome::Waiting),
            (&[Unknown, Waiting], Outcome::NotVerified),
            (&[Fail, Waiting], Outcome::Blocked),
            (&[Pass, Pass], Outcome::Done),
        ];
        for (statuses, want) in cases {
            let lines: Vec<Line> = statuses.iter().copied().map(line).collect();
            assert_eq!(decide(&lines), want, "{statuses:?}");
        }
    }

    fn report_of(items: Vec<Unverified>, today: Date) -> Report {
        Report {
            head: String::new(),
            lines: vec![Line {
                kind: Kind::Unverified,
                status: unverified_status(Some(&items), today).0,
                detail: unverified_status(Some(&items), today).1,
            }],
            unverified: items,
            found: Findings::default(),
        }
    }

    #[test]
    fn a_report_stops_waiting_on_the_day_it_is_due() {
        let report =
            report_of(vec![Unverified::until("live list", day("2026-10-28"))], day("2026-10-27"));
        assert_eq!(report.outcome(), Outcome::Waiting);
        assert_eq!(report.outcome_on(day("2026-10-27")), Outcome::Waiting);
        assert_eq!(report.outcome_on(day("2026-10-28")), Outcome::NotVerified);
        assert_eq!(report.outcome_on(day("2026-11-30")), Outcome::NotVerified);
        let checked_later = report.on(day("2026-10-28"));
        assert_eq!(checked_later.lines[0].detail, "1 item the agent did not verify");
    }

    fn ledger(not_verified: &str, scheduled: &str, status: &str) -> Ledger {
        Ledger::parse(&format!(
            "# Goal\nx\n\n## Status\n{status}\n\n## Not verified\n{not_verified}\n\n## Scheduled\n{scheduled}\n"
        ))
    }

    const OWNERS_SCHEDULE: &str = "- [ ] 2026-10-28 -- Tomorrow (Oct 28), check that a test account's list picked up yesterday's watch -- needs a day to pass";

    #[test]
    fn the_owners_ledger_waits_for_tomorrow() {
        let parsed = ledger("- Live list across a day boundary", OWNERS_SCHEDULE, "state: done");
        assert_eq!(
            unverified_of(&parsed),
            [Unverified::until("Live list across a day boundary", day("2026-10-28"))]
        );
    }

    #[test]
    fn an_item_matches_a_scheduled_one_by_a_shared_topic_word_or_a_sub_step() {
        let cases = [
            ("- the Safari layout", "- [ ] 2026-10-28 -- run the nightly import", None),
            ("- Live list", "- [ ] 2026-10-28 -- Check the lists", Some("2026-10-28")),
            ("- Live list", "- [x] 2026-10-28 -- check the list", None),
            ("- Live list", "- [-] 2026-10-28 -- check the list", None),
            ("- Live list", "- [ ] on: the deploy -- check the list", None),
            ("- Live list", "- [ ] ~2026-11-03 -- check the list", Some("2026-11-03")),
            ("- Live list", "- [ ] you -- check the list", None),
            (
                "- the migration on staging",
                "- [ ] 2026-10-29 -- recheck\n  - [ ] staging row counts",
                Some("2026-10-29"),
            ),
            ("- Check it again tomorrow", "- [ ] 2026-10-28 -- check it tomorrow", None),
            (
                "- the Safari layout and the list",
                "- [ ] 2026-11-05 -- the list\n- [ ] 2026-11-02 -- the Safari layout, the list",
                Some("2026-11-02"),
            ),
            (
                "- Live list",
                "- [ ] 2026-11-05 -- the list\n- [ ] 2026-11-02 -- the list",
                Some("2026-11-02"),
            ),
        ];
        for (not_verified, scheduled, want) in cases {
            let items = unverified_of(&ledger(not_verified, scheduled, "state: done"));
            assert_eq!(items.len(), 1, "{not_verified}");
            assert_eq!(items[0].until, want.map(day), "{not_verified} vs {scheduled}");
        }
    }

    #[test]
    fn a_waiting_ledger_with_a_date_holds_every_item_unless_a_scheduled_one_is_sooner() {
        let status = "state: waiting";
        let none = Ledger::parse(
            "# Goal\nx\n\n## Status\nstate: waiting\n\n## Not verified\n- a thing\n- another\n",
        );
        assert_eq!(unverified_of(&none), [Unverified::new("a thing"), Unverified::new("another")]);
        let mut waiting = ledger("- a thing\n- another", "- [ ] 2026-10-29 -- another", status);
        waiting.waiting_on = Some(WaitingOn::Date(day("2026-11-01")));
        assert_eq!(
            unverified_of(&waiting),
            [
                Unverified::until("a thing", day("2026-11-01")),
                Unverified::until("another", day("2026-10-29")),
            ]
        );
        let mut done = ledger("- a thing", "", "state: done");
        done.waiting_on = Some(WaitingOn::Date(day("2026-11-01")));
        assert_eq!(unverified_of(&done), [Unverified::new("a thing")]);
        let mut event = ledger("- a thing", "", status);
        event.waiting_on = Some(WaitingOn::Event("the deploy".to_owned()));
        assert_eq!(unverified_of(&event), [Unverified::new("a thing")]);
    }

    #[test]
    fn placeholder_not_verified_entries_are_left_out() {
        let items = unverified_of(&ledger("- nothing", "", "state: done"));
        assert_eq!(items, Vec::<Unverified>::new());
    }

    #[test]
    fn the_fix_note_names_every_failed_check_and_only_those() {
        let line =
            |status, detail: &str| Line { kind: Kind::Pushed, status, detail: detail.to_owned() };
        let report = Report {
            head: String::new(),
            lines: vec![
                line(Status::Fail, "2 commits not pushed"),
                line(Status::Unknown, "checks still running"),
                line(Status::Fail, "PR #42 checks failing: build"),
                line(Status::Pass, "fine"),
            ],
            unverified: Vec::new(),
            found: Findings::default(),
        };
        assert_eq!(
            report.fix_note(Workflow::default()).as_deref(),
            Some(
                "The done check found: 2 commits not pushed; PR #42 checks failing: build. Please fix that and tell me when it is done."
            )
        );
        let clean = Report { lines: vec![line(Status::Unknown, "x")], ..report };
        assert_eq!(clean.fix_note(Workflow::default()), None);
    }
}
