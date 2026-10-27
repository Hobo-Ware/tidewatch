//! The row above the History graph: the commit search field (magnifier, the typed text, the place
//! among the matches) with the This branch / Whole repo menu at its right end. The field, the
//! count and the menu chip keep their size whatever is typed or found.

use std::time::Instant;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, CornerRadius, Id, Key, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, TextEdit,
    Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::history::Scope;

use crate::brand;
use crate::commit_search::{self, Focus};
use crate::history_pane::CARD_PAD;
use crate::screen_kit::Size;
use crate::screen_kit::body;
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};
use crate::segmented;
use crate::tab_view::TabView;

pub(crate) const HEIGHT: f32 = 44.0;
const SIDE_PAD: f32 = 10.0;
const FIELD_H: f32 = segmented::HEIGHT;
const ICON_PAD: f32 = 10.0;
const ICON_R: f32 = 4.5;
const TEXT_X: f32 = 30.0;
const COUNT_W: f32 = 74.0;
const COUNT_PAD: f32 = 10.0;
const TEXT_H: f32 = 18.0;
const FONT: f32 = TEXT_SECONDARY;
const COUNT_FONT: f32 = TEXT_TAG;
const CORNER: u8 = 7;
const CHIP_H: f32 = Size::Tag.height();
const CHIP_INSET: f32 = 4.0;
const CHIP_CORNER: u8 = Size::Tag.radius();
const CHIP_PAD: f32 = Size::Tag.pad();
const CHIP_FONT: f32 = TEXT_TAG;
const CHEVRON_W: f32 = 7.0;
const CHEVRON_H: f32 = 4.0;
const CHEVRON_GAP: f32 = 6.0;
const MENU_W: f32 = 120.0;
const HINT: &str = "Search commits";
const SCOPES: [&str; 2] = ["This branch", "Whole repo"];

pub(crate) fn field_id(tab: u64) -> Id {
    Id::new(("commit-search", tab))
}

fn field_rect(row: Rect) -> Rect {
    Rect::from_min_max(
        pos2(row.left() + CARD_PAD, row.center().y - FIELD_H / 2.0),
        pos2(row.right() - SIDE_PAD, row.center().y + FIELD_H / 2.0),
    )
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, view: &mut TabView) {
    show_field(ui, field_rect(rect), view);
}

fn chip_rect(ui: &Ui, field: Rect) -> Rect {
    let widest = SCOPES
        .iter()
        .map(|label| {
            ui.painter().layout_no_wrap((*label).to_owned(), body(CHIP_FONT), brand::MUTED)
        })
        .map(|galley| galley.size().x)
        .fold(0.0, f32::max);
    let width = CHIP_PAD + widest + CHEVRON_GAP + CHEVRON_W + CHIP_PAD;
    Rect::from_center_size(
        pos2(field.right() - CHIP_INSET - width / 2.0, field.center().y),
        vec2(width, CHIP_H),
    )
}

fn show_scope(ui: &mut Ui, chip: Rect, view: &mut TabView) {
    let response = ui
        .interact(chip, ui.id().with("history-scope"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let lit = response.hovered() || open;
    let (ink, line) =
        if lit { (brand::TEXT, brand::FIELD_LINE) } else { (brand::MUTED, brand::SEGMENT_LINE) };
    let active = usize::from(view.scope == Scope::Whole);
    let painter = ui.painter();
    painter.rect_stroke(
        chip,
        CornerRadius::same(CHIP_CORNER),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(chip.left() + CHIP_PAD, chip.center().y),
        Align2::LEFT_CENTER,
        SCOPES[active],
        body(CHIP_FONT),
        ink,
    );
    let tip =
        pos2(chip.right() - CHIP_PAD - CHEVRON_W / 2.0, chip.center().y + CHEVRON_H / 2.0 - 0.5);
    let wing = vec2(CHEVRON_W / 2.0, -CHEVRON_H);
    let stroke = Stroke::new(1.3, ink);
    painter.line_segment([tip, tip + wing], stroke);
    painter.line_segment([tip, tip + vec2(-wing.x, wing.y)], stroke);
    let mut picked = None;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_width(MENU_W);
        for (index, label) in SCOPES.iter().enumerate() {
            let tint = if index == active { brand::TEAL_TEXT } else { brand::TEXT };
            let row = egui::Button::new(RichText::new(*label).size(CHIP_FONT + 1.0).color(tint))
                .frame(false)
                .min_size(vec2(ui.available_width(), 0.0));
            if ui.add(row).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                picked = Some(index);
            }
        }
    });
    if let Some(index) = picked {
        view.set_scope(if index == 1 { Scope::Whole } else { Scope::Current });
    }
}

