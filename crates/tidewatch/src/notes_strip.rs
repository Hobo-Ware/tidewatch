//! The notes strip under the Diff pane's toolbar: one row per open note of the tab, in every file,
//! with where it stands (queued, sent, replied) and the buttons Kelp's thread card has no room for:
//! Send to the agent and Post to PR. A click on the place jumps to that file. The row model is plain
//! data; `show` only draws it.

use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use kelp_core::commit::{calendar_time, relative_time};
use kelp_ui::theme;
use tidewatch_core::notes::{NoteState, Notes};

use crate::brand;
use crate::note_send::{Gate, gate_reason};
use crate::pr_state::{PostAbility, PrState};

pub(crate) const ROW_H: f32 = 26.0;
pub(crate) const MAX_ROWS: usize = 3;
const SIDE_PAD: f32 = 14.0;
const SNIPPET_MAX: usize = 90;
const STATE_W: f32 = 104.0;
const SEND_W: f32 = 58.0;
const POST_W: f32 = 82.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NoteRow {
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) line: u32,
    pub(crate) snippet: String,
    pub(crate) state: NoteState,
    pub(crate) send: Result<(), &'static str>,
    pub(crate) post: PostAbility,
    pub(crate) on_pr: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StripAction {
    Send(u64),
    PostToPr(u64),
    Jump(String),
}

/// The open notes as rows, in the order they were written. Resolved notes are left out.
pub(crate) fn rows(notes: &Notes, agent: &str, gate: Gate, pr: &PrState) -> Vec<NoteRow> {
    notes
        .threads
        .iter()
        .filter(|thread| !thread.resolved)
        .map(|thread| {
            let on_pr = notes.pr_root_of(thread.id).is_some();
            NoteRow {
                id: thread.id,
                path: thread.path.clone(),
                line: thread.anchor.line_hint,
                snippet: snippet(thread.comments.last().map_or("", |c| c.body.as_str())),
                state: notes.state(thread, agent),
                send: gate_reason(gate).map_or(Ok(()), Err),
                post: pr.post_ability(thread, on_pr),
                on_pr,
            }
        })
        .collect()
}

fn snippet(body: &str) -> String {
    let first = body.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    if first.chars().count() <= SNIPPET_MAX {
        return first.to_owned();
    }
    let head: String = first.chars().take(SNIPPET_MAX).collect();
    format!("{head}...")
}

pub(crate) fn state_label(state: NoteState, now: i64) -> String {
    match state {
        NoteState::Resolved => "resolved".to_owned(),
        NoteState::Queued => "queued".to_owned(),
        NoteState::Sent(at) => format!("sent {}", relative_time(at, now)),
        NoteState::Replied => "replied".to_owned(),
        NoteState::FromAgent => "from agent".to_owned(),
    }
}

fn state_tip(state: NoteState) -> Option<String> {
    match state {
        NoteState::Sent(at) => Some(format!("Sent to the agent at {}", calendar_time(at))),
        NoteState::Replied => {
            Some("The agent answered this note: the reply is in the thread".into())
        }
        NoteState::Queued => Some("Not sent to the agent yet".into()),
        NoteState::FromAgent => Some("The agent wrote this note itself".into()),
        NoteState::Resolved => None,
    }
}

fn send_label(state: NoteState) -> &'static str {
    match state {
        NoteState::Sent(_) | NoteState::Replied => "Resend",
        _ => "Send",
    }
}

/// How tall the strip is for `count` open notes: none takes no room.
pub(crate) fn height(count: usize) -> f32 {
    ROW_H * count.min(MAX_ROWS) as f32
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, rows: &[NoteRow], now: i64) -> Vec<StripAction> {
    let mut actions = Vec::new();
    ui.scope_builder(UiBuilder::new().id_salt("notes-strip").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for row in rows {
                draw_row(ui, row, now, &mut actions);
            }
        });
    });
    actions
}

