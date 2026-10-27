//! The notes strip under the Diff pane's toolbar: one row per open note of the tab, in every file,
//! with where it stands (draft, sent, replied) and the buttons Kelp's thread card has no room for:
//! a menu with Send this note now, and Post to PR. A click on the place jumps to that file. The row
//! model is plain data; `show` only draws it.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Sense, Stroke, StrokeKind,
    TextFormat, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::commit::{calendar_time, relative_time};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::notes::{NoteState, Notes, line_suffix};

use crate::brand;
use crate::commit_files::{split_dir, tail_fit};
use crate::note_send::{Gate, gate_reason};
use crate::pr_state::{PostAbility, PrState};
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{Size, TagFill, body, mono, paint_tag, quiet_button, tag_width};

pub(crate) const MAX_ROWS: usize = 3;
const CARD_H: f32 = 36.0;
const GAP: f32 = 6.0;
const INSET: f32 = 10.0;
const CARD_PAD: f32 = 10.0;
const RADIUS: u8 = 8;
const BUTTON_GAP: f32 = 6.0;
const TEXT_GAP: f32 = 10.0;
const SNIPPET_MAX: usize = 90;
const PLACE_FONT: f32 = TEXT_TAG;
const PLACE_SHARE: f32 = 0.42;
const MIN_PLACE_W: f32 = 40.0;
const SNIPPET_FONT: f32 = 12.0;
const MIN_SNIPPET_W: f32 = 20.0;
const MENU_W: f32 = 220.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NoteRow {
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) lines: String,
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
                lines: line_suffix(&thread.anchor),
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
        NoteState::Queued => "draft".to_owned(),
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
        NoteState::Queued => Some("Part of your draft review: not sent to the agent yet".into()),
        NoteState::FromAgent => Some("The agent wrote this note itself".into()),
        NoteState::Resolved => None,
    }
}

fn send_label(state: NoteState) -> &'static str {
    match state {
        NoteState::Sent(_) | NoteState::Replied => "Send this note again",
        _ => "Send this note now",
    }
}

/// How tall the strip is for `count` open notes: none takes no room.
pub(crate) fn height(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    GAP + count.min(MAX_ROWS) as f32 * (CARD_H + GAP)
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, rows: &[NoteRow], now: i64) -> Vec<StripAction> {
    let mut actions = Vec::new();
    ui.scope_builder(UiBuilder::new().id_salt("notes-strip").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, GAP);
            ui.add_space(GAP);
            for row in rows {
                draw_card(ui, row, now, &mut actions);
            }
        });
    });
    actions
}

/// The folder, the file name and the `:line` of a note's place, each painted in its own ink.
fn place_parts<'a>(path: &'a str, lines: &'a str) -> (&'a str, &'a str, &'a str) {
    let (dir, name) = split_dir(path);
    (dir, name, lines)
}

fn state_tone(state: NoteState) -> Color32 {
    match state {
        NoteState::Queued => brand::ORANGE,
        NoteState::Sent(_) => brand::TEAL_TEXT,
        NoteState::Replied => brand::GREEN,
        NoteState::Resolved | NoteState::FromAgent => brand::MUTED,
    }
}

/// `text` cut at the end with an ellipsis until `measure` says it fits `room`.
fn ellipsize_to(measure: impl Fn(&str) -> f32, text: &str, room: f32) -> String {
    if measure(text) <= room {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len())
        .rev()
        .map(|keep| format!("{}...", chars[..keep].iter().collect::<String>().trim_end()))
        .find(|cut| measure(cut) <= room)
        .unwrap_or_default()
}

fn text_width(painter: &egui::Painter, text: &str, font: &FontId) -> f32 {
    painter.layout_no_wrap(text.to_owned(), font.clone(), brand::TEXT).size().x
}

fn place_job(parts: (&str, &str, &str), hot: bool) -> LayoutJob {
    let (dir, name, line) = parts;
    let format = |color| TextFormat { font_id: mono(PLACE_FONT), color, ..TextFormat::default() };
    let (dim, strong) =
        if hot { (brand::TEAL_TEXT, brand::TEAL_TEXT) } else { (brand::FAINT, brand::TEXT) };
    let mut job = LayoutJob::default();
    job.append(dir, 0.0, format(dim));
    job.append(name, 0.0, format(strong));
    job.append(line, 0.0, format(dim));
    job
}

