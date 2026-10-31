use super::*;
use crate::sent::{Reason, Record};
use crate::transcript::{Parser, Tail};

const CLAUDE_USAGE: &str = include_str!("../../tests/fixtures/claude_usage.jsonl");
const CODEX: &str = include_str!("../../tests/fixtures/codex_live.jsonl");
const GEMINI: &str = include_str!("../../tests/fixtures/gemini.jsonl");
const COPILOT: &str = include_str!("../../tests/fixtures/copilot.jsonl");

fn read(source: Source, text: &str) -> Tally {
    let mut parser = Parser::new(source);
    for line in text.lines() {
        parser.feed_line(line);
    }
    parser.tokens()
}

fn usage(input: u64, cache_read: u64, cache_write: u64, output: u64) -> Usage {
    Usage { input, cache_read, cache_write, output }
}

#[test]
fn a_claude_message_written_as_several_lines_counts_once_and_a_synthetic_one_not_at_all() {
    let tally = read(Source::Claude, CLAUDE_USAGE);
    assert_eq!(tally.calls, 3);
    assert_eq!(
        tally.total,
        usage(2 + 3 + 1, 35_772 + 178_967 + 180_172, 143_195 + 1_205 + 4_693, 1_045 + 271 + 1_226)
    );
}

#[test]
fn each_user_message_starts_a_turn_that_owns_the_calls_after_it() {
    let tally = read(Source::Claude, CLAUDE_USAGE);
    assert_eq!(tally.turn_count, 2);
    let spent: Vec<_> =
        tally.turns.iter().map(|turn| (turn.fingerprint.as_str(), turn.usage)).collect();
    assert_eq!(
        spent,
        [
            ("Count the files in src.", usage(5, 214_739, 144_400, 1_316)),
            ("Thanks. Now list them.", usage(1, 180_172, 4_693, 1_226)),
        ]
    );
    let turn_total: u64 = tally.turns.iter().map(|turn| turn.usage.total()).sum();
    assert_eq!(turn_total, tally.total.total());
}

#[test]
fn a_tool_result_is_not_a_turn() {
    let tally = read(Source::Claude, CLAUDE_USAGE);
    assert_eq!(tally.turns.len(), 2);
}

#[test]
fn codex_running_totals_become_the_difference_of_each_reading_with_cached_input_split_out() {
    let tally = read(Source::Codex, CODEX);
    assert_eq!(tally.calls, 4);
    assert_eq!(tally.total, usage(57_632 - 52_224, 52_224, 0, 173));
    assert_eq!(tally.total.total(), 57_805);
}

#[test]
fn a_cli_without_token_counts_in_its_transcript_counts_nothing() {
    for (source, text) in [(Source::Gemini, GEMINI), (Source::Copilot, COPILOT)] {
        assert_eq!(read(source, text).total, Usage::default(), "{source:?}");
        assert_eq!(read(source, text).calls, 0, "{source:?}");
    }
}

#[test]
fn lines_that_are_not_usage_change_nothing() {
    let mut parser = Parser::new(Source::Claude);
    for line in ["", "not json", "[]", r#"{"type":"assistant","message":{"usage":"x"}}"#] {
        parser.feed_line(line);
    }
    assert_eq!(parser.tokens(), Tally::default());
}

#[test]
fn only_the_newest_turns_are_kept_and_the_count_goes_on() {
    let mut counter = Counter::default();
    for n in 0..(TURNS_KEPT + 5) {
        counter.user(&format!("message {n}"), None);
    }
    let tally = counter.tally();
    assert_eq!(tally.turn_count as usize, TURNS_KEPT + 5);
    assert_eq!(tally.turns.len(), TURNS_KEPT);
    assert_eq!(tally.turns[0].fingerprint, "message 5");
}

