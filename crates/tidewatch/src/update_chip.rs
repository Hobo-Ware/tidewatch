//! The update chip: a quiet pill with a dot, the version and one word; orange when an update failed or is stuck.

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
    OpenSettings,
    Fix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Teal,
    Orange,
}

impl Tone {
    fn dot(self) -> egui::Color32 {
        match self {
            Self::Teal => brand::TEAL,
            Self::Orange => brand::ORANGE,
        }
    }

    fn word(self) -> egui::Color32 {
        match self {
            Self::Teal => brand::TEAL_TEXT,
            Self::Orange => brand::ORANGE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) text: String,
    pub(crate) word: Option<&'static str>,
    pub(crate) click: Click,
    pub(crate) tone: Tone,
    pub(crate) tip: Option<String>,
}

/// What the chip says for the updater's state; `None` when there is nothing to show.
pub(crate) fn spec(status: &Status, brew_install: bool) -> Option<Spec> {
    let teal = |text: String, word, click| Spec { text, word, click, tone: Tone::Teal, tip: None };
    let spec = match status {
        Status::Available(release) if brew_install => {
            teal(format!("{} available", release.version), Some("Update"), Click::Install)
        }
        Status::Available(release) => {
            teal(format!("{} available", release.version), Some("View"), Click::Open)
        }
        Status::Installing(release) => Spec {
            tip: Some(RESTART_WAITS.to_owned()),
            ..teal(format!("Installing {}", release.version), None, Click::Nothing)
        },
        Status::Ready(release) => {
            teal(format!("{} ready", release.version), Some("Restart"), Click::Restart)
        }
        Status::Failed(reason) => Spec {
            tone: Tone::Orange,
            tip: Some(format!("{reason}\nClick for details in Settings.")),
            ..teal("Update failed".to_owned(), None, Click::OpenSettings)
        },
        Status::Stuck(path) => Spec {
            tone: Tone::Orange,
            tip: Some(format!(
                "A Homebrew upgrade did not finish and left {}.\nFix repairs the install and reopens Tidewatch.",
                path.display()
            )),
            ..teal("Update stuck \u{b7}".to_owned(), Some("Fix"), Click::Fix)
        },
        Status::Idle | Status::Checking | Status::UpToDate => return None,
    };
    Some(spec)
}

pub(crate) const RESTART_WAITS: &str = "Restart is available once the install finishes.";

fn texts(
    ui: &Ui,
    spec: &Spec,
) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>) {
    let text = ui.painter().layout_no_wrap(spec.text.clone(), body(FONT), brand::SOFT);
    let word = spec
        .word
        .map(|word| ui.painter().layout_no_wrap(word.to_owned(), body(FONT), spec.tone.word()));
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
    let line = if hovered { spec.tone.dot().gamma_multiply(0.5) } else { brand::FIELD_LINE };
    painter.rect(
        rect,
        CornerRadius::same(255),
        brand::CARD,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let y = rect.center().y;
    let mut x = rect.left() + PAD_LEFT;
    painter.circle_filled(pos2(x + DOT / 2.0, y), DOT / 2.0, spec.tone.dot());
    x += DOT + GAP;
    let width = text.size().x;
    painter.galley(pos2(x, y - text.size().y / 2.0), text, brand::SOFT);
    x += width + GAP;
    if let Some(word) = word {
        painter.galley(pos2(x, y - word.size().y / 2.0), word, spec.tone.word());
    }
    let response = match &spec.tip {
        Some(tip) => response.on_hover_text(tip),
        None => response,
    };
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
            let got = spec(&status, brew).map(|s| (s.text, s.word, s.click, s.tone));
            let expected =
                expected.map(|(text, word, click)| (text.to_owned(), word, click, Tone::Teal));
            assert_eq!(got, expected, "{status:?} brew={brew}");
        }
    }

    #[test]
    fn a_failed_update_is_an_orange_chip_that_opens_settings_with_the_error_as_tooltip() {
        for brew in [true, false] {
            let chip = spec(&Status::Failed("brew upgrade failed: no".into()), brew).unwrap();
            assert_eq!(chip.text, "Update failed");
            assert_eq!(
                (chip.word, chip.click, chip.tone),
                (None, Click::OpenSettings, Tone::Orange)
            );
            let tip = chip.tip.unwrap();
            assert!(tip.starts_with("brew upgrade failed: no\n"), "{tip}");
        }
    }

    #[test]
    fn a_stuck_update_is_an_orange_chip_with_a_fix_word() {
        let path = std::path::PathBuf::from("/opt/homebrew/Caskroom/tidewatch/0.1.10.upgrading");
        let chip = spec(&Status::Stuck(path), true).unwrap();
        assert_eq!(chip.text, "Update stuck \u{b7}");
        assert_eq!((chip.word, chip.click, chip.tone), (Some("Fix"), Click::Fix, Tone::Orange));
        assert!(chip.tip.unwrap().contains("0.1.10.upgrading"));
    }

    #[test]
    fn the_installing_chip_says_why_restart_is_not_offered() {
        let chip = spec(&Status::Installing(release()), true).unwrap();
        assert_eq!(chip.click, Click::Nothing);
        assert_eq!(chip.tip.as_deref(), Some(RESTART_WAITS));
    }

    #[test]
    fn the_word_adds_to_the_width() {
        assert_eq!(width_of(50.0, None), PAD_LEFT + DOT + GAP + 50.0 + PAD_RIGHT);
        assert_eq!(width_of(50.0, Some(30.0)) - width_of(50.0, None), GAP + 30.0);
    }
}
