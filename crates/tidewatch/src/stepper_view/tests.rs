use std::collections::BTreeMap;
use std::time::Duration;

use egui::{Event, PointerButton, RawInput, Rect, pos2};
use tidewatch_core::events::{TabEvent, TabMessage};
use tidewatch_core::runner::{RunContext, RunEvent, Runner};
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::workflow_ask::ask_of;
use tidewatch_core::runner::AskReason;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    ctx
}

fn run() -> TabRun {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let ctx = RunContext {
        task: "add a retry".into(),
        base: "main".into(),
        branch: "agent/brave-otter".into(),
        test_command: Some("npm test".into()),
    };
    TabRun::new(Runner::new(&ship, &BTreeMap::new(), ctx).unwrap(), None, Duration::from_secs(600))
}

fn msg(generation: u64, event: TabEvent) -> TabMessage {
    TabMessage { tab_id: 1, generation, event }
}

const SCREEN: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0));

fn terminal() -> Rect {
    Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 340.0))
}

fn frame(ctx: &egui::Context, run: &mut TabRun, events: Vec<Event>) -> (Vec<Exec>, Rect) {
    let (execs, area, _) = frame_with_pill(ctx, run, events);
    (execs, area)
}

/// Also says where the first stage's pill was drawn the frame before.
fn frame_with_pill(
    ctx: &egui::Context,
    run: &mut TabRun,
    events: Vec<Event>,
) -> (Vec<Exec>, Rect, Option<Rect>) {
    let raw = RawInput { screen_rect: Some(SCREEN), events, focused: true, ..RawInput::default() };
    let mut out = (Vec::new(), terminal(), None);
    let _ = ctx.run_ui(raw, |ui| {
        let pill = ui.id().with(("stage-pill", 0));
        out.2 = ui.ctx().read_response(pill).map(|response| response.rect);
        let mut area = terminal();
        out.0 = show(ui, &mut area, run);
        out.1 = area;
    });
    out
}

