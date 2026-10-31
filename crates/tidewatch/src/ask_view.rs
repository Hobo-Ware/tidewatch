//! The Needs you card above the Reader's reply box: question, ready answers and which one of several.

use egui::epaint::text::TextFormat;
use egui::{CornerRadius, Id, Rect, Stroke, StrokeKind, Ui, pos2};
use std::sync::Arc;
use tidewatch_core::link_board::Link;

use crate::answer_chips::{self, Chip, Pick, Row};
use crate::ask_model::Ask;
use crate::brand;
use crate::link_text::{Laid as Text, Text as Question};
use crate::link_view;
use crate::reader_view::{INSET, ellipsized};
use crate::screen_kit::body;

const PAD: f32 = 16.0;
const NARROW_PAD: f32 = 12.0;
const NARROW_BELOW: f32 = 330.0;
const LABEL_GAP: f32 = 8.0;
const BLOCK_GAP: f32 = 12.0;
const BOTTOM_GAP: f32 = 6.0;
const MIN_BODY: f32 = 120.0;
const TITLE_ROWS: usize = 3;

pub(crate) struct Laid {
    pad: f32,
    inner_w: f32,
    label: Arc<egui::Galley>,
    place: Option<Arc<egui::Galley>>,
    why: Option<Arc<egui::Galley>>,
    title: Text,
    chips: Vec<Chip>,
    row: Row,
    read_h: f32,
    card_h: f32,
}

/// The card laid out for a band `width` wide.
pub(crate) fn lay_out(ui: &Ui, width: f32, ask: &Ask) -> Laid {
    let card_w = (width - 2.0 * INSET).max(80.0);
    let pad = if card_w < NARROW_BELOW { NARROW_PAD } else { PAD };
    let inner_w = (card_w - 2.0 * pad).max(40.0);
    let painter = ui.painter().clone();
    let label = painter.layout_no_wrap("NEEDS YOU".to_owned(), body(12.0), brand::NEEDS);
    let place = (ask.total > 1).then(|| {
        painter.layout_no_wrap(
            format!("{} of {}", ask.index + 1, ask.total),
            body(12.0),
            brand::MUTED,
        )
    });
    let lead = label.size().x + place.as_ref().map_or(0.0, |place| place.size().x + LABEL_GAP);
    let why = ask.entry.why.as_deref().map(str::trim).filter(|why| !why.is_empty()).map(|why| {
        ellipsized(
            &painter,
            &answer_chips::sentence(why),
            body(13.0),
            brand::MUTED,
            (inner_w - lead - LABEL_GAP).max(40.0),
        )
    });
    let mut question = Question::new();
    question.run(ask.entry.text.trim(), &TextFormat::simple(body(16.0), brand::TEXT));
    let title = question.wrapped(inner_w, TITLE_ROWS).lay_out(&painter);
    let chips = answer_chips::chips_marked(&ask.entry.options);
    let row = Row::new(&painter, &chips, inner_w);
    let read_h = if ask.read.is_empty() {
        0.0
    } else {
        link_view::read_first_height(ask.read.len()) + BLOCK_GAP
    };
    let card_h =
        pad * 2.0 + label.size().y + BLOCK_GAP + title.size().y + BLOCK_GAP + read_h + row.height();
    Laid { pad, inner_w, label, place, why, title, chips, row, read_h, card_h }
}

impl Laid {
    /// The room the card takes above the reply box, gap included, or nothing when the chat would be squeezed.
    pub(crate) fn band(&self, room: f32) -> f32 {
        let band = self.card_h + BOTTOM_GAP;
        if band > room - MIN_BODY { 0.0 } else { band }
    }
}

