//! The one choice card in its compact form, for an Overview card (pure model and layout, then
//! drawing); a click is answered through the same function as the Reader's card.

use egui::{Painter, Pos2, Rect, Ui, UiBuilder, vec2};
use tidewatch_core::progress::Choice;
use tidewatch_core::text::clip;

use crate::ask_model::choices;
use crate::choice::CHAT_LABEL;
use crate::progress_view::flow;
use crate::screen_kit::{Size, body, primary_button, quiet_button};

pub(crate) const GAP: f32 = 8.0;
const OTHER: &str = "Something else";

/// What a click on the compact card means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// The option at this place in the question's options.
    Option(usize),
    Chat,
    /// Answer it in the tab, where the full card is.
    Open,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    click: Click,
    label: String,
    full: Option<String>,
    main: bool,
}

/// The buttons of one question, laid out in rows of `width`.
pub(crate) struct Compact {
    items: Vec<Item>,
    places: Vec<(f32, f32)>,
    widths: Vec<f32>,
    height: f32,
}

fn fit(painter: &Painter, label: &str, room: f32) -> (String, f32) {
    let pad = 2.0 * Size::Compact.pad();
    let measure = |text: &str| {
        painter
            .layout_no_wrap(text.to_owned(), body(Size::Compact.font()), crate::brand::TEXT)
            .size()
            .x
    };
    let mut keep = label.chars().count();
    loop {
        let shown = clip(label, keep);
        let wide = measure(&shown) + pad;
        if wide <= room || keep <= 1 {
            return (shown, wide.min(room.max(pad)));
        }
        keep -= 1;
    }
}

fn items(options: &[Choice]) -> Vec<Item> {
    let mut items: Vec<Item> = choices(options)
        .into_iter()
        .enumerate()
        .map(|(index, choice)| Item {
            click: Click::Option(index),
            label: choice.text,
            full: None,
            main: choice.recommended,
        })
        .collect();
    items.push(Item { click: Click::Chat, label: CHAT_LABEL.to_owned(), full: None, main: false });
    items.push(Item { click: Click::Open, label: OTHER.to_owned(), full: None, main: false });
    items
}

impl Compact {
    pub(crate) fn new(painter: &Painter, options: &[Choice], width: f32) -> Self {
        let mut items = items(options);
        let mut widths = Vec::with_capacity(items.len());
        for item in &mut items {
            let (shown, wide) = fit(painter, &item.label, width);
            if shown != item.label {
                item.full = Some(std::mem::replace(&mut item.label, shown));
            }
            widths.push(wide);
        }
        let (places, height) = flow(&widths, width, Size::Compact.height(), GAP);
        Self { items, places, widths, height }
    }

    pub(crate) fn height(&self) -> f32 {
        self.height
    }

    /// Draws the buttons with their top left at `origin` and returns the one clicked.
    pub(crate) fn show(&self, ui: &mut Ui, origin: Pos2) -> Option<Click> {
        let mut clicked = None;
        for ((item, (x, y)), width) in self.items.iter().zip(&self.places).zip(&self.widths) {
            let rect =
                Rect::from_min_size(origin + vec2(*x, *y), vec2(*width, Size::Compact.height()));
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
            let response = if item.main {
                primary_button(&mut child, &item.label, true, Size::Compact)
            } else {
                quiet_button(&mut child, &item.label, true, Size::Compact)
            };
            let response = match &item.full {
                Some(full) => response.on_hover_text(full),
                None => response,
            };
            if response.clicked() {
                clicked = Some(item.click);
            }
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(text: &str, recommended: bool) -> Choice {
        Choice { text: text.to_owned(), recommended }
    }

    fn summary(options: &[Choice]) -> Vec<(Click, String, bool)> {
        items(options).into_iter().map(|i| (i.click, i.label, i.main)).collect()
    }

    #[test]
    fn the_options_come_first_then_chat_and_something_else() {
        let made = summary(&[choice("Approve", true), choice("Wait", false)]);
        assert_eq!(
            made,
            [
                (Click::Option(0), "Approve".to_owned(), true),
                (Click::Option(1), "Wait".to_owned(), false),
                (Click::Chat, CHAT_LABEL.to_owned(), false),
                (Click::Open, "Something else".to_owned(), false),
            ]
        );
    }

    #[test]
    fn no_options_gives_done_and_not_now() {
        let labels: Vec<String> = summary(&[]).into_iter().map(|(_, label, _)| label).collect();
        assert_eq!(labels, ["Done", "Not now", CHAT_LABEL, "Something else"]);
    }

    #[test]
    fn a_long_answer_is_cut_to_the_room_and_keeps_its_whole_text_for_the_tooltip() {
        let long = "Keep the new loss and rerun the whole benchmark on the replica before the rollout window opens";
        let ctx = crate::fonts::test_context();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let compact =
                Compact::new(ui.painter(), &[choice(long, true), choice("Wait", false)], 200.0);
            let first = &compact.items[0];
            assert!(first.label.ends_with("..."), "{}", first.label);
            assert_eq!(first.full.as_deref(), Some(long));
            assert!(compact.widths[0] <= 200.0, "{}", compact.widths[0]);
            assert_eq!(compact.items[1].full, None);
        });
    }

    #[test]
    fn the_buttons_flow_into_rows_that_fit_the_width() {
        let ctx = crate::fonts::test_context();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let options = [choice("One", false), choice("Two", false), choice("Three", false)];
            let wide = Compact::new(ui.painter(), &options, 2000.0);
            let narrow = Compact::new(ui.painter(), &options, 160.0);
            assert_eq!(wide.height(), Size::Compact.height());
            assert!(narrow.height() > wide.height(), "{} {}", narrow.height(), wide.height());
        });
    }
}
