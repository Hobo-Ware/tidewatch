//! The panel: pure choices, and headless frames for the clicks and for text that never overlaps.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, pos2, vec2};
use tidewatch_core::progress::Ledger;

use super::*;

const LONG: &str = "# Goal\nAdd a slugify(text) function to lib.py that turns spaces and punctuation into single hyphens, with unit tests, and commit it\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the validator\n\n## Checklist\n### 1. Understand and decide\n- [x] Restate the goal in one line\n- [x] Read the code that owns the behavior\n### 2. Build\n- [x] Smallest change that does the job -- reused two helpers\n- [~] Pure logic in its own small function with a unit test\n- [ ] Remove anything the change left unused\n- [-] Format, type check and lint on the touched files -- no formatter or linter config in this repo at all\n- [!] Wait for the database\n- [ ] Item six\n- [ ] Item seven\n- [ ] Item eight\n- [ ] Item nine\n- [ ] Item ten\n### 3. Gate\n- [-] Lint -- no config\n### 4. Validate against production\n- [ ] Compare timings\n### 5. Close out\n- [ ] Report\n\n## Decisions\n- Follow the unittest style of the repo -- match existing code\n- Keep accented letters\n\n## Verified\n- Ran 8 tests, OK -- python3 -m unittest -v\n- Only two files changed\n\n## Not verified\n- No formatter run\n\n## Needs you\n- Decide if accented letters should become ASCII (cafe). Current code keeps them -- it changes the output\n";

fn known(text: &str) -> TabLedger {
    TabLedger { ledger: Ledger::parse(text), updated_ms: Some(0) }
}

struct Panel {
    ctx: egui::Context,
    ledger: TabLedger,
    state: TabState,
    size: egui::Vec2,
    texts: Vec<(String, Rect)>,
    action: Option<Action>,
}