/// Draws the card at the bottom of `band` and returns the pick.
pub(crate) fn show(
    ui: &mut Ui,
    band: Rect,
    laid: &Laid,
    ask: &Ask,
    (reply, reply_empty): (Id, bool),
) -> Option<Pick> {
    let card = Rect::from_min_max(
        pos2(band.left() + INSET, band.bottom() - BOTTOM_GAP - laid.card_h),
        pos2(band.right() - INSET, band.bottom() - BOTTOM_GAP),
    );
    let painter = ui.painter().clone();
    let radius = CornerRadius::same(12);
    painter.rect_filled(card, radius, brand::NEEDS_FILL);
    painter.rect_stroke(card, radius, Stroke::new(1.0, brand::NEEDS), StrokeKind::Inside);
    let left = card.left() + laid.pad;
    let top = card.top() + laid.pad;
    let label_h = laid.label.size().y;
    let mut x = left;
    painter.galley(pos2(x, top), laid.label.clone(), brand::NEEDS);
    x += laid.label.size().x + LABEL_GAP;
    if let Some(place) = &laid.place {
        painter.galley(
            pos2(x, top + (label_h - place.size().y) / 2.0),
            place.clone(),
            brand::MUTED,
        );
        x += place.size().x + LABEL_GAP;
    }
    if let Some(why) = &laid.why {
        painter.galley(pos2(x, top + (label_h - why.size().y) / 2.0), why.clone(), brand::MUTED);
    }
    let title_top = top + label_h + BLOCK_GAP;
    let salt = Id::new(("needs-card", ask.index, ask.total));
    laid.title.paint_selectable(
        ui,
        ui.id().with((salt, "text")),
        pos2(left, title_top),
        brand::TEXT,
    );
    let read_top = title_top + laid.title.size().y + BLOCK_GAP;
    let links: Vec<&Link> = ask.read.iter().collect();
    link_view::paint_read_first(ui, pos2(left, read_top), laid.inner_w, &links);
    let chips_at = pos2(left, read_top + laid.read_h);
    let clicked = laid.row.show(ui, ui.id().with(salt), chips_at, &laid.chips);
    let keyed =
        || ask.keys.then(|| answer_chips::key_pick_over_reply(ui, reply, reply_empty, &laid.chips));
    let picked = clicked.or_else(|| keyed().flatten())?;
    answer_chips::pick_of(&laid.chips, picked)
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};
    use tidewatch_core::link_board::Link;
    use tidewatch_core::progress::Ledger;

    use super::*;
    use crate::reader_rows::Rows;
    use crate::reader_view::{Action, Facts, Turn, ViewState, show as show_reader};

    const ASKING: &str = "# Goal\nShip it\n\n## Status\nstate: waiting\n\n## Needs you\n- Apply the schema change now? -- Copies come back until the key exists -- options: Applied it* | It failed | Later\n- Post the Slack follow-up? -- Only after the key is on -- options: Post it* | Draft only\n";

    fn ask_of(text: &str, index: usize) -> Ask {
        let entries = Ledger::parse(text).needs_you;
        Ask {
            entry: entries[index].clone(),
            index,
            total: entries.len(),
            read: Vec::new(),
            keys: true,
        }
    }

    struct Harness {
        ctx: egui::Context,
        state: ViewState,
        ask: Ask,
        size: egui::Vec2,
        texts: Vec<(String, Rect)>,
        strokes: Vec<Rect>,
    }

    fn collect(shape: &Shape, texts: &mut Vec<(String, Rect)>, strokes: &mut Vec<Rect>) {
        match shape {
            Shape::Text(text) if !text.galley.text().trim().is_empty() => {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                texts.push((text.galley.text().to_owned(), rect));
            }
            Shape::Rect(rect) if rect.stroke.color == brand::NEEDS => strokes.push(rect.rect),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts, strokes)),
            _ => {}
        }
    }

    impl Harness {
        fn new(ask: Ask, width: f32) -> Self {
            let mut harness = Self {
                ctx: crate::fonts::test_context(),
                state: ViewState::default(),
                ask,
                size: vec2(width, 700.0),
                texts: Vec::new(),
                strokes: Vec::new(),
            };
            for _ in 0..3 {
                harness.frame(Vec::new());
            }
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<Action> {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut actions = Vec::new();
            let (state, ask) = (&mut self.state, &self.ask);
            let output = self.ctx.run_ui(raw, |ui| {
                let facts = Facts {
                    tab: 1,
                    source: None,
                    prompt: None,
                    ready: None,
                    root: None,
                    review: None,
                    pr: None,
                    due: None,
                    ask: Some(ask),
                    profile: "Builder",
                    face: None,
                    turn: Turn::Finished,
                    working: false,
                    reply_blocked: None,
                };
                let rect = ui.max_rect();
                actions = show_reader(ui, rect, &Rows::default(), state, &facts);
            });
            self.texts.clear();
            self.strokes.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut self.texts, &mut self.strokes);
            }
            actions
        }

        fn rect_of(&self, text: &str) -> Rect {
            self.texts
                .iter()
                .find(|(drawn, _)| drawn == text)
                .unwrap_or_else(|| panic!("{text:?} not drawn in {:?}", self.texts))
                .1
        }

        fn drawn(&self, text: &str) -> bool {
            self.texts.iter().any(|(drawn, _)| drawn == text)
        }

        fn click(&mut self, text: &str) -> Vec<Action> {
            let at = self.rect_of(text).center();
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            let actions = self.frame(vec![button(false)]);
            self.frame(Vec::new());
            actions
        }

        fn press(&mut self, key: Key, digit: &str) -> Vec<Action> {
            self.frame(vec![
                Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                Event::Text(digit.to_owned()),
            ])
        }
    }

    const NONE: [&str; 0] = [];

    fn picks(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Pick(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_card_sits_directly_above_the_reply_box_with_every_chip_inside_it() {
        let harness = Harness::new(ask_of(ASKING, 0), 700.0);
        let [card] = harness.strokes[..] else {
            panic!("one amber outline: {:?}", harness.strokes)
        };
        let reply = harness.rect_of("Reply to the agent");
        let chips = harness.rect_of("Something else");
        assert!(chips.bottom() < card.bottom(), "chips {chips:?} in card {card:?}");
        assert!(card.bottom() < reply.top(), "card {card:?} above the box text {reply:?}");
        assert!(reply.top() - card.bottom() < 40.0, "the card hugs the box");
        for text in ["NEEDS YOU", "Applied it (recommended)", "It failed", "Later", "1", "2", "3"] {
            assert!(card.contains_rect(harness.rect_of(text)), "{text:?} outside {card:?}");
        }
    }

    #[test]
    fn a_chip_click_sends_its_exact_text_and_something_else_hands_over_the_reply_box() {
        let mut harness = Harness::new(ask_of(ASKING, 0), 700.0);
        assert_eq!(picks(&harness.click("It failed")), ["It failed"]);
        assert_eq!(picks(&harness.click("Applied it (recommended)")), ["Applied it"]);
        harness.ctx.memory_mut(|memory| memory.request_focus(Id::new("elsewhere")));
        harness.frame(Vec::new());
        assert!(!crate::reader_view::reply_has_focus(&harness.ctx));
        assert_eq!(picks(&harness.click("Something else")), NONE);
        assert!(crate::reader_view::reply_has_focus(&harness.ctx));
    }

    #[test]
    fn a_digit_picks_while_the_reply_box_is_empty_and_never_types_into_it() {
        let mut harness = Harness::new(ask_of(ASKING, 0), 700.0);
        assert_eq!(picks(&harness.press(Key::Num2, "2")), ["It failed"]);
        harness.frame(Vec::new());
        assert_eq!(harness.state.reply, "");
        assert_eq!(picks(&harness.press(Key::Num1, "1")), ["Applied it"]);
        assert_eq!(picks(&harness.press(Key::Num7, "7")), NONE, "no seventh chip");
    }

    #[test]
    fn a_digit_belongs_to_the_reply_box_once_it_holds_text() {
        let mut harness = Harness::new(ask_of(ASKING, 0), 700.0);
        harness.frame(vec![Event::Text("run ".into())]);
        assert_eq!(harness.state.reply, "run ");
        assert_eq!(picks(&harness.press(Key::Num1, "1")), NONE);
        assert_eq!(harness.state.reply, "run 1");
    }

    #[test]
    fn a_digit_does_nothing_while_a_dialog_holds_the_keyboard() {
        let mut ask = ask_of(ASKING, 0);
        ask.keys = false;
        let mut harness = Harness::new(ask, 700.0);
        assert_eq!(picks(&harness.press(Key::Num1, "1")), NONE);
    }

    #[test]
    fn several_questions_say_which_one_this_is_beside_the_tag_and_one_question_says_nothing() {
        let harness = Harness::new(ask_of(ASKING, 1), 700.0);
        assert!(harness.drawn("2 of 2") && harness.drawn("Post the Slack follow-up?"));
        let tag = harness.rect_of("NEEDS YOU");
        let place = harness.rect_of("2 of 2");
        assert!(place.left() > tag.right() && (place.center().y - tag.center().y).abs() < 2.0);
        let single = Ask { total: 1, index: 0, ..ask_of(ASKING, 0) };
        let harness = Harness::new(single, 700.0);
        assert!(!harness.texts.iter().any(|(text, _)| text.contains(" of ")));
    }

    #[test]
    fn no_text_overlaps_or_leaves_the_card_at_any_column_width() {
        let long = "# Goal\nx\n\n## Needs you\n- Keep the new loss and rerun the whole benchmark on the replica before the rollout window opens? -- Copies come back at about 7 a day until the key exists on the primary -- options: Keep the new loss and rerun the benchmark* | It failed on a new duplicate | Later | Skip\n";
        for width in [260.0, 320.0, 420.0, 640.0] {
            let harness = Harness::new(ask_of(long, 0), width);
            let [card] = harness.strokes[..] else { panic!("one outline at {width}") };
            assert!(card.right() <= width, "card {card:?} past {width}");
            let inside: Vec<_> = harness
                .texts
                .iter()
                .filter(|(_, rect)| rect.top() >= card.top())
                .filter(|(text, _)| text != "Reply to the agent" && text != "Send")
                .collect();
            for (index, (a, a_rect)) in inside.iter().enumerate() {
                assert!(
                    card.contains_rect(*a_rect),
                    "{a:?} {a_rect:?} outside {card:?} at {width}"
                );
                for (b, b_rect) in &inside[index + 1..] {
                    assert!(
                        !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                        "{a:?} overlaps {b:?} at {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_link_sent_with_the_question_reads_first_inside_the_card_at_any_width() {
        let link = Link {
            url: "https://claude.ai/artifact/abc".to_owned(),
            title: "Split options compared".to_owned(),
            note: Some("Apply the schema change now?".to_owned()),
            first_seen_ms: 0,
            title_from: tidewatch_core::link_board::TitleFrom::default(),
            state: tidewatch_core::link_board::LinkState::default(),
            status: tidewatch_core::link_board::Status::default(),
        };
        for width in [260.0, 420.0, 700.0] {
            let ask = Ask { read: vec![link.clone()], ..ask_of(ASKING, 0) };
            let harness = Harness::new(ask, width);
            let [card] = harness.strokes[..] else { panic!("one outline at {width}") };
            let label = harness.rect_of("Read first:");
            let title = harness.rect_of("Split options compared");
            assert!(card.contains_rect(label) && card.contains_rect(title), "{width}");
            assert!(title.left() > label.right());
            assert!((title.center().y - label.center().y).abs() < 2.0);
            assert!(label.top() > harness.rect_of("NEEDS YOU").bottom());
        }
    }
}
