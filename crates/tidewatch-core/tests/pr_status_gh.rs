//! One look at a pull request through a fake `gh` that answers by the shape of the call and records
//! it. The real `gh` is never started; the JSON is a redacted copy of what `gh` 2.98.0 printed for
//! public repositories.

mod common;

use std::fmt::Write as _;
use std::path::Path;

use common::{commit_file, git_available, ok, repos, write_executable_from_a_child};
use tidewatch_core::pr_gh::{failed_log, poll, rerun_failed, unpushed};
use tidewatch_core::pr_status::{Body, Seen};
use tidewatch_core::tools::{GhError, Tools};

const FAILING: &str = include_str!("fixtures/pr_view_failing.json");
const LOG: &str = include_str!("fixtures/gh_log_failed.txt");

/// A `gh` that logs each call and prints the first rule whose glob matches its arguments.
struct Gh {
    dir: tempfile::TempDir,
}

impl Gh {
    fn new(rules: &[(&str, i32, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cases = String::new();
        for (i, (pattern, code, stdout)) in rules.iter().enumerate() {
            std::fs::write(dir.path().join(format!("out{i}")), stdout).unwrap();
            writeln!(cases, "  {pattern}) cat '{}/out{i}'; exit {code};;", dir.path().display())
                .unwrap();
        }
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$*\" in\n{cases}  *) echo \"unexpected gh call: $*\" >&2; exit 99;;\nesac\n",
            log = dir.path().join("calls.log").display()
        );
        write_executable_from_a_child(&dir.path().join("gh"), &script);
        Self { dir }
    }

    fn tools(&self) -> Tools {
        let mut dirs = vec![self.dir.path().to_path_buf()];
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        Tools::with_path(std::env::join_paths(dirs).unwrap())
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn cwd() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

const JOBS: &str = r#"{"jobs":[
  {"databaseId":84988385122,"name":"build (ubuntu-latest)","steps":[
    {"name":"Set up job","conclusion":"success"},
    {"name":"Run cargo clippy --workspace --all-targets -- -D warnings","conclusion":"failure"}]},
  {"databaseId":84988385108,"name":"build (windows-latest)","steps":[{"name":"Run tests","conclusion":"failure"}]}]}"#;

#[test]
fn the_first_look_opens_the_card_and_reads_the_failures_of_each_failed_job() {
    let gh = Gh::new(&[
        ("'pr view'*", 0, FAILING),
        ("'run view'*'--json jobs'", 0, JOBS),
        ("'run view'*'--log-failed'", 0, LOG),
    ]);
    let polled = poll(&gh.tools(), cwd().path(), 13788, None, 1_000).unwrap();
    let kinds: Vec<&str> = polled
        .events
        .iter()
        .map(|e| match e.body {
            Body::Opened { .. } => "opened",
            Body::ChecksFailed { .. } => "failed",
            Body::Commented { .. } => "commented",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["opened", "commented", "failed"]);
    let Body::ChecksFailed { jobs, .. } = &polled.events[2].body else { panic!("no failure card") };
    let ubuntu = jobs.iter().find(|j| j.name == "build (ubuntu-latest)").unwrap();
    assert_eq!(
        ubuntu.step.as_deref(),
        Some("Run cargo clippy --workspace --all-targets -- -D warnings")
    );
    assert_eq!(
        ubuntu.log.last().map(String::as_str),
        Some("Process completed with exit code 101.")
    );
    assert!(ubuntu.log.len() <= 12);
    assert!(ubuntu.log.iter().all(|line| !line.contains('\u{1b}')), "{:?}", ubuntu.log);
    assert!(polled.summary.comments == 1 && !polled.checks_pending && !polled.ended);
    let calls = gh.calls();
    assert!(calls[0].starts_with("pr view 13788 --json "), "{calls:?}");
    assert!(calls[0].ends_with(",commits"), "the first look counts the commits");
    assert!(
        calls.iter().all(|c| c.starts_with("pr view") || c.starts_with("run view")),
        "{calls:?}"
    );
}

#[test]
fn a_later_look_is_silent_when_nothing_changed_and_skips_the_commit_list() {
    let gh = Gh::new(&[("'pr view'*", 0, FAILING), ("'run view'*", 1, "")]);
    let first = poll(&gh.tools(), cwd().path(), 13788, None, 1_000).unwrap();
    let again = poll(&gh.tools(), cwd().path(), 13788, Some(&first.seen), 2_000).unwrap();
    assert_eq!(again.events, []);
    assert_eq!(again.seen, first.seen);
    let second = &gh.calls()[gh.calls().len() - 1];
    assert!(second.starts_with("pr view 13788 --json ") && !second.contains("commits"), "{second}");
}

#[test]
fn a_failed_run_whose_logs_are_gone_still_gives_the_card() {
    let gh = Gh::new(&[("'pr view'*", 0, FAILING), ("'run view'*", 1, "")]);
    let polled = poll(&gh.tools(), cwd().path(), 13788, None, 0).unwrap();
    let Body::ChecksFailed { jobs, .. } = &polled.events.last().unwrap().body else {
        panic!("no failure card")
    };
    assert!(jobs.iter().filter(|j| !j.passed).all(|j| j.log.is_empty() && j.step.is_none()));
}

#[test]
fn a_review_brings_its_inline_comments_from_the_rest_api() {
    let view = r#"{"number":42,"title":"t","url":"https://github.com/acme/web/pull/42","state":"OPEN",
        "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviewDecision":"CHANGES_REQUESTED",
        "headRefOid":"abc","baseRefName":"main","headRefName":"agent/x","author":{"login":"me"},
        "statusCheckRollup":[],"comments":[],"createdAt":"2026-10-12T09:00:00Z",
        "reviews":[{"id":"PRR_1","author":{"login":"rectifyer"},"state":"CHANGES_REQUESTED","body":"",
                    "submittedAt":"2026-10-12T10:00:30Z"}]}"#;
    let rest = r#"[{"id":101,"path":"src/auth.rs","line":12,"original_line":12,"side":"RIGHT",
        "user":{"login":"rectifyer"},"body":"Use the shared helper.","created_at":"2026-10-12T10:00:00Z",
        "commit_id":"abc","original_commit_id":"abc","diff_hunk":"@@ -1 +1 @@\n+x","subject_type":"line",
        "html_url":"https://github.com/acme/web/pull/42#r101"},
        {"id":102,"path":"src/other.rs","line":3,"user":{"login":"someone"},"body":"Not part of it.",
        "created_at":"2026-10-12T10:00:10Z","diff_hunk":"@@ -1 +1 @@\n+x","subject_type":"line"}]"#;
    let gh = Gh::new(&[("'pr view'*", 0, view), ("'api repos/'*", 0, rest)]);
    let seen = Seen { opened: true, head: "abc".into(), ..Seen::default() };
    let polled = poll(&gh.tools(), cwd().path(), 42, Some(&seen), 0).unwrap();
    let Body::ChangesRequested { author, comments, .. } = &polled.events[0].body else {
        panic!("no review card: {:?}", polled.events)
    };
    assert_eq!(author, "rectifyer");
    assert_eq!(comments.len(), 1);
    assert_eq!((comments[0].path.as_str(), comments[0].line), ("src/auth.rs", Some(12)));
}

