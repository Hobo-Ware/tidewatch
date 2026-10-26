//! The delivery card and its dialog (PLAN.md, D7). The compact card sits in a band above the panes:
//! "N commits ready - Push and open PR". Opening it shows the commits and an editable title and text;
//! approving pushes and opens the pull request, and the dialog then shows the URL or the typed
//! failure with a plain fix. What each state looks like is in `delivery_card`.

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Id, Layout, Rect, RichText, ScrollArea, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::delivery::DeliveryError;

use crate::brand;
use crate::commands::{self, Command};
use crate::delivery_card::{Delivery, Form, card_text, fix_hint};
use crate::reader_view::{INSET, field_frame, item_frame, paint_item};
use crate::screen_kit::{Size, body, dialog_frame, footer_row, mono, primary_button, quiet_button};

pub(crate) const BAND_H: f32 = 40.0;
const DIALOG_WIDTH: f32 = 560.0;
const COMMITS_MAX_H: f32 = 110.0;
const BODY_ROWS: usize = 6;
const BODY_MAX_H: f32 = 220.0;

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
    let card = rect.shrink2(vec2(INSET, 4.0));
    paint_item(ui.painter(), card, false);
    ui.painter().circle_filled(pos2(card.left() + 16.0, card.center().y), 4.5, brand::GREEN);
    ui.painter().text(
        pos2(card.left() + 32.0, card.center().y),
        Align2::LEFT_CENTER,
        card_text(commits),
        body(13.0),
        brand::TEXT,
    );
    let button_rect = card.shrink2(vec2(10.0, 0.0));
    let label = if building { "Reading commits..." } else { "Push and open PR" };
    let mut action = None;
    ui.scope_builder(
        UiBuilder::new().max_rect(button_rect).layout(Layout::right_to_left(Align::Center)),
        |ui| {
            if primary_button(ui, label, !building, Size::Compact).clicked() {
                action = Some(CardAction::Open);
            }
        },
    );
    action
}

fn dialog_title(delivery: &Delivery) -> &'static str {
    match delivery {
        Delivery::Editing(_) => "Push and open a pull request",
        Delivery::Delivering(_) => "Pushing and opening the pull request...",
        Delivery::Failed { .. } => "The pull request was not opened",
        Delivery::Delivered(done) if done.created => "Pull request opened",
        Delivery::Delivered(_) => "Pull request updated",
        _ => "",
    }
}

/// The dialog for every state past the compact card. `None` while nothing was asked.
pub(crate) fn show_dialog(ctx: &egui::Context, delivery: &mut Delivery) -> Option<DialogAction> {
    let editing = matches!(delivery, Delivery::Editing(_));
    let title = dialog_title(delivery);
    let modal = dialog_frame(ctx, Id::new("delivery"), title, DIALOG_WIDTH, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        match delivery {
            Delivery::Editing(form) => editing_body(ui, form),
            Delivery::Delivering(form) => {
                branch_line(ui, form, None);
                None
            }
            Delivery::Failed { form, error } => failed_body(ui, form, error),
            Delivery::Delivered(done) => {
                let mut answer = None;
                if ui.link(&done.url).clicked() {
                    answer = Some(DialogAction::OpenUrl);
                }
                ui.add_space(4.0);
                footer_row(ui, "", |ui| {
                    if primary_button(ui, "Open in browser", true, Size::Regular).clicked() {
                        answer = Some(DialogAction::OpenUrl);
                    }
                    if quiet_button(ui, "Close", true, Size::Regular).clicked() {
                        answer = Some(DialogAction::Close);
                    }
                });
                answer
            }
            _ => None,
        }
    });
    let mut answer = modal.inner;
    if answer.is_none() && editing && commands::approve_delivery_pressed(ctx) {
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

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
}

/// The branch and its base in mono around an arrow, then `extra` in the body font.
fn branch_line(ui: &mut Ui, form: &Form, extra: Option<String>) {
    let mut job = LayoutJob::default();
    let code = |ink| TextFormat::simple(mono(12.0), ink);
    job.append(&form.plan.branch, 0.0, code(brand::SOFT));
    job.append("->", 6.0, TextFormat::simple(body(12.0), brand::MUTED));
    job.append(&form.plan.base, 6.0, code(brand::SOFT));
    if let Some(extra) = extra {
        job.append(&extra, 12.0, TextFormat::simple(body(12.0), brand::MUTED));
    }
    ui.label(job);
}

