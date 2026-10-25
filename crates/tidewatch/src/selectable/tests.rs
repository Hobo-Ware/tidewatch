use egui::{Event, Modifiers, OutputCommand, PointerButton, RawInput, pos2, vec2};

use super::*;
use crate::brand;
use crate::screen_kit::{Size, body, quiet_button};

const SENTENCE: &str = "Added the promo test";
const AT: Pos2 = pos2(20.0, 20.0);

fn run(ctx: &egui::Context, events: Vec<Event>, draw: &mut dyn FnMut(&mut Ui)) -> Vec<String> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let out = ctx.run_ui(raw, |ui| draw(ui));
    out.platform_output
        .commands
        .into_iter()
        .filter_map(|command| match command {
            OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn drag_across(ctx: &egui::Context, from: Pos2, to: Pos2, draw: &mut dyn FnMut(&mut Ui)) {
    for events in [
        vec![Event::PointerMoved(from)],
        vec![button(from, true)],
        vec![Event::PointerMoved(pos2(f32::midpoint(from.x, to.x), from.y))],
        vec![Event::PointerMoved(to)],
        vec![button(to, false)],
    ] {
        run(ctx, events, draw);
    }
}

fn paint_static(ui: &mut Ui) {
    let laid = ui.painter().layout_no_wrap(SENTENCE.to_owned(), body(14.0), brand::TEXT);
    galley(ui, AT, laid, brand::TEXT);
}

fn paint_button(ui: &mut Ui) {
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(AT, vec2(160.0, 30.0))),
        |ui| {
            quiet_button(ui, SENTENCE, true, Size::Regular);
        },
    );
}

fn copied_after_drag(draw: &mut dyn FnMut(&mut Ui)) -> Vec<String> {
    let ctx = egui::Context::default();
    run(&ctx, vec![], draw);
    drag_across(&ctx, pos2(AT.x + 1.0, AT.y + 8.0), pos2(AT.x + 400.0, AT.y + 8.0), draw);
    run(&ctx, vec![Event::Copy], draw)
}

#[test]
fn dragging_across_static_text_selects_it_and_copy_takes_it() {
    assert_eq!(copied_after_drag(&mut paint_static), [SENTENCE]);
}

#[test]
fn a_button_label_is_not_selectable() {
    assert_eq!(copied_after_drag(&mut paint_button).len(), 0);
}

#[test]
fn a_click_on_a_button_still_lands_after_a_drag_across_static_text() {
    let ctx = egui::Context::default();
    let mut clicked = false;
    let mut draw = |ui: &mut Ui| {
        paint_static(ui);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(20.0, 80.0), vec2(160.0, 30.0))),
            |ui| clicked |= quiet_button(ui, "Close", true, Size::Regular).clicked(),
        );
    };
    run(&ctx, vec![], &mut draw);
    drag_across(&ctx, pos2(21.0, 28.0), pos2(300.0, 28.0), &mut draw);
    let on_button = pos2(40.0, 95.0);
    for events in [
        vec![Event::PointerMoved(on_button)],
        vec![button(on_button, true)],
        vec![button(on_button, false)],
    ] {
        run(&ctx, events, &mut draw);
    }
    assert!(clicked);
}