#[test]
fn state_colors_tell_the_five_states_apart() {
    let colors: Vec<_> = [
        StageState::Todo,
        StageState::Running,
        StageState::NeedsYou,
        StageState::Done,
        StageState::Failed,
    ]
    .into_iter()
    .map(state_color)
    .collect();
    for (i, a) in colors.iter().enumerate() {
        for b in &colors[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn text_lines_guess_wrapped_lines_within_the_cap() {
    assert_eq!(text_lines("short", 600.0, 4), 1);
    assert_eq!(text_lines("a\nb\nc", 600.0, 4), 3);
    assert_eq!(text_lines(&"x".repeat(1000), 340.0, 4), 4, "capped");
    assert_eq!(text_lines("", 600.0, 4), 1);
    assert_eq!(text_lines(&"x".repeat(100), 100.0, 9), 5, "twenty columns at least");
}

#[test]
fn the_ask_card_grows_with_its_detail_and_the_field_it_shows() {
    let plain = ask_of(&AskReason::Approve, "Hand off 4/4", None);
    let with_detail = ask_of(&AskReason::Approve, "Hand off 4/4", Some("feat: retry\n\nbody"));
    let mut ui = RunUi::default();
    let base = ask_height(&plain, &ui, 900.0);
    assert!(ask_height(&with_detail, &ui, 900.0) > base);
    ui.rejecting = true;
    assert_eq!(ask_height(&plain, &ui, 900.0), base + FIELD_H);
    let needs_command = ask_of(&AskReason::NoTestCommand, "Work 1/4", None);
    let typed = RunUi::default();
    assert!(ask_height(&needs_command, &typed, 900.0) > base + FIELD_H - 1.0);
}

#[test]
fn a_clicked_stage_opens_taller_with_findings() {
    let mut run = run();
    run.start();
    run.message(&msg(1, TabEvent::StageDone { summary: "built".into() }));
    run.event(RunEvent::TestResult { exit_code: 0, output: String::new() });
    run.message(&msg(
        2,
        TabEvent::Finding {
            file: "src/a.rs".into(),
            line: 3,
            blocking: true,
            text: "unwrap on user input".into(),
        },
    ));
    let work = stepper::detail(run.runner(), 0).unwrap();
    let review = stepper::detail(run.runner(), 1).unwrap();
    assert!(detail_height(&review, 900.0) > detail_height(&work, 900.0) - 1.0);
    let empty = stepper::detail(run.runner(), 3).unwrap();
    assert!(detail_height(&empty, 900.0) >= 2.0 * CARD_PAD + TITLE_H + LINE_H);
}

#[test]
fn the_header_takes_its_band_off_the_top_of_the_terminal() {
    let ctx = context();
    let mut run = run();
    run.start();
    let (execs, area) = frame(&ctx, &mut run, Vec::new());
    assert_eq!(execs, Vec::new());
    assert_eq!(
        area.top(),
        50.0 - crate::layout::TERMINAL_PAD + STEPPER_H + crate::layout::TERMINAL_PAD
    );
    assert_eq!((area.left(), area.right(), area.bottom()), (10.0, 990.0, 340.0));
}

#[test]
fn clicking_a_pill_opens_the_stage_and_clicking_it_again_closes_it() {
    let ctx = context();
    let mut run = run();
    run.start();
    frame(&ctx, &mut run, Vec::new());
    let (_, _, pill) = frame_with_pill(&ctx, &mut run, Vec::new());
    let target = pill.expect("the first pill was drawn").center();
    let click = |pressed| Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&ctx, &mut run, vec![Event::PointerMoved(target)]);
    frame(&ctx, &mut run, vec![click(true)]);
    frame(&ctx, &mut run, vec![click(false)]);
    assert_eq!(run.ui.open_stage, Some(0));
    let (_, area) = frame(&ctx, &mut run, Vec::new());
    assert!(
        area.top() > 50.0 - crate::layout::TERMINAL_PAD + STEPPER_H + crate::layout::TERMINAL_PAD
    );
    frame(&ctx, &mut run, vec![click(true)]);
    frame(&ctx, &mut run, vec![click(false)]);
    assert_eq!(run.ui.open_stage, None);
}

#[test]
fn every_state_of_a_run_renders_without_asking_for_anything() {
    let ctx = context();
    let mut states: Vec<TabRun> = Vec::new();
    let mut working = run();
    working.start();
    states.push(working);
    let mut silent = run();
    silent.start();
    silent.message(&msg(1, TabEvent::StageDone { summary: "x".into() }));
    states.push(silent);
    let mut no_tests = TabRun::new(
        Runner::new(
            &Workflows::defaults().workflow.remove("ship").unwrap(),
            &BTreeMap::new(),
            RunContext {
                task: "t".into(),
                base: "main".into(),
                branch: "b".into(),
                test_command: None,
            },
        )
        .unwrap(),
        None,
        Duration::from_secs(60),
    );
    no_tests.start();
    no_tests.message(&msg(1, TabEvent::StageDone { summary: "x".into() }));
    states.push(no_tests);
    let mut rejecting = run();
    rejecting.start();
    rejecting.event(RunEvent::AgentStopped { last_message: Some("I stopped".into()) });
    rejecting.ui.open_stage = Some(0);
    states.push(rejecting);
    let mut stopped = run();
    stopped.start();
    stopped.event(RunEvent::UserStop);
    states.push(stopped);
    for mut run in states {
        for _ in 0..3 {
            let (execs, area) = frame(&ctx, &mut run, Vec::new());
            assert_eq!(execs, Vec::new());
            assert!(area.height() > 0.0);
        }
    }
}

#[test]
fn an_agents_markdown_reads_as_plain_lines() {
    assert_eq!(plain_line("**Done**"), "Done");
    assert_eq!(
        plain_line("- added `expiry()` in `src/auth.rs:12`"),
        "\u{b7} added expiry() in src/auth.rs:12"
    );
    assert_eq!(plain_line("## Needs you"), "Needs you");
    assert_eq!(plain_line("plain text"), "plain text");
}
