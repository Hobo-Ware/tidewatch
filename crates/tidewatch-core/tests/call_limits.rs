//! A `git` or `gh` that never answers ends as a failure that names the limit, for the callers whose
//! stuck state the owner sees: the PR look, the Meeseeks checks and the done check. The tools are
//! scripts that sleep; the limit is short so no test waits a minute.

mod common;

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{commit_file, git_available, ok, repos, write_executable_from_a_child};
use tidewatch_core::civil_date::Date;
use tidewatch_core::done_check::{Input, Kind, PrFact, Status, run};
use tidewatch_core::meeseeks_check::{hand_off, proof};
use tidewatch_core::pr_gh::poll;
use tidewatch_core::tools::{GhError, Tools};

const LIMIT: Duration = Duration::from_millis(300);

struct Hung {
    dir: tempfile::TempDir,
}

impl Hung {
    fn new(programs: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for program in programs {
            write_executable_from_a_child(&dir.path().join(program), "#!/bin/sh\nexec sleep 30\n");
        }
        Self { dir }
    }

    fn tools(&self) -> Tools {
        let mut dirs = vec![self.dir.path().to_path_buf()];
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let path: OsString = std::env::join_paths(dirs).unwrap();
        Tools::with_path(path).with_limit(LIMIT)
    }
}

fn quickly<T>(work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let done = work();
    assert!(started.elapsed() < Duration::from_secs(15), "took {:?}", started.elapsed());
    done
}

#[test]
fn a_gh_that_never_answers_ends_a_pr_look_as_stopped_with_the_limit_named() {
    let gh = Hung::new(&["gh"]);
    let dir = tempfile::tempdir().unwrap();
    let result = quickly(|| poll(&gh.tools(), dir.path(), 42, None, 0));
    assert_eq!(
        result,
        Err(GhError::Stopped("gh took longer than 300 ms and was stopped".to_owned()))
    );
}

#[test]
fn a_git_that_never_answers_fails_the_proof_check_in_words() {
    let git = Hung::new(&["git"]);
    let dir = tempfile::tempdir().unwrap();
    let error = quickly(|| proof(&git.tools(), dir.path())).unwrap_err();
    assert_eq!(error, "git took longer than 300 ms and was stopped");
}

#[test]
fn a_gh_that_never_answers_fails_the_hand_off_check_with_the_stop_not_the_push_advice() {
    if !git_available() {
        return;
    }
    let r = repos();
    let gh = Hung::new(&["gh"]);
    let error = quickly(|| hand_off(&gh.tools(), &r.work, "agent/x")).unwrap_err();
    assert_eq!(error, "gh took longer than 300 ms and was stopped");
}

fn line(report: &tidewatch_core::done_check::Report, kind: Kind) -> (Status, String) {
    let line = report.lines.iter().find(|line| line.kind == kind).unwrap();
    (line.status, line.detail.clone())
}

fn check(tools: &Tools, dir: &Path) -> tidewatch_core::done_check::Report {
    let today = Date::parse("2026-10-27").unwrap();
    quickly(|| {
        run(&Input {
            tools,
            dir,
            branch: "agent/x",
            pr: PrFact::Number(42),
            unverified: Some(Vec::new()),
            today,
        })
    })
}

#[test]
fn a_gh_that_never_answers_leaves_the_pull_request_check_unknown_with_the_reason() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["checkout", "-q", "-b", "agent/x"]);
    commit_file(&r.work, "a.txt", "one\n", "feat: a");
    let gh = Hung::new(&["gh"]);
    let report = check(&gh.tools(), &r.work);
    assert_eq!(
        line(&report, Kind::PullRequest),
        (Status::Unknown, "gh took longer than 300 ms and was stopped".to_owned())
    );
}

#[test]
fn a_git_that_never_answers_leaves_the_committed_check_unknown_with_the_reason() {
    let git = Hung::new(&["git"]);
    let dir = tempfile::tempdir().unwrap();
    let report = check(&git.tools(), dir.path());
    assert_eq!(
        line(&report, Kind::Committed),
        (
            Status::Unknown,
            "cannot read the work tree: git took longer than 300 ms and was stopped".to_owned()
        )
    );
}
