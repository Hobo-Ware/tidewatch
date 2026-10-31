//! A message that stays until it is dismissed: what a restore could not bring back. A toast fades
//! on its own, which is too quick for a list of tabs the user has to act on.

use egui::{Margin, Order, Rect, RichText, Stroke, pos2, vec2};

use crate::brand;
use crate::screen_kit::{Size, quiet_button};

const MAX_WIDTH: f32 = 560.0;
const BOTTOM_GAP: f32 = 64.0;
const BAR_W: f32 = 2.0;

/// Draws the notice above the toast line, centered on `column`. True when Dismiss was clicked.
pub(crate) fn show(ctx: &egui::Context, text: &str, column: Rect) -> bool {
    draw(ctx, text, column).0
}

fn draw(ctx: &egui::Context, text: &str, column: Rect) -> (bool, Rect) {
    let mut dismissed = false;
    let area = egui::Area::new(egui::Id::new("restore_notice"))
        .order(Order::Foreground)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(column.center().x, column.bottom() - BOTTOM_GAP))
        .show(ctx, |ui| {
            let card = egui::Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(8)
                .inner_margin(Margin { left: 16, right: 10, top: 10, bottom: 10 })
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(100),
                })
                .show(ui, |ui| {
                    ui.set_max_width(MAX_WIDTH);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        ui.add(egui::Label::new(RichText::new(text).color(brand::TEXT)).wrap());
                        dismissed = quiet_button(ui, "Dismiss", true, Size::Compact).clicked();
                    });
                });
            let rect = card.response.rect;
            let bar = Rect::from_min_max(
                rect.left_top() + vec2(0.0, 7.0),
                rect.left_bottom() + vec2(BAR_W, -7.0),
            );
            ui.painter().rect_filled(bar, 1, brand::ORANGE);
        });
    (dismissed, area.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, text: &str) -> (bool, egui::Rect) {
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(900.0, 600.0));
        let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut dismissed = false;
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run_ui(raw, |ui| (dismissed, rect) = draw(ui.ctx(), text, screen));
        (dismissed, rect)
    }

    #[test]
    fn a_notice_draws_above_the_bottom_edge_and_is_not_dismissed_by_itself() {
        let ctx = egui::Context::default();
        let long = "Could not reopen 2 tabs: calm-comet (worktree folder gone), brave-otter (worktree folder gone)";
        let (dismissed, used) = frame(&ctx, long);
        assert!(!dismissed);
        let (_, again) = frame(&ctx, long);
        assert!(again.width() > 100.0 && again.width() <= MAX_WIDTH + 140.0, "{again:?}");
        assert!(again.bottom() <= 600.0 - BOTTOM_GAP + 1.0, "{again:?} / {used:?}");
    }
}
