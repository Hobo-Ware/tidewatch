//! Shared drawing pieces: headings, key caps, CLI badges, section labels.

use crate::brand;
use egui::{
    Align2, Color32, Context, CornerRadius, FontId, Id, Margin, Modal, ModalResponse, Painter,
    Rect, Response, Sense, Shadow, Stroke, StrokeKind, Ui, Vec2, vec2,
};
use kelp_ui::theme::semibold;

pub(crate) const BUTTON_H: f32 = 22.0;
const BUTTON_PAD: f32 = 10.0;
const BUTTON_RADIUS: u8 = 6;
const BUTTON_FONT: f32 = 12.5;
pub(crate) const DIALOG_HEADER_H: f32 = 48.0;
const DIALOG_PAD: i8 = 16;
const ROW_RADIUS: u8 = 8;
const BAR_W: f32 = 2.0;
const BAR_INSET: f32 = 7.0;

pub(crate) fn heading(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub(crate) fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

pub(crate) fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub(crate) fn badge_colors(agent_id: &str) -> (Color32, Color32) {
    match agent_id {
        "claude" => brand::CLAUDE_BADGE,
        "codex" => brand::CODEX_BADGE,
        "gemini" => brand::GEMINI_BADGE,
        "copilot" => brand::COPILOT_BADGE,
        _ => (brand::BUTTON, brand::MUTED),
    }
}

pub(crate) fn badge_letter(agent_id: &str) -> String {
    match agent_id {
        "claude" => "C".to_owned(),
        "codex" => "X".to_owned(),
        "gemini" => "G".to_owned(),
        "copilot" => "P".to_owned(),
        other => other.chars().next().map_or("?".to_owned(), |c| c.to_uppercase().to_string()),
    }
}

pub(crate) fn cli_short_name(agent_id: &str) -> String {
    let mut chars = agent_id.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

pub(crate) fn paint_cli_badge(painter: &Painter, rect: Rect, agent_id: &str) {
    let (fill, ink) = badge_colors(agent_id);
    painter.rect_filled(rect, CornerRadius::same((rect.width() * 0.28) as u8), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        badge_letter(agent_id),
        heading(rect.width() * 0.48),
        ink,
    );
}

pub(crate) fn paint_key_cap(painter: &Painter, anchor: egui::Pos2, text: &str, from_left: bool) {
    let galley = painter.layout_no_wrap(text.to_owned(), mono(12.0), brand::MUTED);
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let rect = if from_left {
        Rect::from_min_size(anchor - Vec2::new(0.0, size.y / 2.0), size)
    } else {
        Rect::from_min_size(anchor - Vec2::new(size.x, size.y / 2.0), size)
    };
    painter.rect_filled(rect, 6.0, brand::BUTTON);
    painter.galley(rect.center() - galley.size() / 2.0, galley, brand::MUTED);
}

pub(crate) fn section_label(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.5).color(color));
}

pub(crate) fn card_stroke(active: bool) -> Stroke {
    if active { Stroke::new(2.0, brand::TEAL) } else { Stroke::new(1.0, brand::CARD_LINE) }
}

pub(crate) fn clickable(
    ui: &mut Ui,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Response {
    ui.interact(rect, ui.id().with(salt), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonLook {
    Quiet,
    Danger,
    Primary,
}

fn button(ui: &mut Ui, label: &str, enabled: bool, look: ButtonLook) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), body(BUTTON_FONT), brand::SOFT);
    let size = vec2(galley.size().x + 2.0 * BUTTON_PAD, BUTTON_H);
    let (rect, response) =
        ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && response.hovered();
    let (fill, line, ink) = match (look, enabled) {
        (ButtonLook::Primary, true) => {
            let fill = if hot { brand::TEAL_TEXT } else { brand::TEAL };
            (fill, fill, brand::KEY_EDGE)
        }
        (ButtonLook::Primary, false) => (brand::BUTTON, brand::SEGMENT_LINE, brand::FAINT),
        (_, false) => (Color32::TRANSPARENT, brand::SEGMENT_LINE, brand::FAINT),
        (ButtonLook::Danger, true) => {
            (if hot { brand::RED_FILL } else { Color32::TRANSPARENT }, brand::RED_LINE, brand::RED)
        }
        (ButtonLook::Quiet, true) => {
            let line = if hot { brand::TEAL.gamma_multiply(0.5) } else { brand::FIELD_LINE };
            (Color32::TRANSPARENT, line, if hot { brand::TEXT } else { brand::SOFT })
        }
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(BUTTON_RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    painter.text(rect.center(), Align2::CENTER_CENTER, label, body(BUTTON_FONT), ink);
    kelp_ui::widgets::focus_ring(ui, &response, f32::from(BUTTON_RADIUS));
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// A bordered button with no fill: the default for every action that is not the one main action.
#[cfg_attr(not(test), allow(dead_code, reason = "adopted by the UI pass commits that follow"))]
pub(crate) fn quiet_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, enabled, ButtonLook::Quiet)
}

/// A quiet button in red for an action that destroys something.
#[cfg_attr(not(test), allow(dead_code, reason = "adopted by the UI pass commits that follow"))]
pub(crate) fn danger_button(ui: &mut Ui, label: &str) -> Response {
    button(ui, label, true, ButtonLook::Danger)
}

/// The one filled button of a view or dialog.
#[cfg_attr(not(test), allow(dead_code, reason = "adopted by the UI pass commits that follow"))]
pub(crate) fn primary_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    button(ui, label, enabled, ButtonLook::Primary)
}

