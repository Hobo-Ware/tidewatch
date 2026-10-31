//! Draws the Open a repo dialog: a GitHub link field, the folder a clone goes in, one line that
//! says what the link comes to, and while a clone runs a card with git's own progress line.

use std::path::Path;
use std::sync::atomic::Ordering;

use egui::{Align2, CornerRadius, Id, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use tidewatch_core::github_clone::Landing;

use crate::brand;
use crate::open_link::{Cloning, OpenLink, Phase, Verdict, fraction_of};
use crate::screen_kit::{
    DialogWidth, Size, body, dialog_frame, field_label, fit_path, footer_row, mono,
    paint_pill_card, path_line, primary_button_keyed, quiet_button,
};
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};
use crate::settings_ui::text_input;
use crate::welcome::shown_path;

const CARD_H: f32 = 92.0;
const BAR_H: f32 = 4.0;
const SECTION_GAP: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Close,
    Browse,
    Go,
    Back,
}

pub(crate) fn primary_label(verdict: &Verdict) -> Option<&'static str> {
    match verdict {
        Verdict::Ready(_, Landing::Open(_)) => Some("Open"),
        Verdict::Ready(_, Landing::Clone { .. }) => Some("Clone and open"),
        _ => None,
    }
}

pub(crate) fn show(
    ctx: &egui::Context,
    open: &mut OpenLink,
    home: Option<&Path>,
) -> Option<Outcome> {
    let mut outcome = None;
    let verdict = open.verdict();
    let modal = dialog_frame(ctx, Id::new("open_link"), "Open a repo", DialogWidth::Medium, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let cloning = match &open.phase {
            Phase::Cloning(cloning) => Some(cloning.clone()),
            Phase::Form | Phase::Failed(_) => None,
        };
        if let Some(cloning) = cloning {
            outcome = cloning_body(ui, &cloning, home);
        } else if let Phase::Failed(message) = &open.phase {
            outcome = failed_body(ui, message);
        } else {
            outcome = form_body(ui, open, &verdict, home);
        }
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

fn form_body(
    ui: &mut Ui,
    open: &mut OpenLink,
    verdict: &Verdict,
    home: Option<&Path>,
) -> Option<Outcome> {
    let mut outcome = None;
    field_label(ui, "GitHub link");
    let link = text_input(ui, &mut open.link, "https://github.com/owner/repo");
    if std::mem::take(&mut open.focus) {
        link.request_focus();
    }
    let mut entered = link.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    ui.add_space(2.0);
    match verdict {
        Verdict::Idle => {
            note(ui, "Paste a repo, clone address or pull request link.", brand::MUTED);
        }
        Verdict::NotALink => note(ui, "That is not a link to a repo on GitHub.", brand::RED),
        Verdict::Checking(repo) => {
            note(ui, &format!("Looking for {} on this machine...", repo.slug()), brand::MUTED);
        }
        Verdict::Ready(repo, Landing::Open(dir)) => {
            known_card(ui, &format!("{} is already at ", repo.slug()), &shown_path(dir, home));
        }
        Verdict::Ready(_, Landing::Clone { .. }) => {}
        Verdict::Ready(_, Landing::Refuse(why)) => note(ui, why, brand::RED),
    }
    let opens_known = matches!(verdict, Verdict::Ready(_, Landing::Open(_)));
    if open.repo().is_some() && !opens_known {
        ui.add_space(SECTION_GAP - 8.0);
        field_label(ui, "Clone into");
        let parent = text_input(ui, &mut open.parent, "Folder for the new checkout");
        entered |= parent.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if let Verdict::Ready(_, Landing::Clone { target }) = verdict {
            path_line(
                ui,
                "Clones to ",
                &shown_path(target, home),
                &target.display().to_string(),
                body(TEXT_TAG),
                brand::MUTED,
            );
        }
    }
    ui.add_space(6.0);
    let label = primary_label(verdict);
    footer_row(ui, &[("Enter", "opens"), ("Esc", "cancels")], |ui| {
        if primary_button_keyed(
            ui,
            label.unwrap_or("Open"),
            "\u{21b5}",
            label.is_some(),
            Size::Regular,
        )
        .clicked()
        {
            outcome = Some(Outcome::Go);
        }
        if quiet_button(ui, "Browse folders...", true, Size::Regular)
            .on_hover_text("A folder you pick opens as it is")
            .clicked()
        {
            outcome = Some(Outcome::Browse);
        }
        if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
            outcome = Some(Outcome::Close);
        }
    });
    if entered && label.is_some() && outcome.is_none() {
        outcome = Some(Outcome::Go);
    }
    outcome
}

fn failed_body(ui: &mut Ui, message: &str) -> Option<Outcome> {
    let mut outcome = None;
    ui.label(RichText::new("The clone did not finish.").font(body(14.0)).color(brand::TEXT));
    ui.label(RichText::new(message).font(mono(TEXT_SECONDARY)).color(brand::RED));
    ui.add_space(6.0);
    footer_row(ui, &[], |ui| {
        if quiet_button(ui, "Back", true, Size::Regular).clicked() {
            outcome = Some(Outcome::Back);
        }
        if quiet_button(ui, "Close", true, Size::Regular).clicked() {
            outcome = Some(Outcome::Close);
        }
    });
    outcome
}

fn cloning_body(ui: &mut Ui, cloning: &Cloning, home: Option<&Path>) -> Option<Outcome> {
    let mut outcome = None;
    let line = cloning.line.lock().map(|line| line.clone()).unwrap_or_default();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CARD_H), Sense::hover());
    paint_pill_card(ui.painter(), rect);
    let inner = rect.shrink2(vec2(16.0, 14.0));
    let painter = ui.painter();
    painter.text(
        inner.left_top(),
        Align2::LEFT_TOP,
        format!("Cloning {}", cloning.repo.slug()),
        crate::screen_kit::heading(crate::screen_kit::TEXT_TITLE),
        brand::TEXT,
    );
    painter.text(
        inner.left_top() + vec2(0.0, 22.0),
        Align2::LEFT_TOP,
        fit_path(painter, "", &shown_path(&cloning.target, home), &mono(TEXT_TAG), inner.width()),
        mono(TEXT_TAG),
        brand::FAINT,
    );
    let shown = if line.is_empty() { "Starting git...".to_owned() } else { line };
    painter.text(
        inner.left_top() + vec2(0.0, 42.0),
        Align2::LEFT_TOP,
        shown.clone(),
        mono(TEXT_SECONDARY),
        brand::SOFT,
    );
    let track = Rect::from_min_size(
        egui::pos2(inner.left(), inner.bottom() - BAR_H),
        vec2(inner.width(), BAR_H),
    );
    painter.rect_filled(track, CornerRadius::same(2), brand::SEGMENT);
    let fill = if let Some(fraction) = fraction_of(&shown) {
        Rect::from_min_size(track.min, vec2(track.width() * fraction, BAR_H))
    } else {
        let t = ui.input(|i| i.time) as f32;
        let width = track.width() * 0.25;
        let x = track.left() + (track.width() - width) * (0.5 + 0.5 * (t * 2.0).sin());
        Rect::from_min_size(egui::pos2(x, track.top()), vec2(width, BAR_H))
    };
    ui.painter().rect_filled(fill, CornerRadius::same(2), brand::TEAL);
    ui.painter().rect_stroke(
        track,
        CornerRadius::same(2),
        Stroke::new(1.0, brand::SEGMENT_LINE),
        StrokeKind::Outside,
    );
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    ui.add_space(6.0);
    footer_row(ui, &[], |ui| {
        if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
            cloning.cancel.store(true, Ordering::SeqCst);
            outcome = Some(Outcome::Close);
        }
    });
    outcome
}

