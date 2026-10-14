use std::collections::BTreeMap;
use std::time::Duration;

use egui::{Event, Key, Modifiers, RawInput, Rect, pos2, vec2};
use tidewatch_core::runner::RunContext;
use tidewatch_core::workflows::Workflows;

use super::*;

fn run() -> TabRun {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let ctx = RunContext {
        task: String::new(),
        base: "main".into(),
        branch: "agent/otter".into(),
        test_command: None,
    };
    let runner = Runner::new(&ship, &BTreeMap::new(), ctx).unwrap();
    TabRun::new(runner, None, Duration::from_secs(600))
}

fn frame(ctx: &egui::Context, run: &mut TabRun, events: Vec<Event>) -> Option<String> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let mut started = None;
    let _ = ctx.run_ui(raw, |ui| {
        let area = ui.max_rect();
        started = show(ui, area, run);
    });
    started
}

fn key(modifiers: Modifiers) -> Event {
    Event::Key {
        key: Key::Enter,
        physical_key: Some(Key::Enter),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn setup() -> (egui::Context, TabRun) {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    let mut run = run();
    frame(&ctx, &mut run, vec![]);
    frame(&ctx, &mut run, vec![]);
    (ctx, run)
}

#[test]
fn a_card_per_stage_names_the_cli_and_the_level() {
    let cards = cards(run().runner());
    let titles: Vec<_> = cards.iter().map(|card| card.title).collect();
    assert_eq!(titles, ["Work", "Review", "Refactor", "Hand off"]);
    assert_eq!(cards[0].who, "Claude \u{b7} Auto edit");
    assert_eq!(cards[3].tip, "Gate: You approve");
}

#[test]
fn the_card_falls_back_to_a_general_line_when_the_run_has_no_description() {
    assert!(blurb(&run()).starts_with("The stages run one after the other"));
    assert_eq!(blurb(&run().with_blurb("Fix a bug.".into())), "Fix a bug.");
}

#[test]
fn command_enter_in_the_task_box_starts_the_run_with_what_was_typed() {
    let (ctx, mut run) = setup();
    assert_eq!(frame(&ctx, &mut run, vec![Event::Text("add a retry".into())]), None);
    assert_eq!(run.ui.task, "add a retry");
    let started = frame(&ctx, &mut run, vec![key(Modifiers::COMMAND)]);
    assert_eq!(started.as_deref(), Some("add a retry"));
    assert!(run.ui.task.contains("add a retry"), "the box keeps the text until the run takes it");
}

#[test]
fn plain_enter_is_a_new_line_and_a_blank_task_never_starts() {
    let (ctx, mut run) = setup();
    assert_eq!(frame(&ctx, &mut run, vec![key(Modifiers::COMMAND)]), None);
    frame(&ctx, &mut run, vec![Event::Text("one".into())]);
    assert_eq!(frame(&ctx, &mut run, vec![key(Modifiers::NONE)]), None);
    frame(&ctx, &mut run, vec![Event::Text("two".into())]);
    assert_eq!(run.ui.task, "one\ntwo");
}
