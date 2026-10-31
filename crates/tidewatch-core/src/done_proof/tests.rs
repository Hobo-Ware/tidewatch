use super::*;
use crate::workflow_profile::Fixes;

fn run(command: &str, exit_code: i32, output: &str) -> TestsRun {
    let finished = TestRun { exit_code, output: output.to_owned() };
    TestsRun::of(command, &finished, Duration::from_secs(134))
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn item(text: &str, why: Option<&str>) -> Verified {
    Verified { text: text.to_owned(), why: why.map(str::to_owned) }
}

#[test]
fn failed_counts_read_each_runner_the_way_it_prints() {
    let cargo = "test a ... FAILED\n\ntest result: FAILED. 10 passed; 1 failed; 0 ignored\n\ntest result: FAILED. 4 passed; 1 failed; 0 ignored\n";
    let cases = [
        (cargo, Some(2)),
        ("test result: ok. 5 passed; 0 failed; 0 ignored", None),
        (
            "Test Suites: 1 failed, 3 passed, 4 total\nTests:       3 failed, 40 passed, 43 total",
            Some(3),
        ),
        ("=== 2 failed, 3 passed in 0.12s ===", Some(2)),
        (" 5 pass\n 2 fail\nRan 7 tests across 1 file.", Some(2)),
        ("error: could not compile `x`", None),
        ("", None),
    ];
    for (output, want) in cases {
        assert_eq!(failed_count(output), want, "{output:?}");
    }
}

#[test]
fn the_result_line_names_the_command_and_what_happened() {
    let cases = [
        (
            run("cargo test --workspace", 0, "ok"),
            Status::Pass,
            "Ran cargo test --workspace: passed in 134 s",
        ),
        (
            run("cargo test --workspace", 101, "test result: FAILED. 1 passed; 2 failed;"),
            Status::Fail,
            "Ran cargo test --workspace: 2 failed",
        ),
        (
            run("make test", 2, "make: *** [test] Error 2"),
            Status::Fail,
            "Ran make test: failed (exit 2)",
        ),
        (
            run(
                "bun run test",
                -1,
                "[tidewatch: the test command ran longer than 10 min and was stopped]",
            ),
            Status::Unknown,
            "Ran bun run test: did not finish ([tidewatch: the test command ran longer than 10 min and was stopped])",
        ),
    ];
    for (tests, status, detail) in cases {
        assert_eq!(tests.status(), (status, detail.to_owned()), "{tests:?}");
    }
}

#[test]
fn a_failed_run_tells_the_agent_what_ran_how_many_failed_where_the_fix_goes_and_the_output() {
    let output = "running 3 tests\ntest a ... FAILED\ntest b ... FAILED\n\ntest result: FAILED. 1 passed; 2 failed; 0 ignored\n";
    let failed = run("cargo test --workspace", 101, output);
    let fixup = Workflow::default();
    assert_eq!(
        failed.failure_note(fixup),
        "Tidewatch ran cargo test --workspace: 2 failed (output below). Fix them, fold each fix into its commit, and say done again.\n\n```\nrunning 3 tests\ntest a ... FAILED\ntest b ... FAILED\n\ntest result: FAILED. 1 passed; 2 failed; 0 ignored\n```"
    );
    let new_commits = Workflow { fixes: Fixes::NewCommits, ..fixup };
    assert!(
        failed.failure_note(new_commits).contains("Fix them, commit each fix, and say done again."),
        "the words come from the profile"
    );
    let one = run("pytest", 1, "=== 1 failed, 3 passed ===");
    assert!(one.failure_note(fixup).starts_with(
        "Tidewatch ran pytest: 1 failed (output below). Fix it, fold the fix into the commit it belongs to, and say done again."
    ));
    let unknown = run("make test", 2, "");
    assert_eq!(
        unknown.failure_note(fixup),
        "Tidewatch ran make test: failed (exit 2) (output below). Fix it, fold the fix into the commit it belongs to, and say done again."
    );
}

#[test]
fn a_verified_item_names_a_command_in_a_code_span_in_its_text_or_reason_or_by_starting_with_one() {
    let cases = [
        (item("tests pass: `cargo test --workspace`", None), Some("cargo test --workspace")),
        (item("the build is clean", Some("`bun run check` exits 0")), Some("bun run check")),
        (item("cargo clippy --workspace: no warnings", None), Some("cargo clippy --workspace")),
        (item("it works", Some("ran ./scripts/smoke.sh against staging")), None),
        (item("`./scripts/smoke.sh` passes", None), Some("./scripts/smoke.sh")),
        (item("opened `src/auth.rs` and read it", None), None),
        (item("looks right in the browser", None), None),
        (item("the `expiry()` helper handles zero", None), None),
    ];
    for (verified, want) in cases {
        assert_eq!(command_in(&verified).as_deref(), want, "{verified:?}");
    }
}

#[test]
fn verified_items_with_a_command_pass_and_the_rest_are_claimed() {
    let ran = run("cargo test --workspace", 0, "ok");
    let all =
        [item("`cargo test --workspace` passes", None), item("`bun run check` is clean", None)];
    let (status, detail, claimed) = verified_status(&all, Some(&ran)).unwrap();
    assert_eq!(
        (status, detail.as_str(), claimed),
        (Status::Pass, "2 verified items name a command, 1 run by Tidewatch", Vec::new())
    );
    let mixed =
        [item("`cargo test --workspace` passes", None), item("it looks right in Safari", None)];
    let (status, detail, claimed) = verified_status(&mixed, None).unwrap();
    assert_eq!(status, Status::Noted);
    assert_eq!(detail, "1 of 2 verified items name no command, so they are claimed, not checked");
    assert_eq!(claimed, ["it looks right in Safari"]);
    assert_eq!(verified_status(&[], None), None, "nothing verified, nothing to say");
}

#[test]
fn only_a_passing_run_of_the_same_command_counts_as_run_by_tidewatch() {
    let passed = run("cargo test --workspace", 0, "");
    let failed = run("cargo test --workspace", 101, "");
    let cases = [
        ("cargo test --workspace", Some(&passed), true),
        ("cargo test", Some(&passed), true),
        ("cargo clippy", Some(&passed), false),
        ("cargo test --workspace", Some(&failed), false),
        ("cargo test --workspace", None, false),
    ];
    for (command, ran, want) in cases {
        assert_eq!(ran_by_tidewatch(command, ran), want, "{command} {ran:?}");
    }
}

#[test]
fn scope_lists_the_changed_files_the_plan_never_named() {
    let plan = strings(&[
        "Add the proof checks",
        "Edit crates/tidewatch-core/src/done_check.rs",
        "done_proof.rs holds the rules",
        "Switches go in workflow_profile.rs",
    ]);
    let files = strings(&[
        "crates/tidewatch-core/src/done_check.rs",
        "crates/tidewatch-core/src/done_proof.rs",
        "crates/tidewatch-core/src/workflow_profile.rs",
        "crates/tidewatch-core/src/launch.rs",
        "Cargo.lock",
    ]);
    let (status, detail, outside) = scope_status(&files, Some(&plan));
    assert_eq!(status, Status::Noted);
    assert_eq!(detail, "1 changed file outside what the plan named");
    assert_eq!(outside, ["crates/tidewatch-core/src/launch.rs"]);
    let (status, _, outside) = scope_status(&files[..3], Some(&plan));
    assert_eq!((status, outside), (Status::Pass, Vec::new()));
}

#[test]
fn scope_has_nothing_to_compare_without_a_plan_or_a_plan_that_names_no_file() {
    let files = strings(&["src/a.rs"]);
    let vague = strings(&["Make it faster", "Check the results"]);
    assert_eq!(
        scope_status(&files, Some(&vague)),
        (Status::Skipped, "the plan names no files".to_owned(), Vec::new())
    );
    assert_eq!(
        scope_status(&files, None),
        (Status::Skipped, "the agent keeps no progress report".to_owned(), Vec::new())
    );
}

#[test]
fn a_directory_in_the_plan_names_everything_under_it() {
    let plan = strings(&["Rework `crates/tidewatch/src/app/`"]);
    let files =
        strings(&["crates/tidewatch/src/app/done_check.rs", "crates/tidewatch/src/flow.rs"]);
    let (_, _, outside) = scope_status(&files, Some(&plan));
    assert_eq!(outside, ["crates/tidewatch/src/flow.rs"]);
}

#[test]
fn a_branch_is_a_fix_by_a_commit_subject_or_a_word_in_its_name() {
    let cases = [
        (&["fix: stop the hang"][..], "agent/calm-owl", true),
        (&["fix(status): a restarted agent"][..], "agent/calm-owl", true),
        (&["Fix the hang"][..], "agent/calm-owl", true),
        (&["feat: add a card"][..], "agent/calm-owl", false),
        (&["feat: add a card"][..], "agent/fix-login-hang", true),
        (&["chore: prefix change"][..], "agent/suffix-cleanup", false),
        (&["fixture: add data"][..], "agent/calm-owl", false),
    ];
    for (subjects, branch, want) in cases {
        assert_eq!(is_fix(&strings(subjects), branch), want, "{subjects:?} {branch}");
    }
}

#[test]
fn a_diff_adds_a_test_by_a_test_file_or_an_added_test_line() {
    let rust_inline =
        "+++ b/src/auth.rs\n@@ -1,0 +10,4 @@\n+    #[test]\n+    fn expiry_is_checked() {}\n";
    let file_only = "+++ b/tests/auth_roundtrip.rs\n@@ -0,0 +1 @@\n+fn helper() {}\n";
    let spec = "+++ b/web/src/auth.spec.ts\n@@ -1 +1 @@\n+x\n";
    let python = "+++ b/app/auth.py\n@@ -1,0 +2 @@\n+def test_expiry():\n";
    let code_only = "+++ b/src/auth.rs\n@@ -1 +1 @@\n-let a = 1;\n+let a = 2;\n";
    let removed = "+++ b/src/auth.rs\n@@ -3 +3,0 @@\n-    #[test]\n";
    let cases = [
        (rust_inline, true),
        (file_only, true),
        (spec, true),
        (python, true),
        (code_only, false),
        (removed, false),
        ("", false),
    ];
    for (patch, want) in cases {
        assert_eq!(adds_test(patch), want, "{patch}");
    }
    assert!(touches_test_file(&strings(&["src/a.rs", "crates/x/tests/y.rs"])));
    assert!(!touches_test_file(&strings(&["src/a.rs", "docs/testing.md"])));
}
