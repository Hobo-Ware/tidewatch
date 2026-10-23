//! The done check: four checks on an agent that says it is done (committed, pushed, pull request,
//! ledger), each a line with a status, and one outcome over them. Blocks on git and gh.

use std::path::Path;

use crate::delivery::{compare_ref, detect_base};
use crate::pr_gh::unpushed;
use crate::pr_status::{
    FIELDS, Outcome as CheckRun, Phase, Snapshot, State, parse_snapshot, phase,
};
use crate::tools::{GhError, Tools, finish, run_gh};

const REMOTE: &str = "origin";
const NAMED_CHECKS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Committed,
    Pushed,
    PullRequest,
    Unverified,
}

impl Kind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Committed => "Committed",
            Self::Pushed => "Pushed",
            Self::PullRequest => "Pull request",
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
}

/// Blocked when any check failed, else Not verified when any could not run, else Done.
#[must_use]
pub fn decide(lines: &[Line]) -> Outcome {
    if lines.iter().any(|line| line.status == Status::Fail) {
        Outcome::Blocked
    } else if lines.iter().any(|line| line.status == Status::Unknown) {
        Outcome::NotVerified
    } else {
        Outcome::Done
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The commit the checks looked at.
    pub head: String,
    pub lines: Vec<Line>,
    /// The items the agent's ledger lists under Not verified.
    pub unverified: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn outcome(&self) -> Outcome {
        decide(&self.lines)
    }

    /// The short note that asks the agent to fix what failed. `None` when nothing failed.
    #[must_use]
    pub fn fix_note(&self) -> Option<String> {
        let failed: Vec<String> = self
            .lines
            .iter()
            .filter(|line| line.status == Status::Fail)
            .map(|line| line.detail.clone())
            .collect();
        (!failed.is_empty()).then(|| {
            format!(
                "The done check found: {}. Please fix that and tell me when it is done.",
                failed.join("; ")
            )
        })
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
    pub unverified: Option<Vec<String>>,
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

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
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
pub fn unverified_status(items: Option<&[String]>) -> (Status, String) {
    match items {
        None => (Status::Skipped, "the agent keeps no progress ledger".to_owned()),
        Some([]) => (Status::Pass, "nothing left unverified".to_owned()),
        Some(items) => (
            Status::Unknown,
            format!("{} the agent did not verify", plural(items.len(), "item", "items")),
        ),
    }
}

fn git_stdout(tools: &Tools, dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tools.git(dir);
    command.args(args);
    let out = finish(command).map_err(|e| format!("cannot run git: {e}"))?;
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

/// Runs the four checks in `input.dir`.
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
    let unverified = input.unverified.clone().unwrap_or_default();
    let lines = vec![
        Line::new(Kind::Committed, committed),
        Line::new(Kind::Pushed, pushed),
        Line::new(Kind::PullRequest, pull_request_line(&pr, &head)),
        Line::new(Kind::Unverified, unverified_status(input.unverified.as_deref())),
    ];
    Report { head, lines, unverified }
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

    #[test]
    fn the_ledger_line_lists_unverified_items_and_skips_a_tab_without_a_ledger() {
        let items = vec!["the migration on staging".to_owned()];
        let two = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(unverified_status(None).0, Status::Skipped);
        assert_eq!(
            unverified_status(Some(&[])),
            (Status::Pass, "nothing left unverified".to_owned())
        );
        assert_eq!(
            unverified_status(Some(&items)),
            (Status::Unknown, "1 item the agent did not verify".to_owned())
        );
        assert_eq!(
            unverified_status(Some(&two)),
            (Status::Unknown, "2 items the agent did not verify".to_owned())
        );
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
        };
        assert_eq!(
            report.fix_note().as_deref(),
            Some(
                "The done check found: 2 commits not pushed; PR #42 checks failing: build. Please fix that and tell me when it is done."
            )
        );
        let clean = Report { lines: vec![line(Status::Unknown, "x")], ..report };
        assert_eq!(clean.fix_note(), None);
    }
}
