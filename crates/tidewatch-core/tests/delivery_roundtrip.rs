//! Delivery (D7) against a real bare remote and a fake `gh` that records its calls. Skips when
//! `git` is not installed. The real `gh` is never started.

mod common;

use common::{
    FakeGh, commit_file, git_available, ok, path_with_only_git, remote_branches, reply, repos,
};
use tidewatch_core::delivery::{Delivered, DeliveryError, DeliveryPlan, deliver, detect_base};
use tidewatch_core::tools::Tools;

const BRANCH: &str = "agent/brave-otter";
const URL: &str = "https://github.com/acme/web/pull/42";

fn branch_with_two_commits(work: &std::path::Path) {
    ok(work, &["switch", "-q", "-c", BRANCH]);
    commit_file(work, "auth.rs", "fn refresh() {}\n", "feat: add token refresh");
    commit_file(work, "auth_test.rs", "#[test] fn t() {}\n", "test: cover expiry");
}

fn created(url: &str) -> common::Reply {
    reply("pr create", 0, Box::leak(format!("{url}\n").into_boxed_str()), "")
}

#[test]
fn plan_lists_the_commits_ahead_of_the_base_oldest_first() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let plan = DeliveryPlan::build(&Tools::system(), &r.work, BRANCH, None).unwrap();
    assert_eq!(plan.base, "main");
    assert_eq!(plan.branch, BRANCH);
    let subjects: Vec<_> = plan.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["feat: add token refresh", "test: cover expiry"]);
    assert_eq!(plan.title, "feat: add token refresh");
    assert_eq!(plan.body, "## Changes\n\n- feat: add token refresh\n- test: cover expiry");
}

#[test]
fn plan_uses_the_stage_summary_for_the_text() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let summary = "Refresh tokens early\n\nRefresh five minutes before expiry.";
    let plan = DeliveryPlan::build(&Tools::system(), &r.work, BRANCH, Some(summary)).unwrap();
    assert_eq!(plan.title, "Refresh tokens early");
    assert_eq!(plan.body, "Refresh five minutes before expiry.");
}

#[test]
fn an_unpushed_local_main_does_not_leak_into_the_plan() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH]);
    commit_file(&r.work, "a.rs", "a\n", "feat: only change");
    ok(&r.work, &["switch", "-q", "main"]);
    commit_file(&r.work, "local.rs", "x\n", "chore: local only");
    let plan = DeliveryPlan::build(&Tools::system(), &r.work, BRANCH, None).unwrap();
    let subjects: Vec<_> = plan.commits.iter().map(|c| c.subject.as_str()).collect();
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
fn deliver_pushes_the_branch_then_opens_the_pull_request() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[created(URL)]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();

    let delivered = deliver(&tools, &plan, "Refresh tokens early", "Body\n\n- tested").unwrap();

    assert_eq!(delivered, Delivered { url: URL.into(), created: true });
    assert_eq!(remote_branches(&r.remote), [BRANCH, "main"]);
    assert_eq!(
        ok(&r.work, &["rev-parse", "--abbrev-ref", &format!("{BRANCH}@{{upstream}}")]).trim(),
        format!("origin/{BRANCH}")
    );
    let create = gh.calls_to("pr create");
    assert_eq!(create.len(), 1);
    let args = &create[0];
    assert_eq!(
        args[..9],
        [
            "pr",
            "create",
            "--base",
            "main",
            "--head",
            BRANCH,
            "--title",
            "Refresh tokens early",
            "--body-file"
        ]
    );
    assert_eq!(args.len(), 10);
    assert_eq!(gh.body_file_text().as_deref(), Some("Body\n\n- tested"));
    assert!(!std::path::Path::new(&args[9]).exists(), "the temp body file is removed");
}

#[test]
fn deliver_sends_the_approved_text_not_the_proposal() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[created(URL)]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_ne!(plan.title, "My own title");
    deliver(&tools, &plan, "  My own title  ", "").unwrap();
    assert_eq!(gh.calls_to("pr create")[0][7], "My own title");
    assert_eq!(gh.body_file_text().as_deref(), Some(""));
}

