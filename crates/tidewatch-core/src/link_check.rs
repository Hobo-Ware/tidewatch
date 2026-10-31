//! Looking up a pull request or an issue a tab holds a link to, with `gh pr view <url> --json ...`
//! or `gh issue view <url> --json ...`: its state and what goes with it (checks, review, conflicts,
//! draft, close reason, comments, last update), or why GitHub does not answer. It blocks until `gh`
//! exits: call it from a background thread, never the UI thread.

use std::path::Path;

use serde_json::Value;

use crate::link_board::{Access, Checks, Kind, LinkState, Reason, Review, Status, kind_of};
use crate::pr_status::{Decision, State, Summary, SummaryChecks, parse_snapshot};
use crate::rfc3339;
use crate::tools::{GhError, Tools, run_gh};

const PR_FIELDS: &str = "number,state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,comments,isDraft,updatedAt";
const ISSUE_FIELDS: &str = "state,stateReason,comments,updatedAt";

/// What a look at a link found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found {
        state: LinkState,
        status: Status,
    },
    /// GitHub does not show the repository to this account.
    NoAccess,
    /// The pull request or issue does not exist (any more).
    Gone,
    /// `gh` is missing, signed out, stopped or said something else: nothing is known.
    Failed,
}

/// The pull request or issue at `url`, looked up at `now_ms` from `dir`. A link of any other kind
/// is `Failed` without a call.
#[must_use]
pub fn lookup(tools: &Tools, dir: &Path, url: &str, now_ms: i64) -> Lookup {
    let (noun, fields) = match kind_of(url) {
        Kind::Pr => ("pr", PR_FIELDS),
        Kind::Issue => ("issue", ISSUE_FIELDS),
        _ => return Lookup::Failed,
    };
    match run_gh(tools, dir, &[noun, "view", url, "--json", fields]) {
        Ok(json) => {
            let found =
                if noun == "pr" { parse_pr(&json, now_ms) } else { parse_issue(&json, now_ms) };
            found.map_or(Lookup::Failed, |(state, status)| Lookup::Found { state, status })
        }
        Err(error) => sort_failure(&error),
    }
}

/// Which way a failed call went: a repository `gh` cannot read, a pull request or issue that is not
/// there, or nothing learned.
#[must_use]
pub fn sort_failure(error: &GhError) -> Lookup {
    let GhError::Failed(message) = error else { return Lookup::Failed };
    let text = message.to_lowercase();
    if text.contains("could not resolve to a pullrequest")
        || text.contains("could not resolve to an issue")
        || text.contains("no pull requests found")
    {
        Lookup::Gone
    } else if text.contains("could not resolve to a repository")
        || text.contains("http 404")
        || text.contains("http 403")
        || text.contains("not found")
    {
        Lookup::NoAccess
    } else {
        Lookup::Failed
    }
}

/// The access a failed look settles on, or `None` while nothing is known.
#[must_use]
pub fn access_of(lookup: &Lookup) -> Option<Access> {
    match lookup {
        Lookup::NoAccess => Some(Access::NoAccess),
        Lookup::Gone => Some(Access::Gone),
        Lookup::Found { .. } | Lookup::Failed => None,
    }
}

fn stamp(value: &Value) -> Option<i64> {
    value["updatedAt"].as_str().and_then(rfc3339::unix_millis)
}

/// The state and status in `gh pr view --json number,state,...` output.
#[must_use]
pub fn parse_pr(json: &str, now_ms: i64) -> Option<(LinkState, Status)> {
    let snapshot = parse_snapshot(json).ok()?;
    let value: Value = serde_json::from_str(json).ok()?;
    let status = status_of(&snapshot.summary(), snapshot.draft, stamp(&value), now_ms);
    Some((state_of(snapshot.state), status))
}

/// The state and status in `gh issue view --json state,stateReason,...` output.
#[must_use]
pub fn parse_issue(json: &str, now_ms: i64) -> Option<(LinkState, Status)> {
    let value: Value = serde_json::from_str(json).ok()?;
    let state = LinkState::from_gh(value["state"].as_str()?);
    if state.is_unknown() {
        return None;
    }
    let reason = match value["stateReason"].as_str() {
        Some("COMPLETED") if state == LinkState::Closed => Some(Reason::Completed),
        Some("NOT_PLANNED" | "DUPLICATE") if state == LinkState::Closed => Some(Reason::NotPlanned),
        _ => None,
    };
    let comments =
        value["comments"].as_array().map(|list| u32::try_from(list.len()).unwrap_or(u32::MAX));
    Some((
        state,
        Status {
            reason,
            comments,
            updated_ms: stamp(&value),
            checked_ms: Some(now_ms),
            ..Status::default()
        },
    ))
}

/// The status of a pull request from the summary the watch or a look made. Checks, review and
/// conflicts only mean something while the pull request is open.
#[must_use]
pub fn status_of(summary: &Summary, draft: bool, updated_ms: Option<i64>, now_ms: i64) -> Status {
    let open = summary.state == State::Open;
    Status {
        draft: open && draft,
        checks: open
            .then_some(match summary.checks {
                SummaryChecks::None => None,
                SummaryChecks::Running => Some(Checks::Running),
                SummaryChecks::Failed => Some(Checks::Fail),
                SummaryChecks::Passed => Some(Checks::Pass),
            })
            .flatten(),
        review: open
            .then_some(match summary.decision {
                Decision::Approved => Some(Review::Approved),
                Decision::ChangesRequested => Some(Review::ChangesRequested),
                Decision::None | Decision::ReviewRequired => None,
            })
            .flatten(),
        conflicts: open && summary.conflicting,
        comments: Some(u32::try_from(summary.comments).unwrap_or(u32::MAX)),
        updated_ms,
        checked_ms: Some(now_ms),
        ..Status::default()
    }
}

#[must_use]
pub fn state_of(state: State) -> LinkState {
    match state {
        State::Open => LinkState::Open,
        State::Merged => LinkState::Merged,
        State::Closed => LinkState::Closed,
    }
}

#[cfg(test)]
mod tests;
