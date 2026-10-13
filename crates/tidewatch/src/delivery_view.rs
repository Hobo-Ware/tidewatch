//! The delivery card and its dialog (PLAN.md, D7). The compact card sits in a band above the panes:
//! "N commits ready - Push and open PR". Opening it shows the commits and an editable title and text;
//! approving pushes and opens the pull request, and the dialog then shows the URL or the typed
//! failure with a plain fix. What each state looks like is in `delivery_card`.

use egui::{
    Align2, FontId, Id, Key, Modifiers, Rect, RichText, Stroke, StrokeKind, Ui, pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, overlay_frame, text_field};
use tidewatch_core::delivery::DeliveryError;

use crate::delivery_card::{Delivery, Form, card_text, fix_hint};

pub(crate) const BAND_H: f32 = 40.0;
const DIALOG_WIDTH: f32 = 560.0;
const COMMITS_MAX_H: f32 = 110.0;
const BODY_ROWS: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardAction {
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogAction {
    Approve,
    Cancel,
    Back,
    Close,
    OpenUrl,
}

/// Draws the compact card in `rect` (a band `BAND_H` tall) and reports a click on its button.
pub(crate) fn show_card(
    ui: &mut Ui,
    rect: Rect,
    commits: usize,
    building: bool,
) -> Option<CardAction> {
    let card = rect.shrink2(vec2(10.0, 4.0));
    ui.painter().rect_filled(card, 8.0, colors::elevated());
    ui.painter().rect_stroke(card, 8.0, Stroke::new(1.0, colors::border()), StrokeKind::Inside);
    let accent = colors::green();
    ui.painter().circle_filled(pos2(card.left() + 16.0, card.center().y), 4.5, accent);
    ui.painter().text(
        pos2(card.left() + 32.0, card.center().y),
        Align2::LEFT_CENTER,
        card_text(commits),
        FontId::proportional(13.0),
        colors::fg(),
    );
    let button_rect = card.shrink2(vec2(12.0, 4.0));
    let label = if building { "Reading commits..." } else { "Push and open PR" };
    let mut action = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(button_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.add_enabled_ui(!building, |ui| {
                if action_button(ui, label, true).clicked() {
                    action = Some(CardAction::Open);
                }
            });
        },
    );
    action
}

/// The dialog for every state past the compact card. `None` while nothing was asked.
pub(crate) fn show_dialog(ctx: &egui::Context, delivery: &mut Delivery) -> Option<DialogAction> {
    let mut answer = None;
    let editing = matches!(delivery, Delivery::Editing(_));
    let modal = egui::Modal::new(Id::new("delivery")).frame(overlay_frame()).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.spacing_mut().item_spacing.y = 8.0;
        match delivery {
            Delivery::Editing(form) => answer = editing_body(ui, form),
            Delivery::Delivering(form) => {
                heading(ui, "Pushing and opening the pull request...");
                dim(ui, &format!("{} -> {}", form.plan.branch, form.plan.base));
            }
            Delivery::Failed { form, error } => answer = failed_body(ui, form, error),
            Delivery::Delivered(done) => {
                heading(
                    ui,
                    if done.created { "Pull request opened" } else { "Pull request updated" },
                );
                if ui.link(&done.url).clicked() {
                    answer = Some(DialogAction::OpenUrl);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if action_button(ui, "Close", false).clicked() {
                        answer = Some(DialogAction::Close);
                    }
                    if action_button(ui, "Open in browser", true).clicked() {
                        answer = Some(DialogAction::OpenUrl);
                    }
                });
            }
            _ => {}
        }
    });
    if answer.is_none() && editing && approve_shortcut(ctx) {
        answer = Some(DialogAction::Approve);
    }
    if answer.is_none() && modal.should_close() {
        answer = match delivery {
            Delivery::Editing(_) => Some(DialogAction::Cancel),
            Delivery::Failed { .. } => Some(DialogAction::Back),
            Delivery::Delivered(_) => Some(DialogAction::Close),
            _ => None,
        };
    }
    answer
}

fn approve_shortcut(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter))
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong().size(15.0).color(colors::fg()));
}

fn dim(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(colors::dim()));
}

