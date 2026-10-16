//! Draws the leftover worktrees: the chip in the tab strip and the dialog it opens, with what
//! each worktree would lose, Clean up for the clean ones and an explicit Discard for the rest.

use std::path::PathBuf;

use egui::{Id, RichText, ScrollArea};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, overlay_frame};
use tidewatch_core::worktree::Leftover;

use crate::brand;
use crate::leftovers::{Cleanup, Row, RowState, status_text};
use crate::screen_kit::quiet_button;

const DIALOG_WIDTH: f32 = 560.0;
const LIST_HEIGHT: f32 = 280.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Close,
    CleanUp,
    Discard(PathBuf),
}

/// The hover text of the chip: where each leftover is.
pub(crate) fn tip(leftovers: &[Leftover]) -> String {
    let mut lines =
        vec!["Worktrees from earlier runs with no tab. Click to clean them up:".to_owned()];
    lines.extend(leftovers.iter().map(|l| match &l.branch {
        Some(branch) => format!("{}  ({branch})", l.path.display()),
        None => l.path.display().to_string(),
    }));
    lines.join("\n")
}

/// "N leftover" in the tab strip. True when clicked.
pub(crate) fn chip(ui: &mut egui::Ui, leftovers: &[Leftover]) -> bool {
    quiet_button(ui, &format!("{} leftover", leftovers.len()), true)
        .on_hover_text(tip(leftovers))
        .clicked()
}

fn name_of(row: &Row) -> String {
    row.leftover.path.file_name().map_or_else(
        || row.leftover.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn row_ui(ui: &mut egui::Ui, row: &Row, answer: &mut Option<Answer>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(name_of(row)).monospace().color(colors::fg()));
        if let Some(branch) = &row.leftover.branch {
            ui.label(RichText::new(branch).size(12.0).color(colors::dim()));
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let loss = matches!(&row.state, RowState::Ready(i) if i.changed > 0)
            || matches!(row.state, RowState::Failed(_));
        let color = if loss { brand::RED } else { colors::dim() };
        ui.label(RichText::new(status_text(&row.state)).size(12.0).color(color));
        if matches!(&row.state, RowState::Ready(i) if i.changed > 0)
            && action_button(ui, "Discard", false).clicked()
        {
            *answer = Some(Answer::Discard(row.leftover.path.clone()));
        }
    });
    ui.add_space(4.0);
}

pub(crate) fn dialog(ctx: &egui::Context, cleanup: &Cleanup) -> Option<Answer> {
    let mut answer = None;
    let modal = egui::Modal::new(Id::new("leftover_worktrees")).frame(overlay_frame()).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new("Leftover worktrees").strong().size(15.0).color(colors::fg()));
        ui.label(
            RichText::new(
                "No tab owns these. Clean up removes the clean ones and keeps any branch that holds \
                 commits found nowhere else. One with uncommitted changes needs its own Discard.",
            )
            .color(colors::dim()),
        );
        ScrollArea::vertical().max_height(LIST_HEIGHT).show(ui, |ui| {
            for row in &cleanup.rows {
                row_ui(ui, row, &mut answer);
            }
            if cleanup.rows.is_empty() {
                ui.label(RichText::new("Nothing left over.").color(colors::dim()));
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if action_button(ui, "Close", false).clicked() {
                answer = Some(Answer::Close);
            }
            let n = cleanup.clean_count();
            let label = if n == 0 { "Clean up".to_owned() } else { format!("Clean up ({n})") };
            let ready = n > 0 && !cleanup.busy();
            ui.add_enabled_ui(ready, |ui| {
                if action_button(ui, &label, true).clicked() {
                    answer = Some(Answer::CleanUp);
                }
            });
        });
    });
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Close);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};
    use tidewatch_core::worktree::Inspection;

    fn leftover(name: &str) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: Some(format!("agent/{name}")),
            repo: Some(PathBuf::from("/work/web")),
        }
    }

    #[test]
    fn the_tip_lists_each_worktree_with_its_branch_when_known() {
        let bare = Leftover { path: PathBuf::from("/x/by-hand"), branch: None, repo: None };
        let text = tip(&[leftover("calm-comet"), bare]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("Click to clean them up"));
        assert_eq!(lines[1], "/home/.tidewatch/web-abc123/calm-comet  (agent/calm-comet)");
        assert_eq!(lines[2], "/x/by-hand");
    }

    fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            focused: true,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, draw);
    }

    #[test]
    fn the_dialog_renders_every_state_and_asks_for_nothing_until_clicked() {
        let (mut cleanup, _) = Cleanup::open(&[leftover("a"), leftover("b"), leftover("c")]);
        cleanup.inspected(&leftover("a").path, Inspection { changed: 0, unmerged: 0 });
        cleanup.inspected(&leftover("b").path, Inspection { changed: 2, unmerged: 3 });
        cleanup.removed(&leftover("c").path, Err("locked".into()));
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut answer = None;
            frame(&ctx, |ui| answer = dialog(ui.ctx(), &cleanup));
            assert_eq!(answer, None);
        }
        let (empty, _) = Cleanup::open(&[]);
        let mut answer = None;
        frame(&ctx, |ui| answer = dialog(ui.ctx(), &empty));
        assert_eq!(answer, None);
    }

    #[test]
    fn the_chip_renders_and_is_not_clicked_by_itself() {
        let ctx = egui::Context::default();
        let mut clicked = true;
        frame(&ctx, |ui| clicked = chip(ui, &[leftover("a")]));
        assert!(!clicked);
    }
}
