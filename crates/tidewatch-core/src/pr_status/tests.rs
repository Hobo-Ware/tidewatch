use super::*;
use crate::pr::{Placement, PrComment};
use kelp_core::review::{Anchor, Side};

const MERGED: &str = include_str!("../../tests/fixtures/pr_view_merged.json");
const FAILING: &str = include_str!("../../tests/fixtures/pr_view_failing.json");

fn check(name: &str, outcome: Outcome) -> Check {
    Check {
        name: name.into(),
        workflow: "CI".into(),
        outcome,
        url: String::new(),
        run_id: Some(7),
        job_id: Some(8),
        started_ms: Some(1_000),
        finished_ms: (outcome != Outcome::Pending).then_some(61_000),
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        number: 42,
        title: "Retry a stage".into(),
        url: "https://github.com/acme/web/pull/42".into(),
        state: State::Open,
        conflicting: false,
        decision: Decision::None,
        head: "aaa".into(),
        base: "main".into(),
        branch: "agent/lively-lantern".into(),
        author: "me".into(),
        checks: Vec::new(),
        reviews: Vec::new(),
        comments: Vec::new(),
        commits: Some(3),
        created_ms: Some(10),
        ended_ms: None,
    }
}

fn review(id: &str, author: &str, state: ReviewState, at_ms: i64) -> Review {
    Review { id: id.into(), author: author.into(), state, body: String::new(), at_ms }
}

fn comment(id: &str, author: &str, own: bool, at_ms: i64) -> Comment {
    Comment {
        id: id.into(),
        author: author.into(),
        body: format!("text {id}"),
        url: format!("https://github.com/acme/web/pull/42#{id}"),
        at_ms,
        own,
    }
}

fn kinds(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event.body {
            Body::Opened { .. } => "opened",
            Body::ChecksRunning { .. } => "running",
            Body::ChecksFailed { .. } => "failed",
            Body::ChecksPassed { .. } => "passed",
            Body::ChangesRequested { .. } => "changes",
            Body::Commented { .. } => "commented",
            Body::Conflict { .. } => "conflict",
            Body::Approved { .. } => "approved",
            Body::Merged { .. } => "merged",
            Body::Closed { .. } => "closed",
            Body::PushFix { .. } => "push-fix",
            Body::Sent { .. } => "sent",
        })
        .collect()
}

fn look(prev: &Seen, snap: &Snapshot) -> (Vec<&'static str>, Seen) {
    let (events, seen) = diff(Some(prev), snap, 99_000);
    (kinds(&events), seen)
}

#[test]
fn a_real_merged_pull_request_reads_with_one_check_per_name() {
    let snap = parse_snapshot(MERGED).unwrap();
    assert_eq!(snap.number, 64);
    assert_eq!(snap.state, State::Merged);
    assert_eq!(snap.base, "main");
    assert_eq!(snap.branch, "fix/mascot-mask-offset");
    assert_eq!(snap.author, "octo-author");
    assert_eq!(snap.commits, Some(1));
    assert_eq!(snap.decision, Decision::None);
    let names: Vec<&str> = snap.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["check (macos-14)", "check (ubuntu-latest)"]);
    assert!(snap.checks.iter().all(|c| c.outcome == Outcome::Passed));
    let macos = &snap.checks[0];
    assert_eq!(macos.run_id, Some(38_901_944_237), "the later run of the same check wins");
    assert_eq!(macos.job_id, Some(116_976_070_154));
    assert!(snap.ended_ms.is_some());
}

