//! The compact pull request card in a band above the panes: its line and, where there is something to ask, one button.

use egui::{Align, Align2, Layout, Rect, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::delivery_card::{CardView, Dot};
use crate::reader_view::{INSET, paint_item};
use crate::screen_kit::{Size, body, primary_button};

pub(crate) const BAND_H: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardAction {
    Ask,
}

fn dot_ink(dot: Dot) -> egui::Color32 {
    match dot {
        Dot::Ready => brand::GREEN,
        Dot::Asked => brand::TEAL,
        Dot::Missing => brand::AMBER,
    }
}

/// Draws the compact card in `rect` (a band `BAND_H` tall) and reports a click on its button.
pub(crate) fn show_card(ui: &mut Ui, rect: Rect, view: &CardView) -> Option<CardAction> {
    let card = rect.shrink2(vec2(INSET, 4.0));
    paint_item(ui.painter(), card, false);
    ui.painter().circle_filled(pos2(card.left() + 16.0, card.center().y), 4.5, dot_ink(view.dot));
    ui.painter().text(
        pos2(card.left() + 32.0, card.center().y),
        Align2::LEFT_CENTER,
        &view.text,
        body(13.0),
        brand::TEXT,
    );
    let label = view.button.as_deref()?;
    let button_rect = card.shrink2(vec2(10.0, 0.0));
    let mut action = None;
    ui.scope_builder(
        UiBuilder::new().max_rect(button_rect).layout(Layout::right_to_left(Align::Center)),
        |ui| {
            if primary_button(ui, label, true, Size::Compact).clicked() {
                action = Some(CardAction::Ask);
            }
        },
    );
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::painted_strings;
    use egui::RawInput;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx
    }

    fn card_texts(view: &CardView) -> Vec<String> {
        let ctx = context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            focused: true,
            ..RawInput::default()
        };
        let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, BAND_H));
        let _ = ctx.run_ui(raw.clone(), |ui| {
            show_card(ui, band, view);
        });
        let output = ctx.run_ui(raw, |ui| {
            show_card(ui, band, view);
        });
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            painted_strings(&clipped.shape, &mut texts);
        }
        texts
    }

    fn view(text: &str, dot: Dot, button: Option<&str>) -> CardView {
        CardView { text: text.to_owned(), dot, button: button.map(str::to_owned) }
    }

    #[test]
    fn a_ready_card_names_the_commits_and_offers_the_ask() {
        let texts =
            card_texts(&view("2 commits ready", Dot::Ready, Some("Ask Builder to open a PR")));
        assert!(texts.contains(&"2 commits ready".to_owned()), "{texts:?}");
        assert!(texts.contains(&"Ask Builder to open a PR".to_owned()), "{texts:?}");
    }

    #[test]
    fn an_asked_card_says_so_and_has_no_button() {
        let line = "Asked Builder to open the pull request";
        let texts = card_texts(&view(line, Dot::Asked, None));
        assert_eq!(texts, [line]);
    }

    #[test]
    fn a_missing_card_says_no_pull_request_yet_and_offers_only_ask_again() {
        let texts = card_texts(&view("No pull request yet", Dot::Missing, Some("Ask again")));
        assert_eq!(texts, ["No pull request yet", "Ask again"]);
    }

    fn text_rects(view: &CardView) -> Vec<(String, Rect)> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => {
                    let rect = Rect::from_min_size(text.pos, text.galley.size());
                    out.push((text.galley.text().to_owned(), rect));
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
                _ => {}
            }
        }
        let ctx = context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            focused: true,
            ..RawInput::default()
        };
        let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, BAND_H));
        let _ = ctx.run_ui(raw.clone(), |ui| {
            show_card(ui, band, view);
        });
        let output = ctx.run_ui(raw, |ui| {
            show_card(ui, band, view);
        });
        let mut found = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut found);
        }
        found
    }

    #[test]
    fn the_line_starts_after_the_dot_and_the_button_ends_inside_the_card_on_one_centre_line() {
        let card =
            Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, BAND_H)).shrink2(vec2(INSET, 4.0));
        let found = text_rects(&view("No pull request yet", Dot::Missing, Some("Ask again")));
        let line = found.iter().find(|(text, _)| text == "No pull request yet").unwrap().1;
        let button = found.iter().find(|(text, _)| text == "Ask again").unwrap().1;
        assert!((line.left() - (card.left() + 32.0)).abs() < 0.5, "{line:?}");
        assert!(button.right() <= card.right() - 10.0, "{button:?} in {card:?}");
        assert!(button.left() > line.right(), "the button is right of the line");
        assert!((line.center().y - card.center().y).abs() < 1.5, "{line:?}");
        assert!((button.center().y - card.center().y).abs() < 1.5, "{button:?}");
    }
}
