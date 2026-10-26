use super::*;
use egui::{Context, Event, PointerButton, RawInput, Shape};
use tidewatch_core::pr_cards::Cards;
use tidewatch_core::pr_status::{Body, Job, ReviewComment, Seen, Summary, SummaryChecks, Via};
use tidewatch_core::prefs::{GithubPrefs, PrMode};

use crate::pr_watch::{Handling, Pause, Watch};

fn policy(mode: PrMode) -> GithubPrefs {
    GithubPrefs { mode, ..GithubPrefs::default() }
}

fn job(name: &str, passed: bool) -> Job {
    Job {
        name: name.into(),
        passed,
        step: (!passed).then(|| "Run cargo clippy".to_owned()),
        secs: Some(108),
        url: "https://github.com/acme/web/actions/runs/7/job/8".into(),
        run_id: Some(7),
        job_id: Some(8),
        log: if passed {
            vec![]
        } else {
            vec!["error: unused import".into(), "  --> src/a.rs:1:1".into()]
        },
    }
}

fn every_kind() -> Vec<Body> {
    vec![
        Body::Opened {
            number: 42,
            title: "t".into(),
            base: "main".into(),
            branch: "agent/x".into(),
            commits: Some(3),
            draft: true,
        },
        Body::ChecksRunning { head: "h".into(), done: 1, total: 3 },
        Body::ChecksFailed {
            head: "h".into(),
            jobs: vec![job("check (macos-14)", true), job("check (ubuntu-latest)", false)],
        },
        Body::ChecksPassed { total: 2, secs: Some(182) },
        Body::ChangesRequested {
            trusted: true,
            author: "reviewer".into(),
            body: "Two things.".into(),
            comments: vec![ReviewComment {
                path: "runner.rs".into(),
                line: Some(212),
                text: "Move it?".into(),
            }],
        },
        Body::Commented {
            trusted: true,
            author: "reviewer".into(),
            text: "Does it work?".into(),
            url: String::new(),
        },
        Body::Conflict { base: "main".into() },
        Body::Approved { author: "reviewer".into() },
        Body::PushFix { commits: 2, head: "abc".into() },
        Body::Sent {
            via: Via::Auto,
            tag: "CI failure".into(),
            headline: "check failed at clippy on PR #42.".into(),
            extra_lines: 12,
            text: "t".into(),
        },
        Body::Merged { number: 42, base: "main".into() },
        Body::Closed { number: 42 },
    ]
}

fn watch_with(bodies: Vec<Body>) -> Watch {
    let mut cards = Cards::default();
    cards.follow(42);
    cards.seen = Some(Seen { opened: true, conflicting: true, ..Seen::default() });
    cards.summary = Some(Summary {
        number: 42,
        title: "t".into(),
        url: "https://github.com/acme/web/pull/42".into(),
        state: tidewatch_core::pr_status::State::Open,
        checks: SummaryChecks::Failed,
        comments: 0,
        decision: tidewatch_core::pr_status::Decision::None,
        conflicting: true,
    });
    for (n, body) in bodies.into_iter().enumerate() {
        cards.add(n as i64 * 1000, body);
    }
    let mut watch = Watch::new(std::path::PathBuf::from("/x"));
    watch.take_load();
    watch.loaded(cards, 0.0);
    watch
}

fn with_fonts() -> Context {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    ctx
}

fn run_frames(
    watch: &Watch,
    events: Vec<Vec<Event>>,
    only: Option<usize>,
) -> (Vec<(u64, f32)>, Vec<CardAction>) {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let data = Data {
        watch,
        profile: "Builder",
        handling: Handling::Owner,
        policy: policy(PrMode::Ask),
        now_ms: 10_000_000,
    };
    let mut measured = Vec::new();
    let mut clicks = Vec::new();
    for batch in std::iter::once(Vec::new()).chain(events) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 3000.0))),
            events: batch,
            focused: true,
            ..RawInput::default()
        };
        measured.clear();
        let _ = ctx.run_ui(raw, |ui| {
            ui.set_width(820.0);
            for index in 0..data.cards().len() {
                if only.is_some_and(|only| only != index) {
                    continue;
                }
                let drawn = ui.vertical(|ui| card(ui, &data, index));
                measured.push((data.cards()[index].id, drawn.response.rect.height()));
                clicks.extend(drawn.inner);
            }
            clicks.extend(tail(ui, &data));
        });
    }
    (measured, clicks)
}

#[test]
fn every_kind_of_card_draws_headless_with_a_height() {
    let watch = watch_with(every_kind());
    let (measured, clicks) = run_frames(&watch, vec![vec![]], None);
    assert_eq!(measured.len(), watch.cards.cards().len());
    assert_eq!(measured.len(), 10, "the live run and the push offer end with the merge");
    for (id, height) in &measured {
        assert!(*height > 20.0, "card {id} is {height} tall");
    }
    assert_eq!(clicks, []);
    let height_of = |want: fn(&Body) -> bool| {
        let id = watch.cards.cards().iter().find(|card| want(&card.body)).unwrap().id;
        measured.iter().find(|(known, _)| *known == id).unwrap().1
    };
    let failed = height_of(|body| matches!(body, Body::ChecksFailed { .. }));
    let passed = height_of(|body| matches!(body, Body::ChecksPassed { .. }));
    assert!(failed > passed, "a failure with its jobs and log is taller than a pass");
}

