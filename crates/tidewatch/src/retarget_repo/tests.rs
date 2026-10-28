use std::process::Command;

use super::*;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";

fn call(name: &str, input: &str) -> String {
    format!(
        r#"{{"type":"assistant","cwd":"/ws","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"{name}","input":{input}}}]}}}}"#
    )
}

fn touch(path: &str, act: Act) -> Touch {
    Touch { path: PathBuf::from(path), act }
}

#[test]
fn the_tool_calls_of_a_transcript_name_the_paths_they_touched_oldest_first() {
    let lines = [
        call("Read", r#"{"file_path":"/ws/a/src/lib.rs"}"#),
        call("Edit", r#"{"file_path":"/ws/b/src/main.rs","old_string":"x","new_string":"y"}"#),
        call("Write", r#"{"file_path":"/ws/b/NOTES.md","content":"z"}"#),
        call("NotebookEdit", r#"{"notebook_path":"/ws/b/n.ipynb"}"#),
        call("Bash", r#"{"command":"cd /ws/a && cargo test | tail -5"}"#),
        call("Bash", r#"{"command":"git -C /ws/b status; ls ~/x"}"#),
        call("Bash", r#"{"command":"cd repo-c"}"#),
        call("Grep", r#"{"pattern":"x","path":"/ws/a/src"}"#),
        call("TodoWrite", r#"{"todos":[]}"#),
        r#"{"type":"user","message":{"role":"user","content":"cd /nowhere"}}"#.to_owned(),
        "not json but \"tool_use\"".to_owned(),
        call("Edit", r#"{"file_path":"/ws/z/skipped.rs"}"#)
            .replace(r#""cwd""#, r#""isSidechain":true,"cwd""#),
    ]
    .join("\n");
    assert_eq!(
        touches(&lines, Path::new("/elsewhere")),
        [
            touch("/ws/a/src/lib.rs", Act::Read),
            touch("/ws/b/src/main.rs", Act::Wrote),
            touch("/ws/b/NOTES.md", Act::Wrote),
            touch("/ws/b/n.ipynb", Act::Wrote),
            touch("/ws/a", Act::Ran),
            touch("/ws/b", Act::Ran),
            touch("/ws/repo-c", Act::Ran),
            touch("/ws/a/src", Act::Read),
        ]
    );
}

#[test]
fn a_relative_path_is_read_from_the_session_folder_when_a_record_names_none() {
    let line = call("Read", r#"{"file_path":"b/x.rs"}"#).replace(r#""cwd":"/ws","#, "");
    assert_eq!(touches(&line, Path::new("/home/me/ws")), [touch("/home/me/ws/b/x.rs", Act::Read)]);
}

fn repo_by_prefix(path: &Path) -> Option<PathBuf> {
    ["/ws/a", "/ws/b", "/ws/c"].into_iter().find(|repo| path.starts_with(repo)).map(PathBuf::from)
}

#[test]
fn the_repo_with_the_most_weight_wins_and_newer_work_counts_for_more() {
    let cases: Vec<(&str, Vec<Touch>, Option<&str>)> = vec![
        ("nothing", vec![], None),
        ("nothing in a repo", vec![touch("/tmp/x", Act::Wrote)], None),
        (
            "edits beat reads",
            vec![
                touch("/ws/a/1", Act::Read),
                touch("/ws/a/2", Act::Read),
                touch("/ws/b/1", Act::Wrote),
            ],
            Some("/ws/b"),
        ),
        (
            "many touches beat one",
            vec![
                touch("/ws/a/1", Act::Wrote),
                touch("/ws/b/1", Act::Wrote),
                touch("/ws/b/2", Act::Wrote),
            ],
            Some("/ws/b"),
        ),
        (
            "newest first: a single recent edit beats a long ago one of the same kind",
            vec![touch("/ws/a/1", Act::Wrote), touch("/ws/b/1", Act::Wrote)],
            Some("/ws/b"),
        ),
        (
            "a recent run of edits beats an older pile of reads",
            std::iter::repeat_n(touch("/ws/a/1", Act::Wrote), 4)
                .chain(std::iter::repeat_n(touch("/ws/c/9", Act::Read), 10))
                .chain(std::iter::repeat_n(touch("/ws/b/1", Act::Wrote), 4))
                .collect(),
            Some("/ws/b"),
        ),
        (
            "cd targets count",
            vec![touch("/ws/c", Act::Ran), touch("/ws/c/x", Act::Ran)],
            Some("/ws/c"),
        ),
    ];
    for (what, touched, want) in cases {
        let got = most_worked(&touched, &mut |path| repo_by_prefix(path));
        assert_eq!(got.as_deref(), want.map(Path::new), "{what}");
    }
}

#[test]
fn the_newest_repo_wins_and_equal_times_go_to_the_first_path() {
    let repos = |list: &[(&str, u64)]| -> Vec<(PathBuf, u64)> {
        list.iter().map(|(path, time)| (PathBuf::from(path), *time)).collect()
    };
    assert_eq!(newest(&[]), None);
    assert_eq!(newest(&repos(&[("/ws/a", 5), ("/ws/b", 9), ("/ws/c", 7)])), Some("/ws/b".into()));
    assert_eq!(newest(&repos(&[("/ws/b", 5), ("/ws/a", 5)])), Some("/ws/a".into()));
}

fn git(dir: &Path, args: &[&str], when: &str) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", when)
        .env("GIT_COMMITTER_DATE", when)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn repo(parent: &Path, name: &str, when: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"], when);
    std::fs::write(dir.join("src/lib.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "."], when);
    git(&dir, &["commit", "-q", "-m", "init"], when);
    dir.canonicalize().unwrap()
}

fn transcript(home: &Path, cwd: &Path, lines: &[String]) {
    let folder = home
        .join(".claude/projects")
        .join(tidewatch_core::transcript_locate::claude_project_dir_name(cwd));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(format!("{ID}.jsonl")), lines.join("\n")).unwrap();
}

#[test]
fn a_plain_folder_resolves_to_the_repo_its_session_edited_and_else_the_newest_repo() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let ws = home.join("ws");
    let older = repo(&ws, "older", "2026-01-01T00:00:00");
    let newer = repo(&ws, "newer", "2026-06-01T00:00:00");
    let claude = home.join(".claude");

    let edits =
        |repo: &Path| call("Edit", &format!(r#"{{"file_path":"{}/src/lib.rs"}}"#, repo.display()));
    transcript(&home, &ws, &[edits(&older), edits(&older)]);
    assert_eq!(worked_in(&claude, &ws, Some(ID)), Some(older.clone()), "the edited repo");

    assert_eq!(worked_in(&claude, &ws, None), Some(newer.clone()), "no session: newest commit");
    transcript(&home, &ws, &[call("TodoWrite", "{}")]);
    assert_eq!(worked_in(&claude, &ws, Some(ID)), Some(newer), "no evidence: newest commit");

    let empty = home.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(worked_in(&claude, &empty, Some(ID)), None, "no repo at all");
}

#[test]
fn a_path_in_a_linked_worktree_counts_for_its_main_repo() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let main = repo(&home, "main", "2026-01-01T00:00:00");
    let tree = home.join("main-tree");
    git(
        &main,
        &["worktree", "add", "-q", "-b", "side", tree.to_str().unwrap()],
        "2026-01-02T00:00:00",
    );
    let tree = tree.canonicalize().unwrap();
    let text = call("Edit", &format!(r#"{{"file_path":"{}/src/lib.rs"}}"#, tree.display()));
    let got = most_worked(&touches(&text, &home), &mut |path| repo_of_path(path));
    assert_eq!(got, Some(main));
}
