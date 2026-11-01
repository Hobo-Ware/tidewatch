//! The bar says the agent is not running and Resume hands back one click.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

use super::*;
use crate::test_support::painted_text_rects_nonblank;

struct Bar {
    ctx: egui::Context,
    width: f32,
    texts: Vec<(String, Rect)>,
}

impl Bar {
    fn new(width: f32) -> Self {
        let mut bar = Self { ctx: crate::fonts::test_context(), width, texts: Vec::new() };
        bar.frame(Vec::new());
        bar.frame(Vec::new());
        bar
    }

    fn frame(&mut self, events: Vec<Event>) -> bool {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, HEIGHT));
        let mut clicked = false;
        let output = self.ctx.run_ui(raw, |ui| clicked = show(ui, band));
        self.texts.clear();
        for shape in &output.shapes {
            painted_text_rects_nonblank(&shape.shape, &mut self.texts);
        }
        clicked
    }

    fn at(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> bool {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }
}

#[test]
fn the_bar_says_the_agent_is_not_running_and_offers_resume() {
    let bar = Bar::new(600.0);
    for want in [LABEL, BUTTON] {
        assert!(bar.texts.iter().any(|(text, _)| text == want), "{want}: {:?}", bar.texts);
    }
}

#[test]
fn pressing_resume_is_the_only_click_the_bar_hands_back() {
    let mut bar = Bar::new(600.0);
    let label = bar.at(LABEL);
    assert!(!bar.click(label));
    let resume = bar.at(BUTTON);
    assert!(bar.click(resume));
}

#[test]
fn text_stays_inside_the_bar_from_a_narrow_width_up() {
    for width in [260.0, 400.0, 900.0] {
        let bar = Bar::new(width);
        let label = bar.texts.iter().find(|(text, _)| text == LABEL).unwrap().1;
        let resume = bar.texts.iter().find(|(text, _)| text == BUTTON).unwrap().1;
        assert!(label.right() < resume.left(), "{width}: {label:?} against {resume:?}");
        assert!(resume.right() <= width - INSET, "{width}");
        assert!(label.top() >= 0.0 && label.bottom() <= HEIGHT, "{width}");
    }
}