fn note(ui: &mut Ui, text: &str, color: egui::Color32) {
    ui.label(RichText::new(text).size(12.0).color(color));
}

fn known_card(ui: &mut Ui, prefix: &str, path: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    paint_pill_card(ui.painter(), rect);
    let font = body(TEXT_SECONDARY);
    let text = fit_path(ui.painter(), prefix, path, &font, rect.width() - 28.0);
    ui.painter().text(
        rect.left_center() + vec2(14.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        font,
        brand::SOFT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_kit::painted_texts;
    use egui::pos2;

    #[test]
    fn the_buttons_share_a_row_with_the_hint_instead_of_sitting_over_it() {
        let ctx = crate::fonts::test_context();
        let mut open = OpenLink::open(Vec::new(), None, Path::new("/home/u"));
        let mut out = None;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 640.0))),
                focused: true,
                ..Default::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                let _ = show(ui.ctx(), &mut open, Some(Path::new("/home/u")));
            }));
        }
        let texts = painted_texts(&out.unwrap());
        let at = |name: &str| texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect);
        let (hint, cancel) = (at("cancels").unwrap(), at("Cancel").unwrap());
        assert!((hint.center().y - cancel.center().y).abs() < 2.0, "{hint:?} vs {cancel:?}");
        assert!(hint.right() < cancel.left());
        assert_eq!(texts.iter().filter(|(text, _)| text.contains(" - ")).count(), 0);
    }
}
