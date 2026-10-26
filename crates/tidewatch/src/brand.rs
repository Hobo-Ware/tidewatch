//! The Night tide palette: the one set of colors every Tidewatch view reads, and how it is put on egui.

use egui::{Color32, Stroke};
use stdusk_egui::colors::{self, Chrome, Theme};

pub(crate) const GROUND: Color32 = Color32::from_rgb(0x10, 0x13, 0x1a);
pub(crate) const SIDEBAR: Color32 = Color32::from_rgb(0x0d, 0x10, 0x16);
pub(crate) const SIDEBAR_LINE: Color32 = Color32::from_rgb(0x1f, 0x24, 0x30);
pub(crate) const CARD: Color32 = Color32::from_rgb(0x16, 0x1b, 0x24);
pub(crate) const CARD_LINE: Color32 = Color32::from_rgb(0x22, 0x29, 0x37);
pub(crate) const ITEM: Color32 = Color32::from_rgb(0x1a, 0x20, 0x29);
pub(crate) const ITEM_LINE: Color32 = Color32::from_rgb(0x26, 0x2e, 0x3b);
pub(crate) const BAR: Color32 = Color32::from_rgb(0x12, 0x16, 0x1e);
pub(crate) const ANSWER_LINE: Color32 = Color32::from_rgb(0x33, 0x3c, 0x4b);
pub(crate) const FIELD_LINE: Color32 = Color32::from_rgb(0x2f, 0x37, 0x47);
pub(crate) const SEGMENT: Color32 = Color32::from_rgb(0x0f, 0x12, 0x19);
pub(crate) const SEGMENT_LINE: Color32 = Color32::from_rgb(0x26, 0x2d, 0x39);
pub(crate) const SEGMENT_ON: Color32 = Color32::from_rgb(0x1f, 0x2a, 0x31);
pub(crate) const BUTTON: Color32 = Color32::from_rgb(0x1d, 0x22, 0x30);
pub(crate) const KEY_EDGE: Color32 = Color32::from_rgb(0x0b, 0x0e, 0x13);
pub(crate) const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xec);
pub(crate) const SOFT: Color32 = Color32::from_rgb(0xa7, 0xb0, 0xbf);
pub(crate) const STEP_TEXT: Color32 = Color32::from_rgb(0xc4, 0xca, 0xd4);
pub(crate) const MUTED: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa3);
pub(crate) const FAINT: Color32 = Color32::from_rgb(0x6b, 0x73, 0x83);
pub(crate) const TEAL: Color32 = Color32::from_rgb(0x2d, 0xd4, 0xbf);
pub(crate) const TEAL_TEXT: Color32 = Color32::from_rgb(0x5e, 0xea, 0xd4);
pub(crate) const TEAL_NAV: Color32 = Color32::from_rgb(0x16, 0x25, 0x2b);
pub(crate) const GREEN: Color32 = Color32::from_rgb(0x8f, 0xd1, 0x6a);
pub(crate) const ORANGE: Color32 = Color32::from_rgb(0xfb, 0x92, 0x3c);
pub(crate) const ORANGE_SOFT: Color32 = Color32::from_rgb(0xd6, 0xa7, 0x7a);
pub(crate) const BLUE: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);
pub(crate) const BLUE_TEXT: Color32 = Color32::from_rgb(0x93, 0xc5, 0xfd);
pub(crate) const AMBER: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24);
pub(crate) const RED: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
pub(crate) const RED_LINE: Color32 = Color32::from_rgb(0x3a, 0x25, 0x30);
pub(crate) const RED_FILL: Color32 = Color32::from_rgb(0x24, 0x17, 0x1e);

pub(crate) const NEEDS_FILL: Color32 = Color32::from_rgb(0x16, 0x13, 0x0f);
pub(crate) const VIOLET_TEXT: Color32 = Color32::from_rgb(0xc4, 0xb5, 0xfd);
pub(crate) const REPO_INKS: [Color32; 7] = [
    Color32::from_rgb(0xf4, 0x72, 0xb6),
    BLUE,
    Color32::from_rgb(0xa3, 0xe6, 0x35),
    AMBER,
    Color32::from_rgb(0xc0, 0x84, 0xfc),
    Color32::from_rgb(0x22, 0xd3, 0xee),
    Color32::from_rgb(0xfb, 0x71, 0x85),
];

pub(crate) const STRUGGLE_FILL: Color32 = Color32::from_rgb(0x1c, 0x16, 0x10);
pub(crate) const STRUGGLE_LINE: Color32 = Color32::from_rgb(0x4a, 0x34, 0x18);
pub(crate) const SUNKEN: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1c);

