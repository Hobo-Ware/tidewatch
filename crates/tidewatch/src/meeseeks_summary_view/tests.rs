use std::path::PathBuf;

use egui::{Event, Modifiers, OutputCommand, PointerButton, Pos2, RawInput, pos2, vec2};
use jiff::tz::TimeZone;
use tidewatch_core::civil_date::Date;
use tidewatch_core::meeseeks::{CheckLine, Step, Verdict};
use tidewatch_core::meeseeks_branch::{Branch, Changed, Found};
use tidewatch_core::meeseeks_log::{Poofed, StageLine};

use super::*;
use crate::meeseeks_summary::summary;

const PR: &str = "https://github.com/acme/web/pull/213";

fn record() -> Poofed {
    let mut record = Poofed::bare(
        "promo-test",
        "Added the promo test and opened the pull request.",
        Some(PR),
        360,
        Date::new(2026, 10, 25).unwrap(),
    );
    record.task = "Add a test for the promo banner".into();
    record.repo = PathBuf::from("/code/web");
    record.branch = "agent/promo-test".into();
    record.ended_ms = 1_790_000_000_000;
    record.checks = [
        (Step::Change, "2 commits ahead of main"),
        (Step::Tests, "`cargo test` passed"),
        (Step::Proof, "Worktree clean"),
        (Step::HandOff, "Opened PR #213 against main"),
    ]
    .map(|(step, detail)| CheckLine { step, verdict: Verdict::Passed, detail: detail.into() })
    .to_vec();
    record.done = vec!["Added `promo.test.ts`".into()];
    record.next = vec!["Review the pull request".into()];
    record.stages = vec![StageLine { name: "Build".into(), done: 2, total: 2 }];
    record
}

fn facts() -> Branch {
    Branch::Found(Found {
        base: "main".into(),
        ahead: 2,
        changed: Changed { files: 3, added: 42, removed: 7 },
    })
}

struct Harness {
    ctx: egui::Context,
    summary: Summary,
    area: egui::Rect,
    clock: f64,
    action: Option<Action>,
    opened: Vec<String>,
}

impl Harness {
    fn new(summary: Summary, width: f32) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let area = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 900.0));
        let mut harness = Self { ctx, summary, area, clock: 0.0, action: None, opened: Vec::new() };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.clock += 0.1;
        let raw = RawInput {
            screen_rect: Some(self.area),
            events,
            focused: true,
            time: Some(self.clock),
            ..RawInput::default()
        };
        let (summary, area) = (&self.summary, self.area);
        let mut action = None;
        let out = self.ctx.run_ui(raw, |ui| action = show(ui, area, summary));
        self.action = action;
        for command in out.platform_output.commands {
            if let OutputCommand::OpenUrl(open) = command {
                self.opened.push(open.url);
            }
        }
    }

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp(placed_id())).expect("the summary drew")
    }

    fn click(&mut self, at: Pos2) -> Vec<Option<Action>> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut seen = Vec::new();
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
            self.frame(events);
            seen.push(self.action);
        }
        seen
    }
}

fn harness(width: f32) -> Harness {
    Harness::new(summary(&record(), Some(&facts()), &TimeZone::UTC), width)
}

#[test]
fn the_header_holds_the_face_the_task_and_one_close_button_at_the_right() {
    let harness = harness(900.0);
    let placed = harness.placed();
    let (header, close) = (placed.header.unwrap(), placed.close.unwrap());
    assert!(header.contains_rect(close));
    assert_eq!(close.right(), header.right());
    assert!(placed.prompt.unwrap().top() > header.bottom());
}

#[test]
fn close_is_a_click_that_says_so_only_on_release() {
    let mut harness = harness(900.0);
    let at = harness.placed().close.unwrap().center();
    assert_eq!(harness.click(at), [None, None, Some(Action::Close)]);
}

#[test]
fn the_four_checks_sit_in_one_card_under_the_answer() {
    let harness = harness(900.0);
    let placed = harness.placed();
    let card = placed.card.unwrap();
    assert_eq!(placed.checks.len(), 4);
    assert!(card.top() > placed.said.unwrap().bottom());
    assert!(card.contains_rect(placed.checks[0]) && card.contains_rect(placed.checks[3]));
    for pair in placed.checks.windows(2) {
        assert_eq!(pair[0].bottom(), pair[1].top());
    }
    for check in &placed.checks {
        assert_eq!((check.left(), check.right()), (card.left(), card.right()));
    }
    assert_eq!(placed.quiet, None);
}

