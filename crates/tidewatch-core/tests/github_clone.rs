//! Cloning from a local bare repo standing in for GitHub, and finding checkouts by their `origin`.
//! Nothing here reaches the network; the real `gh` is never started.

mod common;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::{FakeGh, git_available, ok, repos};
use tidewatch_core::github_clone::{
    CloneError, Landing, clone_from, clone_with_gh_fallback, find_checkout, origin_of, plan,
};
use tidewatch_core::github_link::GithubRepo;
use tidewatch_core::tools::Tools;

fn web() -> GithubRepo {
    GithubRepo::parse("https://github.com/acme/web").unwrap()
}

fn clone_quietly(url: &str, target: &Path) -> Result<Vec<String>, CloneError> {
    let mut lines = Vec::new();
    clone_from(&Tools::system(), url, target, &AtomicBool::new(false), &mut |l| {
        lines.push(l.to_owned());
    })
    .map(|()| lines)
}

#[test]
fn a_clone_from_the_remote_lands_in_the_target_with_its_history() {
    if !git_available() {
        return;
    }
    let r = repos();
    let target = r.root.path().join("code").join("web");
    let lines = clone_quietly(r.remote.to_str().unwrap(), &target).unwrap();
    assert!(target.join("readme.md").is_file());
    assert_eq!(ok(&target, &["log", "--format=%s"]).trim(), "chore: init");
    assert!(lines.iter().any(|line| line.contains("Cloning into")), "{lines:?}");
}

#[test]
fn a_clone_that_fails_says_why_and_leaves_no_folder_behind() {
    if !git_available() {
        return;
    }
    let r = repos();
    let target = r.root.path().join("web");
    let missing = r.root.path().join("nowhere.git");
    let err = clone_quietly(missing.to_str().unwrap(), &target).unwrap_err();
    assert!(matches!(&err, CloneError::Failed(text) if text.starts_with("fatal:")), "{err:?}");
    assert!(!target.exists());
}

#[test]
fn a_cancelled_clone_stops_before_it_starts_and_removes_nothing_it_did_not_make() {
    if !git_available() {
        return;
    }
    let r = repos();
    let target = r.root.path().join("web");
    let cancelled = AtomicBool::new(true);
    let err =
        clone_from(&Tools::system(), r.remote.to_str().unwrap(), &target, &cancelled, &mut |_| {})
            .unwrap_err();
    assert_eq!(err, CloneError::Cancelled);
    assert!(!target.exists());
}

#[test]
fn when_git_fails_the_github_cli_gets_one_try() {
    if !git_available() {
        return;
    }
    let r = repos();
    let target = r.root.path().join("code").join("web");
    let gh = FakeGh::new(&[]);
    let clone_by_gh = format!(
        "#!/bin/sh\n[ \"$1 $2\" = 'repo clone' ] || exit 99\nexec git clone -q '{}' \"$4\"\n",
        r.remote.display()
    );
    std::fs::write(gh.dir.path().join("gh"), clone_by_gh).unwrap();
    let tools = Tools::with_path(gh.path());
    let broken = r.root.path().join("nowhere.git");
    clone_with_gh_fallback(
        &tools,
        broken.to_str().unwrap(),
        &web(),
        &target,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(target.join("readme.md").is_file());
}

#[test]
fn without_gh_the_git_failure_is_what_comes_back() {
    if !git_available() {
        return;
    }
    let r = repos();
    let target = r.root.path().join("web");
    let (_guard, only_git) = common::path_with_only_git();
    let broken = r.root.path().join("nowhere.git");
    let err = clone_with_gh_fallback(
        &Tools::with_path(only_git),
        broken.to_str().unwrap(),
        &web(),
        &target,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(&err, CloneError::Failed(text) if text.starts_with("fatal:")), "{err:?}");
}

fn checkout_of(r: &common::Repos, name: &str, origin: &str) -> std::path::PathBuf {
    let dir = r.root.path().join(name);
    ok(r.root.path(), &["clone", "-q", r.remote.to_str().unwrap(), name]);
    ok(&dir, &["remote", "set-url", "origin", origin]);
    dir
}

#[test]
fn a_checkout_is_found_by_its_origin_whether_https_or_ssh_and_whatever_the_case() {
    if !git_available() {
        return;
    }
    let r = repos();
    let https = checkout_of(&r, "a", "https://github.com/Acme/Web.git");
    let ssh = checkout_of(&r, "b", "git@github.com:acme/web.git");
    let other = checkout_of(&r, "c", "https://github.com/acme/api");
    let tools = Tools::system();
    let origin = origin_of(&tools, &https).unwrap();
    assert_eq!(origin.slug(), "Acme/Web");
    assert!(origin.same_repo(&web()));
    assert_eq!(find_checkout(&tools, &web(), &[other.clone(), ssh.clone(), https]), Some(ssh));
    assert_eq!(find_checkout(&tools, &web(), &[other, r.root.path().join("gone")]), None);
    assert_eq!(origin_of(&tools, &r.root.path().join("gone")), None);
}

#[test]
fn a_known_checkout_wins_over_cloning() {
    if !git_available() {
        return;
    }
    let r = repos();
    let known = checkout_of(&r, "mine", "git@github.com:acme/web.git");
    let parent = r.root.path().join("code");
    assert_eq!(
        plan(&Tools::system(), &web(), &parent, std::slice::from_ref(&known)),
        Landing::Open(known)
    );
}

#[test]
fn a_free_target_is_cloned_into_an_empty_one_too_and_a_foreign_one_is_refused() {
    if !git_available() {
        return;
    }
    let r = repos();
    let tools = Tools::system();
    let parent = r.root.path().join("code");
    let target = parent.join("web");
    assert_eq!(plan(&tools, &web(), &parent, &[]), Landing::Clone { target: target.clone() });
    std::fs::create_dir_all(&target).unwrap();
    assert_eq!(plan(&tools, &web(), &parent, &[]), Landing::Clone { target: target.clone() });
    std::fs::write(target.join("notes.txt"), "mine").unwrap();
    let Landing::Refuse(why) = plan(&tools, &web(), &parent, &[]) else { panic!("not refused") };
    assert!(why.contains("not a checkout of acme/web"), "{why}");
    std::fs::remove_dir_all(&target).unwrap();
    std::fs::write(&target, "a file").unwrap();
    let Landing::Refuse(why) = plan(&tools, &web(), &parent, &[]) else { panic!("not refused") };
    assert!(why.contains("is a file"), "{why}");
}

#[test]
fn a_target_that_is_already_a_checkout_of_the_repo_is_opened_not_cloned() {
    if !git_available() {
        return;
    }
    let r = repos();
    let parent = r.root.path().join("code");
    std::fs::create_dir_all(&parent).unwrap();
    ok(&parent, &["clone", "-q", r.remote.to_str().unwrap(), "web"]);
    ok(&parent.join("web"), &["remote", "set-url", "origin", "https://github.com/acme/web"]);
    assert_eq!(plan(&Tools::system(), &web(), &parent, &[]), Landing::Open(parent.join("web")));
    ok(&parent.join("web"), &["remote", "set-url", "origin", "https://github.com/acme/other"]);
    assert!(matches!(plan(&Tools::system(), &web(), &parent, &[]), Landing::Refuse(_)));
}