#[test]
fn a_click_on_the_primary_button_asks_for_that_card() {
    let watch = watch_with(vec![every_kind().remove(2)]);
    let id = watch.cards.cards()[0].id;
    let data = Data {
        watch: &watch,
        profile: "Builder",
        handling: Handling::Owner,
        policy: policy(PrMode::Ask),
        now_ms: 0,
    };
    let find = |ctx: &Context, events: Vec<Event>| {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut result = None;
        let _ = ctx.run_ui(raw, |ui| {
            ui.set_width(820.0);
            result = card(ui, &data, 0);
        });
        result
    };
    assert_eq!(find(&with_fonts(), vec![]), None);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut hit = None;
    for y in (170..330).step_by(8) {
        let pos = pos2(60.0, y as f32);
        let ctx = with_fonts();
        find(&ctx, vec![]);
        find(&ctx, vec![Event::PointerMoved(pos)]);
        find(&ctx, vec![button(pos, true)]);
        if let Some(action) = find(&ctx, vec![button(pos, false)]) {
            hit = Some(action);
            break;
        }
    }
    assert_eq!(hit, Some(CardAction::Ask(id)));
}

#[test]
fn a_waiting_message_and_a_pause_draw_at_the_end_and_their_buttons_click() {
    let mut watch = watch_with(vec![]);
    let message = tidewatch_core::pr_prompt::message(&every_kind().remove(2), 42).unwrap();
    watch.pending.push(crate::pr_watch::Pending { id: 3, message });
    watch.pause = Some(Pause::SignedOut);
    let (_, clicks) = run_frames(&watch, vec![vec![]], None);
    assert_eq!(clicks, []);
    let data = Data {
        watch: &watch,
        profile: "Builder",
        handling: Handling::Owner,
        policy: policy(PrMode::Auto),
        now_ms: 0,
    };
    assert!(data.has_tail());
}

#[test]
fn heights_are_kept_per_card_and_forgotten_when_the_width_changes() {
    let mut heights = Heights::default();
    assert_eq!(heights.height(1), GUESS_H);
    heights.for_width(800.0);
    heights.record(1, 64.0);
    assert_eq!(heights.height(1), 64.0);
    heights.for_width(800.4);
    assert_eq!(heights.height(1), 64.0);
    heights.for_width(700.0);
    assert_eq!(heights.height(1), GUESS_H);
}

#[test]
fn a_timeline_with_nothing_to_draw_has_no_tail() {
    let watch = watch_with(vec![]);
    let data = Data {
        watch: &watch,
        profile: "Builder",
        handling: Handling::Owner,
        policy: policy(PrMode::Ask),
        now_ms: 0,
    };
    assert!(!data.has_tail());
    assert_eq!(data.cards().len(), 0);
}

#[test]
fn a_paused_tab_on_its_own_has_a_tail_of_one_quiet_line_and_no_button() {
    let watch = watch_with(vec![]);
    let paused = Data {
        watch: &watch,
        profile: "Builder",
        handling: Handling::Paused,
        policy: policy(PrMode::Auto),
        now_ms: 0,
    };
    assert!(paused.has_tail());
    let ctx = with_fonts();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
        focused: true,
        ..RawInput::default()
    };
    let mut clicked = Some(CardAction::OpenNotes);
    let _ = ctx.run_ui(raw, |ui| clicked = tail(ui, &paused));
    assert_eq!(clicked, None);
    let sending = Data { handling: Handling::Agent, ..paused };
    assert!(!sending.has_tail());
}

fn painted_text(watch: &Watch) -> Vec<String> {
    let ctx = with_fonts();
    let data = Data {
        watch,
        profile: "Builder",
        handling: Handling::Owner,
        policy: policy(PrMode::Ask),
        now_ms: 10_000_000,
    };
    let mut texts = Vec::new();
    for batch in [vec![], vec![]] {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 3000.0))),
            events: batch,
            focused: true,
            ..RawInput::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            ui.set_width(820.0);
            for index in 0..data.cards().len() {
                card(ui, &data, index);
            }
        });
        texts.clear();
        for clipped in &output.shapes {
            if let Shape::Text(text) = &clipped.shape {
                texts.push(text.galley.text().to_owned());
            }
        }
    }
    texts
}

const MARKUP: &str = "<!-- codex-note -->\n**P1 Badge** The retry loop never resets its counter.\n\n| Check | Result |\n| --- | --- |\n| retry | <relative-time datetime=\"2026-10-26T16:09:23Z\">just now</relative-time> |\n\n<details><summary>About this review</summary>\nIt was run by a bot.\n</details>";

#[test]
fn what_someone_wrote_on_github_is_drawn_as_markdown_never_as_raw_text() {
    let comment = Body::Commented {
        trusted: true,
        author: "reviewer".into(),
        text: MARKUP.into(),
        url: String::new(),
    };
    let review = Body::ChangesRequested {
        trusted: true,
        author: "reviewer".into(),
        body: MARKUP.into(),
        comments: vec![ReviewComment {
            path: "src/retry.rs".into(),
            line: Some(12),
            text: MARKUP.into(),
        }],
    };
    for (name, body) in [("comment", comment), ("review", review)] {
        let texts = painted_text(&watch_with(vec![body]));
        let all = texts.join("\n");
        assert!(all.contains("The retry loop never resets its counter."), "{name}: {all}");
        assert!(all.contains("About this review"), "{name}: {all}");
        for raw in ["<!--", "<details", "<summary", "</", "<relative-time", "**P1"] {
            assert!(!all.contains(raw), "{name} shows raw {raw:?}: {all}");
        }
    }
}

#[test]
fn two_cards_with_a_table_each_do_not_share_widget_ids() {
    let table = "| Stage | Counter |\n| --- | --- |\n| first | 0 |";
    let comment = |text: &str| Body::Commented {
        trusted: true,
        author: "reviewer".into(),
        text: text.into(),
        url: String::new(),
    };
    let watch = watch_with(vec![comment(table), comment(table)]);
    let all = painted_text(&watch).join("\n");
    assert!(!all.contains("Second use"), "{all}");
    assert!(all.contains("Counter"), "{all}");
}
