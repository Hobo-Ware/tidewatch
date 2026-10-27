//! Ready answers on a Needs you card: the chip model, its layout and the digit keys 1 to 4.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, CursorIcon, Galley, Id, Key, Modifiers, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};
use tidewatch_core::progress::Choice;

use crate::brand;
use crate::screen_kit::{Size, TEXT_TAG, body};

pub(crate) const GAP: f32 = 8.0;
const RADIUS: u8 = Size::Compact.radius();
const PAD_X: f32 = Size::Compact.pad();
const CHIP_H: f32 = Size::Compact.height();
const NUMBER_GAP: f32 = 7.0;
const LABEL_FONT: f32 = Size::Compact.font();
const NUMBER_FONT: f32 = TEXT_TAG;
const DASH: f32 = 4.0;
const DASH_GAP: f32 = 3.0;
pub(crate) const KEYED: usize = 4;
const FALLBACK: [&str; 2] = ["Done", "Not now"];
pub(crate) const OTHER: &str = "Something else";
const NUMBER_KEYS: [Key; KEYED] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Recommended,
    Plain,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    pub(crate) label: String,
    pub(crate) tone: Tone,
    pub(crate) number: Option<usize>,
    /// Says `(recommended)` after the label, on a card that names the agent's pick in words.
    pub(crate) marked: bool,
}

