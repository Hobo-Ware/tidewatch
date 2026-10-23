use tidewatch_core::comment_flags::Origin;
use tidewatch_core::comments::{Flag, Kind};

use super::*;

fn record(file: &str, line: usize, kind: &str, text: &str) -> Record {
    Record {
        at_ms: 1,
        origin: Origin::Edit,
        file: file.into(),
        line,
        kind: kind.into(),
        text: text.into(),
    }
}

#[test]
fn the_chip_counts_flags_and_says_when_the_tab_is_off() {
    assert_eq!(chip_text(3, false), "Comment guard: 3 flags");
    assert_eq!(chip_text(1, false), "Comment guard: 1 flag");
    assert_eq!(chip_text(0, false), "Comment guard: 0 flags");
    assert_eq!(chip_text(5, true), "Comment guard: off for this tab");
}

#[test]
fn lines_are_newest_first_capped_and_name_file_and_line() {
    let records = [
        record("a.rs", 3, "narration", "loop over a"),
        record("b.rs", 0, "density", "6 comment lines in 20 lines"),
        record("c.rs", 9, "alibi", "hack"),
    ];
    let shown = lines(&records, 2);
    assert_eq!(shown.len(), 2);
    assert_eq!(
        shown[0],
        Line { place: "c.rs:9".into(), kind: "alibi".into(), text: "hack".into() }
    );
    assert_eq!(shown[1].place, "b.rs");
    assert_eq!(lines(&[], 5), vec![]);
}

fn files(count: usize) -> Vec<FileFlags> {
    vec![FileFlags {
        path: "src/a.rs".into(),
        flags: (1..=count)
            .map(|line| Flag { line, kind: Kind::Narration, text: format!("note {line}") })
            .collect(),
    }]
}

#[test]
fn the_paste_note_lists_the_flags_with_the_advice_and_no_dashes() {
    let note = paste_note(&files(2));
    assert!(note.starts_with("Comment guard: 2 comment(s) you added fail the keep-list.\n- src/a.rs:1 [narration] note 1\n- src/a.rs:2 [narration] note 2\n"), "{note}");
    assert!(note.ends_with(ADVICE));
    assert!(!note.contains('\u{2014}') && !note.contains('\u{2013}'));
}

#[test]
fn a_long_note_lists_eight_flags_and_counts_the_rest() {
    let note = paste_note(&files(11));
    assert_eq!(note.matches("[narration]").count(), 8);
    assert!(note.contains("- and 3 more"));
}

#[test]
fn a_turn_has_ended_once_when_a_working_agent_becomes_idle() {
    let mut turn = Turn::default();
    let steps = [
        (AgentStatus::SettingUp, false),
        (AgentStatus::Done, false),
        (AgentStatus::Working, false),
        (AgentStatus::Working, false),
        (AgentStatus::Done, true),
        (AgentStatus::Done, false),
        (AgentStatus::Working, false),
        (AgentStatus::Failed("x".into()), false),
        (AgentStatus::Done, false),
    ];
    for (status, want) in steps {
        assert_eq!(turn.ended(&status), want, "{status:?}");
    }
}

#[test]
fn only_the_clis_without_the_claude_hooks_are_swept_at_the_end_of_a_turn() {
    assert!(!needs_idle_sweep("claude"));
    for agent in ["codex", "gemini", "copilot", "custom"] {
        assert!(needs_idle_sweep(agent), "{agent}");
    }
}

#[test]
fn suppressions_dead_code_and_alibis_read_louder_than_narration() {
    assert_eq!(kind_tone("dead-code"), Tone::Loud);
    assert_eq!(kind_tone("alibi"), Tone::Loud);
    assert_eq!(kind_tone("suppression"), Tone::Loud);
    assert_eq!(kind_tone("narration"), Tone::Quiet);
}
