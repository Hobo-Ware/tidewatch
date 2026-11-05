//! The Away pill beside the Amphetamine button: outline when off, teal when on.

use egui::{Align2, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::away_model::TIP;
use crate::brand;
use crate::screen_kit::{Size, body};

const LABEL: &str = "Away";
const SIZE: Size = Size::Compact;

pub(crate) fn width(ui: &Ui) -> f32 {
    let text = ui.painter().layout_no_wrap(LABEL.to_owned(), body(SIZE.font()), brand::SOFT);
    text.size().x + SIZE.pad() * 2.0
}

/// Draws the pill; the response says whether it was clicked.
pub(crate) fn pill(ui: &mut Ui, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width(ui), SIZE.height()), Sense::click());
    let (fill, line, ink) = match (on, response.hovered()) {
        (true, _) => (brand::TEAL.gamma_multiply(0.18), brand::TEAL, brand::TEAL_TEXT),
        (false, true) => (brand::BUTTON, brand::ANSWER_LINE, brand::TEXT),
        (false, false) => (egui::Color32::TRANSPARENT, brand::FIELD_LINE, brand::SOFT),
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same((SIZE.height() / 2.0) as u8),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    painter.text(rect.center(), Align2::CENTER_CENTER, LABEL, body(SIZE.font()), ink);
    kelp_ui::widgets::focus_ring(ui, &response, SIZE.height() / 2.0);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(TIP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput, Rect, pos2};

    fn frame(ctx: &egui::Context, events: Vec<Event>, time: f64, on: bool) -> Response {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = None;
        let _ = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| out = Some(pill(ui, on)));
        });
        out.expect("the pill is drawn every frame")
    }

    fn click(at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn clicked_at(offset: f32, on: bool) -> bool {
        let ctx = crate::fonts::test_context();
        let at = frame(&ctx, vec![], 0.0, on).rect.center() + vec2(offset, 0.0);
        frame(&ctx, vec![Event::PointerMoved(at)], 0.1, on);
        frame(&ctx, vec![click(at, true)], 0.2, on);
        frame(&ctx, vec![click(at, false)], 0.3, on).clicked()
    }

    #[test]
    fn a_click_on_the_pill_is_reported_in_both_states() {
        assert!(clicked_at(0.0, false));
        assert!(clicked_at(0.0, true));
    }

    #[test]
    fn a_click_beside_the_pill_is_not_reported() {
        assert!(!clicked_at(200.0, false));
    }
}
