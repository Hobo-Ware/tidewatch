use super::*;
use crate::rfc3339::unix_millis;
use crate::transcript::{Parser, Source, Tail};

const CLAUDE: &str = include_str!("../../tests/fixtures/claude.jsonl");
const CLAUDE_ASK: &str = include_str!("../../tests/fixtures/claude_ask_user_question.jsonl");
const CODEX: &str = include_str!("../../tests/fixtures/codex_live.jsonl");
const COPILOT: &str = include_str!("../../tests/fixtures/copilot.jsonl");
const GEMINI: &str = include_str!("../../tests/fixtures/gemini.jsonl");

fn at(text: &str) -> Option<i64> {
    unix_millis(text)
}

fn read(source: Source, text: &str) -> (Vec<Entry>, Vec<TurnMark>) {
    let mut parser = Parser::new(source);
    let entries = text.lines().flat_map(|line| parser.feed_line(line)).collect();
    (entries, parser.take_marks())
}

fn marks(source: Source, text: &str) -> Vec<(bool, Option<i64>, usize)> {
    read(source, text).1.into_iter().map(|m| (m.started, m.ts, m.before)).collect()
}

fn turn(started: bool, ts: &str) -> Fact {
    Fact::Turn { started, ts_ms: at(ts) }
}

fn facts(source: Source, text: &str, since_ms: i64) -> Vec<Fact> {
    let (entries, marks) = read(source, text);
    facts_of(&entries, &marks, since_ms).into_iter().map(|timed| timed.fact).collect()
}

#[test]
fn each_fixture_has_its_turns_marked_where_the_cli_wrote_them() {
    let cases = [
        (
            "claude: the prompt, then the stop hook summary after the last end_turn record",
            Source::Claude,
            CLAUDE,
            vec![
                (true, at("2026-10-12T23:03:55.696Z"), 0),
                (false, at("2026-10-12T23:03:59.253Z"), 4),
            ],
        ),
        (
            "codex: task_started and task_complete, twice",
            Source::Codex,
            CODEX,
            vec![
                (true, at("2026-10-13T17:57:41.806Z"), 0),
                (false, at("2026-10-13T17:57:53.320Z"), 7),
                (true, at("2026-10-13T17:58:06.311Z"), 7),
                (false, at("2026-10-13T17:58:13.415Z"), 9),
            ],
        ),
        (
            "copilot: the first round starts, only the round without tool requests ends",
            Source::Copilot,
            COPILOT,
            vec![
                (true, at("2026-10-12T23:04:27.671Z"), 1),
                (false, at("2026-10-12T23:04:35.939Z"), 4),
            ],
        ),
        (
            "gemini: a user message starts, the answer with no tool calls ends",
            Source::Gemini,
            GEMINI,
            vec![
                (true, at("2026-10-12T23:04:29.334Z"), 0),
                (false, at("2026-10-12T23:04:32.638Z"), 4),
            ],
        ),
    ];
    for (what, source, text, want) in cases {
        assert_eq!(marks(source, text), want, "{what}");
    }
}

#[test]
fn a_claude_fixture_reads_as_a_turn_with_its_tool_call_and_last_words() {
    let want = vec![
        turn(true, "2026-10-12T23:03:55.696Z"),
        Fact::ToolOpened { id: "toolu_01CJPLN1Vrsb8mQ2XYzmCoXm".into(), name: "Bash".into() },
        Fact::ToolClosed { id: "toolu_01CJPLN1Vrsb8mQ2XYzmCoXm".into() },
        Fact::Said { tail: "hello".into() },
        turn(false, "2026-10-12T23:03:59.253Z"),
    ];
    assert_eq!(facts(Source::Claude, CLAUDE, 0), want);
}

#[test]
fn a_question_tool_is_an_ask_that_opens_with_the_call_and_closes_with_its_result() {
    let all = facts(Source::Claude, CLAUDE_ASK, 0);
    let asks: Vec<&Fact> = all
        .iter()
        .filter(|fact| matches!(fact, Fact::AskOpened { .. } | Fact::AskClosed))
        .collect();
    assert_eq!(asks.len(), 7);
    assert_eq!(asks[0], &Fact::AskOpened { summary: "Which color?".into(), n: 2 });
    assert_eq!(asks[1], &Fact::AskClosed);
    assert_eq!(asks[2], &Fact::AskOpened { summary: "Which mode?".into(), n: 1 });
}

#[test]
fn an_answered_question_tool_result_names_each_question_with_its_answer() {
    let all = facts(Source::Claude, CLAUDE_ASK, 0);
    let answered: Vec<(&str, &str)> = all
        .iter()
        .filter_map(|fact| match fact {
            Fact::QuestionAnswered { question, answer } => {
                Some((question.as_str(), answer.as_str()))
            }
            _ => None,
        })
        .collect();
    assert!(answered.contains(&("Which mode?", "turbo mode")), "{answered:?}");
    let closed = all.iter().filter(|fact| matches!(fact, Fact::AskClosed)).count();
    assert_eq!(closed, 3, "every result closes its ask, answered or not");
}

