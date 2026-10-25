//! A sent review in the Reader's timeline: one card in the user bubble's look instead of the pasted
//! block. Each note is a small block with its place, the user's text, the agent's reply when
//! `reply_to_note` arrives and where it stands. Open in the file view and Resolve answered sit at
//! the bottom.

use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, Sense, Shape, Stroke, Ui,
    pos2, vec2,
};

use crate::brand;
use crate::reader_chat_view::USER_LINE;
use crate::review::{ReviewFacts, SentNote, SentReview, Standing};
use crate::review_view::ReviewAction;
use crate::screen_kit::{Size, body, heading, mono, quiet_button};

const RADIUS: CornerRadius = CornerRadius { nw: 14, ne: 14, se: 4, sw: 14 };
const NOTE_RADIUS: u8 = 10;
const INDENT: f32 = 26.0;
const BAR_W: f32 = 2.0;
const RING: f32 = 4.0;

fn ink(standing: Standing) -> Color32 {
    match standing {
        Standing::Answered => brand::GREEN,
        Standing::Working => brand::ORANGE,
        Standing::Waiting => brand::FAINT,
        Standing::Resolved => brand::MUTED,
    }
}

/// The place of a note: `src/runner.rs:211`, `src/runner.rs:211-214` for a block, or the bare path
/// for a note on the whole file.
pub(crate) fn place(note: &SentNote) -> String {
    format!("{}{}", note.path, note.lines)
}

/// Draws the card of the review whose notes are `ids`; `message` is what the user wrote with it.
/// Without the tab's review facts only the tag shows.
pub(crate) fn show(
    ui: &mut Ui,
    ids: &[u64],
    message: &str,
    facts: Option<&ReviewFacts<'_>>,
) -> Vec<ReviewAction> {
    let mut actions = Vec::new();
    let review = facts.map_or(SentReview { sent: ids.len(), notes: Vec::new() }, |facts| {
        SentReview::of(ids, facts.notes, facts.agent_id, facts.working)
    });
    let profile = facts.map_or("The agent", |facts| facts.profile);
    Frame::new()
        .fill(brand::TEAL_NAV)
        .stroke(Stroke::new(1.0, USER_LINE))
        .corner_radius(RADIUS)
        .inner_margin(Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            header(ui, &review);
            if !message.is_empty() {
                indented(ui, |ui| {
                    ui.label(RichText::new(message).size(14.0).color(brand::STEP_TEXT));
                });
            }
            if !review.notes.is_empty() {
                indented(ui, |ui| {
                    Frame::new()
                        .fill(brand::SUNKEN)
                        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                        .corner_radius(CornerRadius::same(NOTE_RADIUS))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (index, note) in review.notes.iter().enumerate() {
                                if index > 0 {
                                    let (line, _) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), 1.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        line,
                                        CornerRadius::ZERO,
                                        brand::SIDEBAR_LINE,
                                    );
                                }
                                if let Some(path) = note_block(ui, note, profile) {
                                    actions.push(ReviewAction::OpenFile(path));
                                }
                            }
                        });
                });
                indented(ui, |ui| buttons(ui, &review, &mut actions));
            }
        });
    actions
}

fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_space(INDENT);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}

fn header(ui: &mut Ui, review: &SentReview) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    let stroke = Stroke::new(1.4, brand::TEAL_TEXT);
    let p = |x: f32, y: f32| {
        rect.left_center() + vec2((x - 12.0) * 16.0 / 24.0 + 8.0, (y - 12.0) * 16.0 / 24.0)
    };
    painter.add(Shape::closed_line(
        vec![p(4.0, 5.0), p(20.0, 5.0), p(20.0, 16.0), p(9.0, 16.0), p(4.0, 20.0)],
        stroke,
    ));
    painter.line_segment([p(8.0, 10.0), p(16.0, 10.0)], stroke);
    let title = painter.layout_no_wrap("Review sent".to_owned(), heading(15.0), brand::TEXT);
    let title_w = title.size().x;
    painter.galley(
        pos2(rect.left() + 26.0, rect.center().y - title.size().y / 2.0),
        title,
        brand::TEXT,
    );
    let sub = painter.layout_no_wrap(review.notes_label(), body(12.0), brand::MUTED);
    painter.galley(
        pos2(rect.left() + 26.0 + title_w + 10.0, rect.center().y - sub.size().y / 2.0),
        sub,
        brand::MUTED,
    );
    if !review.notes.is_empty() {
        let progress = painter.layout_no_wrap(review.progress(), body(12.0), brand::FAINT);
        painter.galley(
            pos2(rect.right() - progress.size().x, rect.center().y - progress.size().y / 2.0),
            progress,
            brand::FAINT,
        );
    }
}

