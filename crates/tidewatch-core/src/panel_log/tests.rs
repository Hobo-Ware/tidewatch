use super::*;
use crate::panel_view::Tone;
use crate::status::{HookEvent, HookKind};

fn hook(kind: HookKind) -> Fact {
    Fact::Hook(HookEvent { kind, session_id: None, transcript_path: None, message: None })
}

fn walk(count: u64) -> Log {
    let mut log = Log::new();
    log.append(Source::App, Fact::Started { hooked: true }, 100);
    for n in 0..count {
        let fact = if n % 2 == 0 { hook(HookKind::TurnStarted) } else { hook(HookKind::TurnDone) };
        log.append(Source::Hook, fact, 1_000 + 10 * n as i64);
    }
    log
}

#[test]
fn events_are_numbered_from_one_and_never_reused() {
    let mut log = Log::new();
    let first = log.append(Source::Hook, Fact::Output, 5);
    let second = log.append(Source::Hook, Fact::Output, 1);
    assert_eq!((first.seq, second.seq), (1, 2));
    assert_eq!(log.next_seq(), 3);
}

#[test]
fn events_since_returns_what_came_after_a_number() {
    let log = walk(9);
    let seqs = |from| log.events_since(from).iter().map(|e| e.seq).collect::<Vec<_>>();
    assert_eq!(seqs(0), (1..=10).collect::<Vec<_>>());
    assert_eq!(seqs(7), [8, 9, 10]);
    assert_eq!(seqs(10), Vec::<u64>::new());
    assert_eq!(seqs(99), Vec::<u64>::new());
}

#[test]
fn a_trimmed_log_replays_to_the_state_of_the_whole_one() {
    for keep in [0, 1, 2, 5, 9, 10, 11, 50] {
        let mut log = walk(9);
        let whole = log.replay();
        log.trim(keep);
        assert_eq!(log.replay(), whole, "keep {keep}");
        assert_eq!(log.len(), if keep >= 10 { 10 } else { keep + 1 }, "keep {keep}");
        assert_eq!(log.next_seq(), 11, "keep {keep}");
    }
}

#[test]
fn trimming_twice_still_replays_exactly() {
    let mut log = walk(40);
    let whole = log.replay();
    log.trim(20);
    log.append(Source::Hook, hook(HookKind::TurnStarted), 9_000);
    let grown = PanelState::replay(walk(40).events().iter().chain([&Event {
        seq: 42,
        at_ms: 9_000,
        source: Source::Hook,
        fact: hook(HookKind::TurnStarted),
    }]));
    assert_eq!(log.replay(), grown);
    log.trim(5);
    assert_eq!(log.replay(), grown);
    assert_ne!(grown, whole);
    assert_eq!(log.replay().view(9_100).tone, Tone::Working);
}

#[test]
fn the_checkpoint_takes_the_number_of_the_last_event_it_stands_for() {
    let mut log = walk(9);
    log.trim(3);
    let first = &log.events()[0];
    assert!(matches!(first.fact, Fact::Checkpoint(_)));
    assert_eq!(first.seq, 7);
    assert_eq!(first.at_ms, 1_050, "the time of the last event it stands for");
    assert_eq!(log.events()[1].seq, 8);
}
