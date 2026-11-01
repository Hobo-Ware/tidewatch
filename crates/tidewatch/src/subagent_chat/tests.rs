use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tidewatch_core::subagents::Phase;

use super::*;
use crate::reader_rows::Kind;

fn agent(id: &str, phase: Phase) -> Agent {
    Agent {
        id: id.into(),
        description: "Build hover actions".into(),
        model: Some("sonnet".into()),
        effort: Some("high".into()),
        started_ms: 1_000,
        phase,
        now: "Read acts-brief.md".into(),
        tokens: 281_000,
    }
}

fn line(text: &str) -> String {
    format!(
        "{{\"isSidechain\":true,\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
    )
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn texts(chat: &Open) -> Vec<String> {
    chat.rows.iter().map(|row| row.text.clone()).collect()
}

#[test]
fn a_fake_chat_shows_the_redacted_transcript_with_its_prompt_calls_and_answer_and_reads_nothing() {
    let ctx = egui::Context::default();
    for (finished, last) in [(false, false), (true, true)] {
        let chat = Open::new(1, agent("fake-0", Phase::Running), Source::Fake { finished }, &ctx);
        let rows: Vec<_> = chat.rows.iter().collect();
        assert_eq!(rows[0].kind, Kind::User);
        assert_eq!(rows[0].text, "Read the hover toolbar code and fold the fix.");
        assert!(rows.iter().any(|row| matches!(row.kind, Kind::Tool(_))));
        assert_eq!(rows.last().is_some_and(|row| row.kind == Kind::Assistant), last);
        assert!(!chat.reading());
    }
}

#[test]
fn a_chat_follows_the_file_while_it_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("agent-a1.jsonl");
    fs::write(&file, line("first")).unwrap();
    let ctx = egui::Context::default();
    let mut chat =
        Open::new(1, agent("agent-a1", Phase::Running), Source::File(file.clone()), &ctx);
    assert!(chat.reading());
    assert!(wait_until(|| {
        chat.drain();
        texts(&chat) == ["first"]
    }));
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str(&line("second"));
    fs::write(&file, text).unwrap();
    assert!(wait_until(|| {
        chat.drain();
        texts(&chat) == ["first", "second"]
    }));
}

#[test]
fn a_chat_that_is_closed_reads_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("agent-a1.jsonl");
    fs::write(&file, line("first")).unwrap();
    let ctx = egui::Context::default();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    ctx.set_request_repaint_callback(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let mut host = Host::default();
    host.open(Open::new(1, agent("agent-a1", Phase::Running), Source::File(file.clone()), &ctx));
    assert!(wait_until(|| wakes.load(Ordering::SeqCst) > 0));
    host.close();
    std::thread::sleep(Duration::from_millis(100));
    let before = wakes.load(Ordering::SeqCst);
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str(&line("second"));
    fs::write(&file, text).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(wakes.load(Ordering::SeqCst), before);
}

#[test]
fn the_chat_belongs_to_the_tab_it_opened_over() {
    let ctx = egui::Context::default();
    let mut host = Host::default();
    assert_eq!(host.id_over(1), None);
    host.open(Open::new(
        1,
        agent("fake-2", Phase::Running),
        Source::Fake { finished: false },
        &ctx,
    ));
    assert_eq!(host.id_over(1), Some("fake-2"));
    assert_eq!(host.id_over(2), None);
    assert!(host.chat_mut(2).is_none());
    host.close();
    assert_eq!(host.id_over(1), None);
}

#[test]
fn a_chat_not_drawn_for_a_frame_is_gone_but_one_that_is_drawn_stays() {
    let ctx = egui::Context::default();
    let mut host = Host::default();
    host.open(Open::new(
        1,
        agent("fake-2", Phase::Running),
        Source::Fake { finished: false },
        &ctx,
    ));
    host.end_frame();
    assert_eq!(host.id_over(1), Some("fake-2"), "the frame it opened in does not count");
    host.chat_mut(1).unwrap().mark_drawn();
    host.end_frame();
    assert_eq!(host.id_over(1), Some("fake-2"));
    host.end_frame();
    assert_eq!(host.id_over(1), None);
}

#[test]
fn an_agent_that_left_the_list_has_ended_and_one_still_listed_is_followed() {
    let ctx = egui::Context::default();
    let mut chat =
        Open::new(1, agent("fake-2", Phase::Running), Source::Fake { finished: false }, &ctx);
    let mut listed = agent("fake-2", Phase::Running);
    listed.tokens = 300_000;
    chat.refresh(Some(&listed), 5_000);
    assert_eq!(chat.agent.tokens, 300_000);
    chat.refresh(None, 9_000);
    assert_eq!(chat.agent.phase, Phase::Finished(9_000));
    chat.refresh(None, 12_000);
    assert_eq!(chat.agent.phase, Phase::Finished(9_000));
}

#[test]
fn two_agents_have_two_scroll_keys() {
    let ctx = egui::Context::default();
    let open =
        |id: &str| Open::new(1, agent(id, Phase::Running), Source::Fake { finished: false }, &ctx);
    assert_ne!(open("a").key(), open("b").key());
    assert_eq!(open("a").key(), open("a").key());
}
