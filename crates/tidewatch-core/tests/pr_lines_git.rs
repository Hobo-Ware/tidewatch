//! `PrDiffLines::load` against a real throwaway repo with a bare remote.

mod common;

use kelp_core::review::Side;
use tidewatch_core::pr_lines::PrDiffLines;
use tidewatch_core::tools::Tools;

#[test]
fn the_branch_diff_against_the_remote_base_says_which_lines_take_a_comment() {
    if !common::git_available() {
        return;
    }
    let repos = common::repos();
    let work = &repos.work;
    let text = "line\n"
        .repeat(30)
        .lines()
        .enumerate()
        .map(|(n, _)| format!("line {}", n + 1))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    common::commit_file(work, "a.txt", &text, "chore: a");
    common::ok(work, &["push", "-q", "origin", "main"]);
    common::ok(work, &["checkout", "-q", "-b", "agent/x"]);
    common::commit_file(work, "a.txt", &text.replace("line 15\n", "changed 15\n"), "fix: 15");
    let tools = Tools::system();
    let lines = PrDiffLines::load(&tools, work, "agent/x").unwrap();
    assert!(lines.contains("a.txt", Side::New, 15));
    assert!(lines.contains("a.txt", Side::New, 13));
    assert!(!lines.contains("a.txt", Side::New, 2));
    assert!(!lines.contains("readme.md", Side::New, 1));
}

#[test]
fn a_bad_head_is_an_error_not_an_empty_diff() {
    if !common::git_available() {
        return;
    }
    let repos = common::repos();
    let tools = Tools::system();
    assert!(PrDiffLines::load(&tools, &repos.work, "no-such-branch").is_err());
    assert!(PrDiffLines::load(&tools, &repos.work, "--evil").is_err());
}
