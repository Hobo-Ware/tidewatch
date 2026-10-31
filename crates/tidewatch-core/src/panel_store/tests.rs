use super::*;
use crate::panel_event::{Fact, Source};
use crate::panel_state::PanelState;
use crate::status::{HookEvent, HookKind};

fn hook(kind: HookKind, message: Option<String>) -> Fact {
    Fact::Hook(HookEvent { kind, session_id: None, transcript_path: None, message })
}

/// A turn of a few facts per round, so a long run folds into something worth comparing.
fn fact_at(n: usize) -> Fact {
    match n % 6 {
        0 => hook(HookKind::TurnStarted, None),
        1 => hook(
            HookKind::ToolStarted { key: format!("Bash\nls {n}"), id: Some(format!("c{n}")) },
            None,
        ),
        2 => Fact::Output,
        3 => hook(
            HookKind::ToolFinished {
                key: format!("Bash\nls {}", n - 2),
                id: Some(format!("c{}", n - 2)),
            },
            None,
        ),
        4 => Fact::Said { tail: format!("round {n}") },
        _ => hook(HookKind::TurnDone, Some(format!("done {n}"))),
    }
}

fn filled(count: usize) -> Log {
    let mut log = Log::new();
    log.append(Source::App, Fact::Started { hooked: true }, 1_000);
    for n in 0..count {
        let source = if n % 6 == 2 { Source::Terminal } else { Source::Hook };
        log.append(source, fact_at(n), 2_000 + 10 * n as i64);
    }
    log
}

fn lines_in(file: &Path) -> usize {
    std::fs::read_to_string(file).unwrap().lines().count()
}

#[test]
fn a_saved_log_loads_back_the_same_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("tab").join("panel-log.jsonl");
    let log = filled(40);
    save(&file, &log).unwrap();
    assert_eq!(load(&file), log);
    assert_eq!(load(&file).replay(), log.replay());
    let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["panel-log.jsonl"]);
}

#[test]
fn a_missing_file_is_an_empty_log_that_starts_at_one() {
    let dir = tempfile::tempdir().unwrap();
    let log = load(&dir.path().join("none.jsonl"));
    assert_eq!((log.len(), log.next_seq()), (0, 1));
}

#[test]
fn a_bad_line_and_a_line_cut_by_a_crash_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let log = filled(9);
    let mut text = text_of(log.events());
    text.insert_str(text.find('\n').unwrap() + 1, "not json at all\n\n{\"seq\":99}\n");
    let cut = serde_json::to_string(&log.events()[3]).unwrap();
    text.push_str(&cut[..cut.len() / 2]);
    std::fs::write(&file, text).unwrap();
    let back = load(&file);
    assert_eq!(back, log);
}

#[test]
fn numbers_that_do_not_grow_are_dropped_so_the_order_is_the_numbers_order() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let log = filled(6);
    let events = log.events();
    let order = [0, 1, 2, 1, 3, 2, 4, 5, 6];
    let picked: Vec<Event> = order.iter().map(|i| events[*i].clone()).collect();
    let text = text_of(&picked);
    std::fs::write(&file, text).unwrap();
    let back = load(&file);
    let seqs: Vec<u64> = back.events().iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(back.next_seq(), 8);
}

#[test]
fn the_writer_appends_in_order_and_a_new_writer_goes_on_after_a_cut_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let mut log = filled(5);
    let writer = Writer::new(file.clone());
    for event in log.events() {
        writer.append(event.clone());
    }
    drop(writer);
    assert_eq!(load(&file), log);
    let mut cut = std::fs::read_to_string(&file).unwrap();
    cut.push_str("{\"seq\":7,\"at_ms\":");
    std::fs::write(&file, cut).unwrap();
    let writer = Writer::new(file.clone());
    let next = log.append(Source::Hook, hook(HookKind::TurnStarted, None), 9_000);
    writer.append(next);
    drop(writer);
    assert_eq!(load(&file), log, "the event after the cut line is whole, not glued to it");
}

#[test]
fn the_log_never_holds_more_than_the_event_cap_and_replays_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let live = filled(CAP_EVENTS * 2 + 345);
    let writer = Writer::new(file.clone());
    for event in live.events() {
        writer.append(event.clone());
    }
    drop(writer);
    let lines = lines_in(&file);
    assert!(lines <= CAP_EVENTS, "{lines} lines on disk");
    assert!(lines > SMALLEST_KEEP, "{lines} lines on disk");
    let back = load(&file);
    assert_eq!(back.next_seq(), live.next_seq());
    assert_eq!(back.replay(), live.replay());
}

#[test]
fn the_cap_holds_after_every_single_append() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let mut disk = Disk::open(file.clone());
    let live = filled(CAP_EVENTS + 50);
    for event in live.events() {
        disk.write(std::slice::from_ref(event));
        assert!(disk.lines <= CAP_EVENTS, "{} lines after seq {}", disk.lines, event.seq);
    }
    assert_eq!(lines_in(&file), disk.lines);
}

#[test]
fn the_byte_cap_holds_even_when_every_event_is_large() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let mut live = Log::new();
    for n in 0..600 {
        let message = format!("{n} {}", "word ".repeat(400));
        live.append(Source::Hook, hook(HookKind::TurnDone, Some(message)), 1_000 + n);
    }
    let writer = Writer::new(file.clone());
    for event in live.events() {
        writer.append(event.clone());
    }
    drop(writer);
    let size = std::fs::metadata(&file).unwrap().len() as usize;
    assert!(size <= CAP_BYTES, "{size} bytes on disk");
    assert_eq!(load(&file).replay(), live.replay());
}

#[test]
fn a_file_already_over_the_cap_is_compacted_when_the_writer_starts() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let live = filled(CAP_EVENTS + 400);
    save(&file, &live).unwrap();
    assert!(lines_in(&file) > CAP_EVENTS);
    drop(Writer::new(file.clone()));
    assert!(lines_in(&file) <= CAP_EVENTS);
    assert_eq!(load(&file).replay(), live.replay());
}

#[test]
fn compacting_keeps_the_state_a_screen_would_have_shown() {
    let mut log = filled(CAP_EVENTS + 100);
    let before: PanelState = log.replay();
    compact(&mut log);
    assert!(log.len() <= CAP_EVENTS / 2 + 1);
    assert_eq!(log.replay(), before);
}
