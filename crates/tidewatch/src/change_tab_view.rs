//! Draws the "Change this tab" popover under the tab strip: Kind, a grid of profile cards, Level,
//! the sentence that says what will happen, and the footer. Enter changes, Esc closes. Every
//! choice comes from `change_tab`, which also says which are refused and why.

use egui::{Align2, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, UiKind, pos2, vec2};
use tidewatch_core::agents::Level;

use crate::brand;
use crate::change_tab::{
    Card, Facts, Kind, Pick, blocked, card_refusal, kind_refusal, level_refusal, offers_stop_now,
    sentence,
};
use crate::screen_kit::{
    Size, TEXT_SECONDARY, TEXT_TAG, body, dialog_frame_in, footer_row, heading,
    primary_button_keyed, quiet_button,
};
use crate::segmented;

pub(crate) const WIDTH: f32 = 460.0;
pub(crate) const TOP: f32 = 60.0;
const CARD_H: f32 = 48.0;
const GAP: f32 = 8.0;
const LABEL_GAP: f32 = 6.0;
const SECTION_GAP: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Close,
    Change { interrupt: bool },
}

fn label(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), body(TEXT_TAG), brand::MUTED);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, brand::MUTED);
    ui.add_space(LABEL_GAP);
}

fn kind_row(ui: &mut Ui, facts: &Facts, pick: &mut Pick) {
    label(ui, "Kind");
    let labels = Kind::ALL.map(Kind::label);
    let refusals = Kind::ALL.map(|kind| kind_refusal(facts, kind).map(str::to_owned));
    let active = Kind::ALL.iter().position(|kind| *kind == pick.kind).unwrap_or(0);
    let clicked =
        segmented::inline_or_chips(ui, Id::new("change_tab_kind"), &labels, &[], &refusals, active);
    if let Some(index) = clicked {
        pick.pick_kind(facts, Kind::ALL[index]);
    }
}

fn card(ui: &mut Ui, rect: Rect, card: &Card, selected: bool, refusal: Option<&str>) -> bool {
    let response = ui.interact(rect, Id::new(("change_tab_card", &card.id)), Sense::click());
    let painter = ui.painter();
    let line = match (selected, response.hovered() && refusal.is_none()) {
        (true, _) => brand::TEAL,
        (false, true) => brand::FAINT,
        (false, false) => brand::FIELD_LINE,
    };
    painter.rect(
        rect,
        egui::CornerRadius::same(Size::Compact.radius()),
        egui::Color32::TRANSPARENT,
        Stroke::new(if selected { 1.5 } else { 1.0 }, line),
        StrokeKind::Inside,
    );
    let (name, detail) = match refusal {
        Some(_) => (brand::FAINT, brand::FAINT),
        None => (brand::TEXT, brand::MUTED),
    };
    let inset = rect.left() + Size::Compact.pad();
    let clip = painter.with_clip_rect(rect.shrink2(vec2(Size::Compact.pad() / 2.0, 0.0)));
    clip.text(
        pos2(inset, rect.top() + 15.0),
        Align2::LEFT_CENTER,
        &card.name,
        heading(TEXT_SECONDARY),
        name,
    );
    clip.text(
        pos2(inset, rect.top() + 33.0),
        Align2::LEFT_CENTER,
        &card.detail,
        body(TEXT_TAG),
        detail,
    );
    let response = match refusal {
        Some(reason) => {
            response.on_hover_text(reason).on_hover_cursor(egui::CursorIcon::NotAllowed)
        }
        None => response.on_hover_cursor(egui::CursorIcon::PointingHand),
    };
    response.clicked()
}

fn profile_grid(ui: &mut Ui, facts: &Facts, pick: &mut Pick) {
    label(ui, "Profile");
    let width = ui.available_width();
    let columns = 2;
    let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    let rows = facts.cards.len().div_ceil(columns);
    let height = rows as f32 * CARD_H + rows.saturating_sub(1) as f32 * GAP;
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut picked = None;
    for (index, profile) in facts.cards.iter().enumerate() {
        let (row, column) = (index / columns, index % columns);
        let at = pos2(
            area.left() + column as f32 * (card_w + GAP),
            area.top() + row as f32 * (CARD_H + GAP),
        );
        let rect = Rect::from_min_size(at, vec2(card_w, CARD_H));
        let refusal = card_refusal(facts, profile);
        if card(ui, rect, profile, profile.id == pick.profile, refusal.as_deref())
            && refusal.is_none()
        {
            picked = Some(profile.id.clone());
        }
    }
    if let Some(id) = picked {
        pick.pick_profile(facts, &id);
    }
}

fn level_row(ui: &mut Ui, facts: &Facts, pick: &mut Pick) {
    label(ui, "Level");
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::RED));
    let refusals = Level::ALL.map(|level| {
        facts
            .cards
            .iter()
            .find(|card| card.id == pick.profile)
            .and_then(|card| level_refusal(facts, card, level))
    });
    let active = Level::ALL.iter().position(|level| *level == pick.level).unwrap_or(0);
    let clicked = segmented::inline_or_chips(
        ui,
        Id::new("change_tab_level"),
        &labels,
        &tones,
        &refusals,
        active,
    );
    if let Some(index) = clicked {
        pick.pick_level(facts, Level::ALL[index]);
    }
}

fn what_happens(ui: &mut Ui, text: &str) {
    let width = ui.available_width();
    let inner = width - 2.0 * 12.0;
    let galley = ui.painter().layout(text.to_owned(), body(TEXT_SECONDARY), brand::SOFT, inner);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 20.0), Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(8),
        brand::SUNKEN,
        Stroke::new(1.0, brand::SEGMENT_LINE),
        StrokeKind::Inside,
    );
    ui.painter().galley(rect.min + vec2(12.0, 10.0), galley, brand::SOFT);
}

pub(crate) fn show(ctx: &egui::Context, facts: &Facts, pick: &mut Pick) -> Option<Outcome> {
    let id = Id::new("change_tab");
    let area = egui::Area::new(id)
        .kind(UiKind::Modal)
        .sense(Sense::hover())
        .anchor(Align2::CENTER_TOP, vec2(0.0, TOP))
        .order(egui::Order::Foreground)
        .interactable(true);
    let mut outcome = None;
    let modal = dialog_frame_in(ctx, (id, area), "Change this tab", WIDTH, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        kind_row(ui, facts, pick);
        if pick.kind == Kind::Agent {
            ui.add_space(SECTION_GAP);
            profile_grid(ui, facts, pick);
            ui.add_space(SECTION_GAP);
            level_row(ui, facts, pick);
        }
        ui.add_space(SECTION_GAP);
        let reason = blocked(facts, pick);
        what_happens(ui, &reason.clone().unwrap_or_else(|| sentence(facts, pick)));
        ui.add_space(SECTION_GAP);
        footer_row(ui, "Esc cancels", |ui| {
            if primary_button_keyed(ui, "Change", "\u{21b5}", reason.is_none(), Size::Regular)
                .clicked()
            {
                outcome = Some(Outcome::Change { interrupt: false });
            }
            if offers_stop_now(facts, pick)
                && quiet_button(ui, "Stop turn and switch now", true, Size::Regular).clicked()
            {
                outcome = Some(Outcome::Change { interrupt: true });
            }
        });
    });
    let enter = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    if outcome.is_none() && enter && blocked(facts, pick).is_none() {
        outcome = Some(Outcome::Change { interrupt: false });
    }
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

#[cfg(test)]
mod tests;