#[test]
fn gh_failures_are_sorted_and_a_missing_gh_is_missing() {
    let gh = Gh::new(&[("'pr view'*", 1, "")]);
    assert!(matches!(poll(&gh.tools(), cwd().path(), 1, None, 0), Err(GhError::Failed(_))));
    let empty = cwd();
    let tools = Tools::with_path(empty.path());
    assert_eq!(poll(&tools, empty.path(), 1, None, 0), Err(GhError::Missing));
}

#[test]
fn re_run_asks_gh_to_rerun_only_the_failed_jobs_of_that_run() {
    let gh = Gh::new(&[("'run rerun'*", 0, "")]);
    rerun_failed(&gh.tools(), cwd().path(), 28_656_994_029).unwrap();
    assert_eq!(gh.calls(), ["run rerun 28656994029 --failed"]);
}

#[test]
fn the_failed_log_is_one_job_of_one_run() {
    let gh = Gh::new(&[("'run view'*", 0, LOG)]);
    assert_eq!(failed_log(&gh.tools(), cwd().path(), 7, 9).unwrap(), LOG);
    assert_eq!(gh.calls(), ["run view 7 --job 9 --log-failed"]);
}

#[test]
fn commits_the_worktree_has_beyond_the_pull_request_are_counted() {
    if !git_available() {
        return;
    }
    let repos = repos();
    let remote_head = ok(&repos.work, &["rev-parse", "HEAD"]).trim().to_owned();
    let tools = Tools::with_path(std::env::var_os("PATH").unwrap_or_default());
    let at = |dir: &Path| unpushed(&tools, dir, &remote_head);
    assert_eq!(at(&repos.work).map(|(_, n)| n), Some(0));
    commit_file(&repos.work, "a.txt", "a\n", "fix: one");
    commit_file(&repos.work, "b.txt", "b\n", "fix: two");
    let (head, count) = at(&repos.work).unwrap();
    assert_eq!(count, 2);
    assert_eq!(head, ok(&repos.work, &["rev-parse", "HEAD"]).trim());
    assert_eq!(unpushed(&tools, &repos.work, "0000000000000000000000000000000000000000"), None);
    assert_eq!(unpushed(&tools, &repos.work, "--evil"), None);
    assert_eq!(unpushed(&tools, &repos.work, ""), None);
}
