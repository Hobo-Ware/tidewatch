use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stdusk_core::progress::Progress;
use tidewatch_core::panel_event::{ChecksWord, PrFacts, PrWord};
use tidewatch_core::question::{AskReply, Question, Select};
use tidewatch_core::status::{HookEvent, HookKind};
use tidewatch_core::transcript_facts::Timed;

use super::*;
use crate::agent_frame::{self, route};
use crate::agent_run::AgentRun;
use crate::notify::Notifier;
use crate::pane::{AgentTab, Body};

const MESSAGE: &str =
    "Added it.\n\n**Done**\n- the helper\n\n**Needs you**\n- nothing\n\n**Next**\n- review";

fn tabs_with_agent(notes: &Path) -> (Tabs<Pane<u32>>, u64) {
    let mut run = AgentRun::setting_up("claude".into(), 0.0);
    run.started(Some(notes.to_path_buf()), 0.0);
    let pane = Pane {
        body: Body::Shell(1),
        worktree: None,
        agent: Some(AgentTab::new("Builder".into(), run)),
    };
    let mut tabs = Tabs::new(Pane::shell(0));
    let id = tabs.push("brave-otter".into(), pane);
    (tabs, id)
}

fn message(tab: u64, kind: HookKind) -> TabMessage {
    TabMessage {
        tab_id: tab,
        generation: 0,
        event: TabEvent::Hook(HookEvent {
            kind,
            session_id: None,
            transcript_path: None,
            message: None,
        }),
    }
}

fn memory_host() -> Host {
    Host::new(false, false, Drops::from_text(None, 1))
}

fn hear(host: &mut Host, tabs: &mut Tabs<Pane<u32>>, tab: u64, kind: HookKind, secs: f64) {
    let message = message(tab, kind);
    host.heard(&message, (secs * 1000.0) as i64);
    route(tabs, message, secs);
}

fn tone(tabs: &Tabs<Pane<u32>>) -> Tone {
    agent_frame::tones(tabs)[0]
}

fn poll(tabs: &mut Tabs<Pane<u32>>, now: f64) {
    let quiet = |_: &u32| TermSnapshot::default();
    let mut notifier = Notifier::new(|_| {});
    agent_frame::poll(tabs, quiet, &mut notifier, false, now);
}

fn timed(fact: Fact, ts_ms: i64) -> Timed {
    Timed { fact, ts_ms: Some(ts_ms) }
}

fn end_of_turn(ts_ms: i64) -> Vec<Timed> {
    vec![
        timed(Fact::Said { tail: MESSAGE.into() }, ts_ms - 100),
        timed(Fact::Turn { started: false, ts_ms: Some(ts_ms) }, ts_ms),
    ]
}

fn feed(tabs: &mut Tabs<Pane<u32>>, tab: u64, facts: Vec<Timed>) {
    let agent = tabs.get_mut(tab).unwrap().pane.agent.as_mut().unwrap();
    agent.reader.push_panel_facts(facts);
}

fn names(host: &Host, tab: u64) -> Vec<&'static str> {
    host.tabs[&tab].ready.as_ref().unwrap().log.events().iter().map(|e| e.fact.name()).collect()
}

#[test]
fn a_turn_whose_stop_hook_was_lost_settles_when_the_transcript_shows_it_ended() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    hear(
        &mut host,
        &mut tabs,
        id,
        HookKind::ToolStarted { key: "shell\nls".into(), id: Some("t1".into()) },
        2.0,
    );
    poll(&mut tabs, 5.0);
    assert_eq!(tone(&tabs), Tone::Working);

    feed(&mut tabs, id, end_of_turn(5_000));
    host.collect(&mut tabs, 5.0, 5_000);
    poll(&mut tabs, 5.0);

    assert_eq!(tone(&tabs), Tone::Done);
    let agent = tabs.get_mut(id).unwrap().pane.agent.as_ref().unwrap();
    assert_eq!(
        agent.run.end_block().map(|block| block.done.clone()),
        Some(vec!["the helper".to_owned()])
    );
    assert_eq!(host.view(id, 5_000).map(|view| view.tone), Some(StoreTone::Done));
}