#[test]
fn a_real_failing_pull_request_reads_its_failures_reviews_and_comments() {
    let snap = parse_snapshot(FAILING).unwrap();
    assert_eq!(snap.state, State::Open);
    assert_eq!(snap.decision, Decision::ReviewRequired);
    assert_eq!(phase(&snap.checks), Phase::Failed);
    let failed: Vec<&str> = snap
        .checks
        .iter()
        .filter(|c| c.outcome == Outcome::Failed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(failed, ["build (ubuntu-latest)", "build (windows-latest)", "build (macos-latest)"]);
    assert_eq!(snap.reviews.len(), 1);
    assert_eq!(snap.reviews[0].state, ReviewState::Commented);
    let own: Vec<bool> = snap.comments.iter().map(|c| c.own).collect();
    assert_eq!(own, [false, true]);
    assert_eq!(snap.summary().comments, 1, "your own comments are not counted");
    assert_eq!(snap.summary().headline(), Some((Tone::Bad, "checks failed")));
}

#[test]
fn bad_json_and_unknown_states_are_errors_not_panics() {
    assert!(parse_snapshot("not json").is_err());
    assert!(parse_snapshot(r#"{"number":1,"state":"WEIRD"}"#).is_err());
    assert!(parse_snapshot(r#"{"state":"OPEN"}"#).is_err());
}

#[test]
fn check_outcomes_follow_status_and_conclusion() {
    let cases = [
        ("QUEUED", "", Outcome::Pending),
        ("IN_PROGRESS", "", Outcome::Pending),
        ("COMPLETED", "SUCCESS", Outcome::Passed),
        ("COMPLETED", "FAILURE", Outcome::Failed),
        ("COMPLETED", "TIMED_OUT", Outcome::Failed),
        ("COMPLETED", "SKIPPED", Outcome::Skipped),
        ("COMPLETED", "NEUTRAL", Outcome::Skipped),
        ("COMPLETED", "CANCELLED", Outcome::Skipped),
    ];
    for (status, conclusion, want) in cases {
        assert_eq!(run_outcome(status, conclusion), want, "{status} {conclusion}");
    }
}

#[test]
fn a_status_context_is_a_check_too() {
    let json = r#"{"number":5,"state":"OPEN","statusCheckRollup":[
        {"__typename":"StatusContext","context":"ci/legacy","state":"PENDING","targetUrl":"https://ci.example/1","startedAt":"2026-10-14T10:00:00Z"},
        {"__typename":"Mystery","name":"x"}]}"#;
    let snap = parse_snapshot(json).unwrap();
    assert_eq!(snap.checks.len(), 1);
    assert_eq!(snap.checks[0].name, "ci/legacy");
    assert_eq!(snap.checks[0].outcome, Outcome::Pending);
}

#[test]
fn run_and_job_ids_come_from_the_details_url() {
    let cases = [
        ("https://github.com/a/b/actions/runs/123/job/456", (Some(123), Some(456))),
        ("https://github.com/a/b/actions/runs/123", (Some(123), None)),
        ("https://ci.example/build/9", (None, None)),
        ("", (None, None)),
    ];
    for (url, want) in cases {
        assert_eq!(ids_of(url), want, "{url}");
    }
}

#[test]
fn the_phase_counts_pending_checks_and_ignores_skipped_ones() {
    let cases = [
        (vec![], Phase::NoChecks),
        (vec![check("a", Outcome::Skipped)], Phase::NoChecks),
        (
            vec![check("a", Outcome::Pending), check("b", Outcome::Passed)],
            Phase::Running { done: 1, total: 2 },
        ),
        (
            vec![check("a", Outcome::Failed), check("b", Outcome::Pending)],
            Phase::Running { done: 1, total: 2 },
        ),
        (vec![check("a", Outcome::Failed), check("b", Outcome::Passed)], Phase::Failed),
        (vec![check("a", Outcome::Passed), check("b", Outcome::Skipped)], Phase::Passed),
    ];
    for (checks, want) in cases {
        assert_eq!(phase(&checks), want);
    }
}

#[test]
fn the_first_look_opens_the_pull_request_once() {
    let snap = snapshot();
    let (events, seen) = diff(None, &snap, 99_000);
    assert_eq!(kinds(&events), ["opened"]);
    assert_eq!(events[0].at_ms, 10, "the card sits where the pull request was created");
    assert_eq!(
        look(&seen, &snap).0,
        Vec::<&str>::new(),
        "a second look with nothing new is silent"
    );
}

#[test]
fn checks_go_running_then_failed_then_running_again_on_a_new_push() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    snap.checks = vec![check("a", Outcome::Pending), check("b", Outcome::Pending)];
    let (names, seen) = look(&seen, &snap);
    assert_eq!(names, ["running"]);
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new(), "no progress, no card");
    snap.checks = vec![check("a", Outcome::Passed), check("b", Outcome::Pending)];
    let (names, seen) = look(&seen, &snap);
    assert_eq!(names, ["running"], "progress updates the live card");
    snap.checks = vec![check("a", Outcome::Passed), check("b", Outcome::Failed)];
    let (events, seen) = diff(Some(&seen), &snap, 99_000);
    assert_eq!(kinds(&events), ["failed"]);
    let Body::ChecksFailed { head, jobs } = &events[0].body else { panic!("not a failure") };
    assert_eq!(head, "aaa");
    assert_eq!(
        jobs.iter().map(|j| (j.name.as_str(), j.passed)).collect::<Vec<_>>(),
        [("a", true), ("b", false)]
    );
    assert_eq!(jobs[1].secs, Some(60));
    assert_eq!(events[0].at_ms, 61_000, "a failure is dated by the last check to finish");
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
    snap.head = "bbb".into();
    snap.checks = vec![check("a", Outcome::Pending), check("b", Outcome::Pending)];
    assert_eq!(look(&seen, &snap).0, ["running"]);
}

#[test]
fn a_new_push_that_fails_again_is_a_new_failure_but_the_same_head_is_not() {
    let mut snap = snapshot();
    snap.checks = vec![check("a", Outcome::Failed)];
    let (_, seen) = diff(None, &snap, 0);
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
    snap.head = "bbb".into();
    assert_eq!(look(&seen, &snap).0, ["failed"]);
}

#[test]
fn passing_checks_report_the_count_and_the_time_once() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    snap.checks = vec![
        check("a", Outcome::Passed),
        check("b", Outcome::Passed),
        check("c", Outcome::Skipped),
    ];
    let (events, seen) = diff(Some(&seen), &snap, 99_000);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].body, Body::ChecksPassed { total: 2, secs: Some(60) });
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
}

