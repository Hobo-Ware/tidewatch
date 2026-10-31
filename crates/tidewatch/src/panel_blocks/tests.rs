use egui::{Rect, pos2, vec2};
use tidewatch_core::panel_counter::Outcome;
use tidewatch_core::panel_event::{AskKind, Event, Fact};
use tidewatch_core::panel_rules::ReportDigest;
use tidewatch_core::panel_state::PanelState;
use tidewatch_core::progress::Ledger;
use tidewatch_core::status::{HookEvent, HookKind};

use super::*;

const MIN: i64 = 60_000;
const DAY: i64 = 86_400_000;

fn digest(state: &str, needs: &str) -> ReportDigest {
    ReportDigest::of(&Ledger::parse(&format!(
        "# Goal\ng\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"
    )))
}

fn hook(kind: HookKind) -> Fact {
    Fact::Hook(HookEvent { kind, session_id: None, transcript_path: None, message: None })
}

fn tool(n: u32) -> Fact {
    hook(HookKind::ToolStarted { key: format!("Bash\nls {n}"), id: Some(format!("c{n}")) })
}

struct Story {
    state: PanelState,
    seq: u64,
}

impl Story {
    fn new() -> Self {
        Self { state: PanelState::new(), seq: 0 }
    }

    fn at(mut self, at_ms: i64, source: Source, fact: Fact) -> Self {
        self.seq += 1;
        self.state.apply(&Event { seq: self.seq, at_ms, source, fact });
        self
    }

    fn report(self, at_ms: i64, state: &str, needs: &str) -> Self {
        let fact = Fact::ReportSaved { saved_ms: at_ms, digest: digest(state, needs) };
        self.at(at_ms, Source::Report, fact)
    }

    fn blocks(&self, now_ms: i64) -> (Blocks, PanelView) {
        let view = self.state.view(now_ms);
        (Blocks::of(&view, now_ms, &TimeZone::UTC, Kept::default()), view)
    }
}

#[test]
fn an_age_reads_now_then_minutes_hours_and_days() {
    let cases = [
        (0, "now"),
        (59_999, "now"),
        (60_000, "1m ago"),
        (2 * MIN + 30_000, "2m ago"),
        (59 * MIN, "59m ago"),
        (60 * MIN, "1h ago"),
        (23 * 60 * MIN, "23h ago"),
        (DAY, "1d ago"),
        (9 * DAY, "9d ago"),
        (-5 * MIN, "now"),
    ];
    for (ago_ms, want) in cases {
        assert_eq!(age_text(1_000_000_000, 1_000_000_000 - ago_ms), want, "{ago_ms}");
    }
}

#[test]
fn a_quiet_source_names_the_last_time_it_spoke_and_the_count_since_the_report_is_plural() {
    let at = 14 * 60 * MIN + 2 * MIN;
    assert_eq!(quiet_text(at, &TimeZone::UTC), "no word since 14:02");
    assert_eq!(since_text(0), None);
    assert_eq!(since_text(1).as_deref(), Some("1 thing happened since"));
    assert_eq!(since_text(3).as_deref(), Some("3 things happened since"));
}

#[test]
fn a_report_that_says_work_goes_on_after_the_turn_ended_is_marked_out_of_date() {
    let story = Story::new()
        .at(0, Source::Hook, hook(HookKind::TurnStarted))
        .report(MIN, "working", "- nothing")
        .at(2 * MIN, Source::Hook, hook(HookKind::TurnDone));
    let (blocks, view) = story.blocks(12 * MIN);
    assert_eq!(blocks.out_of_date, Some(Why::WorkingButTurnEnded));
    assert_eq!(view.tone, tidewatch_core::panel_view::Tone::Done);
    assert_eq!(blocks.report.as_deref(), Some("11m ago"));
}

#[test]
fn a_report_that_needs_you_with_no_question_open_is_marked_out_of_date() {
    let story = Story::new().at(0, Source::Hook, hook(HookKind::TurnStarted)).report(
        MIN,
        "needs_you",
        "- nothing",
    );
    let (blocks, view) = story.blocks(2 * MIN);
    assert_eq!(blocks.out_of_date, Some(Why::NeedsYouNoQuestion));
    assert_ne!(view.tone, tidewatch_core::panel_view::Tone::Needs);
}

