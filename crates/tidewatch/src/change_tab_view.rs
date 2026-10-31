//! Draws the "Change this tab" popover under the tab strip: Kind, a grid of profile cards, Level,
//! the sentence that says what will happen, and the footer. Enter changes, Esc closes. Every
//! choice comes from `change_tab`; a missing CLI turns Change into the button that sets it up.

use egui::{Align2, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, UiKind, pos2, vec2};
use tidewatch_core::agents::Level;

use crate::brand;
use crate::change_tab::{Card, Facts, Kind, Pick, fix, offers_stop_now, sentence};
use crate::screen_kit::{
    DialogWidth, Size, TEXT_SECONDARY, TEXT_TAG, body, dialog_frame_in, field_label, footer_row,
    heading, primary_button_keyed, quiet_button,
};
use crate::segmented;

pub(crate) const TOP: f32 = 60.0;
const CARD_H: f32 = 48.0;
const GAP: f32 = 8.0;
const LABEL_GAP: f32 = 6.0;
const SECTION_GAP: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Close,
    Change {
        interrupt: bool,
    },
    /// The CLI or profile the pick needs is missing: open Settings, Profiles.
    SetUp,
}

fn label(ui: &mut Ui, text: &str) {
    field_label(ui, text);
    ui.add_space(LABEL_GAP);
}

fn kind_row(ui: &mut Ui, pick: &mut Pick) {
    label(ui, "Kind");
    let labels = Kind::ALL.map(Kind::label);
    let refusals = Kind::ALL.map(|_| None);
    let active = Kind::ALL.iter().position(|kind| *kind == pick.kind).unwrap_or(0);
    let clicked =
        segmented::inline_or_chips(ui, Id::new("change_tab_kind"), &labels, &[], &refusals, active);
    if let Some(index) = clicked {
        pick.pick_kind(Kind::ALL[index]);
    }
}

fn card(ui: &mut Ui, rect: Rect, card: &Card, selected: bool) -> bool {
    let response = ui.interact(rect, Id::new(("change_tab_card", &card.id)), Sense::click());
    let painter = ui.painter();
    let line = match (selected, response.hovered()) {
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
    let (name, detail) =
        if card.installed { (brand::TEXT, brand::MUTED) } else { (brand::FAINT, brand::FAINT) };
    let detail_text = if card.installed { card.detail.as_str() } else { "Not installed" };
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
        detail_text,
        body(TEXT_TAG),
        detail,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
        if card(ui, rect, profile, profile.id == pick.profile) {
            picked = Some(profile.id.clone());
        }
    }
    if let Some(id) = picked {
        pick.pick_profile(facts, &id);
    }
}

fn level_row(ui: &mut Ui, pick: &mut Pick) {
    label(ui, "Level");
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::RED));
    let refusals = Level::ALL.map(|_| None);
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
        pick.pick_level(Level::ALL[index]);
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
    let modal = dialog_frame_in(ctx, (id, area), "Change this tab", DialogWidth::Narrow, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        kind_row(ui, pick);
        if pick.kind == Kind::Agent {
            ui.add_space(SECTION_GAP);
            profile_grid(ui, facts, pick);
            ui.add_space(SECTION_GAP);
            level_row(ui, pick);
        }
        ui.add_space(SECTION_GAP);
        let missing = fix(facts, pick);
        what_happens(ui, &sentence(facts, pick));
        ui.add_space(SECTION_GAP);
        footer_row(ui, &[("Esc", "cancels")], |ui| {
            let label = missing
                .as_ref()
                .map_or_else(|| "Change".to_owned(), crate::change_tab::Fix::button);
            if primary_button_keyed(ui, &label, "\u{21b5}", true, Size::Regular).clicked() {
                outcome = Some(if missing.is_some() {
                    Outcome::SetUp
                } else {
                    Outcome::Change { interrupt: false }
                });
            }
            if offers_stop_now(facts, pick)
                && quiet_button(ui, "Stop turn and switch now", true, Size::Regular).clicked()
            {
                outcome = Some(Outcome::Change { interrupt: true });
            }
        });
    });
    let enter = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    if outcome.is_none() && enter {
        outcome = Some(if fix(facts, pick).is_some() {
            Outcome::SetUp
        } else {
            Outcome::Change { interrupt: false }
        });
    }
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

#[cfg(test)]
mod tests;