#[test]
fn a_repo_without_origin_is_no_remote_and_nothing_is_created() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[created(URL)]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    ok(&r.work, &["remote", "remove", "origin"]);
    assert_eq!(deliver(&tools, &plan, "t", "b"), Err(DeliveryError::NoRemote));
    assert_eq!(gh.calls_to("pr create"), Vec::<Vec<String>>::new());
}

#[test]
fn a_missing_gh_fails_before_anything_is_pushed() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let (_keep, path) = path_with_only_git();
    let tools = Tools::with_path(path);
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(deliver(&tools, &plan, "t", "b"), Err(DeliveryError::GhMissing));
    assert_eq!(remote_branches(&r.remote), ["main"]);
}

#[test]
fn a_gh_without_a_login_is_reported_after_the_push() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[reply(
        "pr create",
        4,
        "",
        "To get started with GitHub CLI, please run:  gh auth login\nAlternatively, populate the GH_TOKEN environment variable\n",
    )]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(deliver(&tools, &plan, "t", "b"), Err(DeliveryError::GhNotAuthenticated));
    assert_eq!(remote_branches(&r.remote), [BRANCH, "main"]);
}

#[test]
fn a_rejected_push_stops_before_gh_creates_anything() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let rival = r.root.path().join("rival");
    ok(r.root.path(), &["clone", "-q", r.remote.to_str().unwrap(), "rival"]);
    ok(&rival, &["switch", "-q", "-c", BRANCH]);
    commit_file(&rival, "other.rs", "x\n", "feat: somebody else got here first");
    ok(&rival, &["push", "-q", "origin", BRANCH]);

    let gh = FakeGh::new(&[created(URL)]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    match deliver(&tools, &plan, "t", "b") {
        Err(DeliveryError::PushRejected(message)) => assert!(message.contains("rejected")),
        other => panic!("expected a rejected push, got {other:?}"),
    }
    assert_eq!(gh.calls_to("pr create"), Vec::<Vec<String>>::new());
}

#[test]
fn an_existing_pull_request_is_updated_by_the_push_and_returned() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[reply(
        "pr create",
        1,
        "",
        "a pull request for branch \"agent/brave-otter\" into branch \"main\" already exists:\nhttps://github.com/acme/web/pull/7\n",
    )]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(
        deliver(&tools, &plan, "t", "b"),
        Ok(Delivered { url: "https://github.com/acme/web/pull/7".into(), created: false })
    );
}

#[test]
fn a_remote_that_is_not_github_is_its_own_error() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[reply(
        "pr create",
        1,
        "",
        "none of the git remotes configured for this repository point to a known GitHub host.\n",
    )]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(deliver(&tools, &plan, "t", "b"), Err(DeliveryError::NoGithubRemote));
}

#[test]
fn another_gh_failure_keeps_ghs_message() {
    if !git_available() {
        return;
    }
    let r = repos();
    branch_with_two_commits(&r.work);
    let gh = FakeGh::new(&[reply("pr create", 1, "", "GraphQL: Base ref must be a branch\n")]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(
        deliver(&tools, &plan, "t", "b"),
        Err(DeliveryError::PrFailed("GraphQL: Base ref must be a branch".into()))
    );
}

#[test]
fn an_empty_title_and_an_empty_branch_are_refused_without_running_anything() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH]);
    let gh = FakeGh::new(&[created(URL)]);
    let tools = Tools::with_path(gh.path());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(plan.commits, []);
    assert_eq!(deliver(&tools, &plan, "x", "y"), Err(DeliveryError::NothingToDeliver));
    commit_file(&r.work, "a.rs", "a\n", "feat: a");
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();
    assert_eq!(deliver(&tools, &plan, "   ", "y"), Err(DeliveryError::EmptyTitle));
    assert_eq!(gh.calls(), Vec::<Vec<String>>::new());
    assert_eq!(remote_branches(&r.remote), ["main"]);
}
