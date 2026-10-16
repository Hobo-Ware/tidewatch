//! The "Your review" card that sits directly above an agent tab's chat box: the draft review's notes
//! grouped by file, an optional message and the one place a review is sent from. Discard takes two
//! clicks. The card only draws; sending, holding and discarding are `ReviewAction`s the app carries
//! out.

use egui::text::LayoutJob;
use egui::{
    Align, Id, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, ScrollArea, Sense,
    TextEdit, TextFormat, Ui, UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::commands::Platform;
use crate::reader_view::{INSET, ellipsized, field_frame, paint_item};
use crate::review::{Draft, DraftNote, ReviewFacts, SendState};
use crate::screen_kit::{body, danger_button, heading, mono, primary_button, quiet_button};

const PAD_X: f32 = 14.0;
const PAD_Y: f32 = 10.0;
const HEAD_H: f32 = 24.0;
const FILE_H: f32 = 22.0;
const ROW_H: f32 = 20.0;
const LIST_MAX_H: f32 = 124.0;
const FIELD_H: f32 = 36.0;
const HINT_H: f32 = 18.0;
const GAP: f32 = 8.0;
const LINE_W: f32 = 46.0;
const LINE_GAP: f32 = 8.0;
const MIN_ROOM: f32 = 150.0;
const BOTTOM_GAP: f32 = 6.0;
const FONT: f32 = 13.0;
const SMALL: f32 = 12.0;

/// What the user asked for on the card, or on the review card in the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReviewAction {
    Send(String),
    CancelHeld,
    Discard(Vec<u64>),
    ResolveAnswered(Vec<u64>),
    OpenFile(String),
}

/// What the card keeps between frames.
#[derive(Debug, Default)]
pub(crate) struct CardState {
    pub(crate) message: String,
}

fn list_height(draft: &Draft) -> f32 {
    let natural: f32 =
        draft.files.iter().map(|file| FILE_H + file.notes.len() as f32 * ROW_H).sum();
    natural.min(LIST_MAX_H)
}

/// The room the card takes above the chat box, gap included, out of `room`: nothing for no notes or
/// when the card would not fit.
pub(crate) fn height(draft: &Draft, room: f32) -> f32 {
    if draft.is_empty() {
        return 0.0;
    }
    let natural = 2.0 * PAD_Y + HEAD_H + GAP + list_height(draft) + GAP + FIELD_H + 4.0 + HINT_H;
    let shown = natural.min(room);
    if shown < MIN_ROOM.min(natural) { 0.0 } else { shown + BOTTOM_GAP }
}

pub(crate) fn key_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Mac => "Cmd+Enter",
        Platform::Linux => "Ctrl+Enter",
    }
}

pub(crate) fn send_label(profile: &str) -> String {
    format!("Send review to {profile}")
}

fn hint(send: SendState, profile: &str, key: &str) -> Vec<(String, bool)> {
    let say = |text: String| (text, false);
    let keys = |text: &str| (text.to_owned(), true);
    match send {
        SendState::Ready => vec![keys(key), say(format!(" sends it to {profile}"))],
        SendState::WhenIdle => vec![
            say(format!("{profile} is busy: it goes the moment {profile} is idle. ")),
            keys(key),
            say(" sends".to_owned()),
        ],
        SendState::Held => {
            vec![say(format!("Held: it goes the moment {profile} is idle."))]
        }
        SendState::Refused(reason) => vec![say(reason.to_owned())],
    }
}

fn hint_job(parts: &[(String, bool)], ink: egui::Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (text, is_key) in parts {
        let (font, color) = if *is_key { (mono(SMALL), brand::SOFT) } else { (body(SMALL), ink) };
        job.append(text, 0.0, TextFormat { font_id: font, color, ..TextFormat::default() });
    }
    job
}

