//! The empty state and composer of an agent tab before its first message.

use egui::{
    Align2, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, Rect, RichText, Stroke, TextEdit,
    Ui, UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::commands::Command;
use crate::reader_view::INSET;
use crate::screen_kit::{Size, body, footer_row, heading, paint_cli_badge, primary_button};

pub(crate) const COMPOSER_H: f32 = 164.0;
const BADGE: f32 = 52.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ready<'a> {
    pub(crate) profile: &'a str,
    pub(crate) agent_id: &'a str,
}

pub(crate) fn label(profile: &str) -> String {
    format!("What should {profile} do?")
}

pub(crate) fn empty_state(ui: &Ui, area: Rect, ready: Ready) {
    let painter = ui.painter();
    let badge =
        Rect::from_center_size(pos2(area.center().x, area.center().y - 34.0), vec2(BADGE, BADGE));
    paint_cli_badge(painter, badge, ready.agent_id);
    let title = painter.text(
        pos2(area.center().x, badge.bottom() + 16.0),
        Align2::CENTER_TOP,
        format!("{} is ready", ready.profile),
        heading(24.0),
        brand::TEXT,
    );
    let galley = painter.layout(
        "A fresh worktree on its own branch. Nothing runs until you say what to do.".to_owned(),
        body(14.0),
        brand::MUTED,
        (area.width() - 48.0).min(520.0),
    );
    painter.galley(
        pos2(area.center().x - galley.size().x / 2.0, title.bottom() + 10.0),
        galley,
        brand::MUTED,
    );
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
    let field = ui.add(
        TextEdit::multiline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .font(egui::FontId::proportional(16.0))
            .text_color(brand::TEXT)
            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
    );
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
}