#[test]
fn what_the_cli_wrote_before_the_cut_is_history_and_adds_nothing() {
    let cut = at("2026-10-12T23:03:58Z").unwrap();
    let late = facts(Source::Claude, CLAUDE, cut);
    assert_eq!(
        late,
        [Fact::Said { tail: "hello".into() }, turn(false, "2026-10-12T23:03:59.253Z")]
    );
    let after = at("2026-10-13T00:00:00Z").unwrap();
    assert_eq!(facts(Source::Claude, CLAUDE, after), Vec::<Fact>::new());
}

#[test]
fn only_the_last_thing_said_between_two_boundaries_is_kept() {
    let said = |text: &str| Entry {
        role: Role::Assistant,
        text: text.to_owned(),
        tool: None,
        ts: Some(10),
        images: Vec::new(),
    };
    let entries = [said("one"), said("two"), said("three")];
    let marks = [TurnMark { started: false, ts: Some(11), before: 2 }];
    let got: Vec<Fact> =
        facts_of(&entries, &marks, 0).into_iter().map(|timed| timed.fact).collect();
    assert_eq!(
        got,
        [
            Fact::Said { tail: "two".into() },
            Fact::Turn { started: false, ts_ms: Some(11) },
            Fact::Said { tail: "three".into() },
        ]
    );
}

fn claude_line(kind: &str, extra: &str, ts: &str) -> String {
    format!("{{\"type\":\"{kind}\",\"timestamp\":\"{ts}\",{extra}}}")
}

fn marks_of_lines(lines: &[String]) -> Vec<(bool, usize)> {
    let mut parser = Parser::new(Source::Claude);
    for line in lines {
        parser.feed_line(line);
    }
    parser.take_marks().into_iter().map(|m| (m.started, m.before)).collect()
}

#[test]
fn claude_marks_a_prompt_and_an_end_but_not_tool_results_queued_prompts_or_sub_agents() {
    let prompt = claude_line("user", "\"message\":{\"content\":\"do it\"}", "2026-10-13T10:00:00Z");
    let result = claude_line(
        "user",
        "\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t\",\"content\":\"ok\"}]}",
        "2026-10-13T10:00:01Z",
    );
    let queued = claude_line(
        "attachment",
        "\"attachment\":{\"type\":\"queued_command\",\"prompt\":\"more\",\"commandMode\":\"prompt\",\"origin\":{\"kind\":\"human\"}}",
        "2026-10-13T10:00:02Z",
    );
    let side = claude_line(
        "assistant",
        "\"isSidechain\":true,\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"sub\"}]}",
        "2026-10-13T10:00:03Z",
    );
    let tool_use = claude_line(
        "assistant",
        "\"message\":{\"stop_reason\":\"tool_use\",\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}",
        "2026-10-13T10:00:04Z",
    );
    let end = claude_line(
        "assistant",
        "\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}",
        "2026-10-13T10:00:05Z",
    );
    let lines = [prompt, result, queued, side, tool_use, end];
    assert_eq!(marks_of_lines(&lines), [(true, 0), (false, 5)]);
}

#[test]
fn a_second_prompt_before_any_end_adds_no_second_start_and_two_ends_make_one_at_the_later_line() {
    let prompt = |n: u32| {
        claude_line("user", "\"message\":{\"content\":\"go\"}", &format!("2026-10-13T10:00:0{n}Z"))
    };
    let end = |n: u32| {
        claude_line(
            "assistant",
            "\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}",
            &format!("2026-10-13T10:00:0{n}Z"),
        )
    };
    let lines = [prompt(1), prompt(2), end(3), end(4)];
    let mut parser = Parser::new(Source::Claude);
    for line in &lines {
        parser.feed_line(line);
    }
    let got: Vec<(bool, Option<i64>, usize)> =
        parser.take_marks().into_iter().map(|m| (m.started, m.ts, m.before)).collect();
    assert_eq!(
        got,
        [(true, at("2026-10-13T10:00:01Z"), 0), (false, at("2026-10-13T10:00:04Z"), 4)]
    );
}

