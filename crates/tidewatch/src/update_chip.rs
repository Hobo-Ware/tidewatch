//! The update chip: a quiet pill with a teal dot, the version and one teal word.

use egui::{CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use kelp_ui::update::Status;

use crate::brand;
use crate::screen_kit::body;

pub(crate) const HEIGHT: f32 = 26.0;
const PAD_LEFT: f32 = 10.0;
const PAD_RIGHT: f32 = 12.0;
const DOT: f32 = 7.0;
const GAP: f32 = 8.0;
const FONT: f32 = 13.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    Nothing,
    Install,
    Open,
    Restart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) text: String,
    pub(crate) word: Option<&'static str>,
    pub(crate) click: Click,
}

/// What the chip says for the updater's state; `None` when there is nothing to show.
pub(crate) fn spec(status: &Status, brew_install: bool) -> Option<Spec> {
    let (text, word, click) = match status {
        Status::Available(release) if brew_install => {
            (format!("{} available", release.version), Some("Update"), Click::Install)
        }
        Status::Available(release) => {
            (format!("{} available", release.version), Some("View"), Click::Open)
        }
        Status::Installing(release) => {
            (format!("Installing {}", release.version), None, Click::Nothing)
        }
        Status::Ready(release) => {
            (format!("{} ready", release.version), Some("Restart"), Click::Restart)
        }
        Status::Idle | Status::Checking | Status::UpToDate | Status::Failed(_) => return None,
    };
    Some(Spec { text, word, click })
}

fn texts(
    ui: &Ui,
    spec: &Spec,
) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>) {
    let text = ui.painter().layout_no_wrap(spec.text.clone(), body(FONT), brand::SOFT);
    let word = spec
        .word
        .map(|word| ui.painter().layout_no_wrap(word.to_owned(), body(FONT), brand::TEAL_TEXT));
    (text, word)
}

fn width_of(text: f32, word: Option<f32>) -> f32 {
    PAD_LEFT + DOT + GAP + text + word.map_or(0.0, |w| GAP + w) + PAD_RIGHT
}

pub(crate) fn width(ui: &Ui, spec: &Spec) -> f32 {
    let (text, word) = texts(ui, spec);
    width_of(text.size().x, word.map(|w| w.size().x))
}

pub(crate) fn show(ui: &mut Ui, spec: &Spec) -> Response {
    let (text, word) = texts(ui, spec);
    let width = width_of(text.size().x, word.as_ref().map(|w| w.size().x));
    let sense = if spec.click == Click::Nothing { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), sense);
    let hovered = response.hovered() && spec.click != Click::Nothing;
    let painter = ui.painter();
    let line = if hovered { brand::TEAL.gamma_multiply(0.5) } else { brand::FIELD_LINE };
    painter.rect(
        rect,
        CornerRadius::same(255),
        brand::CARD,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let y = rect.center().y;
    let mut x = rect.left() + PAD_LEFT;
    painter.circle_filled(pos2(x + DOT / 2.0, y), DOT / 2.0, brand::TEAL);
    x += DOT + GAP;
    let width = text.size().x;
    painter.galley(pos2(x, y - text.size().y / 2.0), text, brand::SOFT);
    x += width + GAP;
    if let Some(word) = word {
        painter.galley(pos2(x, y - word.size().y / 2.0), word, brand::TEAL_TEXT);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::update::Release;

    fn release() -> Release {
        Release { version: "0.1.5".into(), url: "https://example.test/0.1.5".into() }
    }

    #[test]
    fn the_chip_names_the_version_and_the_one_thing_a_click_does() {
        let cases = [
            (Status::Idle, true, None),
            (Status::Checking, true, None),
            (Status::UpToDate, false, None),
            (Status::Failed("offline".into()), true, None),
            (
                Status::Available(release()),
                true,
                Some(("0.1.5 available", Some("Update"), Click::Install)),
            ),
            (
                Status::Available(release()),
                false,
                Some(("0.1.5 available", Some("View"), Click::Open)),
            ),
            (Status::Installing(release()), true, Some(("Installing 0.1.5", None, Click::Nothing))),
            (
                Status::Ready(release()),
                true,
                Some(("0.1.5 ready", Some("Restart"), Click::Restart)),
            ),
        ];
        for (status, brew, expected) in cases {
            let expected =
                expected.map(|(text, word, click)| Spec { text: text.to_owned(), word, click });
            assert_eq!(spec(&status, brew), expected, "{status:?} brew={brew}");
        }
    }

    #[test]
    fn the_word_adds_to_the_width() {
        assert_eq!(width_of(50.0, None), PAD_LEFT + DOT + GAP + 50.0 + PAD_RIGHT);
        assert_eq!(width_of(50.0, Some(30.0)) - width_of(50.0, None), GAP + 30.0);
    }
}
