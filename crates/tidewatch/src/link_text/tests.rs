use super::*;
use egui::{Event, PointerButton, RawInput, vec2};

use crate::screen_kit::body;

const TEXT: &str = "Preview at http://localhost:5174/app now";

fn frame(
    ctx: &egui::Context,
    events: Vec<Event>,
) -> (Vec<egui::OutputCommand>, Vec<Rect>, CursorIcon) {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let mut rects = Vec::new();
    let out = ctx.run_ui(raw, |ui| {
        let laid = Text::line(TEXT, body(14.0), brand::TEXT, 500.0).lay_out(ui.painter());
        rects = laid.link_rects(pos2(20.0, 20.0)).into_iter().map(|(_, rect)| rect).collect();
        laid.paint(ui, Id::new("line"), pos2(20.0, 20.0), brand::TEXT);
    });
    (out.platform_output.commands, rects, out.platform_output.cursor_icon)
}

fn selectable_frame(ctx: &egui::Context, events: Vec<Event>) -> (Vec<Rect>, CursorIcon) {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
        events,
        focused: true,
        ..RawInput::default()
    };
    let mut rects = Vec::new();
    let out = ctx.run_ui(raw, |ui| {
        let laid = Text::line(TEXT, body(14.0), brand::TEXT, 500.0).lay_out(ui.painter());
        rects = laid.link_rects(pos2(20.0, 20.0)).into_iter().map(|(_, rect)| rect).collect();
        laid.paint_selectable(ui, Id::new("line"), pos2(20.0, 20.0), brand::TEXT);
    });
    (rects, out.platform_output.cursor_icon)
}

fn click(ctx: &egui::Context, at: Pos2) -> Vec<egui::OutputCommand> {
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut opened = Vec::new();
    for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        opened.extend(frame(ctx, events).0);
    }
    opened
}

fn opened_urls(commands: &[egui::OutputCommand]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_click_on_a_link_opens_it_and_a_click_on_plain_text_does_not() {
    let ctx = egui::Context::default();
    let (_, rects, _) = frame(&ctx, vec![]);
    assert_eq!(rects.len(), 1);
    let on_link = rects[0].center();
    let on_text = pos2(rects[0].left() - 30.0, rects[0].center().y);
    assert_eq!(opened_urls(&click(&ctx, on_link)), ["http://localhost:5174/app"]);
    let ctx = egui::Context::default();
    let _ = frame(&ctx, vec![]);
    assert_eq!(opened_urls(&click(&ctx, on_text)).len(), 0);
}

#[test]
fn the_pointer_over_a_link_is_a_pointing_hand() {
    let ctx = egui::Context::default();
    let (_, rects, _) = frame(&ctx, vec![]);
    let _ = frame(&ctx, vec![Event::PointerMoved(rects[0].center())]);
    let (_, _, cursor) = frame(&ctx, vec![Event::PointerMoved(rects[0].center())]);
    assert_eq!(cursor, CursorIcon::PointingHand);
    let off = pos2(rects[0].left() - 30.0, rects[0].center().y);
    let (_, _, cursor) = frame(&ctx, vec![Event::PointerMoved(off)]);
    assert_eq!(cursor, CursorIcon::Default);
}

#[test]
fn a_link_that_wraps_has_a_rect_per_row() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        let mut text = Text::new();
        text.run(
            "go https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa end",
            &TextFormat::simple(body(14.0), brand::TEXT),
        );
        let laid = text.wrapped(150.0, 3).lay_out(ui.painter());
        let rects = laid.link_rects(pos2(0.0, 0.0));
        assert!(rects.len() >= 2, "{rects:?}");
        assert!(rects.iter().all(|(index, _)| *index == 0));
    });
}

#[test]
fn the_pointer_over_a_selectable_link_is_a_pointing_hand_and_over_its_text_a_text_cursor() {
    let ctx = egui::Context::default();
    let (rects, _) = selectable_frame(&ctx, vec![]);
    let on_link = rects[0].center();
    let _ = selectable_frame(&ctx, vec![Event::PointerMoved(on_link)]);
    let (_, cursor) = selectable_frame(&ctx, vec![Event::PointerMoved(on_link)]);
    assert_eq!(cursor, CursorIcon::PointingHand);
    let on_text = pos2(rects[0].left() - 30.0, rects[0].center().y);
    let _ = selectable_frame(&ctx, vec![Event::PointerMoved(on_text)]);
    let (_, cursor) = selectable_frame(&ctx, vec![Event::PointerMoved(on_text)]);
    assert_eq!(cursor, CursorIcon::Text);
}

#[test]
fn only_link_sections_are_underlined_at_rest() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        let laid = Text::line(TEXT, body(14.0), brand::TEXT, 500.0).lay_out(ui.painter());
        let marks: Vec<(String, Stroke)> = laid
            .job
            .sections
            .iter()
            .map(|s| {
                let bytes = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
                (laid.job.text[bytes].to_owned(), s.format.underline)
            })
            .collect();
        let rest = Stroke::new(1.0, brand::link_rest());
        assert_eq!(
            marks,
            [
                ("Preview at ".to_owned(), Stroke::NONE),
                ("http://localhost:5174/app".to_owned(), rest),
                (" now".to_owned(), Stroke::NONE),
            ]
        );
    });
}

#[test]
fn a_hovered_link_is_underlined_in_full_teal() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        let laid = Text::line(TEXT, body(14.0), brand::TEXT, 500.0).lay_out(ui.painter());
        let strokes: Vec<Stroke> =
            laid.underlined(0).sections.iter().map(|s| s.format.underline).collect();
        assert_eq!(strokes, [Stroke::NONE, Stroke::new(1.0, brand::TEAL_TEXT), Stroke::NONE]);
    });
}
