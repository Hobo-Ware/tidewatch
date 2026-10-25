//! What a poofed Meeseeks left on its branch, read from the main checkout after the worktree is
//! gone. Skips when `git` is not installed.

mod common;

use common::{commit_file, git_available, ok, repos};
use tidewatch_core::meeseeks_branch::{Branch, Changed, Found, read};
use tidewatch_core::tools::Tools;

const BRANCH: &str = "agent/promo-test";

#[test]
fn a_branch_with_two_commits_reports_them_and_the_lines_it_changed() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH]);
    commit_file(&r.work, "promo.rs", "a\nb\nc\n", "feat: add promo");
    commit_file(&r.work, "readme.md", "hello\nmore\n", "docs: more");
    ok(&r.work, &["switch", "-q", "main"]);
    let got = read(&Tools::system(), &r.work, BRANCH);
    let want = Found {
        base: "main".into(),
        ahead: 2,
        changed: Changed { files: 2, added: 4, removed: 0 },
    };
    assert_eq!(got, Branch::Found(want));
}

#[test]
fn a_branch_that_is_gone_says_so() {
    if !git_available() {
        return;
    }
    let r = repos();
    assert_eq!(read(&Tools::system(), &r.work, BRANCH), Branch::Missing);
}

#[test]
fn a_branch_with_nothing_ahead_has_no_files() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["branch", BRANCH]);
    let got = read(&Tools::system(), &r.work, BRANCH);
    let want = Found { base: "main".into(), ahead: 0, changed: Changed::default() };
    assert_eq!(got, Branch::Found(want));
}
