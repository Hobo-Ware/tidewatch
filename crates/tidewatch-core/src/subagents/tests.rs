use std::io::Write;
use std::time::{Duration, SystemTime};

use super::*;

const RUNNING: &str = include_str!("../../tests/fixtures/subagent_running.jsonl");
const FINISHED: &str = include_str!("../../tests/fixtures/subagent_finished.jsonl");
const RUNNING_META: &str = include_str!("../../tests/fixtures/subagent_running.meta.json");
const FINISHED_META: &str = include_str!("../../tests/fixtures/subagent_finished.meta.json");

fn at(text: &str) -> i64 {
    unix_millis(text).unwrap()
}

fn fold(text: &str) -> Fold {
    let mut fold = Fold::default();
    for line in text.lines() {
        fold.line(line);
    }
    fold
}

fn assistant(id: &str, stop: &str, content: &str) -> String {
    format!(
        r#"{{"type":"assistant","isSidechain":true,"timestamp":"2026-11-01T09:00:00.000Z","message":{{"id":"{id}","model":"m","role":"assistant","stop_reason":"{stop}","content":[{content}],"usage":{{"input_tokens":1,"output_tokens":2}}}}}}"#
    )
}

fn tool(name: &str, input: &str) -> String {
    assistant(
        "msg_1",
        "tool_use",
        &format!(r#"{{"type":"tool_use","id":"t","name":"{name}","input":{input}}}"#),
    )
}

fn now_of(line: &str) -> Option<String> {
    let mut fold = Fold::default();
    fold.line(line);
    fold.now().map(str::to_owned)
}

#[test]
fn a_meta_file_without_finished_is_a_running_agent() {
    let running = parse_meta(RUNNING_META).unwrap();
    assert_eq!(
        running,
        Meta {
            description: "Build hover actions".into(),
            model: Some("sonnet".into()),
            effort: Some("high".into()),
            finished: false,
        }
    );
    assert_eq!(parse_meta(FINISHED_META).map(|meta| meta.finished), Some(true));
}

#[test]
fn a_meta_file_takes_what_it_has() {
    let cases = [
        (r#"{"description":"  two\n lines "}"#, Some(("two lines", None, false))),
        (
            r#"{"description":"x","model":"","effort":"low","finished":false}"#,
            Some(("x", None, false)),
        ),
        (r#"{"agentType":"Explore","finished":true}"#, Some(("", None, true))),
        (r#"{"description":"x","finished":"yes"}"#, Some(("x", None, false))),
        ("[]", None),
        ("not json", None),
        ("", None),
    ];
    for (text, want) in cases {
        let got = parse_meta(text);
        let got = got
            .as_ref()
            .map(|meta| (meta.description.as_str(), meta.model.as_deref(), meta.finished));
        assert_eq!(got, want, "meta {text:?}");
    }
}

#[test]
fn the_running_fixture_folds_to_its_newest_call_its_times_and_its_tokens() {
    let running = fold(RUNNING);
    assert!(!running.ended());
    assert_eq!(running.first_ms, Some(at("2026-11-01T08:22:50.803Z")));
    assert_eq!(running.last_ms, Some(at("2026-11-01T08:23:02.776Z")));
    assert_eq!(
        running.now(),
        Some(
            "Bash grep -n \"bubble_actions\\|NotSeen\\|fn is_fresh\\|Meta::\" crates/tidewatch/src/reader_chat_view.rs..."
        )
    );
    assert_eq!(running.tokens(), 10 + 167_475 + 477_948 + 1_302);
}

#[test]
fn the_finished_fixture_ends_on_its_last_message_and_counts_each_message_once() {
    let finished = fold(FINISHED);
    assert!(finished.ended());
    assert_eq!(finished.last_ms, Some(at("2026-11-01T08:48:09.825Z")));
    assert_eq!(finished.tokens(), 12 + 168_274 + 798_680 + 2_289);
}

#[test]
fn what_it_does_now_follows_the_rules_in_order() {
    let cases = [
        (
            tool("Bash", r#"{"description":"Run  the\n tests","command":"cargo test"}"#),
            "Run the tests",
        ),
        (
            tool("Read", r#"{"file_path":"/work/crates/core/src/panel_host.rs","limit":50}"#),
            "Read panel_host.rs",
        ),
        (tool("Edit", r#"{"path":"/work/dir/"}"#), "Edit dir"),
        (tool("Grep", r#"{"pattern":"fn now_ms","path":"/work"}"#), "Grep work"),
        (tool("Grep", r#"{"pattern":"fn now_ms"}"#), "Grep fn now_ms"),
        (tool("Glob", r#"{"pattern":"**/*.rs"}"#), "Glob **/*.rs"),
        (
            tool("Bash", r#"{"command":"cd /work && cargo test --workspace\nmore"}"#),
            "Bash cargo test --workspace",
        ),
        (tool("Bash", r#"{"command":"cd /work"}"#), "Bash cd /work"),
        (tool("WebFetch", r#"{"url":"https://example.com/a"}"#), "WebFetch https://example.com/a"),
    ];
    for (line, want) in cases {
        assert_eq!(now_of(&line).as_deref(), Some(want), "line {line}");
    }
}

#[test]
fn a_call_with_nothing_to_say_falls_back_to_the_newest_text_and_then_to_nothing() {
    let text = assistant(
        "msg_2",
        "end_turn",
        r#"{"type":"text","text":"\n\n  Checking the **hover** toolbar.\nSecond line"}"#,
    );
    let mut fold = Fold::default();
    fold.line(&tool("TodoWrite", r#"{"todos":[]}"#));
    assert_eq!(fold.now(), None);
    fold.line(&text);
    assert_eq!(fold.now(), Some("Checking the **hover** toolbar."));
    fold.line(&tool("Read", r#"{"file_path":"/a/b.rs"}"#));
    assert_eq!(fold.now(), Some("Read b.rs"));
}

#[test]
fn a_long_call_is_cut_and_other_lines_are_ignored() {
    let long = tool("Bash", &format!(r#"{{"command":"{}"}}"#, "x".repeat(300)));
    assert_eq!(now_of(&long).map(|text| text.chars().count()), Some(NOW_CHARS + 3));
    for noise in [
        "",
        "not json",
        "[]",
        r#"{"type":"attachment","isSidechain":true}"#,
        r#"{"type":"api-request"}"#,
    ] {
        assert_eq!(now_of(noise), None, "line {noise:?}");
    }
}

#[test]
fn a_user_line_after_the_last_message_reopens_an_ended_agent() {
    let mut fold = Fold::default();
    fold.line(&assistant("msg_1", "end_turn", r#"{"type":"text","text":"done"}"#));
    assert!(fold.ended());
    fold.line(r#"{"type":"user","timestamp":"2026-11-01T09:05:00.000Z","message":{"role":"user","content":"more"}}"#);
    assert!(!fold.ended());
}

struct Session {
    dir: tempfile::TempDir,
}

impl Session {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn put(&self, id: &str, meta: &str, transcript: Option<&str>, written: i64) {
        std::fs::write(self.path().join(format!("{id}.meta.json")), meta).unwrap();
        if let Some(text) = transcript {
            let file = self.path().join(format!("{id}.jsonl"));
            std::fs::write(&file, text).unwrap();
            self.touch(id, written);
        }
    }

    fn touch(&self, id: &str, written: i64) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(self.path().join(format!("{id}.jsonl")))
            .unwrap();
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_millis(written.unsigned_abs());
        file.set_modified(stamp).unwrap();
    }

    fn append(&self, id: &str, text: &str, written: i64) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.path().join(format!("{id}.jsonl")))
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
        drop(file);
        self.touch(id, written);
    }
}

const RUN_END: &str = "2026-11-01T08:23:02.776Z";
const FIN_END: &str = "2026-11-01T08:48:09.825Z";

#[test]
fn a_running_agent_is_listed_with_what_it_does_and_the_time_it_started() {
    let session = Session::new();
    session.put("agent-a", RUNNING_META, Some(RUNNING), at(RUN_END));
    let mut watch = Watch::default();
    let agents = watch.look(session.path(), at(RUN_END) + 5_000);
    assert_eq!(agents.len(), 1);
    let agent = &agents[0];
    assert_eq!(
        (
            agent.id.as_str(),
            agent.description.as_str(),
            agent.model.as_deref(),
            agent.effort.as_deref()
        ),
        ("agent-a", "Build hover actions", Some("sonnet"), Some("high"))
    );
    assert_eq!((agent.phase, agent.started_ms), (Phase::Running, at("2026-11-01T08:22:50.803Z")));
    assert!(agent.now.starts_with("Bash grep -n"), "{}", agent.now);
    assert_eq!(agent.tokens, 646_735);
    assert_eq!(agent.elapsed_ms(at(RUN_END) + 5_000), at(RUN_END) + 5_000 - agent.started_ms);
}

#[test]
fn a_finished_agent_stays_for_a_minute_and_then_is_not_even_opened() {
    let session = Session::new();
    session.put("agent-a", FINISHED_META, Some(FINISHED), at(FIN_END));
    let mut watch = Watch::default();
    let soon = watch.look(session.path(), at(FIN_END) + 59_000);
    assert_eq!(
        soon.iter().map(|agent| agent.phase).collect::<Vec<_>>(),
        [Phase::Finished(at(FIN_END))]
    );
    assert_eq!(
        soon[0].elapsed_ms(at(FIN_END) + 59_000),
        at(FIN_END) - at("2026-11-01T08:22:50.803Z")
    );
    assert_eq!(soon[0].tokens, 969_255);

    let mut later = Watch::default();
    assert_eq!(later.look(session.path(), at(FIN_END) + 60_000), Vec::new());
    assert_eq!(later.readings.len(), 0);
}

#[test]
fn an_agent_of_an_older_version_is_finished_by_its_last_message_not_its_meta() {
    let session = Session::new();
    let old = r#"{"agentType":"general-purpose","description":"Old one","model":"sonnet"}"#;
    session.put("agent-a", old, Some(FINISHED), at(FIN_END));
    let agents = Watch::default().look(session.path(), at(FIN_END) + 1_000);
    assert_eq!(
        agents.iter().map(|agent| agent.phase).collect::<Vec<_>>(),
        [Phase::Finished(at(FIN_END))]
    );
}

#[test]
fn a_silent_agent_that_never_finished_is_gone_after_ten_minutes() {
    let session = Session::new();
    session.put("agent-a", RUNNING_META, Some(RUNNING), at(RUN_END));
    let mut watch = Watch::default();
    assert_eq!(watch.look(session.path(), at(RUN_END) + STALE_MS - 1).len(), 1);
    assert_eq!(watch.look(session.path(), at(RUN_END) + STALE_MS), Vec::new());
    assert_eq!(watch.readings.len(), 0);
}

#[test]
fn an_agent_whose_transcript_does_not_exist_yet_is_not_listed() {
    let session = Session::new();
    session.put("agent-a", RUNNING_META, None, 0);
    assert_eq!(Watch::default().look(session.path(), at(RUN_END)), Vec::new());
}

#[test]
fn a_missing_folder_holds_no_agents() {
    let session = Session::new();
    let mut watch = Watch::default();
    assert_eq!(watch.look(&session.path().join("nope"), 0), Vec::new());
}

#[test]
fn running_agents_come_oldest_first_and_before_the_finished_ones() {
    let session = Session::new();
    let late = RUNNING.replace("08:22:50.803Z", "08:22:58.000Z");
    session.put("agent-late", RUNNING_META, Some(&late), at(RUN_END));
    session.put("agent-early", RUNNING_META, Some(RUNNING), at(RUN_END));
    let done = FINISHED.replace("08:48:09.825Z", "08:23:01.000Z");
    session.put("agent-done", FINISHED_META, Some(&done), at(RUN_END));
    let agents = Watch::default().look(session.path(), at(RUN_END) + 1_000);
    assert_eq!(
        agents.iter().map(|agent| agent.id.as_str()).collect::<Vec<_>>(),
        ["agent-early", "agent-late", "agent-done"]
    );
}

#[test]
fn a_look_reads_only_what_was_appended_and_waits_for_the_end_of_a_line() {
    let session = Session::new();
    session.put("agent-a", RUNNING_META, Some(RUNNING), at(RUN_END));
    let mut watch = Watch::default();
    let first = watch.look(session.path(), at(RUN_END) + 1_000);
    let offset = watch.readings["agent-a"].offset;
    assert_eq!(offset, RUNNING.len() as u64);

    let next = tool("Read", r#"{"file_path":"/work/new_file.rs"}"#);
    let (head, tail) = next.split_at(40);
    session.append("agent-a", head, at(RUN_END) + 2_000);
    let held = watch.look(session.path(), at(RUN_END) + 3_000);
    assert_eq!(held[0].now, first[0].now);

    session.append("agent-a", &format!("{tail}\n"), at(RUN_END) + 4_000);
    let after = watch.look(session.path(), at(RUN_END) + 5_000);
    assert_eq!(after[0].now, "Read new_file.rs");
    assert_eq!(after[0].tokens, first[0].tokens + 3);
}

#[test]
fn a_transcript_that_was_cut_short_is_read_again_from_the_top() {
    let session = Session::new();
    session.put("agent-a", RUNNING_META, Some(RUNNING), at(RUN_END));
    let mut watch = Watch::default();
    watch.look(session.path(), at(RUN_END) + 1_000);
    let short = tool("Read", r#"{"file_path":"/work/only.rs"}"#) + "\n";
    session.put("agent-a", RUNNING_META, Some(&short), at(RUN_END));
    let agents = watch.look(session.path(), at(RUN_END) + 2_000);
    assert_eq!((agents[0].now.as_str(), agents[0].tokens), ("Read only.rs", 3));
}

#[test]
fn the_list_shows_running_agents_and_finished_ones_for_a_minute() {
    let agent = |id: &str, phase| Agent {
        id: id.into(),
        description: String::new(),
        model: None,
        effort: None,
        started_ms: 0,
        phase,
        now: String::new(),
        tokens: 0,
    };
    let agents = [
        agent("run", Phase::Running),
        agent("fresh", Phase::Finished(100_000)),
        agent("old", Phase::Finished(10_000)),
    ];
    let ids = |now| shown(&agents, now).iter().map(|agent| agent.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(150_000), ["run", "fresh"]);
    assert_eq!(ids(160_000), ["run"]);
    assert_eq!(next_leave_ms(&agents, 150_000), Some(160_000));
    assert_eq!(next_leave_ms(&agents[..1], 150_000), None);
    assert_eq!(next_leave_ms(&agents[2..], 150_000), None);
}

#[test]
fn the_poll_runs_while_the_agent_works_or_a_listed_agent_runs_and_never_otherwise() {
    let agent = |phase| Agent {
        id: "a".into(),
        description: String::new(),
        model: None,
        effort: None,
        started_ms: 0,
        phase,
        now: String::new(),
        tokens: 0,
    };
    let cases = [
        (vec![], false, false),
        (vec![], true, true),
        (vec![agent(Phase::Running)], false, true),
        (vec![agent(Phase::Finished(5))], false, false),
        (vec![agent(Phase::Finished(5))], true, true),
    ];
    for (agents, working, want) in cases {
        assert_eq!(needs_poll(&agents, working), want, "{agents:?} working={working}");
    }
}

#[test]
fn the_folder_sits_beside_the_transcript_under_the_session_id() {
    assert_eq!(
        dir_of(Path::new("/home/.claude/projects/-work/abc-123.jsonl")),
        Path::new("/home/.claude/projects/-work/abc-123/subagents")
    );
}
