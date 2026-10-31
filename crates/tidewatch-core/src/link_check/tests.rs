use super::*;

const NOW: i64 = 1_800_000_000_000;

fn pr_json(state: &str, extra: &str) -> String {
    format!(r#"{{"number":7,"state":"{state}","updatedAt":"2026-10-30T10:00:00Z"{extra}}}"#)
}

fn run(name: &str, conclusion: &str) -> String {
    format!(
        r#"{{"__typename":"CheckRun","name":"{name}","workflowName":"ci","status":"COMPLETED","conclusion":"{conclusion}","detailsUrl":"https://github.com/o/r/actions/runs/1/job/2","startedAt":"2026-10-30T09:00:00Z"}}"#
    )
}

#[test]
fn a_pull_request_answer_gives_its_state_and_everything_that_goes_with_it() {
    let failing = pr_json(
        "OPEN",
        &format!(
            r#","isDraft":true,"mergeable":"CONFLICTING","reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[{}],"comments":[{{"id":"a","body":"hi","author":{{"login":"x"}},"createdAt":"2026-10-30T09:00:00Z"}}]"#,
            run("lint", "FAILURE")
        ),
    );
    let (state, status) = parse_pr(&failing, NOW).unwrap();
    assert_eq!(state, LinkState::Open);
    assert_eq!(
        status,
        Status {
            draft: true,
            checks: Some(Checks::Fail),
            review: Some(Review::ChangesRequested),
            conflicts: true,
            comments: Some(1),
            updated_ms: rfc3339::unix_millis("2026-10-30T10:00:00Z"),
            checked_ms: Some(NOW),
            ..Status::default()
        }
    );
}

#[test]
fn each_checks_and_review_answer_reads_the_way_the_row_shows_it() {
    let cases = [
        (format!(r#","statusCheckRollup":[{}]"#, run("a", "SUCCESS")), Some(Checks::Pass), None),
        (
            r#","statusCheckRollup":[{"__typename":"CheckRun","name":"a","workflowName":"ci","status":"IN_PROGRESS","conclusion":""}]"#
                .to_owned(),
            Some(Checks::Running),
            None,
        ),
        (String::new(), None, None),
        (r#","reviewDecision":"APPROVED""#.to_owned(), None, Some(Review::Approved)),
        (r#","reviewDecision":"REVIEW_REQUIRED""#.to_owned(), None, None),
    ];
    for (extra, checks, review) in cases {
        let (_, status) = parse_pr(&pr_json("OPEN", &extra), NOW).unwrap();
        assert_eq!((status.checks, status.review), (checks, review), "{extra}");
    }
}

#[test]
fn a_merged_or_closed_pull_request_keeps_no_checks_review_or_conflicts() {
    let extra = format!(
        r#","isDraft":true,"mergeable":"CONFLICTING","reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[{}]"#,
        run("lint", "FAILURE")
    );
    for (word, want) in [("MERGED", LinkState::Merged), ("CLOSED", LinkState::Closed)] {
        let (state, status) = parse_pr(&pr_json(word, &extra), NOW).unwrap();
        assert_eq!(state, want);
        assert_eq!(
            (status.draft, status.checks, status.review, status.conflicts),
            (false, None, None, false)
        );
    }
}

#[test]
fn an_issue_answer_gives_its_state_close_reason_comment_count_and_update_time() {
    let cases = [
        (r#""OPEN","stateReason":"REOPENED""#, LinkState::Open, None),
        (r#""CLOSED","stateReason":"COMPLETED""#, LinkState::Closed, Some(Reason::Completed)),
        (r#""CLOSED","stateReason":"NOT_PLANNED""#, LinkState::Closed, Some(Reason::NotPlanned)),
        (r#""CLOSED","stateReason":"DUPLICATE""#, LinkState::Closed, Some(Reason::NotPlanned)),
        (r#""CLOSED""#, LinkState::Closed, None),
        (r#""OPEN","stateReason":"COMPLETED""#, LinkState::Open, None),
    ];
    for (head, state, reason) in cases {
        let json = format!(
            r#"{{"state":{head},"comments":[{{}},{{}},{{}}],"updatedAt":"2026-10-30T10:00:00Z"}}"#
        );
        let (got, status) = parse_issue(&json, NOW).unwrap();
        assert_eq!((got, status.reason, status.comments), (state, reason, Some(3)), "{head}");
        assert_eq!(status.updated_ms, rfc3339::unix_millis("2026-10-30T10:00:00Z"));
        assert_eq!(status.checked_ms, Some(NOW));
    }
}

#[test]
fn an_unreadable_answer_is_nothing_learned() {
    for json in ["", "not json", "{}", r#"{"state":"DRAFT"}"#, r#"{"state":3}"#] {
        assert_eq!(parse_issue(json, NOW), None, "{json:?}");
        assert_eq!(parse_pr(json, NOW), None, "{json:?}");
    }
}

#[test]
fn a_failure_is_sorted_into_no_access_gone_or_nothing_learned() {
    let cases = [
        (
            "GraphQL: Could not resolve to a Repository with the name 'o/r'. (repository)",
            Lookup::NoAccess,
        ),
        ("HTTP 404: Not Found (https://api.github.com/repos/o/r)", Lookup::NoAccess),
        ("HTTP 403: Resource not accessible", Lookup::NoAccess),
        (
            "GraphQL: Could not resolve to a PullRequest with the number of 99. (repository.pullRequest)",
            Lookup::Gone,
        ),
        (
            "GraphQL: Could not resolve to an Issue with the number of 99. (repository.issue)",
            Lookup::Gone,
        ),
        ("no pull requests found for branch", Lookup::Gone),
        ("error connecting to api.github.com", Lookup::Failed),
        ("HTTP 502: Bad Gateway", Lookup::Failed),
    ];
    for (message, want) in cases {
        assert_eq!(sort_failure(&GhError::Failed(message.into())), want, "{message}");
    }
    for other in [GhError::Missing, GhError::NotAuthenticated, GhError::Stopped("slow".into())] {
        assert_eq!(sort_failure(&other), Lookup::Failed, "{other:?}");
    }
    assert_eq!(access_of(&Lookup::NoAccess), Some(Access::NoAccess));
    assert_eq!(access_of(&Lookup::Gone), Some(Access::Gone));
    assert_eq!(access_of(&Lookup::Failed), None);
}

#[test]
fn a_link_that_is_not_a_pull_request_or_issue_is_never_looked_up() {
    let nowhere = Tools::with_path("/nonexistent");
    for url in ["https://example.com/a", "https://claude.ai/artifact/x", "https://github.com/o/r"] {
        assert_eq!(lookup(&nowhere, Path::new("/"), url, NOW), Lookup::Failed, "{url}");
    }
}

#[test]
fn a_missing_gh_is_nothing_learned_not_a_hang() {
    let nowhere = Tools::with_path("/nonexistent");
    assert_eq!(
        lookup(&nowhere, Path::new("/"), "https://github.com/o/r/pull/1", NOW),
        Lookup::Failed
    );
}
