//! The never-push guard (D7) against real git: a bare remote, a clone, and the environment that
//! `launch_command` builds for an agent worktree tab. Skips when `git` is not installed.

use std::path::Path;
use std::process::{Command, Output};

use tidewatch_core::agents::{AgentTable, ArgList, Level};
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

fn guarded_env(base: &[(String, String)], helper_dir: Option<&Path>) -> Vec<(String, String)> {
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
        env: base.to_vec(),
        ..LaunchCtx::default()
    };
    let launch = launch_command(table.get("claude").unwrap(), &profile, &ctx).unwrap();
    let mut env: Vec<(String, String)> =
        base.iter().filter(|(k, _)| !launch.env.iter().any(|(lk, _)| lk == k)).cloned().collect();
    env.extend(launch.env);
    env
}

#[test]
fn agent_tab_cannot_push_by_any_route_but_can_fetch_and_a_normal_shell_still_pushes() {
    if !git_available() {
        eprintln!("git not found; skipping");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote.git");
    let work = root.path().join("work");
    let helper_dir = root.path().join("guard-bin");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    ensure_no_push_helper(&helper_dir).unwrap();

    let normal = base_env();
    let guarded = guarded_env(&normal, Some(&helper_dir));
    assert_eq!(
        guarded.iter().find(|(k, _)| k == "GIT_CONFIG_COUNT").map(|(_, v)| v.as_str()),
        Some("2"),
        "the guard is appended after the existing config entry"
    );

    ok(&remote, &normal, &["init", "--bare", "-q"]);
    ok(&work, &normal, &["init", "-q"]);
    ok(&work, &normal, &["remote", "add", "origin", remote.to_str().unwrap()]);
    ok(&work, &normal, &["commit", "-q", "--allow-empty", "-m", "one"]);
    ok(&work, &normal, &["push", "-q", "origin", "HEAD:main"]);
    let before = ref_lines(&remote, &normal);
    assert_eq!(before.len(), 1, "setup pushed exactly one ref: {before:?}");

    ok(&work, &normal, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let remote_url = remote.to_str().unwrap();
    let attempts: [(&str, Vec<&str>); 5] = [
        ("plain push", vec!["push", "origin", "HEAD:main"]),
        ("push --no-verify", vec!["push", "--no-verify", "origin", "HEAD:main"]),
        ("push to an explicit url", vec!["push", remote_url, "HEAD:refs/heads/explicit"]),
        ("push a new branch", vec!["push", "origin", "HEAD:refs/heads/other"]),
        ("push --force", vec!["push", "--force", "origin", "HEAD:main"]),
    ];
    for (label, args) in attempts {
        let out = git(&work, &guarded, &args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        println!("{label}: exit {:?}: {}", out.status.code(), stderr.trim().replace('\n', " | "));
        assert!(!out.status.success(), "{label} must fail");
        assert!(stderr.contains(NO_PUSH_MESSAGE), "{label} shows the Tidewatch message: {stderr}");
    }

    let fetch = git(&work, &guarded, &["fetch", "origin"]);
    assert!(fetch.status.success(), "fetch works: {}", String::from_utf8_lossy(&fetch.stderr));
    assert_eq!(ref_lines(&remote, &normal), before, "nothing landed on the remote");

    let without_helper = guarded_env(&normal, None);
    let out = git(&work, &without_helper, &["push", "origin", "HEAD:main"]);
    assert!(!out.status.success(), "the guard holds even when the helper script is missing");
    assert_eq!(ref_lines(&remote, &normal), before);

    ok(&work, &normal, &["push", "-q", "origin", "HEAD:main"]);
    let after = ref_lines(&remote, &normal);
    assert_eq!(after.len(), 1);
    assert_ne!(after, before, "a push from a normal shell moved main");
}