/// Draws the card in `rect` (what [`height`] reserved, gap included) and returns what was asked.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut CardState,
    facts: &ReviewFacts<'_>,
) -> Vec<ReviewAction> {
    let mut actions = Vec::new();
    let card = Rect::from_min_max(
        pos2(rect.left() + INSET, rect.top()),
        pos2(rect.right() - INSET, rect.bottom() - BOTTOM_GAP),
    );
    paint_item(ui.painter(), card, false);
    let inner = card.shrink2(vec2(PAD_X, PAD_Y));
    let mut y = inner.top();
    let section = |top: f32, height: f32| {
        Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), height))
    };

    let head = section(y, HEAD_H);
    y += HEAD_H + GAP;
    header(ui, head, facts.draft, &mut actions);

    let list = section(y, list_height(facts.draft));
    y += list.height() + GAP;
    notes_list(ui, list, facts.draft);

    let field = section(y, FIELD_H);
    y += FIELD_H + 4.0;
    let held = matches!(facts.send, SendState::Held);
    let asked = field_row(ui, field, state, facts, held);
    let hint_rect = section(y, HINT_H);

    let sendable = matches!(facts.send, SendState::Ready | SendState::WhenIdle);
    let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
    let keyed = facts.keys_free && sendable && ui.input_mut(|i| i.consume_shortcut(&shortcut));
    if held && asked {
        actions.push(ReviewAction::CancelHeld);
    } else if sendable && (asked || keyed) {
        actions.push(ReviewAction::Send(std::mem::take(&mut state.message)));
    }
    let ink =
        if matches!(facts.send, SendState::Refused(_)) { brand::ORANGE } else { brand::FAINT };
    let parts = hint(facts.send, facts.profile, key_name(Platform::CURRENT));
    let galley = ui.painter().layout_job(hint_job(&parts, ink));
    ui.painter().galley(
        pos2(hint_rect.left(), hint_rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    actions
}

fn header(ui: &mut Ui, rect: Rect, draft: &Draft, actions: &mut Vec<ReviewAction>) {
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    if discard(&mut child) {
        actions.push(ReviewAction::Discard(draft.ids()));
    }
    let left = Rect::from_min_max(rect.min, pos2(child.min_rect().left() - GAP, rect.bottom()));
    let painter = ui.painter();
    let title = painter.layout_no_wrap("Your review".to_owned(), heading(14.0), brand::TEXT);
    let summary = painter.layout_no_wrap(draft.summary(), body(FONT - 0.5), brand::FAINT);
    let title_at = pos2(left.left(), left.center().y - title.size().y / 2.0);
    let summary_at =
        pos2(title_at.x + title.size().x + 10.0, left.center().y - summary.size().y / 2.0);
    painter.galley(title_at, title, brand::TEXT);
    painter.galley(summary_at, summary, brand::FAINT);
}

/// The Discard button: the first click arms it, the second discards, a click elsewhere disarms.
fn discard(ui: &mut Ui) -> bool {
    let id = Id::new("review-discard");
    let armed = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let response = if armed {
        danger_button(ui, "Click again to discard")
    } else {
        quiet_button(ui, "Discard", true).on_hover_text("Delete the notes of this review")
    };
    let clicked = response.clicked();
    let next = if clicked { !armed } else { armed && !response.clicked_elsewhere() };
    ui.data_mut(|data| data.insert_temp(id, next));
    clicked && armed
}

fn notes_list(ui: &mut Ui, rect: Rect, draft: &Draft) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    ScrollArea::vertical().id_salt("review-notes").auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for file in &draft.files {
                file_line(ui, &file.path);
                for note in &file.notes {
                    note_line(ui, note);
                }
            }
        },
    );
}

fn file_line(ui: &mut Ui, path: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_H), Sense::hover());
    let galley = ellipsized(ui.painter(), path, mono(SMALL), brand::MUTED, rect.width());
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        brand::MUTED,
    );
}

fn note_line(ui: &mut Ui, note: &DraftNote) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let painter = ui.painter();
    let at = |x: f32, height: f32| pos2(x, rect.center().y - height / 2.0);
    let place = painter.layout_no_wrap(note.lines.clone(), mono(SMALL), brand::FAINT);
    let mut x = rect.left() + LINE_W.max(place.size().x + LINE_GAP);
    painter.galley(at(rect.left(), place.size().y), place, brand::FAINT);
    if let Some(reviewer) = &note.reviewer {
        let name = painter.layout_no_wrap(reviewer.clone(), body(SMALL), brand::TEAL_TEXT);
        let width = name.size().x;
        painter.galley(at(x, name.size().y), name, brand::TEAL_TEXT);
        x += width + 8.0;
    }
    let text = ellipsized(painter, &note.text, body(FONT), brand::SOFT, rect.right() - x);
    painter.galley(at(x, text.size().y), text, brand::SOFT);
}

