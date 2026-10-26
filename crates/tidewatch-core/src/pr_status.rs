//! A pull request as GitHub shows it right now, and what changed since the last look (PLAN.md, D4
//! follow-up). Pure: the `gh` calls are in `pr_gh`, the persisted timeline in `pr_cards`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pr::PrThread;
use crate::pr_noise::is_noise;
use crate::pr_trust::is_trusted;
use crate::rfc3339;

pub const FIELDS: &str = "number,title,url,state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,reviews,comments,headRefOid,baseRefName,headRefName,author,createdAt,mergedAt,closedAt";
pub const FIELDS_FIRST: &str = "number,title,url,state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,reviews,comments,headRefOid,baseRefName,headRefName,author,createdAt,mergedAt,closedAt,commits";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    None,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub workflow: String,
    pub outcome: Outcome,
    pub url: String,
    pub run_id: Option<u64>,
    pub job_id: Option<u64>,
    pub started_ms: Option<i64>,
    pub finished_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub id: String,
    pub author: String,
    pub association: String,
    pub state: ReviewState,
    pub body: String,
    pub at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub association: String,
    pub body: String,
    pub url: String,
    pub at_ms: i64,
    pub own: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: State,
    pub conflicting: bool,
    pub decision: Decision,
    pub head: String,
    pub base: String,
    pub branch: String,
    pub author: String,
    pub checks: Vec<Check>,
    pub reviews: Vec<Review>,
    pub comments: Vec<Comment>,
    pub commits: Option<usize>,
    pub created_ms: Option<i64>,
    pub ended_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    NoChecks,
    Running { done: usize, total: usize },
    Failed,
    Passed,
}