#[test]
fn without_the_transcript_the_same_turn_is_still_working() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    host.collect(&mut tabs, 5.0, 5_000);
    poll(&mut tabs, 5.0);
    assert_eq!(tone(&tabs), Tone::Working);
}

#[test]
fn an_end_mark_from_before_the_turn_the_hooks_opened_settles_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    hear(&mut host, &mut tabs, id, HookKind::TurnDone, 2.0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 3.5);
    poll(&mut tabs, 4.0);
    assert_eq!(tone(&tabs), Tone::Working);

    feed(&mut tabs, id, end_of_turn(2_900));
    host.collect(&mut tabs, 4.0, 4_000);
    poll(&mut tabs, 4.0);

    assert_eq!(tone(&tabs), Tone::Working);
}

#[test]
fn the_guard_for_an_end_mark_compares_the_cli_time_with_the_hook_that_opened_the_turn() {
    let cases = [
        (Some(5_000), Some(4_000), true),
        (Some(4_000), Some(4_000), true),
        (Some(3_999), Some(4_000), false),
        (None, Some(4_000), true),
        (Some(1), None, true),
        (None, None, true),
    ];
    for (mark, opened, want) in cases {
        assert_eq!(ends_after(mark, opened), want, "{mark:?} {opened:?}");
    }
}

fn start_of_turn(ts_ms: i64) -> Vec<Timed> {
    vec![timed(Fact::Turn { started: true, ts_ms: Some(ts_ms) }, ts_ms)]
}

#[test]
fn with_every_hook_lost_the_transcript_alone_opens_a_turn_and_closes_it_and_opens_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    let steps = [
        (start_of_turn(1_000), 1.0, Tone::Working),
        (end_of_turn(3_000), 3.0, Tone::Done),
        (start_of_turn(5_000), 5.0, Tone::Working),
        (end_of_turn(6_000), 6.0, Tone::Done),
    ];
    for (facts, now, want) in steps {
        feed(&mut tabs, id, facts);
        host.collect(&mut tabs, now, (now * 1000.0) as i64);
        poll(&mut tabs, now);
        assert_eq!(tone(&tabs), want, "at {now}");
    }
}

#[test]
fn a_start_mark_of_a_turn_the_hooks_already_finished_does_not_open_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    hear(&mut host, &mut tabs, id, HookKind::TurnDone, 2.0);
    poll(&mut tabs, 2.0);
    assert_eq!(tone(&tabs), Tone::Done);
    feed(&mut tabs, id, start_of_turn(1_100));
    host.collect(&mut tabs, 2.5, 2_500);
    poll(&mut tabs, 2.5);
    assert_eq!(tone(&tabs), Tone::Done);
}

#[test]
fn an_end_mark_that_comes_with_the_next_start_leaves_the_new_turn_open() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    feed(&mut tabs, id, start_of_turn(1_000));
    host.collect(&mut tabs, 1.0, 1_000);
    let mut batch = end_of_turn(3_000);
    batch.extend(start_of_turn(4_000));
    feed(&mut tabs, id, batch);
    host.collect(&mut tabs, 4.0, 4_000);
    poll(&mut tabs, 4.0);
    assert_eq!(tone(&tabs), Tone::Working);
}

#[test]
fn a_launcher_that_is_gone_ends_the_turn_and_an_agent_gone_from_a_shell_ends_it_without_an_exit() {
    for hosting in [Hosting::Launcher, Hosting::Shell] {
        let dir = tempfile::tempdir().unwrap();
        let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
        let mut host = memory_host();
        host.sync(&tabs, &egui::Context::default());
        host.collect(&mut tabs, 0.0, 0);
        hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
        poll(&mut tabs, 3.0);
        assert_eq!(tone(&tabs), Tone::Working);

        host.presence(id, Presence::Alive, hosting, 2_000);
        host.presence(id, Presence::Gone, hosting, 3_000);
        host.collect(&mut tabs, 3.0, 3_000);
        poll(&mut tabs, 3.0);

        assert_eq!(tone(&tabs), Tone::Done, "{hosting:?}");
        let facts = names(&host, id);
        let tail = &facts[facts.len() - 2..];
        match hosting {
            Hosting::Launcher => assert_eq!(tail, ["process", "process"]),
            Hosting::Shell => assert_eq!(tail, ["process", "turn_ended"]),
        }
    }
}