#[test]
fn only_a_report_the_facts_contradict_drops_its_question() {
    let working = Story::new().at(0, Source::Hook, hook(HookKind::TurnStarted)).report(
        MIN,
        "working",
        "- nothing",
    );
    let ended = working.state.clone();
    let mut ended = Story { state: ended, seq: working.seq };
    ended = ended.at(2 * MIN, Source::Hook, hook(HookKind::TurnDone));
    let answered = Story::new()
        .at(0, Source::Hook, hook(HookKind::TurnStarted))
        .report(MIN, "waiting", "- Pause or go on?")
        .at(2 * MIN, Source::App, Fact::Answered { kind: AskKind::Question });
    let cases = [
        ("no panel", None, false),
        ("a report the facts agree with", Some(working.state.view(MIN + 1_000)), false),
        ("work that ended", Some(ended.state.view(12 * MIN)), true),
        ("a question the owner answered", Some(answered.state.view(3 * MIN)), true),
    ];
    for (name, view, want) in cases {
        assert_eq!(contradicted(view.as_ref()), want, "{name}");
    }
    assert!(!contradicted(Some(&Story::new().state.view(0))), "no report at all");
}

#[test]
fn a_report_the_facts_agree_with_carries_no_mark() {
    let story = Story::new()
        .at(0, Source::Hook, hook(HookKind::TurnStarted))
        .report(2 * MIN, "working", "- nothing")
        .at(2 * MIN + 500, Source::Transcript, Fact::Output);
    let (blocks, _) = story.blocks(2 * MIN + 1_000);
    assert_eq!(blocks.out_of_date, None);
    assert_eq!(blocks.report.as_deref(), Some("now"));
    assert_eq!(blocks.since, None);
}

#[test]
fn what_happened_since_the_report_is_counted_under_it_and_nothing_without_a_report() {
    let mut story = Story::new().at(0, Source::Hook, hook(HookKind::TurnStarted));
    story = story.at(MIN, Source::Hook, tool(0));
    let (blocks, _) = story.blocks(MIN);
    assert_eq!(blocks.since, None, "no report, nothing to count since");
    story = story.report(2 * MIN, "working", "- nothing");
    for n in 1..=3 {
        story = story.at(3 * MIN + i64::from(n), Source::Hook, tool(n));
    }
    let (blocks, _) = story.blocks(4 * MIN);
    assert_eq!(blocks.since.as_deref(), Some("3 things happened since"));
}

#[test]
fn a_source_gone_quiet_replaces_the_age_of_its_block_with_no_word_since() {
    let story = Story::new()
        .at(0, Source::App, Fact::Started { hooked: true })
        .at(MIN, Source::Hook, hook(HookKind::TurnStarted))
        .at(MIN + 100, Source::Hook, tool(1))
        .at(MIN + 200, Source::Transcript, Fact::Output);
    let (blocks, view) = story.blocks(MIN + 20_000);
    assert_eq!(view.quiet.iter().map(|q| q.source).collect::<Vec<_>>(), [Source::Transcript]);
    assert_eq!(blocks.turn.as_deref(), Some("no word since 00:01"));
    let (busy, _) = story.blocks(MIN + 500);
    assert_eq!(busy.turn.as_deref(), Some("now"));
}

#[test]
fn a_block_whose_source_never_spoke_falls_back_to_the_time_its_own_data_holds() {
    let view = Story::new().at(MIN, Source::Hook, hook(HookKind::TurnStarted)).state.view(MIN);
    let kept = Kept { jobs_ms: Some(MIN - 30 * MIN), links_ms: Some(MIN - 2 * DAY) };
    let blocks = Blocks::of(&view, 3 * MIN, &TimeZone::UTC, kept);
    assert_eq!(blocks.jobs.as_deref(), Some("32m ago"));
    assert_eq!(blocks.links.as_deref(), Some("2d ago"));
    let bare = Blocks::of(&view, 3 * MIN, &TimeZone::UTC, Kept::default());
    assert_eq!((bare.jobs, bare.links), (None, None));
}

#[test]
fn a_card_shows_the_silence_of_its_turn_first_then_the_report_then_the_turn() {
    let both = |report: Option<&str>, turn: Option<&str>| Blocks {
        report: report.map(str::to_owned),
        turn: turn.map(str::to_owned),
        ..Blocks::default()
    };
    let cases = [
        (both(Some("2m ago"), Some("no word since 14:02")), Some("no word since 14:02")),
        (both(Some("2m ago"), Some("now")), Some("2m ago")),
        (both(None, Some("now")), Some("now")),
        (both(None, None), None),
    ];
    for (blocks, want) in cases {
        assert_eq!(blocks.card_age(), want);
    }
}

