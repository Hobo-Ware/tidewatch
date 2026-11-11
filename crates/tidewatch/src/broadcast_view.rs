//! Draws the Message all agents sheet in the palette's card: a header with how many agents are
//! ticked, the shared composer, one tick row per agent tab (avatar, name, live state word) and a
//! footer with the key hints and the Send to N agents button. Enter and Cmd+Enter send as in the
//! reply box, Shift+Enter is a new line, Cmd+Shift+Enter sends from anywhere, Esc or a click
//! outside closes.

use egui::{
    Align2, Area, CornerRadius, Frame, Id, Key, Margin, Modal, Modifiers, Order, Rect, ScrollArea,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::ui::icons;

use crate::agent_avatar::{self, Face};
use crate::brand;
use crate::broadcast::{Live, Row, Sheet, send_label};
use crate::code_block::paint_check;
use crate::commands::Platform;
use crate::composer::{self, Draft, Mode, Spec};
use crate::palette_view::{HEADER_H, SIDE, TOP_GAP, WIDTH, card_frame};
use crate::review::plural;
use crate::screen_kit::{
    FACE, Size, TEXT_BODY, TEXT_TAG, TEXT_TITLE, body, footer_row, heading, primary_button_keyed,
    selection_row,
};

const ROW_H: f32 = 36.0;
const LIST_PAD: f32 = 6.0;
const LIST_MAX: f32 = 6.0 * ROW_H;
const BOX_CAP: f32 = 160.0;
const TICK: f32 = 16.0;
const ROW_PAD: f32 = 10.0;
const TICK_GAP: f32 = 12.0;
const FOOTER_PAD_Y: f32 = 12.0;
const HINTS: [(&str, &str); 3] = [("Enter", "send"), ("Shift+Enter", "new line"), ("Esc", "close")];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Close,
    Send(String),
}

fn send_chord() -> &'static str {
    match Platform::CURRENT {
        Platform::Mac => "\u{2318}\u{21e7}\u{21b5}",
        Platform::Linux => "Ctrl Shift \u{21b5}",
    }
}

pub(crate) fn composer_id() -> Id {
    Id::new("broadcast_composer")
}

/// `faces` is indexed like `rows`.
pub(crate) fn show(
    ctx: &egui::Context,
    sheet: &mut Sheet,
    rows: &[Row],
    faces: &[Option<Face>],
) -> Option<Outcome> {
    let chord =
        ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter));
    let mut pressed = chord;
    let mut sent = None;
    let mut flipped = None;
    let area = Area::new(Id::new("broadcast_sheet_area"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, TOP_GAP))
        .order(Order::Foreground);
    let modal =
        Modal::new(Id::new("broadcast_sheet")).area(area).frame(card_frame()).show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            header(ui, rows);
            sent = compose(ui, sheet);
            flipped = list(ui, rows, faces);
            pressed |= footer(ui, sheet, rows);
        });
    if let Some(id) = flipped {
        sheet.toggle(id);
    }
    let wanted = pressed || sent.is_some();
    let text = sent.unwrap_or_else(|| sheet.draft.clone());
    if wanted && !text.trim().is_empty() && rows.iter().any(|row| row.ticked) {
        return Some(Outcome::Send(text));
    }
    if !text.is_empty() {
        sheet.draft = text;
    }
    modal.should_close().then_some(Outcome::Close)
}

fn header(ui: &mut Ui, rows: &[Row]) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, HEADER_H), Sense::hover());
    let mid = rect.center().y;
    let painter = ui.painter();
    painter.text(
        pos2(rect.left() + SIDE + TICK / 2.0, mid),
        Align2::CENTER_CENTER,
        icons::APP_WINDOW,
        egui::FontId::proportional(18.0),
        brand::muted(),
    );
    painter.text(
        pos2(rect.left() + SIDE + TICK + 12.0, mid),
        Align2::LEFT_CENTER,
        "Message all agents",
        heading(TEXT_TITLE),
        brand::text(),
    );
    let ticked = rows.iter().filter(|row| row.ticked).count();
    let count = if rows.is_empty() {
        "no agents".to_owned()
    } else {
        format!("{ticked} of {}", plural(rows.len(), "agent"))
    };
    painter.text(
        pos2(rect.right() - SIDE, mid),
        Align2::RIGHT_CENTER,
        count,
        body(TEXT_TAG),
        brand::faint(),
    );
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::card_line()));
}