#[test]
fn a_permission_prompt_is_not_settled_by_a_transcript_that_has_no_end_mark_yet() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    hear(&mut host, &mut tabs, id, HookKind::PermissionNeeded, 2.0);
    feed(&mut tabs, id, vec![timed(Fact::Said { tail: "ok?".into() }, 2_500)]);
    host.collect(&mut tabs, 3.0, 3_000);
    poll(&mut tabs, 3.0);
    assert_eq!(tone(&tabs), Tone::Needs);
}

fn snapshot(activity: bool, title: &str) -> TermSnapshot {
    TermSnapshot { activity, title: Some(title.into()), ..TermSnapshot::default() }
}

#[test]
fn output_and_title_changes_are_logged_once_a_second_and_progress_and_exit_once_each() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    let before = names(&host, id).len();
    let mut due = None;
    for step in 0..9 {
        let now_ms = 10_000 + step * 100;
        due = host.terminal(&[(id, snapshot(true, &format!("title {step}")))], now_ms);
    }
    let logged: Vec<&str> = names(&host, id)[before..].to_vec();
    assert_eq!(logged, ["terminal", "output"]);
    assert_eq!(due, Some(11_000));
    host.terminal(&[(id, snapshot(false, "title 8"))], 11_050);
    let logged: Vec<&str> = names(&host, id)[before..].to_vec();
    assert_eq!(logged, ["terminal", "output", "terminal", "output"]);

    let progress = TermSnapshot { progress: Progress::Indeterminate, ..snapshot(false, "title 8") };
    for now_ms in [12_000, 12_100, 12_200] {
        host.terminal(&[(id, progress.clone())], now_ms);
    }
    let exited = TermSnapshot { exit_code: Some(0), ..progress };
    for now_ms in [13_000, 13_100] {
        host.terminal(&[(id, exited.clone())], now_ms);
    }
    let logged: Vec<&str> = names(&host, id)[before + 4..].to_vec();
    assert_eq!(logged, ["terminal", "process"]);
}

#[test]
fn a_quiet_window_asks_for_no_frame_and_no_probe() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    let ctx = egui::Context::default();
    host.sync(&tabs, &ctx);
    host.collect(&mut tabs, 0.0, 0);
    let quiet = TermSnapshot::default();
    assert_eq!(host.terminal(&[(id, quiet.clone()), (id, quiet)], 5_000), None);
    assert_eq!(host.probe(Vec::new(), false, &ctx, 5_000), None);
    host.schedule.step(0, true);
    host.schedule.finished();
    assert_eq!(host.probe(Vec::new(), true, &ctx, 500), Some(1_500));
}

#[test]
fn the_start_of_an_agent_a_resume_and_a_kept_run_are_logged_once_each() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    host.collect(&mut tabs, 1.0, 1_000);
    assert_eq!(names(&host, id), ["started"]);
    let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
    run.resumed(2.0);
    run.kept_running();
    host.collect(&mut tabs, 2.0, 2_000);
    assert_eq!(names(&host, id), ["started", "restart"]);
    let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
    run.resumed(3.0);
    host.collect(&mut tabs, 3.0, 3_000);
    assert_eq!(names(&host, id), ["started", "restart", "hook"]);
}

#[test]
fn a_pull_request_is_logged_when_it_changes_and_not_when_it_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    let open = PrFacts { number: 7, state: PrWord::Open, checks: Some(ChecksWord::Running) };
    host.pr(id, open.clone(), 1_000);
    host.pr(id, open, 2_000);
    host.pr(
        id,
        PrFacts { number: 7, state: PrWord::Open, checks: Some(ChecksWord::Passed) },
        3_000,
    );
    assert_eq!(names(&host, id), ["started", "pr", "pr"]);
}

#[test]
fn a_hook_for_a_closed_tab_leaves_no_log_and_no_panic() {
    let mut host = memory_host();
    host.heard(&message(77, HookKind::TurnDone), 1_000);
    assert_eq!(host.view(77, 1_000).map(|view| view.shown_seq), None);
}

