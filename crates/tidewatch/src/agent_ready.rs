//! The empty state and composer of an agent tab before its first message.

use egui::{Align2, Id, Key, Rect, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::composer::{self, Draft, Spec};
use crate::reader_view::INSET;
use crate::screen_kit::{body, heading, paint_cli_badge};

const BADGE: f32 = 52.0;
const TITLE_H: f32 = 30.0;
const HEADER_GAP: f32 = 16.0;
const NOTE_GAP: f32 = 10.0;
const HEADER_PAD: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ready<'a> {
    pub(crate) profile: &'a str,
    pub(crate) agent_id: &'a str,
}

pub(crate) fn label(profile: &str) -> String {
    format!("What should {profile} do?")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Header {
    Full,
    NoNote,
    TitleOnly,
    Hidden,
}

fn header_for(room: f32, note_h: f32) -> Header {
    let pad = 2.0 * HEADER_PAD;
    if room >= BADGE + HEADER_GAP + TITLE_H + NOTE_GAP + note_h + pad {
        Header::Full
    } else if room >= BADGE + HEADER_GAP + TITLE_H + pad {
        Header::NoNote
    } else if room >= TITLE_H + pad {
        Header::TitleOnly
    } else {
        Header::Hidden
    }
}

pub(crate) fn empty_state(ui: &Ui, area: Rect, ready: Ready) {
    let painter = ui.painter();
    let note = painter.layout(
        "A fresh worktree on its own branch. Nothing runs until you say what to do.".to_owned(),
        body(14.0),
        brand::MUTED,
        (area.width() - 48.0).min(520.0),
    );
    let header = header_for(area.height(), note.size().y);
    let (badge_h, note_h) = match header {
        Header::Full => (BADGE + HEADER_GAP, NOTE_GAP + note.size().y),
        Header::NoNote => (BADGE + HEADER_GAP, 0.0),
        Header::TitleOnly => (0.0, 0.0),
        Header::Hidden => return,
    };
    let total = badge_h + TITLE_H + note_h;
    let top = (area.center().y - 60.0)
        .min(area.bottom() - HEADER_PAD - total)
        .max(area.top() + HEADER_PAD);
    if badge_h > 0.0 {
        let badge =
            Rect::from_center_size(pos2(area.center().x, top + BADGE / 2.0), vec2(BADGE, BADGE));
        paint_cli_badge(painter, badge, ready.agent_id);
    }
    let title = painter.text(
        pos2(area.center().x, top + badge_h),
        Align2::CENTER_TOP,
        format!("{} is ready", ready.profile),
        heading(24.0),
        brand::TEXT,
    );
    if note_h > 0.0 {
        painter.galley(
            pos2(area.center().x - note.size().x / 2.0, title.bottom() + NOTE_GAP),
            note,
            brand::MUTED,
        );
    }
}

pub(crate) fn height(ui: &Ui, text: &str, pictures: usize, column: Rect) -> f32 {
    composer::height(ui, text, pictures, column)
}

pub(crate) fn plain_enter(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. }
            if !modifiers.shift && !modifiers.command && !modifiers.alt
    )
}

pub(crate) fn composer_id() -> Id {
    Id::new("agent-composer")
}

pub(crate) fn composer_has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.has_focus(composer_id()))
}