fn compose(ui: &mut Ui, sheet: &mut Sheet) -> Option<String> {
    Frame::new()
        .inner_margin(Margin::symmetric(SIDE as i8, SIDE as i8))
        .show(ui, |ui| {
            let width = ui.available_width();
            let height = composer::answer_box_height(ui, &sheet.draft, (width, BOX_CAP));
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
            let draft = Draft { text: &mut sheet.draft, focus: &mut sheet.focus, recall: None };
            let spec = Spec {
                id: composer_id(),
                hint: "Write once, send to every ticked agent",
                enabled: true,
                commands: &[],
                ghost: None,
                mode: Mode::Answer,
                stop: false,
                held: 0,
                files: None,
            };
            composer::show(&mut child, draft, &spec).sent
        })
        .inner
}

fn list(ui: &mut Ui, rows: &[Row], faces: &[Option<Face>]) -> Option<u64> {
    let (line, _) = ui.allocate_exact_size(vec2(WIDTH, 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, brand::card_line()));
    if rows.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, ROW_H * 2.0), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No agent tabs in this repo",
            body(13.0),
            brand::muted(),
        );
        return None;
    }
    let mut flipped = None;
    ui.add_space(LIST_PAD);
    ScrollArea::vertical()
        .id_salt("broadcast_agents")
        .max_height(LIST_MAX)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(
                    vec2(WIDTH - 2.0 * LIST_PAD - brand::SCROLL_ROOM, ROW_H),
                    Sense::hover(),
                );
                let rect = rect.translate(vec2(LIST_PAD, 0.0));
                let response = ui
                    .interact(rect, Id::new(("broadcast_row", row.id)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                paint_row(ui, rect, row, faces.get(index).and_then(Option::as_ref), &response);
                if response.clicked() {
                    flipped = Some(row.id);
                }
            }
        });
    ui.add_space(LIST_PAD);
    flipped
}

fn paint_row(ui: &Ui, rect: Rect, row: &Row, face: Option<&Face>, response: &egui::Response) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, false, response.hovered());
    let mid = rect.center().y;
    let tick =
        Rect::from_center_size(pos2(rect.left() + ROW_PAD + TICK / 2.0, mid), vec2(TICK, TICK));
    paint_tick(&painter, tick, row.ticked);
    let face_at = pos2(tick.right() + TICK_GAP + FACE / 2.0, mid);
    if let Some(face) = face {
        agent_avatar::paint_face(
            ui.ctx(),
            &painter,
            Rect::from_center_size(face_at, vec2(FACE, FACE)),
            face,
        );
    } else {
        painter.text(
            face_at,
            Align2::CENTER_CENTER,
            icons::TERMINAL_WINDOW,
            egui::FontId::proportional(16.0),
            brand::soft(),
        );
    }
    let ink = if row.live == Live::Parked { brand::muted() } else { brand::text() };
    let word = painter.layout_no_wrap(row.live.word().to_owned(), body(TEXT_TAG), brand::muted());
    let word_w = word.size().x;
    let right = rect.right() - ROW_PAD;
    painter.galley(pos2(right - word_w, mid - word.size().y / 2.0), word, brand::muted());
    let left = face_at.x + FACE / 2.0 + 10.0;
    let name = crate::reader_view::ellipsized(
        &painter,
        &row.name,
        body(TEXT_BODY),
        ink,
        (right - word_w - 12.0 - left).max(40.0),
    );
    painter.galley(pos2(left, mid - name.size().y / 2.0), name, ink);
}

fn paint_tick(painter: &egui::Painter, rect: Rect, on: bool) {
    let radius = CornerRadius::same(4);
    if on {
        painter.rect(rect, radius, brand::teal(), Stroke::NONE, StrokeKind::Inside);
        paint_check(painter, rect, brand::ground());
    } else {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, brand::field_line()),
            StrokeKind::Inside,
        );
    }
}