/// Each CLI's badge, as its fill and its letter's ink: the one place a hue outside the palette lives.
pub(crate) const CLAUDE_BADGE: (Color32, Color32) = (Color32::from_rgb(0x3b, 0x24, 0x18), ORANGE);
pub(crate) const CODEX_BADGE: (Color32, Color32) = (Color32::from_rgb(0x1e, 0x2a, 0x22), GREEN);
pub(crate) const GEMINI_BADGE: (Color32, Color32) =
    (Color32::from_rgb(0x1c, 0x27, 0x40), Color32::from_rgb(0x7a, 0xa2, 0xf7));
pub(crate) const COPILOT_BADGE: (Color32, Color32) =
    (Color32::from_rgb(0x2a, 0x23, 0x33), Color32::from_rgb(0xc4, 0xb5, 0xfd));

/// Agent status inks. They differ in lightness as well as hue, darkest to lightest failed, needs
/// you, working, done, so a color-blind reader still tells them apart.
pub(crate) const WORKING: Color32 = TEAL;
pub(crate) const NEEDS: Color32 = ORANGE;
pub(crate) const DONE: Color32 = Color32::from_rgb(0xb2, 0xe6, 0x8a);
pub(crate) const FAILED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);

pub(crate) fn theme() -> Theme {
    let mut theme = colors::one_half_dark();
    theme.bg = GROUND;
    theme.fg = TEXT;
    theme.cursor = TEAL;
    theme
}

pub(crate) fn chrome() -> Chrome {
    Chrome { accent: Some(TEAL), red: Some(RED), green: Some(GREEN), yellow: None }
}

/// Teal selection, links and active widgets on top of what stdusk derived from the theme.
pub(crate) fn tint_visuals(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.selection.bg_fill = TEAL.gamma_multiply(0.3);
        v.selection.stroke = Stroke::new(1.0, TEAL);
        v.hyperlink_color = TEAL_TEXT;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, TEAL.gamma_multiply(0.5));
        v.widgets.active.bg_stroke = Stroke::new(1.0, TEAL);
        style.spacing.scroll = scroll_style();
        style.interaction.selectable_labels = true;
    });
}

pub(crate) const SCROLL_GAP: f32 = 12.0;
const SCROLL_EDGE: f32 = 2.0;
const SCROLL_BAR_W: f32 = 8.0;
pub(crate) const SCROLL_ROOM: f32 = SCROLL_GAP + SCROLL_BAR_W + SCROLL_EDGE;

fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: SCROLL_BAR_W,
        bar_outer_margin: SCROLL_EDGE,
        floating_width: 3.0,
        floating_allocated_width: SCROLL_ROOM,
        dormant_handle_opacity: 0.22,
        active_handle_opacity: 0.5,
        interact_handle_opacity: 0.8,
        interact_background_opacity: 0.15,
        ..egui::style::ScrollStyle::floating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a) + 0.05, luminance(b) + 0.05);
        a.max(b) / a.min(b)
    }

    #[test]
    fn status_inks_differ_in_lightness_and_read_on_the_ground() {
        let ramp = [FAILED, NEEDS, WORKING, DONE];
        for pair in ramp.windows(2) {
            assert!(contrast(pair[0], pair[1]) >= 1.2, "{:?} against {:?}", pair[0], pair[1]);
        }
        for ink in ramp {
            assert!(contrast(ink, GROUND) >= 4.5, "{ink:?} on the ground");
        }
    }

    #[test]
    fn the_theme_makes_stdusk_read_teal_for_its_accent() {
        let theme = theme();
        assert_eq!((theme.bg, theme.ansi), (GROUND, colors::one_half_dark().ansi));
        assert_eq!(chrome().accent, Some(TEAL));
    }

    #[test]
    fn tinting_sets_selection_links_and_active_widgets() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let v = ctx.global_style().visuals.clone();
        assert_eq!(v.selection.stroke.color, TEAL);
        assert_eq!(v.hyperlink_color, TEAL_TEXT);
        assert_eq!(v.widgets.active.bg_stroke.color, TEAL);
    }

    #[test]
    fn scroll_bars_leave_a_gap_and_take_their_own_room() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let scroll = ctx.global_style().spacing.scroll;
        assert_eq!(scroll.allocated_width(), SCROLL_ROOM);
        assert!(SCROLL_ROOM - SCROLL_EDGE - scroll.bar_width >= SCROLL_GAP);
    }

    #[test]
    fn a_scroll_area_keeps_its_content_clear_of_the_bar() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 200.0),
            )),
            ..egui::RawInput::default()
        };
        let mut widths = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run_ui(raw.clone(), |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    widths.push(ui.available_width());
                    ui.allocate_space(egui::vec2(10.0, 1000.0));
                });
            });
        }
        assert_eq!(widths.last().copied(), Some(400.0 - SCROLL_ROOM));
    }

    #[test]
    fn labels_are_selectable_by_default() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        assert!(ctx.global_style().interaction.selectable_labels);
    }
}