/// The message field with the send button. Returns whether the button was clicked.
fn field_row(
    ui: &mut Ui,
    rect: Rect,
    state: &mut CardState,
    facts: &ReviewFacts<'_>,
    held: bool,
) -> bool {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    let mut clicked = false;
    field_frame(&mut child, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            clicked = if held {
                quiet_button(ui, "Take it back", true).clicked()
            } else {
                let enabled = matches!(facts.send, SendState::Ready | SendState::WhenIdle);
                primary_button(ui, &send_label(facts.profile), enabled).clicked()
            };
            ui.add(
                TextEdit::singleline(&mut state.message)
                    .id_salt("review-message")
                    .interactive(!held)
                    .frame(egui::Frame::NONE)
                    .hint_text(
                        RichText::new("Add a message to the review (optional)").color(brand::FAINT),
                    )
                    .desired_width(f32::INFINITY)
                    .font(body(14.0))
                    .text_color(brand::TEXT),
            )
        })
        .inner
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};
    use kelp_core::review::{Anchor, Comment, Side};
    use tidewatch_core::notes::Notes;

    fn notes() -> Notes {
        let mut notes = Notes::default();
        let anchor = |line| Anchor {
            side: Side::New,
            line_hint: line,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        };
        let comment = |body: &str| Comment { author: "vlad".into(), time: 1, body: body.into() };
        notes.add("src/runner.rs", None, anchor(211), comment("Keep the count on the stage"));
        notes.add("src/runner.rs", None, anchor(240), comment("This branch never logs"));
        notes.add("src/runner/tests.rs", None, anchor(88), comment("Add the passing case"));
        notes
    }

    fn draft() -> Draft {
        Draft::of(&notes(), "claude", "vlad")
    }

    #[test]
    fn no_notes_take_no_room_and_a_cramped_column_hides_the_card() {
        assert_eq!(height(&Draft::default(), 1000.0), 0.0);
        assert_eq!(height(&draft(), 60.0), 0.0);
        let roomy = height(&draft(), 1000.0);
        assert!(roomy > 150.0 && roomy < 320.0, "{roomy}");
        assert_eq!(height(&draft(), 1000.0), height(&draft(), 600.0));
        assert!(height(&draft(), 200.0) <= 200.0 + BOTTOM_GAP);
    }

    #[test]
    fn a_long_review_scrolls_inside_instead_of_growing() {
        let mut many = Notes::default();
        for n in 0..40 {
            let anchor = Anchor {
                side: Side::New,
                line_hint: n,
                text: "x".into(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            };
            many.add(
                "a.rs",
                None,
                anchor,
                Comment { author: "v".into(), time: 1, body: "n".into() },
            );
        }
        let long = Draft::of(&many, "claude", "v");
        assert_eq!(list_height(&long), LIST_MAX_H);
        assert!(height(&long, 2000.0) < 330.0);
    }

    #[test]
    fn the_hint_names_the_key_and_the_agent_per_state() {
        let text = |send| {
            hint(send, "Builder", "Cmd+Enter").into_iter().map(|(t, _)| t).collect::<String>()
        };
        assert_eq!(text(SendState::Ready), "Cmd+Enter sends it to Builder");
        assert_eq!(
            text(SendState::WhenIdle),
            "Builder is busy: it goes the moment Builder is idle. Cmd+Enter sends"
        );
        assert_eq!(text(SendState::Held), "Held: it goes the moment Builder is idle.");
        assert_eq!(text(SendState::Refused("no agent")), "no agent");
        assert_eq!(key_name(Platform::Linux), "Ctrl+Enter");
        assert_eq!(send_label("Builder"), "Send review to Builder");
    }

    struct Bench {
        ctx: egui::Context,
        state: CardState,
        send: SendState,
        draft: Draft,
    }

    impl Bench {
        fn new(send: SendState) -> Self {
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            let mut bench = Self { ctx, state: CardState::default(), send, draft: draft() };
            bench.frame(vec![]);
            bench.frame(vec![]);
            bench
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<ReviewAction> {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 420.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut actions = Vec::new();
            let notes = notes();
            let draft = self.draft.clone();
            let send = self.send;
            let state = &mut self.state;
            let _ = self.ctx.run_ui(raw, |ui| {
                let facts = ReviewFacts {
                    draft: &draft,
                    send,
                    notes: &notes,
                    agent_id: "claude",
                    profile: "Builder",
                    working: false,
                    keys_free: true,
                };
                let rect = Rect::from_min_size(pos2(0.0, 20.0), vec2(700.0, height(&draft, 380.0)));
                actions = show(ui, rect, state, &facts);
            });
            actions
        }

        fn click(&mut self, at: egui::Pos2) -> Vec<ReviewAction> {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            let actions = self.frame(vec![button(false)]);
            self.frame(vec![]);
            actions
        }

        /// Clicks along a row until `wanted` shows up, so a test does not hard-code button places.
        fn click_along(&mut self, y: f32, wanted: &ReviewAction) -> bool {
            (0..700).step_by(8).any(|x| self.click(pos2(x as f32, y)).contains(wanted))
        }

        fn rows(&self) -> (f32, f32) {
            let total = height(&self.draft, 380.0);
            let card_top = 20.0;
            let header_y = card_top + PAD_Y + HEAD_H / 2.0;
            let field_y = card_top + total - BOTTOM_GAP - PAD_Y - HINT_H - 4.0 - FIELD_H / 2.0;
            (header_y, field_y)
        }
    }

    #[test]
    fn the_send_button_sends_the_typed_message_once_and_clears_it() {
        let mut bench = Bench::new(SendState::Ready);
        bench.state.message = "Start with runner.rs".into();
        let (_, field_y) = bench.rows();
        let wanted = ReviewAction::Send("Start with runner.rs".into());
        assert!(bench.click_along(field_y, &wanted));
        assert_eq!(bench.state.message, "");
    }

    #[test]
    fn command_enter_sends_when_the_card_is_ready_or_waits_and_not_when_held_or_refused() {
        let cmd_enter = || Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        let cases = [
            (SendState::Ready, true),
            (SendState::WhenIdle, true),
            (SendState::Held, false),
            (SendState::Refused("no"), false),
        ];
        for (send, sends) in cases {
            let mut bench = Bench::new(send);
            bench.state.message = "go".into();
            let actions = bench.frame(vec![cmd_enter()]);
            let want = if sends { vec![ReviewAction::Send("go".into())] } else { vec![] };
            assert_eq!(actions, want, "{send:?}");
        }
    }

    #[test]
    fn plain_enter_does_not_send_the_review() {
        let mut bench = Bench::new(SendState::Ready);
        let enter = Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(bench.frame(vec![enter]), vec![]);
    }

    #[test]
    fn discard_needs_a_second_click_and_a_click_elsewhere_disarms_it() {
        let mut bench = Bench::new(SendState::Ready);
        let (header_y, _) = bench.rows();
        let ids = bench.draft.ids();
        let right = pos2(700.0 - INSET - PAD_X - 20.0, header_y);
        assert_eq!(bench.click(right), vec![]);
        assert_eq!(bench.click(right), vec![ReviewAction::Discard(ids.clone())]);
        assert_eq!(bench.click(right), vec![]);
        bench.click(pos2(40.0, 400.0));
        assert_eq!(bench.click(right), vec![]);
    }

    #[test]
    fn a_held_review_offers_to_take_it_back_instead_of_sending() {
        let mut bench = Bench::new(SendState::Held);
        let (_, field_y) = bench.rows();
        assert!(bench.click_along(field_y, &ReviewAction::CancelHeld));
    }

    #[test]
    fn a_refused_review_has_a_disabled_button() {
        let mut bench = Bench::new(SendState::Refused("There is no running agent in this tab"));
        let (_, field_y) = bench.rows();
        let any_send = (0..700)
            .step_by(8)
            .flat_map(|x| bench.click(pos2(x as f32, field_y)))
            .any(|action| matches!(action, ReviewAction::Send(_)));
        assert!(!any_send);
    }
}
