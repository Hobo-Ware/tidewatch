//! Draws the leftover worktrees: the chip in the tab strip and the dialog it opens, with what
//! each worktree would lose, Clean up for the clean ones and an explicit Discard for the rest.

use std::path::PathBuf;

use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    vec2,
};
use tidewatch_core::worktree::Leftover;

use crate::brand;
use crate::leftovers::{Cleanup, Row, RowState, status_text};
use crate::notice::key_hint;
use crate::screen_kit::{body, danger_button, dialog_frame, primary_button, quiet_button};

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

fn pill_of(state: &RowState) -> (&'static str, Color32) {
    match state {
        RowState::Inspecting => ("looking", brand::MUTED),
        RowState::Removing => ("removing", brand::MUTED),
        RowState::Unrecorded => ("unrecorded", brand::ORANGE),
        RowState::Failed(_) => ("failed", brand::RED),
        RowState::Ready(i) if i.changed > 0 => ("dirty", brand::ORANGE),
        RowState::Ready(_) => ("clean", brand::GREEN),
    }
}

fn pill(ui: &mut egui::Ui, text: &str, ink: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), body(11.5), ink);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(16.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), ink.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
}

fn row_ui(ui: &mut egui::Ui, row: &Row, answer: &mut Option<Answer>) {
    Frame::new()
        .fill(brand::ITEM)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(name_of(row)).monospace().color(brand::TEXT));
                if let Some(branch) = &row.leftover.branch {
                    ui.label(RichText::new(branch).size(12.0).color(brand::MUTED));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (text, ink) = pill_of(&row.state);
                    pill(ui, text, ink);
                    if matches!(&row.state, RowState::Ready(i) if i.changed > 0)
                        && danger_button(ui, "Discard").clicked()
                    {
                        *answer = Some(Answer::Discard(row.leftover.path.clone()));
                    }
                });
            });
            let loss = matches!(&row.state, RowState::Ready(i) if i.changed > 0)
                || matches!(row.state, RowState::Failed(_));
            let color = if loss { brand::RED } else { brand::MUTED };
            if !matches!(&row.state, RowState::Ready(i) if i.changed == 0 && i.unmerged == 0) {
                ui.label(RichText::new(status_text(&row.state)).size(12.0).color(color));
            }
        });
    ui.add_space(6.0);
}

pub(crate) fn dialog(ctx: &egui::Context, cleanup: &Cleanup) -> Option<Answer> {
    let mut answer = None;
    let modal = dialog_frame(
        ctx,
        Id::new("leftover_worktrees"),
        "Leftover worktrees",
        DIALOG_WIDTH,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new(
                    "No tab owns these. Clean up removes the clean ones and keeps any branch that \
                     holds commits found nowhere else. One with uncommitted changes needs its own \
                     Discard.",
                )
                .color(brand::MUTED),
            );
            ui.add_space(4.0);
            ScrollArea::vertical().max_height(LIST_HEIGHT).show(ui, |ui| {
                for row in &cleanup.rows {
                    row_ui(ui, row, &mut answer);
                }
                if cleanup.rows.is_empty() {
                    ui.label(RichText::new("Nothing left over.").color(brand::MUTED));
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let n = cleanup.clean_count();
                    let label =
                        if n == 0 { "Clean up".to_owned() } else { format!("Clean up ({n})") };
                    if primary_button(ui, &label, n > 0 && !cleanup.busy()).clicked() {
                        answer = Some(Answer::CleanUp);
                    }
                    if quiet_button(ui, "Close", true).clicked() {
                        answer = Some(Answer::Close);
                    }
                });
            });
            key_hint(ui, "Esc closes");
        },
    );
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
