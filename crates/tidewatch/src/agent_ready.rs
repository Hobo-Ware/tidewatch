//! The empty state and composer of an agent tab before its first message.

use egui::{
    Align, Align2, Frame, Id, Key, KeyboardShortcut, Layout, Margin, Modifiers, Rect, RichText,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::action_button;

use crate::commands::Command;
use crate::screen_kit::{body, heading, paint_cli_badge};

pub(crate) const COMPOSER_H: f32 = 164.0;
const SIDE_PAD: f32 = 20.0;
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
        colors::fg(),
    );
    let galley = painter.layout(
        "A fresh worktree on its own branch. Nothing runs until you say what to do.".to_owned(),
        body(14.0),
        colors::dim(),
        (area.width() - 48.0).min(520.0),
    );
    painter.galley(
        pos2(area.center().x - galley.size().x / 2.0, title.bottom() + 10.0),
        galley,
        colors::dim(),
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
    let outer = rect.shrink2(vec2(SIDE_PAD, 0.0)).with_max_y(rect.bottom() - 18.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(outer));
    let id = Id::new("agent-composer");
    let was_focused = child.memory(|m| m.has_focus(id));
    let enter = was_focused && child.input(|i| i.events.iter().any(plain_enter));
    let mut clicked = false;
    Frame::new()
        .fill(colors::bg())
        .stroke(egui::Stroke::new(2.0, colors::accent()))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(16, 12))
        .show(&mut child, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new(label(ready.profile).to_uppercase())
                    .size(12.0)
                    .color(colors::accent()),
            );
            let field = ui.add(
                TextEdit::multiline(text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(16.0))
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
            );
            if std::mem::take(focus) {
                field.request_focus();
            }
            ui.horizontal(|ui| {
                let hint = format!(
                    "Enter sends  -  Shift+Enter new line  -  {} terminal",
                    Command::Reader.keys()
                );
                ui.label(RichText::new(hint).size(13.0).color(colors::dim()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    clicked = action_button(ui, "Send", true).clicked();
                });
            });
        });
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
}