fn editing_body(ui: &mut Ui, form: &mut Form) -> Option<DialogAction> {
    let mut answer = None;
    heading(ui, "Push and open a pull request");
    let count = form.plan.commits.len();
    dim(
        ui,
        &format!(
            "{} -> {}  -  {} {}",
            form.plan.branch,
            form.plan.base,
            count,
            if count == 1 { "commit" } else { "commits" }
        ),
    );
    egui::ScrollArea::vertical().max_height(COMMITS_MAX_H).id_salt("delivery-commits").show(
        ui,
        |ui| {
            for commit in &form.plan.commits {
                ui.label(
                    RichText::new(format!("{}  {}", commit.sha, commit.subject))
                        .monospace()
                        .size(12.0)
                        .color(colors::fg()),
                );
            }
        },
    );
    ui.add_space(2.0);
    dim(ui, "Title");
    let width = ui.available_width();
    text_field(ui, &mut form.title, "Pull request title", width, colors::fg());
    dim(ui, "Text");
    ui.add(
        egui::TextEdit::multiline(&mut form.body)
            .desired_rows(BODY_ROWS)
            .desired_width(f32::INFINITY)
            .font(egui::FontId::monospace(12.0)),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if action_button(ui, "Cancel", false).clicked() {
            answer = Some(DialogAction::Cancel);
        }
        ui.add_enabled_ui(!form.title.trim().is_empty(), |ui| {
            if action_button(ui, "Push and open PR", true).clicked() {
                answer = Some(DialogAction::Approve);
            }
        });
        dim(ui, "Cmd+Enter");
    });
    answer
}

fn failed_body(ui: &mut Ui, form: &Form, error: &DeliveryError) -> Option<DialogAction> {
    let mut answer = None;
    heading(ui, "The pull request was not opened");
    dim(ui, &format!("{} -> {}", form.plan.branch, form.plan.base));
    ui.label(RichText::new(error.to_string()).color(colors::red()));
    if let Some(hint) = fix_hint(error) {
        ui.label(RichText::new(hint).color(colors::fg()));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if action_button(ui, "Close", false).clicked() {
            answer = Some(DialogAction::Close);
        }
        if action_button(ui, "Back to the text", true).clicked() {
            answer = Some(DialogAction::Back);
        }
    });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, RawInput};
    use std::path::PathBuf;
    use tidewatch_core::delivery::{CommitLine, Delivered, DeliveryPlan};

    fn plan() -> DeliveryPlan {
        DeliveryPlan {
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            base: "main".into(),
            commits: vec![CommitLine {
                sha: "abc1234".into(),
                subject: "feat: x".into(),
                body: String::new(),
            }],
            title: "Add x".into(),
            body: "text".into(),
        }
    }

    fn editing() -> Delivery {
        Delivery::Editing(Form { title: "Add x".into(), body: "text".into(), plan: plan() })
    }

    fn frame(
        ctx: &egui::Context,
        delivery: &mut Delivery,
        events: Vec<Event>,
    ) -> Option<DialogAction> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut answer = None;
        let _ = ctx.run_ui(raw, |ui| {
            answer = show_dialog(ui.ctx(), delivery);
        });
        answer
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn escape_cancels_the_form_and_cmd_enter_approves_it() {
        let ctx = egui::Context::default();
        let mut d = editing();
        assert_eq!(frame(&ctx, &mut d, vec![]), None);
        assert_eq!(
            frame(&ctx, &mut d, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Cancel)
        );
        let ctx = egui::Context::default();
        let mut d = editing();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(
            frame(&ctx, &mut d, vec![key(Key::Enter, Modifiers::COMMAND)]),
            Some(DialogAction::Approve)
        );
    }

    #[test]
    fn a_plain_enter_in_the_form_does_not_approve() {
        let ctx = egui::Context::default();
        let mut d = editing();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(frame(&ctx, &mut d, vec![key(Key::Enter, Modifiers::NONE)]), None);
    }

    #[test]
    fn escape_on_a_failure_goes_back_and_on_success_closes() {
        let mut failed = Delivery::Failed {
            form: Form { title: "t".into(), body: String::new(), plan: plan() },
            error: DeliveryError::GhMissing,
        };
        let ctx = egui::Context::default();
        frame(&ctx, &mut failed, vec![]);
        assert_eq!(
            frame(&ctx, &mut failed, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Back)
        );
        let mut done = Delivery::Delivered(Delivered {
            url: "https://github.com/o/r/pull/1".into(),
            created: true,
        });
        let ctx = egui::Context::default();
        frame(&ctx, &mut done, vec![]);
        assert_eq!(
            frame(&ctx, &mut done, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Close)
        );
    }

    #[test]
    fn a_delivery_in_flight_cannot_be_dismissed() {
        let mut d =
            Delivery::Delivering(Form { title: "t".into(), body: String::new(), plan: plan() });
        let ctx = egui::Context::default();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(frame(&ctx, &mut d, vec![key(Key::Escape, Modifiers::NONE)]), None);
    }
}