#[test]
fn a_step_tidewatch_ticked_reads_done_only_when_the_report_left_it_open() {
    let marks = vec![
        MarkSeen { step: "Run the tests".into(), by: StepBy::Tidewatch, at_ms: 0 },
        MarkSeen { step: "Write it".into(), by: StepBy::Agent, at_ms: 0 },
    ];
    assert!(ticked_by_tidewatch(&marks, " Run the tests "));
    assert!(!ticked_by_tidewatch(&marks, "Write it"));
    assert!(!ticked_by_tidewatch(&marks, "Something else"));
    let cases = [
        (Mark::Todo, true, (Mark::Done, true)),
        (Mark::Doing, true, (Mark::Done, true)),
        (Mark::Done, true, (Mark::Done, false)),
        (Mark::Blocked, true, (Mark::Blocked, false)),
        (Mark::Skipped, true, (Mark::Skipped, false)),
        (Mark::Todo, false, (Mark::Todo, false)),
    ];
    for (mark, ticked, want) in cases {
        assert_eq!(tick(mark, ticked), want, "{mark:?} {ticked}");
    }
    assert_eq!(why_with_tick(None, true).as_deref(), Some("by Tidewatch"));
    assert_eq!(
        why_with_tick(Some("ran 8 tests"), true).as_deref(),
        Some("ran 8 tests \u{b7} by Tidewatch")
    );
    assert_eq!(why_with_tick(Some("ran 8 tests"), false).as_deref(), Some("ran 8 tests"));
    assert_eq!(why_with_tick(None, false), None);
}

fn counter_with(on_time: u32, late: u32, missed: u32) -> (Counter, i64) {
    let now = 1_800_000_000_000;
    let mut counter = Counter::new();
    for _ in 0..on_time {
        counter.record(now, Source::Transcript, Outcome::OnTime);
    }
    for _ in 0..late {
        counter.record(now, Source::Hook, Outcome::Late);
    }
    for _ in 0..missed {
        counter.record(now, Source::Hook, Outcome::Missed);
    }
    (counter, now)
}

#[test]
fn the_about_row_says_the_share_on_time_and_names_the_worst_source_when_something_was_late() {
    let cases = [
        (counter_with(0, 0, 0), "no changes yet this week", None),
        (counter_with(40, 0, 0), "100% this week", None),
        (counter_with(2_499, 1, 0), "99.96% this week", Some("Late: hooks 1")),
        (counter_with(997, 2, 1), "99.70% this week", Some("Late: hooks 3")),
        (counter_with(1, 2, 0), "33.33% this week", Some("Late: hooks 2")),
        (counter_with(0, 0, 3), "0.00% this week", Some("Late: hooks 3")),
    ];
    for ((counter, now), line, late) in cases {
        let row = OnTime::of(&counter, now);
        assert_eq!((row.line.as_str(), row.late.as_deref()), (line, late));
    }
}

#[test]
fn the_row_never_rounds_a_late_change_up_to_a_hundred() {
    let (counter, now) = counter_with(99_999, 1, 0);
    assert_eq!(OnTime::of(&counter, now).line, "99.99% this week");
}

#[test]
fn the_row_reads_no_changes_in_a_week_that_has_none_yet() {
    let (counter, now) = counter_with(10, 1, 0);
    let next_week = now + 8 * DAY;
    assert_eq!(OnTime::of(&counter, next_week), OnTime::none());
}

#[test]
fn a_stand_in_percent_fills_the_row_for_a_capture() {
    let row = OnTime::fake(99.96);
    assert_eq!(row.line, "99.96% this week");
    assert_eq!(row.late.as_deref(), Some("Late: hooks 3"));
    let perfect = OnTime::fake(100.0);
    assert_eq!((perfect.line.as_str(), perfect.late), ("100% this week", None));
    assert_eq!(OnTime::fake(250.0).line, "100% this week");
}

#[test]
fn the_tag_and_the_age_sit_where_the_header_row_says() {
    let ctx = crate::fonts::test_context();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
        ..egui::RawInput::default()
    };
    let mut drawn = (Rect::NOTHING, 0.0, 0.0, 0.0);
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| {
            let painter = ui.painter().clone();
            let tag = paint_out_of_date(ui, 100.0, 20.0, Why::WorkingButTurnEnded);
            let left = paint_age(&painter, 390.0, 20.0, "14m ago");
            drawn = (tag, left, age_width(&painter, "14m ago"), out_of_date_width(&painter));
        });
    }
    let (tag, left, age_w, tag_w) = drawn;
    assert_eq!(tag.left(), 100.0);
    assert!((tag.center().y - 20.0).abs() < 0.5, "{tag:?}");
    assert!((tag.width() - tag_w).abs() < 0.5);
    assert!((390.0 - left - age_w).abs() < 0.5, "{left} {age_w}");
}
