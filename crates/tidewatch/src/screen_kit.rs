//! Shared drawing pieces: headings, key caps, CLI badges, section labels.

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Sense, Stroke, Ui, Vec2};
use kelp_ui::theme::semibold;
use stdusk_egui::colors;

pub(crate) const ORANGE: Color32 = Color32::from_rgb(0xfb, 0x92, 0x3c);

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
        "claude" => (Color32::from_rgb(0x3b, 0x24, 0x18), ORANGE),
        "codex" => (Color32::from_rgb(0x1e, 0x2a, 0x22), Color32::from_rgb(0x8f, 0xd1, 0x6a)),
        "gemini" => (Color32::from_rgb(0x1c, 0x27, 0x40), Color32::from_rgb(0x7a, 0xa2, 0xf7)),
        "copilot" => (Color32::from_rgb(0x2a, 0x23, 0x33), Color32::from_rgb(0xc4, 0xb5, 0xfd)),
        _ => (colors::hover_elevated(), colors::dim()),
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
    let galley = painter.layout_no_wrap(text.to_owned(), mono(12.0), colors::dim());
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let rect = if from_left {
        Rect::from_min_size(anchor - Vec2::new(0.0, size.y / 2.0), size)
    } else {
        Rect::from_min_size(anchor - Vec2::new(size.x, size.y / 2.0), size)
    };
    painter.rect_filled(rect, 6.0, colors::hover_elevated());
    painter.galley(rect.center() - galley.size() / 2.0, galley, colors::dim());
}

pub(crate) fn section_label(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.5).color(color));
}

pub(crate) fn card_stroke(active: bool) -> Stroke {
    if active { Stroke::new(2.0, colors::accent()) } else { Stroke::new(1.0, colors::border()) }
}

pub(crate) fn clickable(
    ui: &mut Ui,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Response {
    ui.interact(rect, ui.id().with(salt), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
