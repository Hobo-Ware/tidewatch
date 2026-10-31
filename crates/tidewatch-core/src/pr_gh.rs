//! The `gh` and `git` calls behind the pull request cards (PLAN.md, D4 follow-up). They block: call
//! them off the UI thread.

use std::collections::HashMap;
use std::path::Path;

use crate::pr::{self, PullRequest};
use crate::pr_log;
use crate::pr_status::{
    Body, Event, FIELDS, FIELDS_FIRST, Job, ReviewComment, Seen, Snapshot, State, Summary, diff,
    parse_snapshot, phase, review_comments,
};
use crate::tools::{GhError, Tools, run_gh};

/// The jobs whose logs are read for one failure card; a matrix of twenty fails as one card.
const LOGGED_JOBS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Polled {
    pub events: Vec<Event>,
    pub seen: Seen,
    pub summary: Summary,
    pub head: String,
    pub checks_pending: bool,
    pub ended: bool,
}

pub fn poll(
    tools: &Tools,
    dir: &Path,
    number: u64,
    prev: Option<&Seen>,
    now_ms: i64,
) -> Result<Polled, GhError> {
    let fields = if prev.is_some_and(|seen| seen.opened) { FIELDS } else { FIELDS_FIRST };
    let json = run_gh(tools, dir, &["pr", "view", &number.to_string(), "--json", fields])?;
    let snapshot = parse_snapshot(&json).map_err(GhError::Failed)?;
    let (mut events, seen) = diff(prev, &snapshot, now_ms);
    for event in &mut events {
        match &mut event.body {
            Body::ChecksFailed { jobs, .. } => read_failures(tools, dir, jobs),
            Body::ChangesRequested { author, comments, .. } => {
                let (author, at) = (author.clone(), event.at_ms);
                *comments = inline_comments(tools, dir, &snapshot, &author, at);
            }
            _ => {}
        }
    }
    Ok(Polled {
        events,
        seen,
        summary: snapshot.summary(),
        head: snapshot.head.clone(),
        checks_pending: phase(&snapshot.checks).running() && snapshot.state == State::Open,
        ended: snapshot.state != State::Open,
    })
}

fn read_failures(tools: &Tools, dir: &Path, jobs: &mut [Job]) {
    let mut steps: HashMap<u64, HashMap<u64, String>> = HashMap::new();
    for job in jobs.iter_mut().filter(|job| !job.passed).take(LOGGED_JOBS) {
        let (Some(run), Some(id)) = (job.run_id, job.job_id) else { continue };
        let known = steps.entry(run).or_insert_with(|| failed_steps(tools, dir, run));
        job.step = known.get(&id).cloned();
        job.log = failed_log(tools, dir, run, id).map(|raw| pr_log::tail(&raw)).unwrap_or_default();
    }
}

fn inline_comments(
    tools: &Tools,
    dir: &Path,
    snapshot: &Snapshot,
    author: &str,
    at_ms: i64,
) -> Vec<ReviewComment> {
    let since = snapshot
        .reviews
        .iter()
        .filter(|review| review.author == author && review.at_ms < at_ms)
        .map(|review| review.at_ms)
        .max()
        .unwrap_or(0);
    let pull = PullRequest {
        number: snapshot.number,
        url: snapshot.url.clone(),
        head_sha: snapshot.head.clone(),
    };
    pr::fetch_threads(tools, dir, &pull)
        .map(|threads| review_comments(&threads, author, since, at_ms))
        .unwrap_or_default()
}

fn failed_steps(tools: &Tools, dir: &Path, run: u64) -> HashMap<u64, String> {
    let Ok(json) = run_gh(tools, dir, &["run", "view", &run.to_string(), "--json", "jobs"]) else {
        return HashMap::new();
    };
    parse_failed_steps(&json)
}

#[must_use]
pub fn parse_failed_steps(json: &str) -> HashMap<u64, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return HashMap::new();
    };
    value["jobs"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|job| {
            let id = job["databaseId"].as_u64()?;
            let step = job["steps"]
                .as_array()?
                .iter()
                .find(|step| step["conclusion"].as_str() == Some("failure"))?;
            Some((id, step["name"].as_str()?.to_owned()))
        })
        .collect()
}

/// The raw log of the failed steps of one job. GitHub drops old logs: that is an error here, and
/// the card goes without them.
pub fn failed_log(tools: &Tools, dir: &Path, run: u64, job: u64) -> Result<String, GhError> {
    run_gh(
        tools,
        dir,
        &["run", "view", &run.to_string(), "--job", &job.to_string(), "--log-failed"],
    )
}

/// Re-runs the failed jobs of a run. Writes to GitHub: only after the user pressed Re-run.
pub fn rerun_failed(tools: &Tools, dir: &Path, run: u64) -> Result<(), GhError> {
    run_gh(tools, dir, &["run", "rerun", &run.to_string(), "--failed"]).map(|_| ())
}

#[must_use]
pub fn unpushed(tools: &Tools, dir: &Path, remote_head: &str) -> Option<(String, usize)> {
    if remote_head.is_empty() || remote_head.starts_with('-') {
        return None;
    }
    let git = |args: &[&str]| {
        let mut command = tools.git(dir);
        command.args(args);
        tools.finish(command).ok().filter(|out| out.success).map(|out| out.stdout.trim().to_owned())
    };
    let head = git(&["rev-parse", "HEAD"])?;
    let range = format!("{remote_head}..HEAD");
    let count = git(&["rev-list", "--count", &range])?.parse().ok()?;
    Some((head, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOBS: &str = include_str!("../tests/fixtures/gh_run_jobs.json");

    #[test]
    fn the_failed_step_of_each_job_is_named_by_the_job_id() {
        let steps = parse_failed_steps(JOBS);
        assert_eq!(
            steps.get(&116_158_421_158).map(String::as_str),
            Some("Run cargo clippy --workspace --all-targets -- -D warnings")
        );
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn bad_job_json_names_nothing() {
        for json in ["", "not json", "{}", r#"{"jobs":[{"databaseId":1}]}"#] {
            assert_eq!(parse_failed_steps(json), HashMap::new(), "{json:?}");
        }
    }
}