#[test]
fn the_pull_request_link_opens_in_the_browser_and_is_not_the_close_button() {
    let mut harness = harness(900.0);
    let pr = harness.placed().pr.unwrap();
    let at = pos2(pr.left() + 180.0, pr.center().y);
    assert_eq!(harness.click(at), [None, None, None]);
    assert_eq!(harness.opened, [PR]);
}

#[test]
fn the_branch_sits_under_the_pull_request_and_a_gone_branch_still_draws() {
    let harness = harness(900.0);
    let placed = harness.placed();
    assert!(placed.branch.unwrap().top() > placed.pr.unwrap().bottom());
    let gone = Harness::new(summary(&record(), Some(&Branch::Missing), &TimeZone::UTC), 900.0);
    assert!(gone.placed().branch.is_some());
}

#[test]
fn an_old_poof_with_nothing_but_five_facts_draws_the_checks_and_no_extra_sections() {
    let old = Poofed::bare("old", "Did it.", None, 60, Date::new(2026, 10, 20).unwrap());
    let harness = Harness::new(summary(&old, None, &TimeZone::UTC), 900.0);
    let placed = harness.placed();
    assert_eq!(placed.checks.len(), 4);
    assert_eq!((placed.pr, placed.branch, placed.prompt), (None, None, None));
}

#[test]
fn nothing_leaves_the_window_from_a_narrow_one_up() {
    for width in [320.0, 420.0, 600.0, 900.0, 1400.0] {
        let harness = harness(width);
        let placed = harness.placed();
        let inside = |rect: egui::Rect| rect.left() >= 0.0 && rect.right() <= width;
        assert!(inside(placed.header.unwrap()), "{width}");
        assert!(placed.checks.iter().all(|rect| inside(*rect)), "{width}");
        assert!(inside(placed.close.unwrap()), "{width}");
    }
}

fn read_only_record() -> Poofed {
    let mut record = record();
    record.kind = tidewatch_core::kind::Kind::ReadOnly;
    record.pr_url = None;
    record.summary = "- the cache is an LRU\n- it lives on disk".into();
    record.checks = Step::ALL
        .iter()
        .map(|&step| CheckLine {
            step,
            verdict: Verdict::Skipped,
            detail: "Nothing changed".into(),
        })
        .collect();
    record
}

#[test]
fn a_read_only_poof_draws_the_answer_and_one_quiet_line_and_no_checks_card_or_branch() {
    let none = Branch::Found(Found {
        base: "main".into(),
        ahead: 0,
        changed: Changed { files: 0, added: 0, removed: 0 },
    });
    let harness = Harness::new(summary(&read_only_record(), Some(&none), &TimeZone::UTC), 900.0);
    let placed = harness.placed();
    assert_eq!((placed.card, placed.branch, placed.pr), (None, None, None));
    assert_eq!(placed.checks.len(), 0);
    assert!(placed.quiet.unwrap().top() > placed.said.unwrap().bottom());
}

#[test]
fn a_long_prompt_wraps_whole_in_its_card_instead_of_being_cut_to_one_line() {
    let mut long = record();
    long.task = "Why is the compile step so slow? https://github.com/acme/web/pull/1 ".repeat(12);
    let harness = Harness::new(summary(&long, Some(&facts()), &TimeZone::UTC), 900.0);
    let one_line = Harness::new(summary(&record(), Some(&facts()), &TimeZone::UTC), 900.0);
    let (tall, short) = (harness.placed().prompt.unwrap(), one_line.placed().prompt.unwrap());
    assert!(tall.height() > short.height() + 60.0, "{} vs {}", tall.height(), short.height());
    assert!(harness.placed().said.unwrap().top() > tall.bottom());
}

#[test]
fn content_keeps_twelve_pixels_clear_of_the_scrollbar_edge() {
    for width in [320.0, 600.0, 900.0, 1400.0] {
        let harness = harness(width);
        let placed = harness.placed();
        let edge = placed.column.unwrap().right() - 12.0;
        let rects = [placed.header, placed.close, placed.prompt, placed.said, placed.card];
        for rect in rects.into_iter().flatten().chain(placed.checks.iter().copied()) {
            assert!(rect.right() <= edge, "{width}: {rect:?} past {edge}");
        }
    }
}