#[test]
fn copilot_tool_rounds_do_not_end_the_interaction_and_a_new_interaction_starts_again() {
    let line = |kind: &str, data: &str, ts: &str| {
        format!("{{\"type\":\"{kind}\",\"timestamp\":\"{ts}\",\"data\":{data}}}")
    };
    let lines = [
        line(
            "assistant.turn_start",
            "{\"turnId\":\"0\",\"interactionId\":\"a\"}",
            "2026-10-13T10:00:01Z",
        ),
        line(
            "assistant.message",
            "{\"content\":\"\",\"toolRequests\":[{\"name\":\"bash\"}]}",
            "2026-10-13T10:00:02Z",
        ),
        line("assistant.turn_end", "{\"turnId\":\"0\"}", "2026-10-13T10:00:03Z"),
        line(
            "assistant.turn_start",
            "{\"turnId\":\"1\",\"interactionId\":\"a\"}",
            "2026-10-13T10:00:03Z",
        ),
        line(
            "assistant.message",
            "{\"content\":\"hi\",\"toolRequests\":[]}",
            "2026-10-13T10:00:04Z",
        ),
        line("assistant.turn_end", "{\"turnId\":\"1\"}", "2026-10-13T10:00:05Z"),
        line(
            "assistant.turn_start",
            "{\"turnId\":\"2\",\"interactionId\":\"b\"}",
            "2026-10-13T10:00:09Z",
        ),
        line("abort", "{\"reason\":\"user\"}", "2026-10-13T10:00:10Z"),
    ];
    let mut parser = Parser::new(Source::Copilot);
    for line in &lines {
        parser.feed_line(line);
    }
    let got: Vec<(bool, Option<i64>)> =
        parser.take_marks().into_iter().map(|m| (m.started, m.ts)).collect();
    assert_eq!(
        got,
        [
            (true, at("2026-10-13T10:00:01Z")),
            (false, at("2026-10-13T10:00:05Z")),
            (true, at("2026-10-13T10:00:09Z")),
            (false, at("2026-10-13T10:00:10Z")),
        ]
    );
}

#[test]
fn codex_marks_an_interrupted_turn_as_ended() {
    let line = |kind: &str, ts: &str| {
        format!(
            "{{\"type\":\"event_msg\",\"timestamp\":\"{ts}\",\"payload\":{{\"type\":\"{kind}\"}}}}"
        )
    };
    let mut parser = Parser::new(Source::Codex);
    for text in
        [line("task_started", "2026-10-13T10:00:01Z"), line("turn_aborted", "2026-10-13T10:00:02Z")]
    {
        parser.feed_line(&text);
    }
    let got: Vec<bool> = parser.take_marks().into_iter().map(|m| m.started).collect();
    assert_eq!(got, [true, false]);
}

#[test]
fn a_tail_places_each_mark_among_the_entries_of_its_own_poll_and_keeps_counting_across_polls() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("session.jsonl");
    let prompt = |n: u32| {
        claude_line("user", "\"message\":{\"content\":\"go\"}", &format!("2026-10-13T10:00:0{n}Z"))
    };
    let end = |n: u32, text: &str| {
        claude_line(
            "assistant",
            &format!(
                "\"message\":{{\"stop_reason\":\"end_turn\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}"
            ),
            &format!("2026-10-13T10:00:0{n}Z"),
        )
    };
    std::fs::write(&file, format!("{}\n", prompt(1))).unwrap();
    let mut tail = Tail::new(Source::Claude, &file);
    let (first, first_marks) = tail.poll_marked();
    assert_eq!(first.len(), 1);
    assert_eq!(first_marks.iter().map(|m| (m.started, m.before)).collect::<Vec<_>>(), [(true, 0)]);

    let mut text = std::fs::read_to_string(&file).unwrap();
    for line in [end(2, "a"), prompt(3), end(4, "b")] {
        text.push_str(&line);
        text.push('\n');
    }
    std::fs::write(&file, text).unwrap();
    let (second, second_marks) = tail.poll_marked();
    assert_eq!(second.len(), 3);
    let placed: Vec<(bool, usize)> = second_marks.iter().map(|m| (m.started, m.before)).collect();
    assert_eq!(placed, [(false, 1), (true, 1), (false, 3)]);
}

#[test]
fn gemini_ends_a_turn_only_on_an_answer_with_text_and_no_tool_calls() {
    let user =
        r#"{"id":"u1","timestamp":"2026-10-13T10:00:00Z","type":"user","content":[{"text":"go"}]}"#;
    let narrating = r#"{"id":"g1","timestamp":"2026-10-13T10:00:01Z","type":"gemini","content":"Let me look.","toolCalls":[{"id":"c1","name":"read_file","args":{"file_path":"a"}}]}"#;
    let answer = r#"{"id":"g2","timestamp":"2026-10-13T10:00:02Z","type":"gemini","content":"All good.","toolCalls":[]}"#;
    let mut parser = Parser::new(Source::Gemini);
    for line in [user, narrating] {
        parser.feed_line(line);
    }
    let so_far: Vec<bool> = parser.take_marks().into_iter().map(|m| m.started).collect();
    assert_eq!(so_far, [true]);
    parser.feed_line(answer);
    let ended: Vec<bool> = parser.take_marks().into_iter().map(|m| m.started).collect();
    assert_eq!(ended, [false]);
}