fn state_pill(ui: &mut Ui, label: &str, tone: Color32) -> egui::Response {
    let width = tag_width(ui.painter(), label);
    let (rect, response) = ui.allocate_exact_size(vec2(width, Size::Tag.height()), Sense::hover());
    paint_tag(
        ui.painter(),
        (rect.left(), rect.center().y),
        label,
        tone,
        TagFill::Tint(tone.gamma_multiply(0.12)),
    );
    response
}

fn draw_card(ui: &mut Ui, row: &NoteRow, now: i64, actions: &mut Vec<StripAction>) {
    let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), CARD_H), Sense::hover());
    let card = slot.shrink2(vec2(INSET, 0.0));
    ui.painter().rect(
        card,
        CornerRadius::same(RADIUS),
        brand::ITEM,
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
    let inner = card.shrink2(vec2(CARD_PAD, 0.0));
    let mut buttons = ui.new_child(
        UiBuilder::new()
            .id_salt(("note-card", row.id))
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = BUTTON_GAP;
    let post_label = if row.on_pr { "Reply on PR" } else { "Post to PR" };
    let post = quiet_button(&mut buttons, post_label, row.post == PostAbility::Yes, Size::Compact);
    let post = match &row.post {
        PostAbility::No(reason) => post.on_disabled_hover_text(reason.as_str()),
        PostAbility::Yes => {
            post.on_hover_text("Post this note to the pull request, after a confirm")
        }
    };
    if post.clicked() {
        actions.push(StripAction::PostToPr(row.id));
    }
    let more =
        quiet_button(&mut buttons, "...", true, Size::Compact).on_hover_text("More for this note");
    egui::Popup::menu(&more).show(|ui| {
        ui.set_max_width(MENU_W);
        style_menu(ui);
        let send = ui.add_enabled_ui(row.send.is_ok(), |ui| {
            menu_item(ui, send_label(row.state), "")
                .on_disabled_hover_text(row.send.err().unwrap_or(""))
        });
        if send.inner.clicked() {
            actions.push(StripAction::Send(row.id));
            ui.close();
        }
    });
    let pill = state_pill(&mut buttons, &state_label(row.state, now), state_tone(row.state));
    if let Some(tip) = state_tip(row.state) {
        pill.on_hover_text(tip);
    }
    let room =
        (buttons.available_rect_before_wrap().right() - inner.left() - 2.0 * BUTTON_GAP).max(0.0);
    draw_text(ui, row, Rect::from_min_size(inner.min, vec2(room, inner.height())), actions);
}

fn draw_text(ui: &mut Ui, row: &NoteRow, area: Rect, actions: &mut Vec<StripAction>) {
    let painter = ui.painter().with_clip_rect(area);
    let (dir, name, line) = place_parts(&row.path, &row.lines);
    let font = mono(PLACE_FONT);
    let place_room = (area.width() * PLACE_SHARE).max(MIN_PLACE_W).min(area.width());
    let tail = text_width(&painter, &format!("{name}{line}"), &font);
    let dir = tail_fit(ui, dir, &font, (place_room - tail).max(0.0));
    let parts = (dir.as_str(), name, line);
    let galley = painter.layout_job(place_job(parts, false));
    let place = Rect::from_min_size(
        pos2(area.left(), area.center().y - galley.size().y / 2.0),
        galley.size().min(vec2(area.width(), f32::INFINITY)),
    );
    let response = ui.interact(place, ui.id().with(("note-place", row.id)), Sense::click());
    let galley =
        if response.hovered() { painter.layout_job(place_job(parts, true)) } else { galley };
    painter.galley(place.min, galley, brand::TEXT);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        actions.push(StripAction::Jump(row.path.clone()));
    }
    let left = place.right() + TEXT_GAP;
    let room = area.right() - left;
    if room > MIN_SNIPPET_W {
        let font = body(SNIPPET_FONT);
        let text = ellipsize_to(|text| text_width(&painter, text, &font), &row.snippet, room);
        painter.text(pos2(left, area.center().y), Align2::LEFT_CENTER, text, font, brand::SOFT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    fn anchor(line: u32) -> Anchor {
        Anchor {
            side: Side::New,
            line_hint: line,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        }
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
        let places: Vec<_> = rows.iter().map(|r| (r.path.as_str(), r.lines.as_str())).collect();
        assert_eq!(places, [("src/a.rs", ":3"), ("src/b.rs", ":9")]);
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
        assert_eq!(state_label(NoteState::Queued, 0), "draft");
        assert_eq!(state_label(NoteState::Sent(100), 100), "sent just now");
        assert_eq!(state_label(NoteState::Sent(100), 100 + 180), "sent 3 min ago");
        assert_eq!(state_label(NoteState::Replied, 0), "replied");
        assert_eq!(state_label(NoteState::FromAgent, 0), "from agent");
        assert_eq!(send_label(NoteState::Queued), "Send this note now");
        assert_eq!(send_label(NoteState::Sent(1)), "Send this note again");
    }

    #[test]
    fn the_strip_takes_room_for_up_to_three_cards_with_a_gap_around_each() {
        assert_eq!(height(0), 0.0);
        assert_eq!(height(1), GAP + CARD_H + GAP);
        assert_eq!(height(2), GAP + 2.0 * (CARD_H + GAP));
        assert_eq!(height(9), height(MAX_ROWS));
    }

    #[test]
    fn a_long_note_is_cut_for_the_row() {
        let long = "word ".repeat(60);
        let cut = snippet(&long);
        assert!(cut.ends_with("..."));
        assert_eq!(cut.chars().count(), SNIPPET_MAX + 3);
        assert_eq!(snippet(""), "");
    }

    #[test]
    fn a_snippet_that_does_not_fit_is_cut_at_a_word_end_with_an_ellipsis() {
        let width = |text: &str| text.chars().count() as f32;
        assert_eq!(ellipsize_to(width, "short", 10.0), "short");
        assert_eq!(
            ellipsize_to(width, "Looks off by one on the boundary", 16.0),
            "Looks off by..."
        );
        assert_eq!(ellipsize_to(width, "anything", 2.0), "");
    }

    #[test]
    fn the_place_splits_into_folder_file_and_line() {
        assert_eq!(place_parts("src/a/b.rs", ":7"), ("src/a/", "b.rs", ":7"));
        assert_eq!(place_parts("src/a/b.rs", ":7-9"), ("src/a/", "b.rs", ":7-9"));
        assert_eq!(place_parts("README.md", ""), ("", "README.md", ""));
    }

    #[test]
    fn each_state_has_its_own_ink() {
        assert_eq!(state_tone(NoteState::Queued), brand::ORANGE);
        assert_eq!(state_tone(NoteState::Sent(1)), brand::TEAL_TEXT);
        assert_eq!(state_tone(NoteState::Replied), brand::GREEN);
        assert_eq!(state_tone(NoteState::FromAgent), brand::MUTED);
    }

    fn click_strip(rows: &[NoteRow], clicks: &[egui::Pos2]) -> Vec<StripAction> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let button = |at, pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut steps = vec![vec![]];
        for at in clicks {
            steps.extend([
                vec![egui::Event::PointerMoved(*at)],
                vec![button(*at, true)],
                vec![button(*at, false)],
                vec![],
            ]);
        }
        for events in steps {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 300.0))),
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

    fn collect(into: &mut Vec<StripAction>, found: Vec<StripAction>) {
        for action in found {
            if !into.contains(&action) {
                into.push(action);
            }
        }
    }

    fn actions_along(rows: &[NoteRow], y: f32) -> Vec<StripAction> {
        let mut seen = Vec::new();
        for x in (0..800).step_by(24) {
            collect(&mut seen, click_strip(rows, &[pos2(x as f32, y)]));
        }
        seen
    }

    fn actions_through_the_menu(rows: &[NoteRow], y: f32) -> Vec<StripAction> {
        let mut seen = Vec::new();
        for x in (650..720).step_by(6) {
            for below in [36, 48, 60] {
                let clicks = [pos2(x as f32, y), pos2(x as f32, y + below as f32)];
                collect(&mut seen, click_strip(rows, &clicks));
            }
        }
        seen
    }

    fn card_y(index: usize) -> f32 {
        GAP + index as f32 * (CARD_H + GAP) + CARD_H / 2.0
    }

    #[test]
    fn the_place_asks_to_jump_and_the_menu_asks_to_send_that_note_only_while_the_gate_is_open() {
        let open = rows(&sample(), "claude", Gate::Ready, &PrState::default());
        for (index, path) in [(0, "src/a.rs"), (1, "src/b.rs")] {
            assert_eq!(actions_along(&open, card_y(index)), [StripAction::Jump(path.into())]);
            let found = actions_through_the_menu(&open, card_y(index));
            assert!(found.contains(&StripAction::Send(open[index].id)), "{found:?}");
            let other = open[1 - index].id;
            assert!(!found.contains(&StripAction::Send(other)), "{found:?}");
        }
        let blocked = rows(&sample(), "claude", Gate::WaitingOnYou, &PrState::default());
        let found = actions_through_the_menu(&blocked, card_y(0));
        assert!(found.iter().all(|action| matches!(action, StripAction::Jump(_))), "{found:?}");
    }
}