fn show_field(ui: &mut Ui, field: Rect, view: &mut TabView) {
    let id = field_id(view.id());
    let chip = chip_rect(ui, field);
    let count_right = chip.left() - COUNT_PAD;
    let area = ui.interact(field, id.with("field"), Sense::click());
    let focused = ui.memory(|memory| memory.has_focus(id));
    let step = if focused { enter_step(ui) } else { None };
    let text_rect = Rect::from_min_max(
        pos2(field.left() + TEXT_X, field.center().y - TEXT_H / 2.0),
        pos2((count_right - COUNT_W).max(field.left() + TEXT_X), field.center().y + TEXT_H / 2.0),
    );
    let focus = view.search.want_focus.take();
    if focus.is_some() || area.clicked() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    if focus == Some(Focus::SelectAll) {
        select_all(ui, id, &view.search.query);
    }
    ui.painter().rect_filled(field, CornerRadius::same(CORNER), brand::SEGMENT);
    let mut child = ui.new_child(UiBuilder::new().max_rect(text_rect));
    let response = child.add(
        TextEdit::singleline(&mut view.search.query)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .return_key(None)
            .font(body(FONT))
            .text_color(brand::TEXT)
            .desired_width(text_rect.width()),
    );
    let holds_keyboard = ui.memory(|memory| memory.has_focus(id));
    let line = if holds_keyboard { brand::TEAL } else { brand::FIELD_LINE };
    let painter = ui.painter();
    painter.rect_stroke(
        field,
        CornerRadius::same(CORNER),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    if view.search.query.is_empty() {
        painter.text(
            pos2(text_rect.left(), field.center().y),
            Align2::LEFT_CENTER,
            HINT,
            body(FONT),
            brand::FAINT,
        );
    }
    paint_magnifier(painter, pos2(field.left() + ICON_PAD + ICON_R, field.center().y - 1.0));
    if response.changed() {
        view.search.edited(Instant::now());
    }
    view.search.drew(ui.ctx().cumulative_frame_nr(), holds_keyboard);
    let selected = match view.selection() {
        Some(kelp_ui::graph::Selection::Commit(row)) => Some(row),
        _ => None,
    };
    let count = commit_search::count_text(view.search.matches(), view.search.position(selected));
    let tint = if view.search.matches() == Some(0) { brand::RED } else { brand::MUTED };
    ui.painter().text(
        pos2(count_right, field.center().y),
        Align2::RIGHT_CENTER,
        count,
        body(COUNT_FONT),
        tint,
    );
    show_scope(ui, chip, view);
    if let Some(delta) = step {
        view.step_match(delta);
        response.request_focus();
    }
}

fn enter_step(ui: &Ui) -> Option<isize> {
    ui.input_mut(|input| {
        if input.consume_key(Modifiers::SHIFT, Key::Enter) {
            Some(-1)
        } else if input.consume_key(Modifiers::NONE, Key::Enter) {
            Some(1)
        } else {
            None
        }
    })
}

fn select_all(ui: &Ui, id: Id, text: &str) {
    let mut state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
    let end = CCursor::new(text.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
    state.store(ui.ctx(), id);
}

fn paint_magnifier(painter: &egui::Painter, center: egui::Pos2) {
    let stroke = Stroke::new(1.5, brand::MUTED);
    painter.circle_stroke(center, ICON_R, stroke);
    let from = center + vec2(ICON_R * 0.72, ICON_R * 0.72);
    painter.line_segment([from, from + vec2(3.5, 3.5)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit_search::Ask;
    use egui::{Context, Event, RawInput, Shape};
    use kelp_ui::graph::Selection;
    use std::time::Duration;

    struct Bar {
        ctx: Context,
        view: TabView,
        texts: Vec<(String, Rect)>,
        width: f32,
    }

    fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }

    impl Bar {
        fn new(width: f32) -> Self {
            let mut bar = Self {
                ctx: crate::fonts::test_context(),
                view: TabView::new(1, "/work/web".into(), false, None),
                texts: Vec::new(),
                width,
            };
            bar.frame(Vec::new());
            bar.frame(Vec::new());
            bar
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 200.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (view, width) = (&mut self.view, self.width);
            let output = self.ctx.run_ui(raw, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT));
                show(ui, rect, view);
            });
            self.texts.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut self.texts);
            }
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) {
            let event = |pressed| Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            };
            self.frame(vec![event(true)]);
            self.frame(vec![event(false)]);
        }

        fn answer(&mut self, query: &str, rows: Vec<usize>) {
            self.view.search.query = query.to_owned();
            let now = Instant::now();
            self.view.search.edited(now);
            let Ask::Now { ticket, query } = self.view.search.ask(now + Duration::from_secs(1))
            else {
                panic!("no ask")
            };
            self.view.search.answered(ticket, query, rows, 20);
        }

        fn drawn(&self, text: &str) -> Option<Rect> {
            self.texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| *rect)
        }
    }

    impl Bar {
        fn click(&mut self, at: egui::Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }
    }

    #[test]
    fn the_field_hints_what_it_searches_and_the_scope_chip_sits_inside_its_right_end() {
        let bar = Bar::new(420.0);
        assert!(bar.drawn(HINT).is_some());
        let branch = bar.drawn("This branch").unwrap();
        assert_eq!(bar.drawn("Whole repo"), None);
        assert!(bar.drawn(HINT).unwrap().right() < branch.left());
        assert!(branch.right() < 420.0 - SIDE_PAD);
    }

    #[test]
    fn the_field_lines_up_with_the_header_row_and_is_as_tall_as_its_segment() {
        let field = field_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, HEIGHT)));
        assert_eq!(field.left(), CARD_PAD);
        assert_eq!(field.right(), 420.0 - SIDE_PAD);
        assert_eq!(field.height(), segmented::HEIGHT);
    }

    #[test]
    fn the_chip_opens_a_two_item_menu_and_picking_whole_repo_changes_the_scope() {
        let mut bar = Bar::new(420.0);
        assert_eq!(bar.view.scope, Scope::Current);
        let chip = bar.drawn("This branch").unwrap().center();
        bar.click(chip);
        let whole = bar.drawn("Whole repo").unwrap().center();
        assert_eq!(bar.texts.iter().filter(|(text, _)| text == "This branch").count(), 2);
        bar.click(whole);
        assert_eq!(bar.view.scope, Scope::Whole);
        assert!(bar.drawn("Whole repo").is_some());
        assert_eq!(bar.drawn("This branch"), None);
    }

    #[test]
    fn the_chip_keeps_its_size_and_place_whichever_scope_it_names() {
        let mut bar = Bar::new(420.0);
        let before = bar.drawn("This branch").unwrap();
        bar.view.set_scope(Scope::Whole);
        bar.frame(Vec::new());
        let after = bar.drawn("Whole repo").unwrap();
        assert_eq!(before.left(), after.left());
        assert_eq!(before.center().y, after.center().y);
    }

    #[test]
    fn typing_fills_the_query_and_waits_for_a_pause_before_it_asks() {
        let mut bar = Bar::new(420.0);
        bar.view.search.want_focus = Some(Focus::Caret);
        bar.frame(Vec::new());
        bar.frame(vec![Event::Text("fix".into())]);
        assert_eq!(bar.view.search.query, "fix");
        assert!(matches!(bar.view.search.ask(Instant::now()), Ask::Wait(_)));
        assert!(bar.drawn(HINT).is_none(), "the hint goes once there is text");
    }

    #[test]
    fn enter_selects_the_next_match_and_shift_enter_the_previous_and_the_field_keeps_the_keyboard()
    {
        let mut bar = Bar::new(420.0);
        bar.view.search.want_focus = Some(Focus::Caret);
        bar.frame(Vec::new());
        bar.answer("fix", vec![2, 5, 9]);
        bar.key(Key::Enter, Modifiers::NONE);
        assert_eq!(bar.view.graph.scroll_to, Some(Selection::Commit(2)));
        bar.view.graph.scroll_to = None;
        bar.key(Key::Enter, Modifiers::SHIFT);
        assert_eq!(bar.view.graph.scroll_to, Some(Selection::Commit(9)));
        assert!(bar.ctx.memory(|memory| memory.has_focus(field_id(1))));
    }

    #[test]
    fn enter_with_nothing_found_selects_nothing() {
        let mut bar = Bar::new(420.0);
        bar.view.search.want_focus = Some(Focus::Caret);
        bar.frame(Vec::new());
        bar.answer("zzz", Vec::new());
        bar.key(Key::Enter, Modifiers::NONE);
        assert_eq!(bar.view.graph.scroll_to, None);
        bar.frame(Vec::new());
        assert!(bar.drawn("No matches").is_some());
    }

    #[test]
    fn the_count_sits_at_the_right_of_the_field_whatever_it_says() {
        let mut bar = Bar::new(420.0);
        bar.answer("fix", vec![1, 3]);
        bar.frame(Vec::new());
        let two = bar.drawn("2 matches").unwrap();
        bar.answer("fixes", (0..214).collect());
        bar.frame(Vec::new());
        let many = bar.drawn("214 matches").unwrap();
        assert_eq!(two.right(), many.right());
        let field_start = bar.drawn("This branch").unwrap().left();
        assert!(many.right() < field_start);
    }

    #[test]
    fn the_field_and_the_count_keep_their_size_when_the_text_grows() {
        let mut short = Bar::new(420.0);
        short.answer("a", vec![1]);
        short.frame(Vec::new());
        let mut long = Bar::new(420.0);
        long.answer("a very long search text that fills the whole field and more", vec![1]);
        long.frame(Vec::new());
        assert_eq!(
            short.drawn("1 match").map(|r| r.right()),
            long.drawn("1 match").map(|r| r.right())
        );
        assert_eq!(short.drawn("This branch"), long.drawn("This branch"));
    }
}
