//! A message that stays until it is dismissed: what a restore could not bring back. A toast fades
//! on its own, which is too quick for a list of tabs the user has to act on.

use egui::{Align2, Area, Id, Order, RichText, vec2};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, overlay_frame};

const MAX_WIDTH: f32 = 560.0;
const BOTTOM_GAP: f32 = 64.0;

/// Draws the notice above the toast line. True when Dismiss was clicked.
pub(crate) fn show(ctx: &egui::Context, text: &str) -> bool {
    draw(ctx, text).0
}

fn draw(ctx: &egui::Context, text: &str) -> (bool, egui::Rect) {
    let mut dismissed = false;
    let area = Area::new(Id::new("restore_notice"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -BOTTOM_GAP))
        .show(ctx, |ui| {
            overlay_frame().show(ui, |ui| {
                ui.set_max_width(MAX_WIDTH);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.add(egui::Label::new(RichText::new(text).color(colors::fg())).wrap());
                    dismissed = action_button(ui, "Dismiss", false).clicked();
                });
            });
        });
    (dismissed, area.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, text: &str) -> (bool, egui::Rect) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(900.0, 600.0))),
            ..Default::default()
        };
        let mut dismissed = false;
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run_ui(raw, |ui| (dismissed, rect) = draw(ui.ctx(), text));
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