fn draw_row(ui: &mut Ui, row: &NoteRow, now: i64, actions: &mut Vec<StripAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, theme::border()));
    let right = rect.right() - SIDE_PAD;
    let post_rect =
        Rect::from_min_size(pos2(right - POST_W, rect.top() + 2.0), vec2(POST_W, ROW_H - 4.0));
    let send_rect = Rect::from_min_size(
        pos2(post_rect.left() - 6.0 - SEND_W, rect.top() + 2.0),
        vec2(SEND_W, ROW_H - 4.0),
    );
    let state_rect = Rect::from_min_size(
        pos2(send_rect.left() - 8.0 - STATE_W, rect.top()),
        vec2(STATE_W, ROW_H),
    );
    let left = rect.left() + SIDE_PAD;
    let room = (state_rect.left() - 8.0 - left).max(0.0);
    let place_w = (room * 0.42).max(40.0).min(room);
    let place = Rect::from_min_size(pos2(left, rect.top()), vec2(place_w, ROW_H));
    let place_text =
        if row.line > 0 { format!("{}:{}", row.path, row.line) } else { row.path.clone() };
    let place_id = ui.id().with(("note-place", row.id));
    let place_resp = ui.interact(place, place_id, Sense::click());
    let color = if place_resp.hovered() { theme::text_strong() } else { brand::TEAL_TEXT };
    painter.with_clip_rect(place).text(
        pos2(place.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &place_text,
        FontId::monospace(11.5),
        color,
    );
    if place_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        actions.push(StripAction::Jump(row.path.clone()));
    }
    let snippet_rect = Rect::from_min_max(
        pos2(place.right() + 8.0, rect.top()),
        pos2(state_rect.left() - 8.0, rect.bottom()),
    );
    if snippet_rect.width() > 20.0 {
        painter.with_clip_rect(snippet_rect).text(
            pos2(snippet_rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            &row.snippet,
            FontId::proportional(12.0),
            theme::text_muted(),
        );
    }
    let label = state_label(row.state, now);
    let tone = match row.state {
        NoteState::Queued => theme::modified(),
        NoteState::Sent(_) => brand::TEAL_TEXT,
        NoteState::Replied => theme::added(),
        NoteState::Resolved | NoteState::FromAgent => theme::text_faint(),
    };
    let state_resp = ui.interact(state_rect, ui.id().with(("note-state", row.id)), Sense::hover());
    painter.with_clip_rect(state_rect).text(
        pos2(state_rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        label,
        FontId::proportional(11.5),
        tone,
    );
    if let Some(tip) = state_tip(row.state) {
        state_resp.on_hover_text(tip);
    }
    let send_label = send_label(row.state);
    let send = ui.put(send_rect, |ui: &mut Ui| {
        ui.add_enabled(
            row.send.is_ok(),
            egui::Button::new(RichText::new(send_label).size(11.5)).corner_radius(5),
        )
    });
    let send = match row.send {
        Err(reason) => send.on_disabled_hover_text(reason),
        Ok(()) => send.on_hover_text("Paste this note into the agent's terminal"),
    };
    if send.clicked() {
        actions.push(StripAction::Send(row.id));
    }
    let post_label = if row.on_pr { "Reply on PR" } else { "Post to PR" };
    let post_ok = row.post == PostAbility::Yes;
    let post = ui.put(post_rect, |ui: &mut Ui| {
        ui.add_enabled(
            post_ok,
            egui::Button::new(RichText::new(post_label).size(11.5)).corner_radius(5),
        )
    });
    let post = match &row.post {
        PostAbility::No(reason) => post.on_disabled_hover_text(reason.as_str()),
        PostAbility::Yes => {
            post.on_hover_text("Post this note to the pull request, after a confirm")
        }
    };
    if post.clicked() {
        actions.push(StripAction::PostToPr(row.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    fn anchor(line: u32) -> Anchor {
        Anchor { side: Side::New, line_hint: line, text: "x".into(), before: vec![], after: vec![] }
    }

    fn comment(author: &str, body: &str) -> Comment {
        Comment { author: author.into(), time: 1, body: body.into() }
    }

    fn sample() -> Notes {
        let mut notes = Notes::default();
        notes.add("src/a.rs", None, anchor(3), comment("You", "use formatDate\nand test it"));
        let sent = notes.add("src/b.rs", None, anchor(9), comment("You", "rename"));
        notes.mark_sent(&[sent], 100);
        let done = notes.add("src/c.rs", None, anchor(1), comment("You", "gone"));
        notes.set_resolved(done, true);
        notes
    }

    #[test]
    fn rows_list_open_notes_with_their_state_and_the_first_line_of_the_newest_comment() {
        let rows = rows(&sample(), "claude", Gate::Ready, &PrState::default());
        let places: Vec<_> = rows.iter().map(|r| (r.path.as_str(), r.line)).collect();
        assert_eq!(places, [("src/a.rs", 3), ("src/b.rs", 9)]);
        assert_eq!(rows[0].snippet, "use formatDate");
        assert_eq!(rows[0].state, NoteState::Queued);
        assert_eq!(rows[1].state, NoteState::Sent(100));
        assert!(rows.iter().all(|r| r.send.is_ok()));
    }

    #[test]
    fn a_gate_that_is_closed_disables_send_with_its_reason() {
        let rows = rows(&sample(), "claude", Gate::WaitingOnYou, &PrState::default());
        assert!(rows.iter().all(|r| r.send == Err("The agent is waiting on you. Answer it first")));
    }

    #[test]
    fn without_a_pull_request_post_is_disabled() {
        let rows = rows(&sample(), "claude", Gate::Ready, &PrState::default());
        assert!(rows.iter().all(|r| matches!(r.post, PostAbility::No(_))));
    }

    #[test]
    fn state_words_say_sent_with_how_long_ago() {
        assert_eq!(state_label(NoteState::Queued, 0), "queued");
        assert_eq!(state_label(NoteState::Sent(100), 100), "sent just now");
        assert_eq!(state_label(NoteState::Sent(100), 100 + 180), "sent 3 min ago");
        assert_eq!(state_label(NoteState::Replied, 0), "replied");
        assert_eq!(state_label(NoteState::FromAgent, 0), "from agent");
        assert_eq!(send_label(NoteState::Queued), "Send");
        assert_eq!(send_label(NoteState::Sent(1)), "Resend");
    }

    #[test]
    fn the_strip_takes_room_for_up_to_three_rows() {
        assert_eq!(height(0), 0.0);
        assert_eq!(height(2), ROW_H * 2.0);
        assert_eq!(height(9), ROW_H * MAX_ROWS as f32);
    }

    #[test]
    fn a_long_note_is_cut_for_the_row() {
        let long = "word ".repeat(60);
        let cut = snippet(&long);
        assert!(cut.ends_with("..."));
        assert_eq!(cut.chars().count(), SNIPPET_MAX + 3);
        assert_eq!(snippet(""), "");
    }

    fn click_strip(rows: &[NoteRow], at: egui::Pos2) -> Vec<StripAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let steps =
            [vec![], vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]];
        for events in steps {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0))),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, height(rows.len())));
                actions.extend(show(ui, rect, rows, 0));
            });
        }
        actions
    }

    #[test]
    fn the_buttons_and_the_place_ask_for_their_note_and_a_disabled_send_asks_for_nothing() {
        let open = rows(&sample(), "claude", Gate::Ready, &PrState::default());
        let send_x = 800.0 - SIDE_PAD - POST_W - 6.0 - SEND_W / 2.0;
        assert_eq!(click_strip(&open, pos2(send_x, ROW_H / 2.0)), [StripAction::Send(open[0].id)]);
        assert_eq!(click_strip(&open, pos2(send_x, ROW_H * 1.5)), [StripAction::Send(open[1].id)]);
        assert_eq!(
            click_strip(&open, pos2(SIDE_PAD + 20.0, ROW_H / 2.0)),
            [StripAction::Jump("src/a.rs".into())]
        );
        let blocked = rows(&sample(), "claude", Gate::WaitingOnYou, &PrState::default());
        assert_eq!(click_strip(&blocked, pos2(send_x, ROW_H / 2.0)), []);
    }
}
