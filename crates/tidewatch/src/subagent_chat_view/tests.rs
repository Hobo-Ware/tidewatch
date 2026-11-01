use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, pos2};
use tidewatch_core::subagents::{Agent, Phase};

use super::*;
use crate::subagent_chat::Source;
use crate::test_support::painted_text_rects_nonblank;

const NOW: i64 = 2_000_000;

fn agent(phase: Phase) -> Agent {
    Agent {
        id: "fake-0".into(),
        description: "Build hover actions".into(),
        model: Some("sonnet".into()),
        effort: Some("high".into()),
        started_ms: NOW - 253_000,
        phase,
        now: "Read acts-brief.md".into(),
        tokens: 281_400,
    }
}

struct Frames {
    ctx: egui::Context,
    chat: Open,
    face: Face,
    area: Rect,
    texts: Vec<(String, Rect)>,
    repaint: std::time::Duration,
}

impl Frames {
    fn new(phase: Phase, width: f32) -> Self {
        let ctx = crate::fonts::test_context();
        let finished = !matches!(phase, Phase::Running);
        let chat = Open::new(1, agent(phase), Source::Fake { finished }, &ctx);
        let mut frames = Self {
            ctx,
            chat,
            face: Face::of("Builder", "claude", None),
            area: Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0)),
            texts: Vec::new(),
            repaint: std::time::Duration::MAX,
        };
        for _ in 0..3 {
            frames.run(Vec::new(), true);
        }
        frames
    }

    fn run(&mut self, events: Vec<Event>, keys_free: bool) -> Option<Out> {
        let raw =
            RawInput { screen_rect: Some(self.area), events, focused: true, ..RawInput::default() };
        let mut out = None;
        let (area, chat, face) = (self.area, &mut self.chat, &self.face);
        let output = self.ctx.run_ui(raw, |ui| {
            let facts = Facts { chat, parent: ("Builder", face), now_ms: NOW, keys_free };
            out = show(ui, area, facts);
        });
        self.repaint = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        self.texts.clear();
        for shape in &output.shapes {
            painted_text_rects_nonblank(&shape.shape, &mut self.texts);
        }
        out
    }

    fn at(&self, text: &str) -> Option<Rect> {
        self.texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| *rect)
    }

    fn click(&mut self, at: Pos2) -> Option<Out> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.run(vec![Event::PointerMoved(at)], true);
        self.run(vec![button(true)], true);
        self.run(vec![button(false)], true)
    }

    fn escape(&mut self, keys_free: bool) -> Option<Out> {
        let key = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.run(vec![key(true), key(false)], keys_free)
    }
}

#[test]
fn the_header_reads_like_the_logs_header_with_the_agent_its_model_its_time_and_tokens() {
    let frames = Frames::new(Phase::Running, 900.0);
    let back = frames.at(BACK_LABEL).expect("the back button");
    let what = frames.at("Build hover actions").expect("what the agent does");
    let tag = frames.at("sonnet \u{b7} high").expect("model and effort");
    let right = frames.at("4m 13s \u{b7} 281k").expect("time and tokens");
    assert!(back.right() < what.left() && what.right() < tag.left() && tag.right() < right.left());
    for rect in [back, what, tag, right] {
        assert!(rect.bottom() <= layout::CHROME_H + 4.0, "{rect:?}");
    }
}

#[test]
fn a_finished_agent_says_done_and_keeps_the_same_columns() {
    let frames = Frames::new(Phase::Finished(NOW - 4_000), 900.0);
    assert!(frames.at("done 4m 09s \u{b7} 281k").is_some(), "{:?}", frames.texts);
    assert!(frames.at("Build hover actions").is_some());
}

#[test]
fn the_prompt_is_a_bubble_signed_with_the_parent_and_the_agents_own_turns_are_not_signed_you() {
    let frames = Frames::new(Phase::Running, 900.0);
    assert!(frames.at("Read the hover toolbar code and fold the fix.").is_some());
    let signed: Vec<_> = frames.texts.iter().map(|(text, _)| text.as_str()).collect();
    assert!(signed.iter().any(|text| text.starts_with("Builder")), "{signed:?}");
    assert!(signed.iter().all(|text| !text.starts_with("You")), "{signed:?}");
}

#[test]
fn nothing_to_reply_with_is_drawn() {
    let frames = Frames::new(Phase::Running, 900.0);
    let drawn: Vec<_> = frames.texts.iter().map(|(text, _)| text.to_lowercase()).collect();
    for word in ["reply", "message", "send", "needs you"] {
        assert!(drawn.iter().all(|text| !text.contains(word)), "{word}: {drawn:?}");
    }
}

#[test]
fn the_back_button_and_escape_go_back_but_escape_waits_while_a_field_or_dialog_has_the_keys() {
    let mut frames = Frames::new(Phase::Running, 900.0);
    let back = frames.at(BACK_LABEL).unwrap().center();
    assert_eq!(frames.click(back), Some(Out::Back));
    assert_eq!(frames.escape(false), None);
    assert_eq!(frames.escape(true), Some(Out::Back));
}

#[test]
fn no_text_leaves_the_area_or_crosses_another_header_text_at_any_width() {
    for width in [360.0, 480.0, 700.0, 1200.0] {
        let frames = Frames::new(Phase::Running, width);
        for (text, rect) in &frames.texts {
            assert!(
                rect.left() >= -0.5 && rect.right() <= width + 0.5,
                "{width}: {text:?} {rect:?}"
            );
        }
        let header: Vec<_> =
            frames.texts.iter().filter(|(_, rect)| rect.top() < layout::CHROME_H).collect();
        for (n, (a, first)) in header.iter().enumerate() {
            for (b, second) in header.iter().skip(n + 1) {
                assert!(!first.intersects(*second), "{width}: {a:?} {b:?}");
            }
        }
    }
}

#[test]
fn a_running_agent_asks_for_a_frame_each_second_for_its_clock() {
    let mut frames = Frames::new(Phase::Running, 900.0);
    frames.run(Vec::new(), true);
    assert!(frames.repaint <= std::time::Duration::from_secs(1), "{:?}", frames.repaint);
}