impl Chip {
    pub(crate) fn shown(&self) -> String {
        if self.marked { format!("{} (recommended)", self.label) } else { self.label.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Answer(String),
    Other,
}

pub(crate) fn chips(options: &[Choice]) -> Vec<Chip> {
    build(options, false)
}

/// The chips of a card that writes `(recommended)` after the agent's pick.
pub(crate) fn chips_marked(options: &[Choice]) -> Vec<Chip> {
    build(options, true)
}

fn build(options: &[Choice], mark: bool) -> Vec<Chip> {
    let answers: Vec<(String, bool)> = if options.is_empty() {
        FALLBACK.iter().map(|label| ((*label).to_owned(), false)).collect()
    } else {
        options.iter().map(|choice| (choice.text.clone(), choice.recommended)).collect()
    };
    let mut chips: Vec<Chip> = answers
        .into_iter()
        .enumerate()
        .map(|(index, (label, recommended))| Chip {
            label,
            tone: if recommended { Tone::Recommended } else { Tone::Plain },
            number: (index < KEYED).then_some(index + 1),
            marked: mark && recommended,
        })
        .collect();
    chips.push(Chip { label: OTHER.to_owned(), tone: Tone::Other, number: None, marked: false });
    chips
}

/// The agent's why line as shown: its first letter in capitals, the rest as written.
pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.trim().chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

pub(crate) fn chip_for_number(chips: &[Chip], number: usize) -> Option<usize> {
    chips.iter().position(|chip| chip.number == Some(number))
}

pub(crate) fn pick_of(chips: &[Chip], index: usize) -> Option<Pick> {
    let chip = chips.get(index)?;
    Some(match chip.tone {
        Tone::Other => Pick::Other,
        Tone::Recommended | Tone::Plain => Pick::Answer(chip.label.clone()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Holder {
    Typing,
    Card,
    Other,
    Nobody,
}

/// Whether the digit keys belong to the card: never while something is typed into or another widget
/// holds the keyboard, always after the card was clicked, and otherwise while the pointer is over it.
pub(crate) fn keys_free(holder: Holder, hovered: bool) -> bool {
    match holder {
        Holder::Typing | Holder::Other => false,
        Holder::Card => true,
        Holder::Nobody => hovered,
    }
}

pub(crate) struct Card {
    pub(crate) id: Id,
    pub(crate) rect: Rect,
    pub(crate) terminal_hidden: bool,
}

/// The chip a digit key picks, consuming the key; `None` when the keys are not the card's.
pub(crate) fn key_pick(ui: &Ui, card: &Card, chips: &[Chip]) -> Option<usize> {
    let ctx = ui.ctx();
    let held = ctx.memory(egui::Memory::focused);
    let hidden_grid = |id: Id| card.terminal_hidden && stdusk_egui::focus::is_grid(ctx, id);
    let holder = match held {
        _ if ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) => Holder::Typing,
        Some(id) if id == card.id => Holder::Card,
        Some(id) if !hidden_grid(id) => Holder::Other,
        _ => Holder::Nobody,
    };
    let hovered = ctx.pointer_hover_pos().is_some_and(|at| card.rect.contains(at));
    if !keys_free(holder, hovered) || ui.input(|i| i.modifiers.any() || !i.focused) {
        return None;
    }
    NUMBER_KEYS.iter().enumerate().find_map(|(index, key)| {
        let pressed = ui.input_mut(|i| i.consume_key(Modifiers::NONE, *key));
        pressed.then(|| chip_for_number(chips, index + 1)).flatten()
    })
}

struct Laid {
    number: Option<Arc<Galley>>,
    label: Arc<Galley>,
    full: String,
    width: f32,
}

/// The chips laid out in rows of `width`: where each goes and how tall they are together.
pub(crate) struct Row {
    laid: Vec<Laid>,
    places: Vec<(f32, f32)>,
    height: f32,
}

fn ink(tone: Tone) -> Color32 {
    match tone {
        Tone::Recommended => brand::TEAL_TEXT,
        Tone::Plain => brand::TEXT,
        Tone::Other => brand::SOFT,
    }
}

fn line(tone: Tone) -> Color32 {
    match tone {
        Tone::Recommended => brand::TEAL,
        Tone::Plain | Tone::Other => brand::ANSWER_LINE,
    }
}

impl Row {
    pub(crate) fn new(painter: &Painter, chips: &[Chip], width: f32) -> Self {
        let laid: Vec<Laid> = chips
            .iter()
            .map(|chip| {
                let number = chip.number.map(|number| {
                    painter.layout_no_wrap(number.to_string(), body(NUMBER_FONT), brand::MUTED)
                });
                let lead = number.as_ref().map_or(0.0, |galley| galley.size().x + NUMBER_GAP);
                let room = (width - 2.0 * PAD_X - lead).max(20.0);
                let label = crate::reader_view::ellipsized(
                    painter,
                    &chip.shown(),
                    body(LABEL_FONT),
                    ink(chip.tone),
                    room,
                );
                let chip_w = 2.0 * PAD_X + lead + label.size().x;
                Laid { number, label, full: chip.shown(), width: chip_w }
            })
            .collect();
        let widths: Vec<f32> = laid.iter().map(|chip| chip.width).collect();
        let (places, height) = crate::progress_view::flow(&widths, width, CHIP_H, GAP);
        Self { laid, places, height }
    }

    pub(crate) fn height(&self) -> f32 {
        self.height
    }

    pub(crate) fn rects(&self, origin: Pos2) -> Vec<Rect> {
        self.laid
            .iter()
            .zip(&self.places)
            .map(|(chip, (x, y))| {
                Rect::from_min_size(origin + vec2(*x, *y), vec2(chip.width, CHIP_H))
            })
            .collect()
    }

    /// Draws the chips with their top left at `origin` and returns the one clicked.
    pub(crate) fn show(
        &self,
        ui: &mut Ui,
        salt: Id,
        origin: Pos2,
        chips: &[Chip],
    ) -> Option<usize> {
        let mut clicked = None;
        for (index, (rect, chip)) in self.rects(origin).into_iter().zip(chips).enumerate() {
            let laid = &self.laid[index];
            let response = ui.interact(rect, salt.with(("answer-chip", index)), Sense::CLICK);
            let hovered = response.hovered();
            paint_chip(ui.painter(), rect, laid, chip.tone, hovered);
            let response = response.on_hover_cursor(CursorIcon::PointingHand);
            if laid.label.text() != laid.full {
                response.clone().on_hover_text(&laid.full);
            }
            if response.clicked() {
                clicked = Some(index);
            }
        }
        clicked
    }
}

fn paint_chip(painter: &Painter, rect: Rect, laid: &Laid, tone: Tone, hovered: bool) {
    let edge = line(tone);
    let edge = if hovered { edge.lerp_to_gamma(ink(tone), 0.6) } else { edge };
    let stroke = Stroke::new(1.0, edge);
    if tone == Tone::Other {
        let path = rounded_path(rect.shrink(0.5), f32::from(RADIUS) - 0.5);
        painter.extend(Shape::dashed_line(&path, stroke, DASH, DASH_GAP));
    } else {
        painter.rect_stroke(rect, CornerRadius::same(RADIUS), stroke, StrokeKind::Inside);
    }
    let mut x = rect.left() + PAD_X;
    if let Some(number) = &laid.number {
        let at = pos2(x, rect.center().y - number.size().y / 2.0);
        x += number.size().x + NUMBER_GAP;
        painter.galley(at, number.clone(), brand::MUTED);
    }
    let at = pos2(x, rect.center().y - laid.label.size().y / 2.0);
    painter.galley(at, laid.label.clone(), ink(tone));
}

/// A closed outline of `rect` with corners of radius `r`, as points to dash along.
fn rounded_path(rect: Rect, r: f32) -> Vec<Pos2> {
    const STEPS: usize = 6;
    let corners = [
        (rect.right_top() + vec2(-r, r), -90.0_f32),
        (rect.right_bottom() + vec2(-r, -r), 0.0),
        (rect.left_bottom() + vec2(r, -r), 90.0),
        (rect.left_top() + vec2(r, r), 180.0),
    ];
    let mut path = Vec::with_capacity(4 * (STEPS + 1) + 1);
    for (center, start) in corners {
        for step in 0..=STEPS {
            let angle = (start + 90.0 * step as f32 / STEPS as f32).to_radians();
            path.push(center + vec2(angle.cos(), angle.sin()) * r);
        }
    }
    path.push(path[0]);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(text: &str, recommended: bool) -> Choice {
        Choice { text: text.to_owned(), recommended }
    }

    fn summary(chips: &[Chip]) -> Vec<(&str, Tone, Option<usize>)> {
        chips.iter().map(|chip| (chip.label.as_str(), chip.tone, chip.number)).collect()
    }

    #[test]
    fn the_agents_options_become_numbered_chips_with_the_starred_one_marked_and_something_else_last()
     {
        let made = chips(&[
            choice("Approve", true),
            choice("Allow the bot", false),
            choice("Wait", false),
        ]);
        assert_eq!(
            summary(&made),
            [
                ("Approve", Tone::Recommended, Some(1)),
                ("Allow the bot", Tone::Plain, Some(2)),
                ("Wait", Tone::Plain, Some(3)),
                ("Something else", Tone::Other, None),
            ]
        );
    }

    #[test]
    fn a_why_line_starts_with_a_capital_and_is_otherwise_untouched() {
        let cases = [
            ("the thread is quiet", "The thread is quiet"),
            ("  padded", "Padded"),
            ("Already fine", "Already fine"),
            ("\u{e9}t\u{e9} done", "\u{c9}t\u{e9} done"),
            ("`code` first", "`code` first"),
            ("7 files changed", "7 files changed"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(sentence(input), want, "{input:?}");
        }
    }

    #[test]
    fn no_options_gives_done_and_not_now_before_something_else() {
        assert_eq!(
            summary(&chips(&[])),
            [
                ("Done", Tone::Plain, Some(1)),
                ("Not now", Tone::Plain, Some(2)),
                ("Something else", Tone::Other, None),
            ]
        );
    }

    #[test]
    fn only_the_first_four_options_take_a_number() {
        let options: Vec<Choice> = (1..=6).map(|n| choice(&format!("o{n}"), false)).collect();
        let numbers: Vec<Option<usize>> = chips(&options).iter().map(|chip| chip.number).collect();
        assert_eq!(
            numbers,
            [Some(1), Some(2), Some(3), Some(4), None, None, None],
            "six options and Something else"
        );
    }

    #[test]
    fn a_digit_maps_to_the_chip_that_carries_it_and_nothing_else() {
        let made = chips(&[choice("A", false), choice("B", true)]);
        let cases = [(1, Some(0)), (2, Some(1)), (3, None), (4, None), (0, None), (5, None)];
        for (number, want) in cases {
            assert_eq!(chip_for_number(&made, number), want, "key {number}");
        }
    }

    #[test]
    fn a_chip_picks_its_exact_text_and_something_else_picks_the_reply_box() {
        let made = chips(&[choice("Post it", false), choice("Skip it, for now", true)]);
        assert_eq!(pick_of(&made, 0), Some(Pick::Answer("Post it".to_owned())));
        assert_eq!(pick_of(&made, 1), Some(Pick::Answer("Skip it, for now".to_owned())));
        assert_eq!(pick_of(&made, 2), Some(Pick::Other));
        assert_eq!(pick_of(&made, 3), None);
    }

    #[test]
    fn the_digits_belong_to_the_card_only_when_nobody_else_is_typing_or_holding_the_keys() {
        let cases = [
            ("hovered with nothing held", Holder::Nobody, true, true),
            ("not hovered, nothing held", Holder::Nobody, false, false),
            ("hovered while the terminal holds the keys", Holder::Other, true, false),
            ("clicked card, pointer away", Holder::Card, false, true),
            ("typing in the reply box, card clicked before", Holder::Typing, true, false),
        ];
        for (name, holder, hovered, want) in cases {
            assert_eq!(keys_free(holder, hovered), want, "{name}");
        }
    }
}