fn footer(ui: &mut Ui, sheet: &Sheet, rows: &[Row]) -> bool {
    let (line, _) = ui.allocate_exact_size(vec2(WIDTH, 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, brand::card_line()));
    let mut clicked = false;
    Frame::new().inner_margin(Margin::symmetric(SIDE as i8, FOOTER_PAD_Y as i8)).show(ui, |ui| {
        footer_row(ui, &HINTS, |ui| {
            let label = send_label(rows);
            let enabled = sheet.can_send(rows);
            clicked =
                primary_button_keyed(ui, &label, send_chord(), enabled, Size::Regular).clicked();
        });
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcast::{Candidate, Kind};
    use crate::test_support::painted_text_rects;
    use egui::{Event, PointerButton, RawInput};
    use tidewatch_core::status::AgentStatus;

    fn candidate(id: u64, status: AgentStatus, parked: bool) -> Candidate {
        Candidate { id, name: format!("tab-{id}"), kind: Kind::Agent, status, parked }
    }

    struct Harness {
        ctx: egui::Context,
        sheet: Sheet,
        candidates: Vec<Candidate>,
        texts: Vec<(String, Rect)>,
    }

    fn chord(pressed: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key: pressed,
            physical_key: Some(pressed),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    impl Harness {
        fn new(draft: &str) -> Self {
            let mut harness = Self {
                ctx: crate::fonts::test_context(),
                sheet: Sheet::open(draft.to_owned()),
                candidates: vec![
                    candidate(1, AgentStatus::Working, false),
                    candidate(2, AgentStatus::Done, true),
                    candidate(3, AgentStatus::Done, false),
                ],
                texts: Vec::new(),
            };
            harness.frame(Vec::new());
            harness.frame(Vec::new());
            harness
        }

        fn rows(&self) -> Vec<Row> {
            self.sheet.rows(&self.candidates)
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<Outcome> {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let rows = self.rows();
            let faces = vec![None; rows.len()];
            let mut outcome = None;
            let sheet = &mut self.sheet;
            let output = self.ctx.run_ui(raw, |ui| outcome = show(ui.ctx(), sheet, &rows, &faces));
            self.texts.clear();
            for clipped in &output.shapes {
                painted_text_rects(&clipped.shape, &mut self.texts);
            }
            outcome
        }

        fn typed(&mut self, text: &str) {
            self.frame(vec![Event::Text(text.to_owned())]);
            self.frame(Vec::new());
        }

        fn text(&self, wanted: &str) -> Rect {
            self.texts
                .iter()
                .find(|(text, _)| text == wanted)
                .map_or_else(|| panic!("{wanted:?} in {:?}", self.texts), |(_, rect)| *rect)
        }

        fn click(&mut self, at: egui::Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
        }
    }

    #[test]
    fn the_sheet_lists_each_agent_with_its_word_and_counts_the_ticked_ones_on_the_button() {
        let harness = Harness::new("rebase");
        for wanted in [
            "Message all agents",
            "tab-1",
            "working",
            "tab-2",
            "parked",
            "tab-3",
            "idle",
            "2 of 3 agents",
            "Send to 2 agents",
        ] {
            harness.text(wanted);
        }
    }

    #[test]
    fn a_click_on_a_row_flips_its_tick_and_the_button_follows() {
        let mut harness = Harness::new("rebase");
        let parked = harness.text("tab-2").center();
        harness.click(parked);
        let ticked: Vec<bool> = harness.rows().iter().map(|row| row.ticked).collect();
        assert_eq!(ticked, [true, true, true]);
        harness.frame(Vec::new());
        harness.text("Send to 3 agents");
    }

    #[test]
    fn enter_sends_the_typed_text_as_it_does_in_the_reply_box() {
        let mut harness = Harness::new("");
        harness.typed("rebase on main");
        let sent = harness.frame(vec![chord(Key::Enter, Modifiers::NONE)]);
        assert_eq!(sent, Some(Outcome::Send("rebase on main".into())));
    }

    #[test]
    fn shift_enter_adds_a_line_and_sends_nothing() {
        let mut harness = Harness::new("");
        harness.typed("one");
        assert_eq!(harness.frame(vec![chord(Key::Enter, Modifiers::SHIFT)]), None);
        harness.frame(Vec::new());
        assert_eq!(harness.sheet.draft, "one\n");
    }

    #[test]
    fn cmd_shift_enter_sends_from_the_sheet() {
        let mut harness = Harness::new("");
        harness.typed("run the tests");
        let sent = harness.frame(vec![chord(Key::Enter, Modifiers::COMMAND | Modifiers::SHIFT)]);
        assert_eq!(sent, Some(Outcome::Send("run the tests".into())));
    }

    #[test]
    fn nothing_ticked_or_nothing_typed_sends_nothing_and_keeps_the_text() {
        let mut harness = Harness::new("");
        assert_eq!(harness.frame(vec![chord(Key::Enter, Modifiers::NONE)]), None);
        harness.typed("go");
        for id in [1, 3] {
            harness.sheet.toggle(id);
        }
        assert_eq!(harness.frame(vec![chord(Key::Enter, Modifiers::NONE)]), None);
        harness.frame(Vec::new());
        assert_eq!(harness.sheet.draft, "go");
        let chord_sent =
            harness.frame(vec![chord(Key::Enter, Modifiers::COMMAND | Modifiers::SHIFT)]);
        assert_eq!(chord_sent, None);
        harness.text("Send to 0 agents");
    }

    #[test]
    fn escape_closes_and_the_draft_stays_for_the_next_open() {
        let mut harness = Harness::new("half a thought");
        assert_eq!(harness.frame(vec![chord(Key::Escape, Modifiers::NONE)]), Some(Outcome::Close));
        assert_eq!(harness.sheet.draft, "half a thought");
    }
}