#[test]
fn a_conflict_shows_up_once_and_again_after_it_was_resolved() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    snap.conflicting = true;
    let (names, seen) = look(&seen, &snap);
    assert_eq!(names, ["conflict"]);
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
    snap.conflicting = false;
    let (names, seen) = look(&seen, &snap);
    assert_eq!(names, Vec::<&str>::new());
    snap.conflicting = true;
    assert_eq!(look(&seen, &snap).0, ["conflict"]);
}

#[test]
fn reviews_make_one_card_each_and_your_own_are_skipped() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    snap.decision = Decision::ChangesRequested;
    snap.reviews = vec![
        review("r1", "reviewer", ReviewState::ChangesRequested, 500),
        review("r2", "me", ReviewState::Commented, 600),
        review("r3", "reviewer", ReviewState::Other, 700),
    ];
    let (events, seen) = diff(Some(&seen), &snap, 99_000);
    assert_eq!(kinds(&events), ["changes"]);
    assert_eq!(events[0].at_ms, 500);
    snap.decision = Decision::Approved;
    snap.reviews.push(review("r4", "reviewer", ReviewState::Approved, 800));
    let (names, _) = look(&seen, &snap);
    assert_eq!(names, ["approved"]);
}

#[test]
fn a_review_with_a_note_but_no_verdict_is_a_comment() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    let mut noted = review("r1", "reviewer", ReviewState::Commented, 500);
    noted.body = "Looks fine".into();
    snap.reviews = vec![noted, review("r2", "reviewer", ReviewState::Commented, 600)];
    assert_eq!(
        look(&seen, &snap).0,
        ["commented"],
        "an empty COMMENTED review only carries inline notes"
    );
}

#[test]
fn new_comments_by_others_make_cards_and_yours_do_not() {
    let mut snap = snapshot();
    let (_, seen) = diff(None, &snap, 0);
    snap.comments = vec![comment("c1", "reviewer", false, 900), comment("c2", "me", true, 950)];
    let (events, seen) = diff(Some(&seen), &snap, 99_000);
    assert_eq!(kinds(&events), ["commented"]);
    assert_eq!(events[0].at_ms, 900);
    assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
}

#[test]
fn a_first_look_at_an_old_pull_request_shows_its_standing_not_its_whole_history() {
    let mut snap = snapshot();
    snap.decision = Decision::ChangesRequested;
    snap.reviews = vec![
        review("r1", "reviewer", ReviewState::ChangesRequested, 100),
        review("r2", "reviewer", ReviewState::Approved, 200),
        review("r3", "reviewer", ReviewState::ChangesRequested, 300),
    ];
    snap.comments =
        (0..6).map(|n| comment(&format!("c{n}"), "reviewer", false, 1000 + n)).collect();
    let (events, _) = diff(None, &snap, 99_000);
    assert_eq!(
        kinds(&events),
        ["opened", "changes", "commented", "commented", "commented"],
        "the latest request only, and the three newest comments"
    );
    let Body::ChangesRequested { .. } = events[1].body else { panic!("not a request") };
    assert_eq!(events[1].at_ms, 300);
}