#[test]
fn drops_are_none_all_or_about_the_share_asked_for() {
    let mut none = Drops::from_text(None, 7);
    let mut all = Drops::from_text(Some("100"), 7);
    let mut zero = Drops::from_text(Some("0"), 7);
    let mut junk = Drops::from_text(Some("lots"), 7);
    let mut huge = Drops::from_text(Some("900"), 7);
    let mut over = Drops::from_text(Some("150"), 7);
    for _ in 0..200 {
        assert!(!none.should_drop() && !zero.should_drop() && !junk.should_drop());
        assert!(!huge.should_drop());
        assert!(all.should_drop() && over.should_drop());
    }
    let mut half = Drops::from_text(Some("50"), 12345);
    let dropped = (0..2_000).filter(|_| half.should_drop()).count();
    assert!((800..1_200).contains(&dropped), "{dropped}");
}

#[test]
fn the_shadow_speaks_once_after_two_seconds_of_disagreement_and_not_for_a_blip() {
    let mut slot = None;
    assert_eq!(Shadow::observe(&mut slot, Tone::Working, Tone::Working, 0), Verdict::Agree);
    assert_eq!(Shadow::observe(&mut slot, Tone::Working, Tone::Done, 1_000), Verdict::Wait(3_001));
    assert_eq!(Shadow::observe(&mut slot, Tone::Working, Tone::Done, 2_500), Verdict::Wait(3_001));
    assert_eq!(
        Shadow::observe(&mut slot, Tone::Working, Tone::Done, 3_100),
        Verdict::Tell { for_ms: 2_100 }
    );
    assert_eq!(Shadow::observe(&mut slot, Tone::Working, Tone::Done, 4_000), Verdict::Told);
    assert_eq!(Shadow::observe(&mut slot, Tone::Done, Tone::Done, 4_100), Verdict::Agree);
    assert_eq!(Shadow::observe(&mut slot, Tone::Needs, Tone::Done, 5_000), Verdict::Wait(7_001));
    assert_eq!(Shadow::observe(&mut slot, Tone::Done, Tone::Working, 5_500), Verdict::Wait(7_501));
}

fn wait_until(host: &mut Host, tabs: &Tabs<Pane<u32>>, done: impl Fn(&Host) -> bool) {
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        host.sync(tabs, &ctx);
        if done(host) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the log was not read in time");
}

fn written(file: &Path, want: usize) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let events = panel_store::load(file).events().to_vec();
        if events.len() >= want || Instant::now() > deadline {
            return events;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_log_on_disk_is_read_first_and_the_facts_that_came_meanwhile_continue_its_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.json");
    let file: PathBuf = panel_log_file_beside(&notes);
    let mut earlier = Log::new();
    for at in [100, 200, 300] {
        earlier.append(Source::Hook, Fact::Output, at);
    }
    panel_store::save(&file, &earlier).unwrap();

    let (mut tabs, id) = tabs_with_agent(&notes);
    let mut host = Host::new(true, false, Drops::from_text(None, 1));
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 400);
    wait_until(&mut host, &tabs, |host| host.tabs.get(&id).is_some_and(|p| p.ready.is_some()));
    host.collect(&mut tabs, 1.0, 500);
    let seqs: Vec<u64> =
        host.tabs[&id].ready.as_ref().unwrap().log.events().iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    drop(host);
    let on_disk = written(&file, 4);
    assert_eq!(on_disk.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(on_disk[3].fact.name(), "started");
}

#[test]
fn what_a_tab_logs_is_on_disk_when_the_window_closes_and_replays_to_the_same_state() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.json");
    let file = panel_log_file_beside(&notes);
    let (mut tabs, id) = tabs_with_agent(&notes);
    let mut host = Host::new(true, false, Drops::from_text(None, 1));
    wait_until(&mut host, &tabs, |host| host.tabs.get(&id).is_some_and(|p| p.ready.is_some()));
    host.collect(&mut tabs, 0.0, 0);
    hear(&mut host, &mut tabs, id, HookKind::TurnStarted, 1.0);
    feed(&mut tabs, id, end_of_turn(5_000));
    host.collect(&mut tabs, 5.0, 5_000);
    let live = host.view(id, 6_000).unwrap();
    drop(host);
    let log = panel_store::load(&file);
    assert_eq!(log.len(), 4);
    assert_eq!(log.replay().view(6_000), live);
}

