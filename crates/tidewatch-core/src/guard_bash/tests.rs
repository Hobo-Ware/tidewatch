use super::*;

fn refused(command: &str) -> bool {
    refusal(command, None).is_some()
}

#[test]
fn a_gh_call_with_a_trailer_or_footer_is_refused() {
    let table = [
        ("gh pr create --title t --body \"Fixes it.\n\nCo-Authored-By: Claude <x@y.z>\"", true),
        ("gh pr create -t t -b 'Done.\n\nGenerated with [Claude Code](https://claude.com)'", true),
        (
            "gh pr create -t t -b 'Done.\n\n🤖 Generated with [Claude Code](https://claude.com)'",
            true,
        ),
        ("gh pr create --body='a\nco-authored-by: Bot <b@c.d>'", true),
        ("gh pr edit 12 --body 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh pr comment 12 --body 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh pr review 12 --approve --body 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh issue create --title t --body 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh issue comment 3 -b 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh issue edit 3 --body 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("cd /w && gh pr create -t t -b 'a\nCo-Authored-By: Bot <b@c.d>' && echo ok", true),
        ("/opt/homebrew/bin/gh pr create -t t -b 'a\nCo-Authored-By: Bot <b@c.d>'", true),
        ("gh pr create --title 'Drop Co-Authored-By: lines' --body 'plain'", false),
        ("gh pr create --title t --body 'plain text'", false),
        ("gh pr view 12 --json body", false),
        ("gh pr list --search 'Co-Authored-By:'", false),
        ("echo 'Co-Authored-By: Bot <b@c.d>' > notes.txt", false),
        ("git commit -m 'a\n\nCo-Authored-By: Bot <b@c.d>'", false),
        ("gh pr create --title 't", false),
    ];
    for (command, want) in table {
        assert_eq!(refused(command), want, "{command}");
    }
}

#[test]
fn a_heredoc_body_is_read_in_both_shapes() {
    let substituted = "gh pr create --title t --body \"$(cat <<'EOF'\nSummary.\n\nCo-Authored-By: Bot <b@c.d>\nEOF\n)\"";
    let from_stdin = "gh pr create --title t --body-file - <<'EOF'\nSummary.\n\nGenerated with [Claude Code](x)\nEOF";
    let clean = "gh pr create --title t --body \"$(cat <<'EOF'\nSummary only.\nEOF\n)\"";
    let unrelated = "cat > notes.txt <<'EOF'\nCo-Authored-By: Bot <b@c.d>\nEOF\ngh pr create --title t --body plain";
    assert!(refused(substituted));
    assert!(refused(from_stdin));
    assert!(!refused(clean));
    assert!(!refused(unrelated));
}

#[test]
fn a_body_file_is_read_from_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("body.md"), "Summary.\nCo-Authored-By: Bot <b@c.d>\n").unwrap();
    std::fs::write(dir.path().join("clean.md"), "Summary.\n").unwrap();
    assert!(refusal("gh pr create -t t --body-file body.md", Some(dir.path())).is_some());
    assert!(refusal("gh pr create -t t -F body.md", Some(dir.path())).is_some());
    assert!(refusal("gh pr create -t t --body-file clean.md", Some(dir.path())).is_none());
    assert!(refusal("gh pr create -t t --body-file missing.md", Some(dir.path())).is_none());
}

#[test]
fn the_reason_is_one_line_that_says_where_and_what_to_do() {
    let reason =
        refusal("gh pr create --title t --body 'a\nb\nCo-Authored-By: Bot <b@c.d>'", None).unwrap();
    assert_eq!(reason.lines().count(), 1);
    assert!(
        reason.contains("the body has a Co-Authored-By trailer at line 3, column 1"),
        "{reason}"
    );
    assert!(reason.contains("Drop that line"), "{reason}");
}

fn env_with(rules: &str) -> (tempfile::TempDir, GuardEnv) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("settings.toml");
    std::fs::write(&config, rules).unwrap();
    let env = GuardEnv {
        config,
        repo: "/work/web".into(),
        cache: None,
        shim_dir: PathBuf::new(),
        tab: None,
        path: std::ffi::OsString::new(),
    };
    (dir, env)
}

fn payload(tool: &str, command: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": { "command": command },
        "cwd": "/work/web",
    })
    .to_string()
}

#[test]
fn the_hook_exits_2_for_a_trailer_and_0_for_everything_else() {
    let (_dir, env) = env_with("");
    let bad = "gh pr create -t t -b 'a\nCo-Authored-By: Bot <b@c.d>'";
    assert_eq!(run(&payload("Bash", bad), &env), 2);
    assert_eq!(run(&payload("Bash", "gh pr create -t t -b plain"), &env), 0);
    assert_eq!(run(&payload("Write", bad), &env), 0);
    assert_eq!(run("not json", &env), 0);
    assert_eq!(run(r#"{"tool_name":"Bash","tool_input":{}}"#, &env), 0);
}

#[test]
fn the_hook_follows_the_trailers_switch_and_the_repo_answer() {
    let bad = payload("Bash", "gh pr create -t t -b 'a\nCo-Authored-By: Bot <b@c.d>'");
    let (_off, env) = env_with("[guards]\ntrailers = false\n");
    assert_eq!(run(&bad, &env), 0);
    let (_own, env) = env_with("[guards.repos.\"/work/web\"]\ntrailers = false\n");
    assert_eq!(run(&bad, &env), 0);
}