/// One note. Returns the file when its place was clicked.
fn note_block(ui: &mut Ui, note: &SentNote, profile: &str) -> Option<String> {
    let mut open = None;
    Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            let label = RichText::new(place(note)).font(mono(12.0)).color(brand::MUTED);
            let link = ui.add(egui::Label::new(label).sense(Sense::click()));
            if link
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Open this file in the file view")
                .clicked()
            {
                open = Some(note.path.clone());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(note.standing.label()).size(12.0).color(ink(note.standing)));
                if note.standing == Standing::Working {
                    ring(ui, ink(note.standing));
                }
            });
        });
        ui.label(RichText::new(&note.text).size(14.0).color(brand::TEXT));
        if let Some(reply) = &note.reply {
            reply_line(ui, profile, reply);
        }
    });
    open
}

fn ring(ui: &mut Ui, color: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(2.0 * RING + 2.0, 2.0 * RING + 2.0), Sense::hover());
    ui.painter().circle_stroke(rect.center(), RING, Stroke::new(2.0, color));
}

fn reply_line(ui: &mut Ui, profile: &str, reply: &str) {
    let shown = Frame::new().inner_margin(Margin { left: 10, ..Margin::ZERO }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let text = format!("{profile}: {reply}");
        ui.label(RichText::new(text).size(13.0).color(brand::SOFT));
    });
    let rect = shown.response.rect;
    let bar = Rect::from_min_max(rect.left_top(), pos2(rect.left() + BAR_W, rect.bottom()));
    ui.painter().rect_filled(bar, CornerRadius::same(1), brand::FIELD_LINE);
}