#[test]
fn a_hook_that_is_dropped_never_reaches_the_log_but_the_other_events_do() {
    let dir = tempfile::tempdir().unwrap();
    let (mut tabs, id) = tabs_with_agent(&dir.path().join("notes.json"));
    let mut host = Host::new(false, false, Drops::from_text(Some("100"), 1));
    host.sync(&tabs, &egui::Context::default());
    host.collect(&mut tabs, 0.0, 0);
    assert!(host.drops_hook());
    let ask = TabMessage {
        tab_id: id,
        generation: 0,
        event: TabEvent::Ask {
            questions: vec![Question {
                text: "Which one?".into(),
                header: None,
                decision: None,
                select: Select::Single,
                options: Vec::new(),
            }],
            reply: AskReply::new(std::sync::mpsc::channel().0),
        },
    };
    host.heard(&ask, 1_000);
    assert_eq!(names(&host, id), ["started", "ask_opened"]);
    let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
    run.set_asking(Some("Which one?".into()));
    host.collect(&mut tabs, 2.0, 2_000);
    let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
    run.set_asking(None);
    host.collect(&mut tabs, 3.0, 3_000);
    assert_eq!(names(&host, id), ["started", "ask_opened", "ask_closed"]);
}

fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0))
}

fn rfc3339(ms: i64) -> String {
    let date = tidewatch_core::civil_date::Date::from_unix_secs(ms / 1000);
    let day = ms.rem_euclid(86_400_000);
    format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        date,
        day / 3_600_000,
        day / 60_000 % 60,
        day / 1000 % 60,
        day % 1000
    )
}

#[test]
fn the_real_feed_and_reader_end_a_turn_whose_stop_hook_never_came_within_three_seconds() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let session = "44444444-aaaa-bbbb-cccc-000000000004";
    let (mut tabs, id) = tabs_with_agent(&home.path().join("notes.json"));
    {
        let agent = tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap();
        agent.reader =
            crate::reader::Reader::for_agent("claude", Some(session.into()), cwd.clone());
        agent.reader.start(home.path(), || {});
    }
    let mut host = memory_host();
    host.sync(&tabs, &egui::Context::default());
    let opened = wall_ms();
    host.collect(&mut tabs, 0.0, opened);
    let message = message(id, HookKind::TurnStarted);
    host.heard(&message, opened);
    route(&mut tabs, message, 0.0);

    let dir = home
        .path()
        .join(".claude")
        .join("projects")
        .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let prompt = format!(
        "{{\"type\":\"user\",\"timestamp\":\"{}\",\"message\":{{\"content\":\"go\"}}}}\n",
        rfc3339(opened + 1)
    );
    std::fs::write(dir.join(format!("{session}.jsonl")), prompt).unwrap();
    std::thread::sleep(Duration::from_millis(1_200));
    poll(&mut tabs, 1.2);
    assert_eq!(tone(&tabs), Tone::Working);

    let wrote = wall_ms();
    let end = format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"stop_reason\":\"end_turn\",\"content\":[{{\"type\":\"text\",\"text\":\"done\"}}]}}}}\n",
        rfc3339(wrote)
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(format!("{session}.jsonl")))
        .unwrap();
    std::io::Write::write_all(&mut file, end.as_bytes()).unwrap();
    let began = Instant::now();
    while tone(&tabs) != Tone::Done && began.elapsed() < Duration::from_secs(6) {
        let agent = tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap();
        agent.reader.drain();
        let now = 1.2 + began.elapsed().as_secs_f64();
        host.collect(&mut tabs, now, wall_ms());
        poll(&mut tabs, now);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(tone(&tabs), Tone::Done);
    assert!(began.elapsed() < Duration::from_secs(3), "{:?}", began.elapsed());
}