/// The first message box: the Reader's composer, asking what the agent should do.
pub(crate) fn composer(ui: &mut Ui, rect: Rect, draft: Draft<'_>, ready: Ready) -> Option<String> {
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(INSET, composer::GAP_Y))));
    let hint = label(ready.profile);
    let spec = Spec {
        id: composer_id(),
        hint: &hint,
        enabled: true,
        commands: &[],
        ghost: None,
        mode: composer::Mode::Reply,
        stop: false,
        held: 0,
    };
    composer::show(&mut child, draft, &spec).sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader_view::ViewState;
    use egui::{Event, Frame, Modifiers, RawInput};

    fn ready() -> Ready<'static> {
        Ready { profile: "Builder", agent_id: "claude" }
    }

    fn frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        text: &mut String,
        focus: &mut bool,
    ) -> Option<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut sent = None;
        let _ = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, 64.0));
            let draft = Draft { text, pictures: &mut Vec::new(), focus, recall: None };
            sent = composer(ui, rect, draft, ready());
        });
        sent
    }

    fn enter(shift: bool) -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers { shift, ..Modifiers::NONE },
        }
    }

    fn started() -> (egui::Context, String, bool) {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let (mut text, mut focus) = (String::new(), true);
        frame(&ctx, vec![], &mut text, &mut focus);
        frame(&ctx, vec![], &mut text, &mut focus);
        (ctx, text, focus)
    }

    #[test]
    fn the_label_names_the_profile() {
        assert_eq!(label("Builder"), "What should Builder do?");
    }

    #[test]
    fn enter_sends_the_text_and_clears_the_box() {
        let (ctx, mut text, mut focus) = started();
        frame(&ctx, vec![Event::Text("add a guard".into())], &mut text, &mut focus);
        assert_eq!(text, "add a guard");
        let sent = frame(&ctx, vec![enter(false)], &mut text, &mut focus);
        assert_eq!(sent.as_deref(), Some("add a guard"));
        assert_eq!(text, "");
        assert!(focus, "the box takes the keyboard again");
    }

    #[test]
    fn shift_enter_adds_a_line_and_sends_nothing() {
        let (ctx, mut text, mut focus) = started();
        frame(&ctx, vec![Event::Text("one".into())], &mut text, &mut focus);
        assert_eq!(frame(&ctx, vec![enter(true)], &mut text, &mut focus), None);
        frame(&ctx, vec![Event::Text("two".into())], &mut text, &mut focus);
        assert_eq!(text, "one\ntwo");
    }

    #[test]
    fn enter_on_a_blank_box_sends_nothing() {
        let (ctx, mut text, mut focus) = started();
        frame(&ctx, vec![Event::Text("  ".into())], &mut text, &mut focus);
        assert_eq!(frame(&ctx, vec![enter(false)], &mut text, &mut focus), None);
    }

    #[test]
    fn an_image_only_cmd_v_becomes_a_chip_and_leaves_the_text_clean() {
        use crate::image_paste::testing::screenshot;
        use crate::image_paste::{Delivery, resolve};
        use tidewatch_core::image_paste::Target;

        let (ctx, mut text, mut focus) = started();
        frame(&ctx, vec![Event::Text("see ".into())], &mut text, &mut focus);
        let dir = tempfile::tempdir().unwrap();
        let Delivery::Pictures(paths) =
            resolve(&mut screenshot(), Target::TextField, dir.path(), 3)
        else {
            panic!("an image-only clipboard in a text field must give pictures");
        };
        assert!(paths[0].is_file());
        let mut pictures = Vec::new();
        crate::pictures::attach(&mut pictures, paths.clone());
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut chip_h = 0.0;
        let out = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, 130.0));
            let draft =
                Draft { text: &mut text, pictures: &mut pictures, focus: &mut focus, recall: None };
            composer(ui, rect, draft, ready());
            chip_h = crate::pictures::CHIP_H;
        });
        let chips = out
            .shapes
            .iter()
            .filter(|clipped| matches!(&clipped.shape, egui::Shape::Rect(r) if r.fill == brand::ITEM && r.rect.height() == chip_h))
            .count();
        assert_eq!((text.as_str(), chips), ("see ", 1));
    }

    #[test]
    fn a_clipboard_with_text_and_a_picture_still_pastes_the_text() {
        use crate::image_paste::testing::{Fake, screenshot};
        use crate::image_paste::{Delivery, resolve};
        use tidewatch_core::image_paste::{Clip, Target};

        let (ctx, mut text, mut focus) = started();
        let dir = tempfile::tempdir().unwrap();
        let mut mixed = Fake { clip: Clip::TextAndImage, ..screenshot() };
        assert_eq!(resolve(&mut mixed, Target::TextField, dir.path(), 3), Delivery::Nothing);
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
            events: vec![Event::Paste("copied words".into())],
            focused: true,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, 64.0));
            let draft = Draft {
                text: &mut text,
                pictures: &mut Vec::new(),
                focus: &mut focus,
                recall: None,
            };
            composer(ui, rect, draft, ready());
        });
        assert_eq!(text, "copied words");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_box_asks_what_the_agent_should_do() {
        let (ctx, mut text, mut focus) = started();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
            focused: true,
            ..RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, 64.0));
            let draft = Draft {
                text: &mut text,
                pictures: &mut Vec::new(),
                focus: &mut focus,
                recall: None,
            };
            composer(ui, rect, draft, ready());
        });
        let asked = out.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(t) if t.galley.text() == "What should Builder do?")
        });
        assert!(asked);
    }

    #[test]
    fn the_header_gives_way_before_it_runs_out_of_room() {
        let cases = [
            (400.0, Header::Full),
            (150.0, Header::NoNote),
            (110.0, Header::TitleOnly),
            (40.0, Header::Hidden),
            (0.0, Header::Hidden),
        ];
        for (room, want) in cases {
            assert_eq!(header_for(room, 40.0), want, "room {room}");
        }
    }

    struct Seen {
        column: Rect,
        status_top: f32,
        boxed: Option<Rect>,
        header: Option<Rect>,
        last_row: Option<(Rect, Rect)>,
    }

    fn text_with<'a>(
        shapes: &'a [egui::epaint::ClippedShape],
        needle: &str,
    ) -> Option<(&'a egui::epaint::TextShape, Rect)> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                Some((text, clipped.clip_rect))
            }
            _ => None,
        })
    }

    fn column_frame(
        ctx: &egui::Context,
        state: &mut ViewState,
        events: Vec<Event>,
        h: f32,
    ) -> Seen {
        use crate::reader_view::{Facts, Turn};
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, h))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut column = Rect::NOTHING;
        let out = ctx.run_ui(raw, |ui| {
            egui::Panel::bottom("statusbar")
                .exact_size(crate::status_bar::HEIGHT)
                .frame(Frame::NONE)
                .show(ui, |_| {});
            egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
                column = ui.max_rect();
                let facts = Facts {
                    tab: 1,
                    source: None,
                    prompt: None,
                    ready: Some(ready()),
                    root: None,
                    review: None,
                    pr: None,
                    due: None,
                    ask: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Finished,
                    working: false,
                    reply_blocked: None,
                };
                crate::reader_view::show(
                    ui,
                    column,
                    &crate::reader_rows::Rows::default(),
                    state,
                    &facts,
                );
            });
        });
        let boxed = out.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == brand::CARD => Some(rect.rect),
            _ => None,
        });
        let header = text_with(&out.shapes, "is ready")
            .map(|(shape, _)| Rect::from_min_size(shape.pos, shape.galley.size()));
        let last_row = text_with(&out.shapes, "line 60").and_then(|(shape, clip)| {
            let row = shape.galley.rows.last()?.rect().translate(shape.pos.to_vec2());
            Some((row, clip))
        });
        Seen { column, status_top: h - crate::status_bar::HEIGHT, boxed, header, last_row }
    }

    fn typing_60_lines_in(window_h: f32) {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut state = ViewState::default();
        column_frame(&ctx, &mut state, vec![], window_h);
        column_frame(&ctx, &mut state, vec![], window_h);
        for line in 1..=60 {
            let mut seen = column_frame(
                &ctx,
                &mut state,
                vec![Event::Text(format!("line {line}")), enter(true)],
                window_h,
            );
            seen = column_frame(&ctx, &mut state, vec![], window_h).or(seen);
            let boxed = seen.boxed.expect("the composer box is drawn");
            assert!(
                boxed.bottom() <= seen.status_top,
                "line {line}: {boxed:?} below the status bar"
            );
            assert!(boxed.top() >= seen.column.top(), "line {line}: {boxed:?} above the column");
            assert!(
                boxed.height()
                    <= (seen.column.height() * composer::SHARE).max(composer::BOX_H) + 0.5,
                "line {line}: {boxed:?} past half the column"
            );
            if let Some(header) = seen.header {
                assert!(header.bottom() <= boxed.top(), "line {line}: header overlaps the box");
            }
        }
        assert!(state.reply.ends_with("line 60\n"), "every line was typed");
        let last = column_frame(&ctx, &mut state, vec![Event::Text("line 60".into())], window_h);
        let last = column_frame(&ctx, &mut state, vec![], window_h).or(last);
        let (row, clip) = last.last_row.expect("the last typed line is painted");
        let boxed = last.boxed.expect("the composer box is drawn");
        assert!(clip.contains_rect(row.shrink(0.5)), "cursor row {row:?} outside visible {clip:?}");
        assert!(boxed.contains_rect(clip), "visible {clip:?} outside the box {boxed:?}");
    }

    impl Seen {
        fn or(self, earlier: Seen) -> Seen {
            if self.boxed.is_some() { self } else { earlier }
        }
    }

    #[test]
    fn sixty_typed_lines_scroll_inside_a_box_above_the_status_bar() {
        typing_60_lines_in(600.0);
    }

    #[test]
    fn sixty_typed_lines_in_a_short_window_still_stay_above_the_status_bar() {
        typing_60_lines_in(360.0);
    }
}
