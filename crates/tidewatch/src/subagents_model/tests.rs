use super::*;

fn agent(id: &str, phase: Phase, tokens: u64) -> Agent {
    Agent {
        id: id.into(),
        description: format!("Task {id}"),
        model: Some("sonnet".into()),
        effort: Some("high".into()),
        started_ms: 1_000_000,
        phase,
        now: format!("Reading {id}.rs"),
        tokens,
    }
}

#[test]
fn the_running_line_names_the_count_and_is_none_for_none() {
    let cases = [(0, None), (1, Some("1 agent running")), (4, Some("4 agents running"))];
    for (running, want) in cases {
        assert_eq!(running_line(running).as_deref(), want, "{running} running");
    }
}

#[test]
fn time_reads_in_seconds_then_minutes_and_seconds_then_hours() {
    let cases = [
        (-5, "0s"),
        (0, "0s"),
        (42_999, "42s"),
        (60_000, "1m 00s"),
        (253_000, "4m 13s"),
        (241_000, "4m 01s"),
        (3_599_000, "59m 59s"),
        (3_600_000, "1h 00m"),
        (3_725_000, "1h 02m"),
    ];
    for (ms, want) in cases {
        assert_eq!(clock(ms), want, "{ms} ms");
    }
}

#[test]
fn a_running_row_shows_what_it_does_its_tag_and_time_with_tokens() {
    let list = list(&[agent("a", Phase::Running, 281_400)], 1_253_000).unwrap();
    assert_eq!(
        list.rows,
        [Row {
            id: "a".into(),
            what: "Reading a.rs".into(),
            tag: Some("sonnet \u{b7} high".into()),
            right: "4m 13s \u{b7} 281k".into(),
            running: true,
        }]
    );
}

#[test]
fn a_finished_row_shows_its_description_and_how_long_it_ran() {
    let list = list(&[agent("a", Phase::Finished(1_241_000), 5)], 1_250_000).unwrap();
    assert_eq!(
        (list.rows[0].what.as_str(), list.rows[0].right.as_str(), list.rows[0].running),
        ("Task a", "done 4m 01s", false)
    );
}

#[test]
fn a_row_without_tokens_or_model_drops_those_parts() {
    let mut bare = agent("a", Phase::Running, 0);
    bare.model = None;
    bare.effort = Some("low".into());
    let mut none = bare.clone();
    none.effort = None;
    let list = list(&[bare, none], 1_005_000).unwrap();
    assert_eq!(
        list.rows.iter().map(|row| (row.tag.as_deref(), row.right.as_str())).collect::<Vec<_>>(),
        [(Some("low"), "5s"), (None, "5s")]
    );
}

#[test]
fn the_header_counts_running_agents_and_notes_the_finished_ones() {
    let cases = [
        (vec![("a", Phase::Running)], "1 agent running", None),
        (
            vec![("a", Phase::Running), ("b", Phase::Running), ("c", Phase::Finished(1_190_000))],
            "2 agents running",
            Some("1 finished"),
        ),
        (vec![("a", Phase::Finished(1_190_000))], "1 agent finished", None),
        (
            vec![("a", Phase::Finished(1_190_000)), ("b", Phase::Finished(1_195_000))],
            "2 agents finished",
            None,
        ),
    ];
    for (agents, head, note) in cases {
        let agents: Vec<Agent> =
            agents.into_iter().map(|(id, phase)| agent(id, phase, 1)).collect();
        let list = list(&agents, 1_200_000).unwrap();
        assert_eq!((list.head().as_str(), list.finished_note().as_deref()), (head, note));
    }
}

#[test]
fn the_list_is_gone_when_nothing_runs_and_nothing_ended_in_the_last_minute() {
    assert_eq!(list(&[], 0), None);
    let old = agent("a", Phase::Finished(1_000_000), 1);
    assert_eq!(list(std::slice::from_ref(&old), 1_059_999).map(|list| list.rows.len()), Some(1));
    assert_eq!(list(std::slice::from_ref(&old), 1_060_000), None);
}

#[test]
fn six_rows_show_and_the_rest_wait_behind_a_more_line_that_opens_them_in_place() {
    let agents: Vec<Agent> = (0..9).map(|i| agent(&format!("a{i}"), Phase::Running, 1)).collect();
    let list = list(&agents, 1_100_000).unwrap();
    let cases = [
        (false, 99, Plan { rows: 6, more: 3, can_fold_back: false }),
        (true, 99, Plan { rows: 9, more: 0, can_fold_back: true }),
        (true, 7, Plan { rows: 7, more: 2, can_fold_back: true }),
        (false, 4, Plan { rows: 4, more: 5, can_fold_back: false }),
        (false, 0, Plan { rows: 1, more: 8, can_fold_back: false }),
    ];
    for (opened, fit, want) in cases {
        assert_eq!(list.plan(opened, fit), want, "opened={opened} fit={fit}");
    }
    let few = self::list(&agents[..4], 1_100_000).unwrap();
    assert_eq!(few.plan(true, 99), Plan { rows: 4, more: 0, can_fold_back: false });
}

#[test]
fn the_chat_header_reads_the_description_and_what_the_agent_spent_running_or_done() {
    let running = agent("a", Phase::Running, 281_400);
    let finished = agent("b", Phase::Finished(1_253_000), 190_000);
    let silent = agent("c", Phase::Running, 0);
    let cases = [
        (&running, "Task a", "4m 13s \u{b7} 281k"),
        (&finished, "Task b", "done 4m 13s \u{b7} 190k"),
        (&silent, "Task c", "4m 13s"),
    ];
    for (agent, what, right) in cases {
        let line = head(agent, 1_253_000);
        assert_eq!((line.what.as_str(), line.right.as_str()), (what, right), "{}", agent.id);
        assert_eq!(line.tag.as_deref(), Some("sonnet \u{b7} high"));
    }
}

#[test]
fn the_chat_header_falls_back_to_what_the_agent_does_without_a_description() {
    let mut nameless = agent("a", Phase::Running, 0);
    nameless.description.clear();
    assert_eq!(head(&nameless, 1_000_000).what, "Reading a.rs");
}