#[test]
fn merging_and_closing_end_the_story_once() {
    for (state, want) in [(State::Merged, "merged"), (State::Closed, "closed")] {
        let mut snap = snapshot();
        let (_, seen) = diff(None, &snap, 0);
        snap.state = state;
        snap.ended_ms = Some(5_000);
        snap.checks = vec![check("a", Outcome::Failed)];
        let (events, seen) = diff(Some(&seen), &snap, 99_000);
        assert_eq!(kinds(&events), [want]);
        assert_eq!(events[0].at_ms, 5_000);
        assert!(seen.ended);
        assert_eq!(look(&seen, &snap).0, Vec::<&str>::new());
    }
}

#[test]
fn headlines_put_the_most_urgent_thing_first() {
    let mut summary = snapshot().summary();
    let cases = [
        (SummaryChecks::Failed, Decision::Approved, true, Some((Tone::Bad, "checks failed"))),
        (
            SummaryChecks::Passed,
            Decision::ChangesRequested,
            true,
            Some((Tone::Warn, "changes requested")),
        ),
        (SummaryChecks::Passed, Decision::None, true, Some((Tone::Warn, "conflicts"))),
        (SummaryChecks::Running, Decision::Approved, false, Some((Tone::Good, "approved"))),
        (SummaryChecks::Running, Decision::None, false, Some((Tone::Warn, "checks running"))),
        (SummaryChecks::Passed, Decision::None, false, Some((Tone::Good, "checks passed"))),
        (SummaryChecks::None, Decision::None, false, Some((Tone::Quiet, "open"))),
    ];
    for (checks, decision, conflicting, want) in cases {
        summary.checks = checks;
        summary.decision = decision;
        summary.conflicting = conflicting;
        assert_eq!(summary.headline(), want, "{checks:?} {decision:?} {conflicting}");
    }
    summary.state = State::Merged;
    assert_eq!(summary.headline(), None);
}

fn thread(path: &str, placement: Placement, author: &str, time: i64, text: &str) -> PrThread {
    PrThread {
        root_id: 1,
        path: path.into(),
        placement,
        side: Side::New,
        commit: "abc".into(),
        url: String::new(),
        anchor: Anchor {
            side: Side::New,
            line_hint: 0,
            text: String::new(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        },
        comments: vec![PrComment {
            id: 1,
            author: author.into(),
            time,
            body: format!("  {text}\n"),
        }],
    }
}

#[test]
fn a_review_owns_the_threads_its_author_started_since_their_last_review() {
    let threads = [
        thread(
            "runner.rs",
            Placement::Line(212),
            "reviewer",
            1000,
            "Can the retry count live in the stage?",
        ),
        thread(
            "tests.rs",
            Placement::Outdated(Some(88)),
            "reviewer",
            1001,
            "Add the passing case.",
        ),
        thread("lib.rs", Placement::File, "reviewer", 1002, "A word on the whole file."),
        thread("other.rs", Placement::Line(1), "someone-else", 1000, "Not theirs."),
        thread("old.rs", Placement::Line(3), "reviewer", 400, "From the review before."),
        thread("late.rs", Placement::Line(3), "reviewer", 2000, "From the next review."),
    ];
    let got = review_comments(&threads, "reviewer", 500_000, 1_010_000);
    assert_eq!(
        got,
        [
            ReviewComment {
                path: "runner.rs".into(),
                line: Some(212),
                text: "Can the retry count live in the stage?".into()
            },
            ReviewComment {
                path: "tests.rs".into(),
                line: Some(88),
                text: "Add the passing case.".into()
            },
            ReviewComment {
                path: "lib.rs".into(),
                line: None,
                text: "A word on the whole file.".into()
            },
        ]
    );
}

#[test]
fn cards_survive_a_round_trip_through_json() {
    let bodies = vec![
        Body::Opened {
            number: 1,
            title: "t".into(),
            base: "main".into(),
            branch: "b".into(),
            commits: None,
        },
        Body::ChecksFailed {
            head: "h".into(),
            jobs: vec![Job {
                name: "check".into(),
                passed: false,
                step: Some("clippy".into()),
                secs: Some(3),
                url: String::new(),
                run_id: Some(1),
                job_id: Some(2),
                log: vec!["error".into()],
            }],
        },
        Body::Sent {
            via: Via::Auto,
            tag: "CI failure".into(),
            headline: "h".into(),
            extra_lines: 4,
            text: "t".into(),
        },
        Body::PushFix { commits: 2, head: "h".into() },
    ];
    for body in bodies {
        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(serde_json::from_str::<Body>(&json).unwrap(), body, "{json}");
    }
}