fn buttons(ui: &mut Ui, review: &SentReview, actions: &mut Vec<ReviewAction>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(note) = review.first_open()
            && quiet_button(ui, "Open in the file view", true, Size::Compact).clicked()
        {
            actions.push(ReviewAction::OpenFile(note.path.clone()));
        }
        let resolvable = review.resolvable();
        if !resolvable.is_empty()
            && quiet_button(ui, "Resolve answered", true, Size::Compact).clicked()
        {
            actions.push(ReviewAction::ResolveAnswered(resolvable));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};
    use kelp_core::review::{Anchor, Comment, Side};
    use tidewatch_core::notes::Notes;

    use crate::review::{Draft, SendState};

    fn comment(author: &str, body: &str) -> Comment {
        Comment { author: author.into(), time: 1, body: body.into() }
    }

    fn sent_notes() -> (Notes, Vec<u64>) {
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
        let ids: Vec<u64> = [
            ("runner.rs", 211, "Keep the count"),
            ("runner.rs", 240, "Log the retry"),
            ("tests.rs", 88, "Add the case"),
        ]
        .iter()
        .map(|(path, line, text)| notes.add(path, None, anchor(*line), comment("vlad", text)))
        .collect();
        notes.mark_sent(&ids, 5);
        notes.reply(ids[0], comment("claude", "moved it to Stage::retries"));
        (notes, ids)
    }

    #[test]
    fn the_place_is_the_path_with_the_line_or_the_range_when_there_is_one() {
        let note = |lines: &str| SentNote {
            id: 1,
            path: "src/a.rs".into(),
            lines: lines.into(),
            text: String::new(),
            reply: None,
            standing: Standing::Waiting,
        };
        assert_eq!(place(&note(":12")), "src/a.rs:12");
        assert_eq!(place(&note(":12-15")), "src/a.rs:12-15");
        assert_eq!(place(&note("")), "src/a.rs");
    }

    #[test]
    fn each_standing_has_its_own_ink() {
        let inks =
            [Standing::Answered, Standing::Working, Standing::Waiting, Standing::Resolved].map(ink);
        for (i, a) in inks.iter().enumerate() {
            assert!(inks.iter().skip(i + 1).all(|b| a != b), "{inks:?}");
        }
    }

    struct Bench {
        ctx: egui::Context,
        notes: Notes,
        ids: Vec<u64>,
        working: bool,
    }

    impl Bench {
        fn new(working: bool) -> Self {
            let (notes, ids) = sent_notes();
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            let mut bench = Self { ctx, notes, ids, working };
            bench.frame(vec![]);
            bench.frame(vec![]);
            bench
        }

        fn frame(&mut self, events: Vec<Event>) -> (Vec<ReviewAction>, f32) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 600.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (notes, ids, working) = (&self.notes, &self.ids, self.working);
            let draft = Draft::default();
            let mut out = (Vec::new(), 0.0);
            let _ = self.ctx.run_ui(raw, |ui| {
                let facts = ReviewFacts {
                    draft: &draft,
                    send: SendState::Ready,
                    notes,
                    agent_id: "claude",
                    profile: "Builder",
                    working,
                    keys_free: true,
                };
                let before = ui.next_widget_position().y;
                out.0 = show(ui, ids, "Start with runner.rs", Some(&facts));
                out.1 = ui.next_widget_position().y - before;
            });
            out
        }

        fn click(&mut self, at: egui::Pos2) -> Vec<ReviewAction> {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]).0
        }

        fn click_down(&mut self, wanted: &ReviewAction) -> bool {
            let height = self.frame(vec![]).1;
            (0..(height as i32 + 20)).step_by(6).any(|y| {
                (0..700).step_by(10).any(|x| self.click(pos2(x as f32, y as f32)).contains(wanted))
            })
        }
    }

    #[test]
    fn the_card_grows_with_the_reply_and_stays_one_card() {
        let waiting = Bench::new(false).frame(vec![]).1;
        let mut answered = Bench::new(false);
        let id = answered.ids[1];
        answered.notes.reply(id, comment("claude", "added a divider line for the retry"));
        let taller = answered.frame(vec![]).1;
        assert!(taller > waiting, "{taller} {waiting}");
        assert!(waiting > 150.0 && waiting < 420.0, "{waiting}");
    }

    #[test]
    fn open_in_the_file_view_goes_to_the_first_note_still_waiting() {
        let mut bench = Bench::new(true);
        let wanted = ReviewAction::OpenFile("runner.rs".into());
        assert!(bench.click_down(&wanted));
    }

    #[test]
    fn resolve_answered_closes_only_the_notes_the_agent_replied_to() {
        let mut bench = Bench::new(false);
        let wanted = ReviewAction::ResolveAnswered(vec![bench.ids[0]]);
        assert!(bench.click_down(&wanted));
    }

    #[test]
    fn without_anything_answered_resolve_answered_asks_for_nothing() {
        let mut bench = Bench::new(false);
        bench.notes = {
            let mut fresh = Notes::default();
            let anchor = Anchor {
                side: Side::New,
                line_hint: 1,
                text: "x".into(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            };
            bench.ids = vec![fresh.add("a.rs", None, anchor, comment("vlad", "x"))];
            fresh.mark_sent(&bench.ids, 1);
            fresh
        };
        let any = (0..300).step_by(6).any(|y| {
            (0..700).step_by(10).any(|x| {
                bench
                    .click(pos2(x as f32, y as f32))
                    .iter()
                    .any(|action| matches!(action, ReviewAction::ResolveAnswered(_)))
            })
        });
        assert!(!any);
    }
}