impl Panel {
    fn new(text: &str, width: f32) -> Self {
        let mut panel = Self {
            ctx: crate::fonts::test_context(),
            ledger: known(text),
            state: TabState::default(),
            size: vec2(width, 900.0),
            texts: Vec::new(),
            action: None,
        };
        panel.frame(Vec::new());
        panel.frame(Vec::new());
        panel
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), self.size)),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let (ledger, state, size) = (&self.ledger, &mut self.state, self.size);
        let output = self.ctx.run_ui(raw, |ui| {
            let input = Input { ledger, now_ms: 240_000, time: 1.0, animate: false };
            action = show(ui, Rect::from_min_size(pos2(0.0, 0.0), size), &input, state);
        });
        self.texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.texts);
        }
        self.action = action;
        action
    }

    fn center_of(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} not drawn in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        let action = self.frame(vec![button(false)]);
        self.frame(Vec::new());
        action
    }

    fn drawn(&self, text: &str) -> bool {
        self.texts.iter().any(|(drawn, _)| drawn == text)
    }
}

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            if !text.galley.text().trim().is_empty() {
                out.push((text.galley.text().to_owned(), rect));
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

#[test]
fn history_shows_until_a_ledger_arrives_then_the_choice_or_progress() {
    let cases = [
        (None, false, Side::History),
        (Some(Side::Progress), false, Side::History),
        (None, true, Side::Progress),
        (Some(Side::History), true, Side::History),
        (Some(Side::Progress), true, Side::Progress),
    ];
    for (chosen, has_ledger, want) in cases {
        assert_eq!(side_shown(chosen, has_ledger), want, "{chosen:?} {has_ledger}");
    }
    assert_eq!(SIDES.map(str::len), [8, 7]);
    assert_eq!([Side::Progress, Side::History].map(Side::index), [0, 1]);
    assert_eq!([0, 1].map(Side::at), [Side::Progress, Side::History]);
}

#[test]
fn a_tab_remembers_its_choices_and_forgets_closed_tabs() {
    let mut memory = Memory::default();
    memory.tab(1).chosen = Some(Side::History);
    memory.tab(1).toggle(Tag::Decisions);
    memory.tab(2);
    assert_eq!(memory.tab(1).chosen, Some(Side::History));
    assert!(memory.tab(1).is_open(Tag::Decisions) && !memory.tab(1).is_open(Tag::Verified));
    memory.tab(1).toggle(Tag::Decisions);
    assert!(!memory.tab(1).is_open(Tag::Decisions));
    memory.forget_except(|id| id == 2);
    assert_eq!(memory.tab(1).chosen, None, "a closed tab starts over");
}

#[test]
fn phase_bars_wrap_into_rows_that_keep_a_readable_label() {
    let cases = [(400.0, 4, 4), (400.0, 10, 7), (196.0, 10, 3), (50.0, 5, 1), (1000.0, 3, 3)];
    for (width, count, want) in cases {
        assert_eq!(bar_columns(width, count), want, "{width} {count}");
    }
}

#[test]
fn tags_flow_into_rows_and_a_row_break_never_leaves_the_edge() {
    let (places, height) = flow(&[80.0, 90.0, 70.0], 300.0);
    assert_eq!(places, [(0.0, 0.0), (88.0, 0.0), (186.0, 0.0)]);
    assert_eq!(height, TAG_H);
    let (places, height) = flow(&[120.0, 120.0, 120.0], 260.0);
    assert_eq!(places, [(0.0, 0.0), (128.0, 0.0), (0.0, TAG_H + TAG_GAP)]);
    assert_eq!(height, 2.0 * TAG_H + TAG_GAP);
    assert_eq!(flow(&[], 100.0), (Vec::new(), 0.0));
}

#[test]
fn the_answer_button_asks_to_answer_and_nothing_else_does() {
    let mut panel = Panel::new(LONG, 420.0);
    assert_eq!(panel.action, None);
    let at = panel.center_of("Answer");
    assert_eq!(panel.click(at), Some(Action::Answer));
}

#[test]
fn no_question_means_no_card_and_no_button() {
    let text = LONG.replace("## Needs you\n- Decide if accented letters should become ASCII (cafe). Current code keeps them -- it changes the output\n", "## Needs you\n- Nothing needs you.\n");
    let panel = Panel::new(&text, 420.0);
    assert!(!panel.drawn("Answer") && !panel.drawn("?"));
}

#[test]
fn more_than_eight_rows_hide_behind_show_all_and_it_toggles() {
    let mut panel = Panel::new(LONG, 420.0);
    assert!(panel.drawn("Show all 10"));
    assert!(!panel.drawn("Item nine") && !panel.drawn("Item ten"));
    let at = panel.center_of("Show all 10");
    panel.click(at);
    assert!(panel.state.show_all_rows());
    assert!(panel.drawn("Item ten") && panel.drawn("Show less"));
    let at = panel.center_of("Show less");
    panel.click(at);
    assert!(!panel.drawn("Item ten"));
}

#[test]
fn a_tag_opens_its_list_in_place_and_a_second_click_closes_it() {
    let mut panel = Panel::new(LONG, 420.0);
    assert!(
        panel.drawn("2 verified") && panel.drawn("1 not verified") && panel.drawn("2 decisions")
    );
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
    let at = panel.center_of("2 verified");
    panel.click(at);
    assert!(panel.state.is_open(Tag::Verified));
    assert!(panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
    assert!(!panel.state.is_open(Tag::Decisions));
    let at = panel.center_of("2 verified");
    panel.click(at);
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
}

#[test]
fn a_tag_with_nothing_behind_it_is_not_drawn() {
    let panel = Panel::new("# Goal\nx\n\n## Verified\n- ok\n\n## Not verified\n- nothing\n", 420.0);
    assert!(panel.drawn("1 verified"));
    assert!(!panel.texts.iter().any(|(text, _)| text.contains("not verified")));
}

#[test]
fn the_count_sits_in_the_ring_and_the_meta_line_says_state_and_age() {
    let panel = Panel::new(LONG, 420.0);
    assert!(panel.drawn("3/13"), "{:?}", panel.texts);
    assert!(
        panel.drawn("\u{25cf} Blocked \u{b7} updated 4 min ago"),
        "a blocked item makes it blocked"
    );
}

#[test]
fn no_text_overlaps_at_any_width_the_card_has() {
    for width in [200.0, 240.0, 300.0, 330.0, 420.0, 640.0] {
        let mut panel = Panel::new(LONG, width);
        for tag in TAGS {
            let at = panel.center_of(&tag.label(entries(&panel.ledger.ledger, tag).len()));
            panel.click(at);
        }
        let at = panel.center_of("Show all 10");
        panel.click(at);
        for (index, (a, a_rect)) in panel.texts.iter().enumerate() {
            assert!(a_rect.right() <= width + 0.5, "{a:?} runs past the card at {width}");
            for (b, b_rect) in &panel.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}