#[test]
fn a_file_that_shrinks_is_counted_again_from_its_top_and_one_that_grows_goes_on() {
    let dir = std::env::temp_dir().join(format!("tw-tokens-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("session.jsonl");
    let lines: Vec<&str> = CLAUDE_USAGE.lines().collect();
    std::fs::write(&file, format!("{}\n", lines[..5].join("\n"))).unwrap();
    let mut tail = Tail::new(Source::Claude, &file);
    tail.poll();
    assert_eq!((tail.tokens().calls, tail.tokens().turn_count), (2, 1));
    std::fs::write(&file, CLAUDE_USAGE).unwrap();
    tail.poll();
    assert_eq!((tail.tokens().calls, tail.tokens().turn_count), (3, 2));
    std::fs::write(&file, format!("{}\n", lines[0])).unwrap();
    tail.poll();
    assert_eq!((tail.tokens().calls, tail.tokens().turn_count), (0, 1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_cached_share_is_a_whole_percent_of_what_the_model_read() {
    let cases = [
        (usage(0, 0, 0, 50), None),
        (usage(10, 0, 0, 0), Some(0)),
        (usage(2, 94, 4, 100), Some(94)),
        (usage(0, 1, 2, 0), Some(33)),
        (usage(0, 200, 0, 0), Some(100)),
    ];
    for (usage, want) in cases {
        assert_eq!(usage.cached_percent(), want, "{usage:?}");
    }
}

#[test]
fn counts_read_in_a_few_characters() {
    let cases = [
        (0, "0"),
        (980, "980"),
        (999, "999"),
        (1_000, "1k"),
        (1_240, "1.2k"),
        (14_000, "14k"),
        (14_600, "15k"),
        (999_499, "999k"),
        (999_999, "1M"),
        (1_000_000, "1M"),
        (1_240_000, "1.2M"),
        (12_400_000, "12M"),
    ];
    for (count, want) in cases {
        assert_eq!(compact(count), want, "{count}");
    }
}

#[test]
fn the_summary_names_tokens_cache_and_turns_and_stays_silent_before_the_first_call() {
    assert_eq!(summary(&Tally::default()), None);
    assert_eq!(short(&Tally::default()), None);
    let tally = Tally {
        total: usage(2_000, 1_100_000, 40_000, 58_000),
        calls: 40,
        turn_count: 18,
        turns: Vec::new(),
    };
    assert_eq!(short(&tally).as_deref(), Some("1.2M tokens"));
    assert_eq!(summary(&tally).as_deref(), Some("1.2M tokens \u{b7} 96% cached \u{b7} 18 turns"));
    let one = Tally { turn_count: 1, ..tally };
    assert_eq!(summary(&one).as_deref(), Some("1.2M tokens \u{b7} 96% cached \u{b7} 1 turn"));
}

fn sent(text: &str, at_ms: i64) -> Record {
    Record { at_ms, typed: true, ..Record::new(Reason::DoneCheck, "the done check", text, None) }
}

fn turn(text: &str, started_ms: Option<i64>, output: u64) -> Turn {
    Turn { started_ms, fingerprint: fingerprint(text), usage: usage(0, 0, 0, output) }
}

#[test]
fn the_turns_tidewatch_started_are_summed_with_the_messages_that_started_them() {
    let turns = [
        turn("Build the thing", Some(1_000), 500),
        turn("Fix  what the\ndone check found", Some(61_000), 300),
        turn("Thanks", Some(120_000), 900),
        turn("Fix what the done check found", Some(180_000), 100),
    ];
    let records = [
        sent("Fix what the done check found", 60_000),
        sent("Fix what the done check found", 179_000),
    ];
    let share = share_of(&records, &turns);
    assert_eq!(share, Share { messages: 2, usage: usage(0, 0, 0, 400) });
}

#[test]
fn a_turn_answers_one_record() {
    let turns = [turn("Check", Some(10_000), 70)];
    let twice = [sent("Check", 9_000), sent("Check", 9_500)];
    assert_eq!(share_of(&twice, &turns).messages, 1);
}

#[test]
fn a_record_matches_a_turn_that_started_a_little_before_it_or_up_to_ten_minutes_after() {
    let turns = [turn("Check", Some(10_000), 70)];
    let cases = [
        (10_000 + SENT_SLACK_MS, 1),
        (10_000 + SENT_SLACK_MS + 1, 0),
        (10_000 - SENT_WINDOW_MS, 1),
        (10_000 - SENT_WINDOW_MS - 1, 0),
    ];
    for (at_ms, messages) in cases {
        assert_eq!(share_of(&[sent("Check", at_ms)], &turns).messages, messages, "{at_ms}");
    }
}

#[test]
fn a_reminder_told_through_a_hook_is_not_in_the_transcript_so_it_counts_nothing() {
    let told = Record { typed: false, ..Record::new(Reason::LedgerGuard, "stale", "Report", None) };
    assert_eq!(share_of(&[told], &[turn("Report", Some(1), 10)]), Share::default());
}

#[test]
fn the_share_line_says_how_many_messages_and_how_much() {
    assert_eq!(share_line(Share::default()), None);
    let one = Share { messages: 1, usage: usage(0, 0, 0, 14_000) };
    assert_eq!(share_line(one).as_deref(), Some("Tidewatch cost this tab 14k tokens (1 message)"));
    let four = Share { messages: 4, usage: usage(1_000, 0, 0, 13_000) };
    assert_eq!(
        share_line(four).as_deref(),
        Some("Tidewatch cost this tab 14k tokens (4 messages)")
    );
}