/// A modal dialog: a card with a 48 px header (title, divider) over a padded body of `width`.
#[allow(dead_code, reason = "adopted by the UI pass commits that follow")]
pub(crate) fn dialog_frame<R>(
    ctx: &Context,
    id: Id,
    title: &str,
    width: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    let frame = egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::FIELD_LINE))
        .corner_radius(12)
        .shadow(Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(100),
        })
        .inner_margin(Margin::ZERO);
    Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (header, _) = ui.allocate_exact_size(vec2(width, DIALOG_HEADER_H), Sense::hover());
        let painter = ui.painter();
        painter.text(
            header.left_center() + vec2(f32::from(DIALOG_PAD), 0.0),
            Align2::LEFT_CENTER,
            title,
            heading(15.0),
            brand::TEXT,
        );
        painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, brand::CARD_LINE));
        egui::Frame::new().inner_margin(Margin::same(DIALOG_PAD)).show(ui, content).inner
    })
}

/// Paints a selectable row: a teal tint with a short teal bar on the left when selected, a fainter tint when hovered.
#[allow(dead_code, reason = "adopted by the UI pass commits that follow")]
pub(crate) fn selection_row(painter: &Painter, rect: Rect, selected: bool, hovered: bool) {
    let radius = CornerRadius::same(ROW_RADIUS);
    if selected {
        painter.rect_filled(rect, radius, brand::TEAL_NAV);
        let bar = Rect::from_min_max(
            rect.left_top() + vec2(0.0, BAR_INSET),
            rect.left_bottom() + vec2(BAR_W, -BAR_INSET),
        );
        painter.rect_filled(bar, CornerRadius::same(1), brand::TEAL);
    } else if hovered {
        painter.rect_filled(rect, radius, brand::TEAL_NAV.gamma_multiply(0.6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut add: impl FnMut(&mut Ui)) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        let _ = Context::default().run_ui(input, |ui| add(ui));
    }

    #[test]
    fn buttons_are_one_height_and_grow_with_their_label() {
        let mut sizes = Vec::new();
        run(|ui| {
            sizes.clear();
            for label in ["Ok", "A much longer label"] {
                sizes.push(quiet_button(ui, label, true).rect.size());
            }
            sizes.push(primary_button(ui, "Go", false).rect.size());
            sizes.push(danger_button(ui, "Delete").rect.size());
        });
        assert!(sizes.iter().all(|size| size.y == BUTTON_H));
        assert!(sizes[1].x > sizes[0].x);
    }

    #[test]
    fn a_disabled_button_is_never_clicked() {
        let mut clicked = false;
        run(|ui| clicked |= quiet_button(ui, "Send", false).clicked());
        assert!(!clicked);
    }

    #[test]
    fn badges_use_the_brand_hues_and_unknown_clis_fall_back_to_muted() {
        assert_eq!(badge_colors("claude"), brand::CLAUDE_BADGE);
        assert_eq!(badge_colors("gemini"), brand::GEMINI_BADGE);
        assert_eq!(badge_colors("other"), (brand::BUTTON, brand::MUTED));
        assert_eq!(badge_letter("copilot"), "P");
        assert_eq!(badge_letter("zed"), "Z");
    }
}
