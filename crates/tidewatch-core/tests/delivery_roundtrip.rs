//! What a branch has ahead of its base (D7) against a real bare remote. Tidewatch only reads: these
//! tests also check that nothing is pushed and no `gh` is needed. Skips when `git` is not installed.

mod common;

use common::{commit_file, git_available, ok, remote_branches, repos};
use tidewatch_core::delivery::{Ahead, DeliveryError, detect_base, pushed_as};
use tidewatch_core::tools::Tools;

const BRANCH: &str = "agent/brave-otter";

fn branch_with_two_commits(work: &std::path::Path) {
    ok(work, &["switch", "-q", "-c", BRANCH]);
    commit_file(work, "auth.rs", "fn refresh() {}\n", "feat: add token refresh");
    commit_file(work, "auth_test.rs", "#[test] fn t() {}\n", "test: cover expiry");
}

#[derive(Clone, Copy)]
enum Merge {
    Rebase,
    Squash,
}

fn merge_into_main(r: &common::Repos, how: Merge) -> String {
    let tip = ok(&r.work, &["rev-parse", "HEAD"]).trim().to_owned();
    ok(&r.work, &["push", "-q", "-u", "origin", BRANCH]);
    ok(&r.work, &["switch", "-q", "-c", "merging", "origin/main"]);
    commit_file(&r.work, "other.md", "someone else\n", "docs: someone else");
    match how {
        Merge::Rebase => {
            ok(&r.work, &["cherry-pick", &format!("origin/main..{tip}")]);
        }
        Merge::Squash => {
            ok(&r.work, &["merge", "--squash", &tip]);
            ok(&r.work, &["commit", "-q", "-m", "feat: token refresh (#1)"]);
        }
    }
    ok(&r.work, &["push", "-q", "origin", "merging:main"]);
    ok(&r.work, &["fetch", "-q", "origin"]);
    ok(&r.work, &["switch", "-q", BRANCH]);
    ok(&r.work, &["branch", "-q", "-D", "merging"]);
    ok(&r.work, &["config", "user.name", "t"]);
    ok(&r.work, &["config", "user.email", "t@example.com"]);
    tip
}

fn left_after(r: &common::Repos, merged_head: &str) -> Vec<String> {
    let ahead = Ahead::build_after(&Tools::system(), &r.work, BRANCH, Some(merged_head)).unwrap();
    ahead.commits.into_iter().map(|c| c.subject).collect()
}

#[test]
fn ahead_lists_the_commits_ahead_of_the_base_oldest_first_and_pushes_nothing() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let ahead = Ahead::build(&Tools::system(), &r.work, BRANCH).unwrap();
    assert_eq!(ahead.base, "main");
    let subjects: Vec<_> = ahead.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["feat: add token refresh", "test: cover expiry"]);
    assert_eq!(remote_branches(&r.remote), ["main"], "reading the branch pushes nothing");
}

#[test]
fn an_unpushed_local_main_does_not_leak_into_the_commits_ahead() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH]);
    commit_file(&r.work, "a.rs", "a\n", "feat: only change");
    ok(&r.work, &["switch", "-q", "main"]);
    commit_file(&r.work, "local.rs", "x\n", "chore: local only");
    let ahead = Ahead::build(&Tools::system(), &r.work, BRANCH).unwrap();
    let subjects: Vec<_> = ahead.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["feat: only change"]);
}

#[test]
fn base_comes_from_origin_head_then_main_then_master() {
    if !git_available() {
        return;
    }
    let tools = Tools::system();
    let r = repos();
    ok(&r.work, &["branch", "develop"]);
    ok(&r.work, &["push", "-q", "origin", "develop"]);
    ok(&r.work, &["remote", "set-head", "origin", "develop"]);
    assert_eq!(detect_base(&tools, &r.work).as_deref(), Some("develop"));
    ok(&r.work, &["remote", "set-head", "origin", "-d"]);
    assert_eq!(detect_base(&tools, &r.work).as_deref(), Some("main"));
    ok(&r.work, &["branch", "-q", "-m", "main", "master"]);
    ok(&r.work, &["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(detect_base(&tools, &r.work).as_deref(), Some("master"));
    ok(&r.work, &["branch", "-q", "-m", "master", "trunk"]);
    assert_eq!(detect_base(&tools, &r.work), None);
}

#[test]
fn a_repo_with_no_base_and_an_option_like_branch_are_typed_errors() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["branch", "-q", "-m", "main", "trunk"]);
    ok(&r.work, &["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(Ahead::build(&Tools::system(), &r.work, "trunk"), Err(DeliveryError::NoBase));
    assert_eq!(
        Ahead::build(&Tools::system(), &r.work, "--force"),
        Err(DeliveryError::BadBranch("--force".into()))
    );
}

#[test]
fn a_branch_the_agent_pushed_under_a_new_numbered_name_is_found_as_that_name() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    ok(&r.work, &["push", "-q", "-u", "origin", &format!("{BRANCH}:{BRANCH}-2")]);
    assert_eq!(pushed_as(&Tools::system(), &r.work, BRANCH), format!("{BRANCH}-2"));
}

#[test]
fn a_branch_that_tracks_something_else_is_still_looked_for_as_itself() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH, "origin/main"]);
    ok(&r.work, &["branch", "-q", "--set-upstream-to=origin/main", BRANCH]);
    assert_eq!(pushed_as(&Tools::system(), &r.work, BRANCH), BRANCH);
}

#[test]
fn a_merged_pull_request_leaves_nothing_ahead_however_it_merged() {
    if !git_available() {
        return;
    }
    for how in [Merge::Rebase, Merge::Squash] {
        let r = repos();
        branch_with_two_commits(&r.work);
        let tip = merge_into_main(&r, how);
        assert_eq!(left_after(&r, &tip), Vec::<String>::new());
    }
}

#[test]
fn only_the_commits_made_after_the_merge_are_ahead_however_it_merged() {
    if !git_available() {
        return;
    }
    for how in [Merge::Rebase, Merge::Squash] {
        let r = repos();
        branch_with_two_commits(&r.work);
        let tip = merge_into_main(&r, how);
        commit_file(&r.work, "later.rs", "fn later() {}\n", "fix: after the merge");
        assert_eq!(left_after(&r, &tip), ["fix: after the merge"]);
    }
}

#[test]
fn a_merged_head_the_checkout_does_not_have_falls_back_to_what_the_base_already_holds() {
    if !git_available() {
        return;
    }
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    let r = repos();
    branch_with_two_commits(&r.work);
    merge_into_main(&r, Merge::Rebase);
    assert_eq!(left_after(&r, unknown), Vec::<String>::new());
    commit_file(&r.work, "later.rs", "fn later() {}\n", "fix: after the merge");
    assert_eq!(left_after(&r, unknown), ["fix: after the merge"]);
}
