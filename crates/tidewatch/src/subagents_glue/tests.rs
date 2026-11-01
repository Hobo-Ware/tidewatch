//! The poll: one look when a transcript is first known, one every 2 s while the agent works or a
//! listed agent runs, none while everything is quiet.

use std::time::{Duration, Instant};

use super::*;

const RUNNING: &str = include_str!("../../../tidewatch-core/tests/fixtures/subagent_running.jsonl");
const RUNNING_META: &str =
    include_str!("../../../tidewatch-core/tests/fixtures/subagent_running.meta.json");
const FINISHED: &str =
    include_str!("../../../tidewatch-core/tests/fixtures/subagent_finished.jsonl");
const FINISHED_META: &str =
    include_str!("../../../tidewatch-core/tests/fixtures/subagent_finished.meta.json");

const FINISHED_AT: i64 = 1_793_522_889_825;

struct Session {
    dir: tempfile::TempDir,
    transcript: PathBuf,
}

impl Session {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("abc.jsonl");
        std::fs::write(&transcript, "").unwrap();
        std::fs::create_dir_all(dir.path().join("abc/subagents")).unwrap();
        Self { dir, transcript }
    }

    fn agent(&self, id: &str, meta: &str, transcript: &str) {
        let folder = self.dir.path().join("abc/subagents");
        std::fs::write(folder.join(format!("{id}.meta.json")), meta).unwrap();
        std::fs::write(folder.join(format!("{id}.jsonl")), transcript).unwrap();
    }
}

fn now_ms() -> i64 {
    tidewatch_core::clock::now_ms()
}

fn settle(host: &mut Host, facts: &[Facts<'_>], ctx: &egui::Context, at: i64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
        host.tick(facts, ctx, at);
        if host.tabs.values().all(|tab| !tab.in_flight) {
            return;
        }
    }
    panic!("the look did not finish");
}

fn fact(session: &Session, working: bool) -> Facts<'_> {
    Facts { id: 1, transcript: Some(&session.transcript), working }
}

#[test]
fn a_tab_with_no_transcript_yet_is_never_looked_at() {
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let facts = [Facts { id: 1, transcript: None, working: true }];
    assert_eq!(host.tick(&facts, &ctx, 0), None);
    assert_eq!(host.looks, 0);
}

#[test]
fn the_first_look_happens_once_and_then_a_quiet_tab_is_not_looked_at_again() {
    let session = Session::new();
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let facts = [fact(&session, false)];
    let start = now_ms();
    host.tick(&facts, &ctx, start);
    settle(&mut host, &facts, &ctx, start);
    assert_eq!(host.looks, 1);
    for step in 1..=20 {
        assert_eq!(host.tick(&facts, &ctx, start + step * 10_000), None);
    }
    assert_eq!(host.looks, 1);
}

#[test]
fn a_working_agent_is_looked_at_every_two_seconds_and_not_between() {
    let session = Session::new();
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let facts = [fact(&session, true)];
    let start = now_ms();
    host.tick(&facts, &ctx, start);
    settle(&mut host, &facts, &ctx, start);
    assert_eq!(host.looks, 1);
    assert_eq!(host.tick(&facts, &ctx, start + 1_999), Some(start + POLL_MS));
    assert_eq!(host.looks, 1);
    host.tick(&facts, &ctx, start + POLL_MS);
    settle(&mut host, &facts, &ctx, start + POLL_MS);
    assert_eq!(host.looks, 2);
}

#[test]
fn the_look_stops_once_the_agent_is_idle_and_every_listed_agent_has_finished() {
    let session = Session::new();
    session.agent("agent-a", RUNNING_META, RUNNING);
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let start = now_ms();
    let working = [fact(&session, true)];
    host.tick(&working, &ctx, start);
    settle(&mut host, &working, &ctx, start);
    assert_eq!(host.running(1), 1);

    let idle = [fact(&session, false)];
    host.tick(&idle, &ctx, start + POLL_MS);
    settle(&mut host, &idle, &ctx, start + POLL_MS);
    assert_eq!(host.looks, 2);

    session.agent("agent-a", FINISHED_META, FINISHED);
    host.tick(&idle, &ctx, start + 2 * POLL_MS);
    settle(&mut host, &idle, &ctx, start + 2 * POLL_MS);
    assert_eq!(host.looks, 3);
    assert_eq!(host.running(1), 0);
    for step in 3..30 {
        host.tick(&idle, &ctx, start + step * POLL_MS);
    }
    assert_eq!(host.looks, 3);
}

#[test]
fn a_finished_agent_asks_for_a_frame_when_it_leaves_the_list() {
    let session = Session::new();
    session.agent("agent-a", FINISHED_META, FINISHED);
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let at = FINISHED_AT;
    let facts = [fact(&session, false)];
    host.tick(&facts, &ctx, at + 10_000);
    settle(&mut host, &facts, &ctx, at + 10_000);
    assert_eq!(host.agents(1).len(), 1);
    assert_eq!(host.tick(&facts, &ctx, at + 20_000), Some(at + 60_000));
}

#[test]
fn a_tab_that_closes_is_forgotten_and_a_new_transcript_starts_over() {
    let session = Session::new();
    session.agent("agent-a", RUNNING_META, RUNNING);
    let ctx = egui::Context::default();
    let mut host = Host::new(false);
    let facts = [fact(&session, true)];
    let start = now_ms();
    host.tick(&facts, &ctx, start);
    settle(&mut host, &facts, &ctx, start);
    assert_eq!(host.agents(1).len(), 1);
    host.tick(&[], &ctx, start + 10);
    assert_eq!(host.agents(1).len(), 0);
}

#[test]
fn the_fake_lists_four_running_agents_and_one_finished_without_reading_anything() {
    let ctx = egui::Context::default();
    let mut host = Host::new(true);
    let facts = [Facts { id: 7, transcript: None, working: false }];
    host.tick(&facts, &ctx, 1_000_000);
    assert_eq!(host.running(7), 4);
    assert_eq!(host.agents(7).len(), 5);
    assert_eq!(host.looks, 0);
}
