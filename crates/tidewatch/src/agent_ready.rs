//! The empty state and composer of an agent tab before its first message.

use egui::{
    Align2, FontId, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, Rect, RichText,
    ScrollArea, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::commands::Command;
use crate::reader_view::INSET;
use crate::screen_kit::{Size, body, footer_row, heading, paint_cli_badge, primary_button};

const COMPOSER_H: f32 = 164.0;
const COMPOSER_SHARE: f32 = 0.5;
const COMPOSER_PAD_X: f32 = 32.0;
const COMPOSER_FONT_PX: f32 = 16.0;
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

fn text_font() -> FontId {
    FontId::proportional(COMPOSER_FONT_PX)
}

fn text_heights(ui: &Ui, text: &str, width: f32) -> (f32, f32) {
    let wrap = width - 2.0 * INSET - COMPOSER_PAD_X;
    let typed = ui.painter().layout(text.to_owned(), text_font(), brand::TEXT, wrap).size().y;
    let two_rows = ui.painter().layout("\n".to_owned(), text_font(), brand::TEXT, wrap).size().y;
    (typed, two_rows)
}

fn grown_height(typed: f32, two_rows: f32, column_h: f32) -> f32 {
    let cap = (column_h * COMPOSER_SHARE).max(COMPOSER_H);
    (COMPOSER_H + (typed - two_rows).max(0.0)).min(cap)
}

pub(crate) fn height(ui: &Ui, text: &str, column: Rect) -> f32 {
    let (typed, two_rows) = text_heights(ui, text, column.width());
    grown_height(typed, two_rows, column.height())
}

fn plain_enter(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. }
            if !modifiers.shift && !modifiers.command && !modifiers.alt
    )
}

pub(crate) fn composer(
    ui: &mut Ui,
    rect: Rect,
    text: &mut String,
    focus: &mut bool,
    ready: Ready,
) -> Option<String> {
    let outer = rect.shrink2(vec2(INSET, 0.0)).with_max_y(rect.bottom() - 18.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(outer));
    let id = Id::new("agent-composer");
    let was_focused = child.memory(|m| m.has_focus(id));
    let enter = was_focused && child.input(|i| i.events.iter().any(plain_enter));
    let mut clicked = false;
    let mut prepared = Frame::new()
        .fill(brand::ITEM)
        .corner_radius(12)
        .inner_margin(Margin::symmetric(16, 12))
        .begin(&mut child);
    let ui = &mut prepared.content_ui;
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(RichText::new(label(ready.profile)).size(12.5).color(brand::MUTED));
    let (_, two_rows) = text_heights(ui, "", rect.width());
    let shown = (rect.height() - COMPOSER_H + two_rows).max(two_rows);
    let field = ScrollArea::vertical()
        .id_salt("agent-composer-scroll")
        .max_height(shown)
        .min_scrolled_height(shown)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                TextEdit::multiline(text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .min_size(vec2(0.0, shown))
                    .desired_width(f32::INFINITY)
                    .font(text_font())
                    .text_color(brand::TEXT)
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
            )
        })
        .inner;
    if field.changed() {
        ui.ctx().request_repaint();
    }
    if std::mem::take(focus) {
        field.request_focus();
    }
    let hint =
        format!("Enter sends  -  Shift+Enter new line  -  {} terminal", Command::Reader.keys());
    footer_row(ui, &hint, |ui| {
        clicked = primary_button(ui, "Send", !text.trim().is_empty(), Size::Regular).clicked();
    });
    let line = if field.has_focus() { brand::TEAL } else { brand::FIELD_LINE };
    prepared.frame.stroke = Stroke::new(1.0, line);
    prepared.end(&mut child);
    ((enter || clicked) && !text.trim().is_empty()).then(|| {
        *focus = true;
        std::mem::take(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader_view::ViewState;
    use egui::{Event, RawInput};

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
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, COMPOSER_H));
            sent = composer(ui, rect, text, focus, ready());
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

    fn frame_with_paste(ctx: &egui::Context, pasted: &str, text: &mut String, focus: &mut bool) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
            focused: true,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            crate::image_paste::inject_paste(ui.ctx(), pasted.to_owned());
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, COMPOSER_H));
            composer(ui, rect, text, focus, ready());
        });
    }

    #[test]
    fn an_image_only_cmd_v_puts_the_saved_path_in_the_composer() {
        use crate::image_paste::testing::screenshot;
        use crate::image_paste::{Delivery, resolve};
        use tidewatch_core::image_paste::Target;

        let (ctx, mut text, mut focus) = started();
        frame(&ctx, vec![Event::Text("see ".into())], &mut text, &mut focus);
        let dir = tempfile::tempdir().unwrap();
        let Delivery::Text(path) = resolve(&mut screenshot(), Target::TextField, dir.path(), 3)
        else {
            panic!("an image-only clipboard in a text field must give a path");
        };
        frame_with_paste(&ctx, &path, &mut text, &mut focus);
        assert_eq!(text, format!("see {path}"));
        assert!(std::path::Path::new(&path).is_file());
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
            let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(800.0, COMPOSER_H));
            composer(ui, rect, &mut text, &mut focus, ready());
        });
        assert_eq!(text, "copied words");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_composer_grows_with_its_text_up_to_half_the_column() {
        let two_rows = 38.0;
        let cases = [
            (0.0, 600.0, COMPOSER_H),
            (two_rows, 600.0, COMPOSER_H),
            (two_rows + 20.0, 600.0, COMPOSER_H + 20.0),
            (5_000.0, 600.0, 300.0),
            (5_000.0, 200.0, COMPOSER_H),
        ];
        for (typed, column, want) in cases {
            assert_eq!(grown_height(typed, two_rows, column), want, "{typed} in {column}");
        }
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
                    source: None,
                    prompt: None,
                    ready: Some(ready()),
                    root: None,
                    review: None,
                    pr: None,
                    due: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Finished,
                    stopped: false,
                };
                crate::reader_view::show(
                    ui,
                    column,
                    &crate::reader_rows::Rows::default(),
                    state,
                    facts,
                );
            });
        });
        let boxed = out.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == brand::ITEM => Some(rect.rect),
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
                boxed.height() <= (seen.column.height() * COMPOSER_SHARE).max(COMPOSER_H) + 0.5,
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
