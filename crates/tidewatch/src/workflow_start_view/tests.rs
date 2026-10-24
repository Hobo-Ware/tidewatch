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
    let cards = cards(run().runner(), None);
    let titles: Vec<_> = cards.iter().map(|card| card.title).collect();
    assert_eq!(titles, ["Work", "Review", "Refactor", "Hand off"]);
    assert_eq!(cards[0].who, "Claude \u{b7} Auto edit");
    assert_eq!(cards[3].tip, "Gate: You approve");
}

fn run_testing(command: Option<&str>) -> TabRun {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let ctx = RunContext {
        task: String::new(),
        base: "main".into(),
        branch: "agent/otter".into(),
        test_command: command.map(str::to_owned),
    };
    let runner = Runner::new(&ship, &BTreeMap::new(), ctx).unwrap();
    TabRun::new(runner, None, Duration::from_secs(600))
}

#[test]
fn a_test_gate_card_shows_the_command_and_what_picked_it() {
    let run = run_testing(Some("bun test")).with_test_why(Some("bun.lock".into()));
    let cards = cards(run.runner(), run.test_why());
    assert_eq!(
        cards[0].tip,
        "Gate: Tests pass\nRuns bun test (bun.lock). Wrong command? Set test = \"...\" in .tidewatch.toml at the repo root."
    );
    assert_eq!(cards[3].tip, "Gate: You approve");
}

#[test]
fn a_test_gate_card_without_a_command_says_how_to_set_one() {
    let run = run_testing(None);
    let cards = cards(run.runner(), run.test_why());
    assert_eq!(cards[0].tip, format!("Gate: Tests pass\n{}", test_command::MISSING_TIP));
}

#[test]
fn a_configured_command_names_the_repo_file_without_asking_to_set_it() {
    let run = run_testing(Some("make check")).with_test_why(Some(".tidewatch.toml".into()));
    assert_eq!(
        cards(run.runner(), run.test_why())[0].tip,
        "Gate: Tests pass\nRuns make check (.tidewatch.toml)."
    );
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

fn painted<'a>(
    shapes: &'a [egui::epaint::ClippedShape],
    needle: &str,
) -> Option<(&'a egui::epaint::TextShape, Rect)> {
    shapes.iter().find_map(|clipped| match &clipped.shape {
        egui::Shape::Text(text) if text.galley.text().contains(needle) => {
            Some((text, clipped.clip_rect))
        }
        _ => None,
    })
}

#[test]
fn sixty_typed_lines_scroll_inside_the_task_box_and_keep_start_run_on_screen() {
    let (ctx, mut run) = setup();
    let mut step = 0;
    let mut column = Rect::NOTHING;
    let mut draw = |events: Vec<Event>, run: &mut TabRun| {
        step += 1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0))),
            events,
            focused: true,
            time: Some(10.0 + f64::from(step) * 0.05),
            ..RawInput::default()
        };
        ctx.run_ui(raw, |ui| {
            egui::Panel::bottom("statusbar")
                .exact_size(crate::status_bar::HEIGHT)
                .frame(Frame::NONE)
                .show(ui, |_| {});
            egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
                column = ui.max_rect();
                show(ui, column, run);
            });
        })
    };
    for _ in 0..2 {
        draw(vec![], &mut run);
    }
    for line in 1..=60 {
        draw(vec![Event::Text(format!("line {line}")), key(Modifiers::NONE)], &mut run);
    }
    draw(vec![Event::Text("line 61".into())], &mut run);
    let out = (0..20).map(|_| draw(vec![], &mut run)).last().unwrap();
    let (hint, _) = painted(&out.shapes, "starts").expect("the footer is painted");
    let hint = Rect::from_min_size(hint.pos, hint.galley.size());
    assert!(hint.bottom() <= column.bottom(), "footer {hint:?} below the window {column:?}");
    let (text, clip) = painted(&out.shapes, "line 61").expect("the last typed line is painted");
    let row = text.galley.rows.last().unwrap().rect().translate(text.pos.to_vec2());
    assert!(clip.contains_rect(row.shrink(0.5)), "cursor row {row:?} outside visible {clip:?}");
    assert!(clip.bottom() < hint.top(), "the box {clip:?} runs into the footer {hint:?}");
}
