use egui::pos2;

use super::*;

const BOX: f32 = 28.0;

fn shapes_of(look: Look, hover: f64) -> Vec<egui::epaint::ClippedShape> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0))),
        ..Default::default()
    };
    ctx.run_ui(raw, |ui| {
        let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(BOX, BOX));
        paint(ui.painter(), rect, look, hover);
    })
    .shapes
}

fn bounds(shapes: &[egui::epaint::ClippedShape]) -> Rect {
    shapes.iter().fold(Rect::NOTHING, |all, s| all.union(s.shape.visual_bounding_rect()))
}

fn looks() -> Vec<(&'static str, Look, f64)> {
    vec![
        ("off", Look::Off, 0.0),
        ("hover", Look::Off, 1.0),
        ("pending", Look::Pending, 0.0),
        ("activating", Look::On { age: 0.4 }, 0.0),
        ("puffing", Look::On { age: 0.6 }, 1.0),
        ("steady", Look::On { age: 31.7 }, 0.0),
        ("stopping", Look::Stopping { age: 0.2 }, 0.0),
    ]
}

#[test]
fn every_look_draws_something_that_stays_inside_the_button() {
    let button = Rect::from_min_size(pos2(40.0, 40.0), vec2(BOX, BOX));
    for (name, look, hover) in looks() {
        let shapes = shapes_of(look, hover);
        assert!(!shapes.is_empty(), "{name}");
        let drawn = bounds(&shapes);
        assert!(button.expand(1.0).contains_rect(drawn), "{name}: {drawn:?} outside {button:?}");
    }
}

#[test]
fn off_is_a_few_shapes_and_on_adds_the_crumbled_rim_and_the_specks() {
    let off = shapes_of(Look::Off, 0.0).len();
    let steady = shapes_of(Look::On { age: 31.7 }, 0.0).len();
    assert!(off <= 8, "{off}");
    assert!(steady > off + 20, "{steady} against {off}");
}

#[test]
fn the_eyes_peek_only_on_hover_while_on_and_while_the_prompt_is_open() {
    let count = |look, hover| shapes_of(look, hover).len();
    assert!(count(Look::Off, 1.0) > count(Look::Off, 0.0));
    assert!(count(Look::Pending, 0.0) > count(Look::Off, 0.0));
}

#[test]
fn the_glyph_is_eighteen_pixels_on_a_twenty_four_unit_grid() {
    assert_eq!(GLYPH_PX / GRID, 0.75);
}
