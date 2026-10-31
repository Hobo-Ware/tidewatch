//! The confirm before Fix: the app quits, Homebrew repairs the install and the app reopens.

use egui::{Id, RichText};

use crate::brand;
use crate::screen_kit::{
    DialogWidth, Size, dialog_frame, footer_row, primary_button, quiet_button,
};

pub(crate) const TITLE: &str = "Fix the update";
pub(crate) const MESSAGE: &str =
    "Tidewatch will quit, repair the Homebrew install and reopen. Your tabs come back.";
pub(crate) const QUIT_AND_FIX: &str = "Quit and fix";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Cancel,
    QuitAndFix,
}

pub(crate) fn show(ctx: &egui::Context) -> Option<Answer> {
    let mut answer = None;
    let modal = dialog_frame(ctx, Id::new("fix_update"), TITLE, DialogWidth::Narrow, |ui| {
        answer = body(ui);
    });
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

fn body(ui: &mut egui::Ui) -> Option<Answer> {
    let mut answer = None;
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(RichText::new(MESSAGE).color(brand::MUTED));
    ui.add_space(6.0);
    footer_row(ui, &[("Esc", "cancels")], |ui| {
        if primary_button(ui, QUIT_AND_FIX, true, Size::Regular).clicked() {
            answer = Some(Answer::QuitAndFix);
        }
        if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
            answer = Some(Answer::Cancel);
        }
    });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};

    fn frame(ctx: &egui::Context, events: Vec<Event>) -> (Option<Answer>, Rect) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 300.0))),
            focused: true,
            events,
            ..RawInput::default()
        };
        let mut out = (None, Rect::NOTHING);
        let _ = ctx.run_ui(input, |ui| {
            out.1 = ui.scope(|ui| out.0 = body(ui)).response.rect;
        });
        out
    }

    fn click(at: egui::Pos2) -> Vec<Event> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![Event::PointerMoved(at), button(true), button(false)]
    }

    fn answers_by_clicking_everywhere() -> Vec<Answer> {
        let ctx = crate::fonts::test_context();
        frame(&ctx, vec![]);
        let (_, used) = frame(&ctx, vec![]);
        let mut found = Vec::new();
        for y in (used.top() as i32..=used.bottom() as i32).step_by(6) {
            for x in (used.left() as i32..=used.right() as i32).step_by(6) {
                let (answer, _) = frame(&ctx, click(pos2(x as f32, y as f32)));
                if let Some(answer) = answer
                    && !found.contains(&answer)
                {
                    found.push(answer);
                }
            }
        }
        found
    }

    #[test]
    fn the_confirm_offers_cancel_and_quit_and_fix_and_nothing_else() {
        let mut found = answers_by_clicking_everywhere();
        found.sort_by_key(|a| *a as u8);
        assert_eq!(found, [Answer::Cancel, Answer::QuitAndFix]);
    }

    #[test]
    fn the_confirm_says_what_will_happen() {
        assert_eq!(
            MESSAGE,
            "Tidewatch will quit, repair the Homebrew install and reopen. Your tabs come back."
        );
        assert_eq!(QUIT_AND_FIX, "Quit and fix");
    }

    #[test]
    fn nothing_is_answered_until_a_button_is_clicked() {
        let ctx = crate::fonts::test_context();
        for _ in 0..3 {
            assert_eq!(frame(&ctx, vec![]).0, None);
        }
    }
}
