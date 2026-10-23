//! The push guard (D7) against real git: a bare remote, a clone, and the environment that
//! `launch_command` builds for an agent worktree tab. Skips when `git` is not installed.

use std::path::Path;
use std::process::{Command, Output};

mod common;

use tidewatch_core::agents::{AgentTable, ArgList, Level};
use tidewatch_core::guard_shim::{GuardCtx, shims};
use tidewatch_core::launch::{LaunchCtx, NO_PUSH_MESSAGE, ensure_no_push_helper, launch_command};
use tidewatch_core::profiles::Profile;

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn base_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        std::env::vars().filter(|(k, _)| !k.starts_with("GIT_")).collect();
    env.extend([
        ("GIT_AUTHOR_NAME".into(), "t".into()),
        ("GIT_AUTHOR_EMAIL".into(), "t@example.com".into()),
        ("GIT_COMMITTER_NAME".into(), "t".into()),
        ("GIT_COMMITTER_EMAIL".into(), "t@example.com".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), "init.defaultBranch".into()),
        ("GIT_CONFIG_VALUE_0".into(), "main".into()),
    ]);
    env
}

fn git(dir: &Path, env: &[(String, String)], args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .args(args)
        .output()
        .expect("git is installed because git_available() passed")
}

fn ok(dir: &Path, env: &[(String, String)], args: &[&str]) -> String {
    let out = git(dir, env, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ref_lines(remote: &Path, env: &[(String, String)]) -> Vec<String> {
    ok(remote, env, &["for-each-ref"]).lines().map(str::to_owned).collect()
}

fn guarded_env(
    base: &[(String, String)],
    helper_dir: Option<&Path>,
    push_branch: Option<&str>,
) -> Vec<(String, String)> {
    let table = AgentTable::builtin();
    let profile = Profile {
        name: "p".into(),
        agent: "claude".into(),
        model: None,
        effort: None,
        level: Level::Edits,
        setup: String::new(),
        never_push: true,
        extra_args: ArgList::default(),
        house_style: true,
    };
    let ctx = LaunchCtx {
        worktree_tab: true,
        guard_bin_dir: helper_dir.map(Path::to_path_buf),
        push_branch: push_branch.map(str::to_owned),
        env: base.to_vec(),
        ..LaunchCtx::default()
    };
    let launch = launch_command(table.get("claude").unwrap(), &profile, &ctx).unwrap();
    let mut env: Vec<(String, String)> =
        base.iter().filter(|(k, _)| !launch.env.iter().any(|(lk, _)| lk == k)).cloned().collect();
    env.extend(launch.env);
    env
}

const OWN: &str = "agent/brave-otter";

struct Repo {
    _root: tempfile::TempDir,
    remote: std::path::PathBuf,
    work: std::path::PathBuf,
    helper_dir: std::path::PathBuf,
    normal: Vec<(String, String)>,
}

impl Repo {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        let work = root.path().join("work");
        let helper_dir = root.path().join("guard-bin");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        ensure_no_push_helper(&helper_dir).unwrap();
        let normal = base_env();
        ok(&remote, &normal, &["init", "--bare", "-q"]);
        ok(&work, &normal, &["init", "-q"]);
        ok(&work, &normal, &["remote", "add", "origin", remote.to_str().unwrap()]);
        ok(&work, &normal, &["commit", "-q", "--allow-empty", "-m", "one"]);
        ok(&work, &normal, &["push", "-q", "origin", "HEAD:main"]);
        ok(&work, &normal, &["branch", "other"]);
        ok(&work, &normal, &["checkout", "-q", "-b", OWN]);
        Self { _root: root, remote, work, helper_dir, normal }
    }

    fn guarded(&self) -> Vec<(String, String)> {
        guarded_env(&self.normal, Some(&self.helper_dir), Some(OWN))
    }

    fn remote_refs(&self) -> Vec<String> {
        ref_lines(&self.remote, &self.normal)
    }

    fn remote_sha(&self, name: &str) -> Option<String> {
        let out = git(&self.remote, &self.normal, &["rev-parse", "--verify", "-q", name]);
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn head(&self) -> String {
        ok(&self.work, &self.normal, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn rewrite_head(&self) {
        ok(&self.work, &self.normal, &["commit", "-q", "--amend", "--allow-empty", "-m", "folded"]);
    }
}

#[test]
fn an_agent_pushes_its_own_branch_in_every_form_including_force() {
    if !git_available() {
        eprintln!("git not found; skipping");
        return;
    }
    let repo = Repo::new();
    let env = repo.guarded();
    assert_eq!(
        env.iter().find(|(k, _)| k == "GIT_CONFIG_COUNT").map(|(_, v)| v.as_str()),
        Some("2"),
        "the guard is appended after the existing config entry"
    );
    let main_before = repo.remote_sha("main");
    let url = repo.remote.to_str().unwrap().to_owned();
    let own_ref = format!("refs/heads/{OWN}");
    let plus = format!("+{OWN}");
    let to_url = format!("HEAD:{own_ref}");
    let steps: [(&str, Vec<&str>, bool); 7] = [
        ("push to the explicit url", vec!["push", &url, &to_url], false),
        ("plain push", vec!["push", "origin", OWN], false),
        ("push -u", vec!["push", "-u", "origin", OWN], false),
        ("push HEAD", vec!["push", "origin", "HEAD"], false),
        ("push --force after a fold", vec!["push", "--force", "origin", OWN], true),
        (
            "push --force-with-lease after a fold",
            vec!["push", "--force-with-lease", "origin", OWN],
            true,
        ),
        ("push +refspec after a fold", vec!["push", "origin", &plus], true),
    ];
    for (label, args, fold) in steps {
        if fold {
            repo.rewrite_head();
        } else {
            ok(&repo.work, &repo.normal, &["commit", "-q", "--allow-empty", "-m", label]);
        }
        let out = git(&repo.work, &env, &args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{label} must work: {stderr}");
        assert_eq!(repo.remote_sha(&own_ref), Some(repo.head()), "{label} moved the branch");
    }
    assert_eq!(repo.remote_sha("main"), main_before, "main never moved");
    let tracking = ok(&repo.work, &repo.normal, &["rev-parse", &format!("origin/{OWN}")]);
    assert_eq!(tracking.trim(), repo.head(), "the remote-tracking branch follows the pushes");
}

#[test]
fn every_other_push_is_refused_with_the_tidewatch_message_and_fetch_still_works() {
    if !git_available() {
        eprintln!("git not found; skipping");
        return;
    }
    let repo = Repo::new();
    let env = repo.guarded();
    ok(&repo.work, &env, &["push", "-q", "origin", OWN]);
    ok(&repo.work, &repo.normal, &["tag", "v1"]);
    ok(&repo.work, &repo.normal, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let before = repo.remote_refs();
    let url = repo.remote.to_str().unwrap().to_owned();
    let delete_own = format!(":{OWN}");
    let attempts: [(&str, Vec<&str>); 15] = [
        ("push to main", vec!["push", "origin", "HEAD:main"]),
        ("push --no-verify to main", vec!["push", "--no-verify", "origin", "HEAD:main"]),
        ("push --force to main", vec!["push", "--force", "origin", "HEAD:main"]),
        (
            "push --force-with-lease to main",
            vec!["push", "--force-with-lease", "origin", "HEAD:main"],
        ),
        ("push +refspec to main", vec!["push", "origin", "+HEAD:main"]),
        ("push another branch", vec!["push", "origin", "other"]),
        ("push a new branch", vec!["push", "origin", "HEAD:refs/heads/new-one"]),
        ("push to the explicit url", vec!["push", &url, "HEAD:refs/heads/explicit"]),
        ("push --delete own branch", vec!["push", "origin", "--delete", OWN]),
        ("push :own branch", vec!["push", "origin", &delete_own]),
        ("push a tag", vec!["push", "origin", "v1"]),
        ("push --tags", vec!["push", "origin", "--tags"]),
        ("push --all", vec!["push", "--all", "origin"]),
        ("push --mirror", vec!["push", "--mirror", "origin"]),
        ("push own branch with a tag in the batch", vec!["push", "origin", OWN, "v1"]),
    ];
    for (label, args) in attempts {
        let out = git(&repo.work, &env, &args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        println!("{label}: exit {:?}: {}", out.status.code(), stderr.trim().replace('\n', " | "));
        assert!(!out.status.success(), "{label} must fail");
        assert!(stderr.contains(NO_PUSH_MESSAGE), "{label} shows the Tidewatch message: {stderr}");
        assert_eq!(repo.remote_refs(), before, "{label} changed nothing on the remote");
    }
    let fetch = git(&repo.work, &env, &["fetch", "origin"]);
    assert!(fetch.status.success(), "fetch works: {}", String::from_utf8_lossy(&fetch.stderr));
}

#[test]
fn without_a_branch_or_a_helper_nothing_can_be_pushed_and_a_normal_shell_still_pushes() {
    if !git_available() {
        eprintln!("git not found; skipping");
        return;
    }
    let repo = Repo::new();
    ok(&repo.work, &repo.normal, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let before = repo.remote_refs();
    let no_branch = guarded_env(&repo.normal, Some(&repo.helper_dir), None);
    let out = git(&repo.work, &no_branch, &["push", "origin", OWN]);
    assert!(!out.status.success(), "with no branch named, even an agent/ branch is refused");
    let no_helper = guarded_env(&repo.normal, None, Some(OWN));
    let out = git(&repo.work, &no_helper, &["push", "origin", OWN]);
    assert!(!out.status.success(), "the guard holds even when the helper script is missing");
    assert_eq!(repo.remote_refs(), before);

    ok(&repo.work, &repo.normal, &["push", "-q", "origin", "HEAD:main"]);
    assert_eq!(repo.remote_sha("main"), Some(repo.head()), "a push from a normal shell moved main");
}

#[test]
fn the_push_rule_holds_with_the_outbound_shims_in_front_of_git() {
    if !git_available() {
        eprintln!("git not found; skipping");
        return;
    }
    let repo = Repo::new();
    for (name, content) in shims() {
        common::write_executable_from_a_child(&repo.helper_dir.join(name), &content);
    }
    let mut env = repo.guarded();
    env.extend(
        GuardCtx {
            bin: "/nonexistent/tidewatch".into(),
            config: repo.helper_dir.join("settings.toml"),
            repo: repo.work.clone(),
            cache_dir: repo.helper_dir.join("cache"),
            tab_dir: repo.helper_dir.join("tab"),
        }
        .env(&repo.helper_dir),
    );
    ok(&repo.work, &env, &["commit", "-q", "--allow-empty", "-m", "three"]);
    ok(&repo.work, &env, &["push", "origin", OWN]);
    assert_eq!(repo.remote_sha(&format!("refs/heads/{OWN}")), Some(repo.head()));

    let before = repo.remote_refs();
    let out = git(&repo.work, &env, &["push", "origin", "HEAD:main"]);
    assert!(!out.status.success(), "main stays refused");
    assert!(String::from_utf8_lossy(&out.stderr).contains(NO_PUSH_MESSAGE));
    assert_eq!(repo.remote_refs(), before);
}