impl Phase {
    #[must_use]
    pub fn running(self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

#[must_use]
pub fn phase(checks: &[Check]) -> Phase {
    let counted: Vec<&Check> = checks.iter().filter(|c| c.outcome != Outcome::Skipped).collect();
    if counted.is_empty() {
        return Phase::NoChecks;
    }
    let pending = counted.iter().filter(|c| c.outcome == Outcome::Pending).count();
    if pending > 0 {
        return Phase::Running { done: counted.len() - pending, total: counted.len() };
    }
    if counted.iter().any(|c| c.outcome == Outcome::Failed) { Phase::Failed } else { Phase::Passed }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub secs: Option<i64>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub run_id: Option<u64>,
    #[serde(default)]
    pub job_id: Option<u64>,
    #[serde(default)]
    pub log: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    You,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Opened {
        number: u64,
        title: String,
        base: String,
        branch: String,
        #[serde(default)]
        commits: Option<usize>,
    },
    ChecksRunning {
        head: String,
        done: usize,
        total: usize,
    },
    ChecksFailed {
        head: String,
        jobs: Vec<Job>,
    },
    ChecksPassed {
        total: usize,
        #[serde(default)]
        secs: Option<i64>,
    },
    ChangesRequested {
        author: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        comments: Vec<ReviewComment>,
        #[serde(default = "trusted_by_default")]
        trusted: bool,
    },
    Commented {
        author: String,
        text: String,
        #[serde(default)]
        url: String,
        #[serde(default = "trusted_by_default")]
        trusted: bool,
    },
    Conflict {
        base: String,
    },
    Approved {
        author: String,
    },
    Merged {
        number: u64,
        base: String,
    },
    Closed {
        number: u64,
    },
    PushFix {
        commits: usize,
        head: String,
    },
    Sent {
        via: Via,
        tag: String,
        headline: String,
        extra_lines: usize,
        text: String,
    },
}

fn trusted_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub at_ms: i64,
    pub body: Body,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    pub opened: bool,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub phase: SeenPhase,
    #[serde(default)]
    pub done: usize,
    #[serde(default)]
    pub conflicting: bool,
    #[serde(default)]
    pub ended: bool,
    #[serde(default)]
    pub reviews: BTreeSet<String>,
    #[serde(default)]
    pub comments: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeenPhase {
    #[default]
    None,
    Running,
    Failed,
    Passed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: State,
    pub checks: SummaryChecks,
    pub comments: usize,
    pub decision: Decision,
    pub conflicting: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryChecks {
    None,
    Running,
    Failed,
    Passed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Bad,
    Warn,
    Good,
    Quiet,
}

impl Summary {
    #[must_use]
    pub fn headline(&self) -> Option<(Tone, &'static str)> {
        if self.state != State::Open {
            return None;
        }
        Some(match (self.checks, self.decision, self.conflicting) {
            (SummaryChecks::Failed, ..) => (Tone::Bad, "checks failed"),
            (_, Decision::ChangesRequested, _) => (Tone::Warn, "changes requested"),
            (_, _, true) => (Tone::Warn, "conflicts"),
            (_, Decision::Approved, _) => (Tone::Good, "approved"),
            (SummaryChecks::Running, ..) => (Tone::Warn, "checks running"),
            (SummaryChecks::Passed, ..) => (Tone::Good, "checks passed"),
            (SummaryChecks::None, ..) => (Tone::Quiet, "open"),
        })
    }
}

impl Snapshot {
    #[must_use]
    pub fn summary(&self) -> Summary {
        let checks = match phase(&self.checks) {
            Phase::NoChecks => SummaryChecks::None,
            Phase::Running { .. } => SummaryChecks::Running,
            Phase::Failed => SummaryChecks::Failed,
            Phase::Passed => SummaryChecks::Passed,
        };
        Summary {
            number: self.number,
            title: self.title.clone(),
            url: self.url.clone(),
            state: self.state,
            checks,
            comments: self
                .comments
                .iter()
                .filter(|c| !c.own && !is_noise(&c.author, &c.body))
                .count(),
            decision: self.decision,
            conflicting: self.conflicting,
        }
    }
}

/// Field names and values were checked against `gh` 2.98.0 on public repositories: `statusCheckRollup`
/// holds `CheckRun` items (`status`, `conclusion`, `detailsUrl` with the run and job ids) and
/// `StatusContext` items (`context`, `state`, `targetUrl`); a workflow that runs for both a push and a
/// pull request appears twice, so the latest run of each check name wins. `comments` are issue
/// comments with `viewerDidAuthor`; `reviews` carry `state`, `body` and `submittedAt`.
pub fn parse_snapshot(json: &str) -> Result<Snapshot, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("GitHub sent an unreadable answer: {e}"))?;
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    let state = match value["state"].as_str() {
        Some("OPEN") => State::Open,
        Some("MERGED") => State::Merged,
        Some("CLOSED") => State::Closed,
        other => return Err(format!("unknown pull request state {other:?}")),
    };
    let Some(number) = value["number"].as_u64() else {
        return Err("GitHub's answer has no pull request number".to_owned());
    };
    let stamp = |key: &str| value[key].as_str().and_then(real_time);
    Ok(Snapshot {
        number,
        title: text("title"),
        url: text("url"),
        state,
        conflicting: value["mergeable"].as_str() == Some("CONFLICTING")
            || value["mergeStateStatus"].as_str() == Some("DIRTY"),
        decision: match value["reviewDecision"].as_str() {
            Some("APPROVED") => Decision::Approved,
            Some("CHANGES_REQUESTED") => Decision::ChangesRequested,
            Some("REVIEW_REQUIRED") => Decision::ReviewRequired,
            _ => Decision::None,
        },
        head: text("headRefOid"),
        base: text("baseRefName"),
        branch: text("headRefName"),
        author: value["author"]["login"].as_str().unwrap_or_default().to_owned(),
        checks: latest_checks(value["statusCheckRollup"].as_array().map_or(&[][..], Vec::as_slice)),
        reviews: value["reviews"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(review)
            .collect(),
        comments: value["comments"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(comment)
            .collect(),
        commits: value["commits"].as_array().map(Vec::len),
        created_ms: stamp("createdAt"),
        ended_ms: stamp("mergedAt").or_else(|| stamp("closedAt")),
    })
}

/// GitHub writes `0001-01-01T00:00:00Z` for "not yet".
fn real_time(text: &str) -> Option<i64> {
    rfc3339::unix_millis(text).filter(|ms| *ms > 0)
}

fn review(value: &Value) -> Option<Review> {
    Some(Review {
        id: value["id"].as_str()?.to_owned(),
        author: value["author"]["login"].as_str().unwrap_or_default().to_owned(),
        association: value["authorAssociation"].as_str().unwrap_or_default().to_owned(),
        state: match value["state"].as_str() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            _ => ReviewState::Other,
        },
        body: value["body"].as_str().unwrap_or_default().to_owned(),
        at_ms: value["submittedAt"].as_str().and_then(real_time).unwrap_or_default(),
    })
}

fn comment(value: &Value) -> Option<Comment> {
    Some(Comment {
        id: value["id"].as_str()?.to_owned(),
        author: value["author"]["login"].as_str().unwrap_or_default().to_owned(),
        association: value["authorAssociation"].as_str().unwrap_or_default().to_owned(),
        body: value["body"].as_str().unwrap_or_default().to_owned(),
        url: value["url"].as_str().unwrap_or_default().to_owned(),
        at_ms: value["createdAt"].as_str().and_then(real_time).unwrap_or_default(),
        own: value["viewerDidAuthor"].as_bool().unwrap_or(false),
    })
}

fn latest_checks(items: &[Value]) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();
    for check in items.iter().filter_map(check) {
        match checks.iter_mut().find(|c| c.name == check.name && c.workflow == check.workflow) {
            Some(known) if check.started_ms >= known.started_ms => *known = check,
            Some(_) => {}
            None => checks.push(check),
        }
    }
    checks
}

fn check(value: &Value) -> Option<Check> {
    let stamp = |key: &str| value[key].as_str().and_then(real_time);
    let url = value["detailsUrl"].as_str().or_else(|| value["targetUrl"].as_str()).unwrap_or("");
    let (run_id, job_id) = ids_of(url);
    match value["__typename"].as_str()? {
        "CheckRun" => Some(Check {
            name: value["name"].as_str()?.to_owned(),
            workflow: value["workflowName"].as_str().unwrap_or_default().to_owned(),
            outcome: run_outcome(
                value["status"].as_str().unwrap_or_default(),
                value["conclusion"].as_str().unwrap_or_default(),
            ),
            url: url.to_owned(),
            run_id,
            job_id,
            started_ms: stamp("startedAt"),
            finished_ms: stamp("completedAt"),
        }),
        "StatusContext" => Some(Check {
            name: value["context"].as_str()?.to_owned(),
            workflow: String::new(),
            outcome: match value["state"].as_str().unwrap_or_default() {
                "SUCCESS" => Outcome::Passed,
                "FAILURE" | "ERROR" => Outcome::Failed,
                _ => Outcome::Pending,
            },
            url: url.to_owned(),
            run_id,
            job_id,
            started_ms: stamp("startedAt"),
            finished_ms: None,
        }),
        _ => None,
    }
}

fn run_outcome(status: &str, conclusion: &str) -> Outcome {
    if status != "COMPLETED" {
        return Outcome::Pending;
    }
    match conclusion {
        "SUCCESS" => Outcome::Passed,
        "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "STALE" => Outcome::Failed,
        _ => Outcome::Skipped,
    }
}

/// `https://github.com/o/r/actions/runs/<run>/job/<job>`.
fn ids_of(url: &str) -> (Option<u64>, Option<u64>) {
    let mut parts = url.split('/');
    let mut run = None;
    let mut job = None;
    while let Some(part) = parts.next() {
        match part {
            "runs" => run = parts.next().and_then(|id| id.parse().ok()),
            "job" => job = parts.next().and_then(|id| id.parse().ok()),
            _ => {}
        }
    }
    (run, job)
}

fn secs_between(from: Option<i64>, to: Option<i64>) -> Option<i64> {
    Some(((to? - from?) / 1000).max(0))
}

#[must_use]
pub fn diff(prev: Option<&Seen>, snap: &Snapshot, now_ms: i64) -> (Vec<Event>, Seen) {
    let first = prev.is_none_or(|p| !p.opened);
    let empty = Seen::default();
    let before = prev.unwrap_or(&empty);
    let mut events = Vec::new();
    if first {
        events.push(Event {
            at_ms: snap.created_ms.unwrap_or(now_ms),
            body: Body::Opened {
                number: snap.number,
                title: snap.title.clone(),
                base: snap.base.clone(),
                branch: snap.branch.clone(),
                commits: snap.commits,
            },
        });
    }
    let mut seen = Seen {
        opened: true,
        head: snap.head.clone(),
        phase: before.phase,
        done: before.done,
        conflicting: snap.conflicting,
        ended: snap.state != State::Open,
        reviews: snap.reviews.iter().map(|r| r.id.clone()).collect(),
        comments: snap.comments.iter().map(|c| c.id.clone()).collect(),
    };
    let ended_at = snap.ended_ms.unwrap_or(now_ms);
    match snap.state {
        State::Merged if !before.ended => events.push(Event {
            at_ms: ended_at,
            body: Body::Merged { number: snap.number, base: snap.base.clone() },
        }),
        State::Closed if !before.ended => {
            events.push(Event { at_ms: ended_at, body: Body::Closed { number: snap.number } });
        }
        State::Open | State::Merged | State::Closed => {}
    }
    if snap.state != State::Open {
        return (events, seen);
    }
    events.extend(check_events(before, first, snap, now_ms, &mut seen));
    if snap.conflicting && !before.conflicting {
        events.push(Event { at_ms: now_ms, body: Body::Conflict { base: snap.base.clone() } });
    }
    events.extend(talk_events(before, first, snap));
    events.sort_by_key(|event| event.at_ms);
    (events, seen)
}

fn check_events(
    before: &Seen,
    first: bool,
    snap: &Snapshot,
    now_ms: i64,
    seen: &mut Seen,
) -> Vec<Event> {
    let head_moved = !first && before.head != snap.head;
    let latest_finish = snap.checks.iter().filter_map(|c| c.finished_ms).max();
    let earliest_start = snap.checks.iter().filter_map(|c| c.started_ms).min();
    match phase(&snap.checks) {
        Phase::NoChecks => {
            seen.phase = SeenPhase::None;
            Vec::new()
        }
        Phase::Running { done, total } => {
            seen.phase = SeenPhase::Running;
            seen.done = done;
            let changed = before.phase != SeenPhase::Running || head_moved || before.done != done;
            if !changed {
                return Vec::new();
            }
            vec![Event {
                at_ms: now_ms,
                body: Body::ChecksRunning { head: snap.head.clone(), done, total },
            }]
        }
        Phase::Failed => {
            seen.phase = SeenPhase::Failed;
            if before.phase == SeenPhase::Failed && !head_moved {
                return Vec::new();
            }
            let jobs = snap
                .checks
                .iter()
                .filter(|c| matches!(c.outcome, Outcome::Passed | Outcome::Failed))
                .map(|c| Job {
                    name: c.name.clone(),
                    passed: c.outcome == Outcome::Passed,
                    step: None,
                    secs: secs_between(c.started_ms, c.finished_ms),
                    url: c.url.clone(),
                    run_id: c.run_id,
                    job_id: c.job_id,
                    log: Vec::new(),
                })
                .collect();
            vec![Event {
                at_ms: latest_finish.unwrap_or(now_ms),
                body: Body::ChecksFailed { head: snap.head.clone(), jobs },
            }]
        }
        Phase::Passed => {
            seen.phase = SeenPhase::Passed;
            if before.phase == SeenPhase::Passed && !head_moved {
                return Vec::new();
            }
            let total = snap.checks.iter().filter(|c| c.outcome == Outcome::Passed).count();
            vec![Event {
                at_ms: latest_finish.unwrap_or(now_ms),
                body: Body::ChecksPassed {
                    total,
                    secs: secs_between(earliest_start, latest_finish),
                },
            }]
        }
    }
}

/// Newest comments shown on a first look, so an old pull request does not open with a wall of cards.
const FIRST_LOOK_COMMENTS: usize = 3;

fn talk_events(before: &Seen, first: bool, snap: &Snapshot) -> Vec<Event> {
    let mut events = Vec::new();
    let new_reviews: Vec<&Review> = snap
        .reviews
        .iter()
        .filter(|r| !before.reviews.contains(&r.id) && r.author != snap.author)
        .collect();
    let latest_request = new_reviews.iter().rev().find(|r| {
        r.state == ReviewState::ChangesRequested && snap.decision == Decision::ChangesRequested
    });
    let latest_approval = new_reviews
        .iter()
        .rev()
        .find(|r| r.state == ReviewState::Approved && snap.decision == Decision::Approved);
    for review in &new_reviews {
        let wanted = match review.state {
            ReviewState::ChangesRequested => {
                !first || latest_request.is_some_and(|l| l.id == review.id)
            }
            ReviewState::Approved => !first || latest_approval.is_some_and(|l| l.id == review.id),
            ReviewState::Commented => {
                !first && !review.body.trim().is_empty() && !is_noise(&review.author, &review.body)
            }
            ReviewState::Other => false,
        };
        if !wanted {
            continue;
        }
        let body = match review.state {
            ReviewState::ChangesRequested => Body::ChangesRequested {
                author: review.author.clone(),
                body: review.body.clone(),
                comments: Vec::new(),
                trusted: is_trusted(&review.author, &review.association),
            },
            ReviewState::Approved => Body::Approved { author: review.author.clone() },
            ReviewState::Commented | ReviewState::Other => Body::Commented {
                author: review.author.clone(),
                text: review.body.clone(),
                url: String::new(),
                trusted: is_trusted(&review.author, &review.association),
            },
        };
        events.push(Event { at_ms: review.at_ms, body });
    }
    let new_comments: Vec<&Comment> = snap
        .comments
        .iter()
        .filter(|c| !before.comments.contains(&c.id) && !c.own && !is_noise(&c.author, &c.body))
        .collect();
    let skip = if first { new_comments.len().saturating_sub(FIRST_LOOK_COMMENTS) } else { 0 };
    for comment in new_comments.into_iter().skip(skip) {
        events.push(Event {
            at_ms: comment.at_ms,
            body: Body::Commented {
                author: comment.author.clone(),
                text: comment.body.clone(),
                url: comment.url.clone(),
                trusted: is_trusted(&comment.author, &comment.association),
            },
        });
    }
    events
}

/// The inline comments a review left: threads its author started between the author's previous
/// review (`since_ms`) and a minute after this one (`submitted_ms`), since GitHub's REST comments do
/// not name the review in `gh`'s answer.
#[must_use]
pub fn review_comments(
    threads: &[PrThread],
    author: &str,
    since_ms: i64,
    submitted_ms: i64,
) -> Vec<ReviewComment> {
    threads
        .iter()
        .filter_map(|thread| {
            let first = thread.comments.first()?;
            let at = first.time * 1000;
            let inside = at > since_ms && at <= submitted_ms + 60_000;
            (first.author == author && inside).then(|| ReviewComment {
                path: thread.path.clone(),
                line: match thread.placement {
                    crate::pr::Placement::Line(line) => Some(line),
                    crate::pr::Placement::Outdated(line) => line,
                    crate::pr::Placement::File => None,
                },
                text: first.body.trim().to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
