//! The tab's GitHub feed: one JSON line each time a pull request or issue the tab holds (its own
//! pull request and the links it saw) changes, appended to the file in `TIDEWATCH_EVENTS` so the
//! agent can watch it with `tail -F`. A line says what GitHub shows now, never what to do about it.
//! The changes come from webhook-driven lookups, the timed lookups and the pull request watch alike.

use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::link_board::{Access, Checks, Kind, Link, LinkState, Reason, Review};

pub const EVENTS_ENV: &str = "TIDEWATCH_EVENTS";

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Facts {
    state: LinkState,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    draft: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks: Option<Checks>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review: Option<Review>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    conflicts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comments: Option<u32>,
    #[serde(skip_serializing_if = "Access::is_readable")]
    access: Access,
}

#[derive(Serialize)]
struct Line<'a> {
    at_ms: i64,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    number: Option<u64>,
    url: &'a str,
    title: &'a str,
    #[serde(flatten)]
    facts: Facts,
}

fn facts(link: &Link) -> Option<Facts> {
    if !matches!(link.kind(), Kind::Pr | Kind::Issue) {
        return None;
    }
    let status = &link.status;
    let facts = Facts {
        state: link.state,
        draft: status.draft,
        checks: status.checks,
        review: status.review,
        conflicts: status.conflicts,
        reason: status.reason,
        comments: status.comments,
        access: status.access,
    };
    let nothing_known = facts.state.is_unknown() && facts.access.is_readable();
    (!nothing_known).then_some(facts)
}

#[must_use]
pub fn line(before: Option<&Link>, after: &Link, at_ms: i64) -> Option<String> {
    let now = facts(after)?;
    if before.and_then(facts).as_ref() == Some(&now) {
        return None;
    }
    let kind = if after.kind() == Kind::Pr { "pull_request" } else { "issue" };
    let line = Line {
        at_ms,
        kind,
        repo: after.repo(),
        number: after.number(),
        url: &after.url,
        title: &after.title,
        facts: now,
    };
    serde_json::to_string(&line).ok()
}

/// Appends `lines` to `file` in one write, so a reader never sees half a line.
pub fn append(file: &Path, lines: &[String]) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let text: String = lines.iter().flat_map(|line| [line.as_str(), "\n"]).collect();
    std::fs::OpenOptions::new().create(true).append(true).open(file)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_board::Status;

    fn pr(state: LinkState, status: Status) -> Link {
        Link {
            url: "https://github.com/hobo-ware/tidewatch/pull/42".into(),
            title: "Feed the agent".into(),
            note: None,
            first_seen_ms: 1,
            title_from: crate::link_board::TitleFrom::Seen,
            state,
            status,
        }
    }

    fn json(line: &str) -> serde_json::Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn a_pull_request_whose_checks_fail_gets_one_line_with_the_facts() {
        let before =
            pr(LinkState::Open, Status { checks: Some(Checks::Running), ..Status::default() });
        let after = pr(
            LinkState::Open,
            Status { checks: Some(Checks::Fail), comments: Some(2), ..Status::default() },
        );
        let line = line(Some(&before), &after, 7).unwrap();
        assert_eq!(
            json(&line),
            serde_json::json!({
                "at_ms": 7, "kind": "pull_request", "repo": "hobo-ware/tidewatch", "number": 42,
                "url": "https://github.com/hobo-ware/tidewatch/pull/42", "title": "Feed the agent",
                "state": "open", "checks": "fail", "comments": 2
            })
        );
    }

    #[test]
    fn only_a_change_in_what_github_shows_makes_a_line() {
        let open = Status { checks: Some(Checks::Pass), ..Status::default() };
        let rechecked = Status { checked_ms: Some(99), updated_ms: Some(98), ..open.clone() };
        let cases = [
            (Some(pr(LinkState::Open, open.clone())), pr(LinkState::Open, rechecked), false),
            (Some(pr(LinkState::Open, open.clone())), pr(LinkState::Merged, open.clone()), true),
            (None, pr(LinkState::Open, open.clone()), true),
            (None, pr(LinkState::Unknown, Status::default()), false),
            (Some(pr(LinkState::Unknown, Status::default())), pr(LinkState::Open, open), true),
        ];
        for (before, after, want) in cases {
            assert_eq!(line(before.as_ref(), &after, 1).is_some(), want, "{before:?} -> {after:?}");
        }
    }

    #[test]
    fn a_closed_issue_and_a_gone_link_say_so() {
        let mut issue =
            pr(LinkState::Closed, Status { reason: Some(Reason::NotPlanned), ..Status::default() });
        issue.url = "https://github.com/hobo-ware/tidewatch/issues/9".into();
        let got = json(&line(None, &issue, 1).unwrap());
        assert_eq!(got["kind"], "issue");
        assert_eq!(got["reason"], "not_planned");

        let gone = pr(LinkState::Unknown, Status { access: Access::Gone, ..Status::default() });
        assert_eq!(json(&line(None, &gone, 1).unwrap())["access"], "gone");
    }

    #[test]
    fn artifacts_and_web_pages_never_make_a_line() {
        let mut link = pr(LinkState::Open, Status::default());
        link.url = "https://claude.ai/artifact/abc".into();
        assert_eq!(line(None, &link, 1), None);
    }

    #[test]
    fn lines_append_whole_and_nothing_is_written_for_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("feed.jsonl");
        append(&file, &[]).unwrap();
        assert!(!file.exists());
        append(&file, &["{\"a\":1}".into()]).unwrap();
        append(&file, &["{\"b\":2}".into(), "{\"c\":3}".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n");
    }
}