fn editing_body(ui: &mut Ui, form: &mut Form) -> Option<DialogAction> {
    let mut answer = None;
    let count = form.plan.commits.len();
    let noun = if count == 1 { "commit" } else { "commits" };
    branch_line(ui, form, Some(format!("{count} {noun}")));
    item_frame().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ScrollArea::vertical().max_height(COMMITS_MAX_H).id_salt("delivery-commits").show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                for commit in &form.plan.commits {
                    let mut job = LayoutJob::default();
                    job.append(&commit.sha, 0.0, TextFormat::simple(mono(11.5), brand::FAINT));
                    job.append(&commit.subject, 10.0, TextFormat::simple(body(12.5), brand::TEXT));
                    ui.add(egui::Label::new(job).truncate());
                }
            },
        );
    });
    ui.add_space(2.0);
    label(ui, "Title");
    field_frame(ui, |ui| {
        ui.add(
            TextEdit::singleline(&mut form.title)
                .frame(egui::Frame::NONE)
                .hint_text("Pull request title")
                .desired_width(f32::INFINITY)
                .font(body(14.0))
                .text_color(brand::TEXT),
        )
    });
    label(ui, "Text");
    field_frame(ui, |ui| {
        ScrollArea::vertical()
            .max_height(BODY_MAX_H)
            .id_salt("delivery-body")
            .show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut form.body)
                        .frame(egui::Frame::NONE)
                        .desired_rows(BODY_ROWS)
                        .desired_width(f32::INFINITY)
                        .font(mono(12.0))
                        .text_color(brand::TEXT),
                )
            })
            .inner
    });
    ui.add_space(4.0);
    let hint = format!("{} pushes and opens it", Command::ApproveDelivery.keys());
    footer_row(ui, &hint, |ui| {
        if primary_button(ui, "Push and open PR", !form.title.trim().is_empty(), Size::Regular)
            .clicked()
        {
            answer = Some(DialogAction::Approve);
        }
        if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
            answer = Some(DialogAction::Cancel);
        }
    });
    answer
}

fn failed_body(ui: &mut Ui, form: &Form, error: &DeliveryError) -> Option<DialogAction> {
    let mut answer = None;
    branch_line(ui, form, None);
    ui.label(RichText::new(error.to_string()).color(brand::RED));
    if let Some(hint) = fix_hint(error) {
        ui.label(RichText::new(hint).color(brand::TEXT));
    }
    ui.add_space(4.0);
    footer_row(ui, "", |ui| {
        if primary_button(ui, "Back to the text", true, Size::Regular).clicked() {
            answer = Some(DialogAction::Back);
        }
        if quiet_button(ui, "Close", true, Size::Regular).clicked() {
            answer = Some(DialogAction::Close);
        }
    });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, RawInput};
    use egui::{Key, Modifiers};
    use std::path::PathBuf;
    use tidewatch_core::delivery::{CommitLine, Delivered, DeliveryPlan, Opens};

    fn plan() -> DeliveryPlan {
        DeliveryPlan {
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            base: "main".into(),
            head: "agent/brave-otter".into(),
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
        Delivery::Editing(Form {
            title: "Add x".into(),
            body: "text".into(),
            plan: plan(),
            opens: Opens::Ready,
        })
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx
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
        let ctx = context();
        let mut d = editing();
        assert_eq!(frame(&ctx, &mut d, vec![]), None);
        assert_eq!(
            frame(&ctx, &mut d, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Cancel)
        );
        let ctx = context();
        let mut d = editing();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(
            frame(&ctx, &mut d, vec![commands::press_event(Command::ApproveDelivery).unwrap()]),
            Some(DialogAction::Approve)
        );
    }

    #[test]
    fn a_plain_enter_in_the_form_does_not_approve() {
        let ctx = context();
        let mut d = editing();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(frame(&ctx, &mut d, vec![key(Key::Enter, Modifiers::NONE)]), None);
    }

    #[test]
    fn escape_on_a_failure_goes_back_and_on_success_closes() {
        let mut failed = Delivery::Failed {
            form: Form {
                title: "t".into(),
                body: String::new(),
                plan: plan(),
                opens: Opens::Ready,
            },
            error: DeliveryError::GhMissing,
        };
        let ctx = context();
        frame(&ctx, &mut failed, vec![]);
        assert_eq!(
            frame(&ctx, &mut failed, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Back)
        );
        let mut done = Delivery::Delivered(Delivered {
            url: "https://github.com/o/r/pull/1".into(),
            head: "agent/brave-otter".into(),
            created: true,
        });
        let ctx = context();
        frame(&ctx, &mut done, vec![]);
        assert_eq!(
            frame(&ctx, &mut done, vec![key(Key::Escape, Modifiers::NONE)]),
            Some(DialogAction::Close)
        );
    }

    #[test]
    fn a_delivery_in_flight_cannot_be_dismissed() {
        let mut d = Delivery::Delivering(Form {
            title: "t".into(),
            body: String::new(),
            plan: plan(),
            opens: Opens::Ready,
        });
        let ctx = context();
        frame(&ctx, &mut d, vec![]);
        assert_eq!(frame(&ctx, &mut d, vec![key(Key::Escape, Modifiers::NONE)]), None);
    }
}
