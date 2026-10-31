//! Draws the leftover worktrees: the chip in the tab strip and the dialog it opens, with what
//! each worktree would lose, Clean up for the ones proven safe and a Discard on the row of any
//! other, with the branch kept or deleted as that click says.

use std::path::PathBuf;

use egui::{Align, Color32, Frame, Id, Layout, Margin, RichText, ScrollArea, Stroke};
use tidewatch_core::worktree::{BranchChoice, Leftover};
use tidewatch_core::worktree_safety::{Risk, Safety};

use crate::brand;
use crate::leftovers::{Cleanup, Row, RowState, row_text};
use crate::review::plural;
use crate::screen_kit::{
    DialogWidth, Size, danger_button, dialog_frame, footer_row, primary_button, quiet_button,
};
use crate::settings_ui::pill;

const LIST_HEIGHT: f32 = 400.0;
const UNREADABLE_DISCARD: &str = "Moves the folder to the Trash, where you can take it back. Nothing is deleted, and its branch stays.";
const UNRECORDED_DISCARD: &str = "Removes the folder. The branch goes with it only when every commit on it also exists elsewhere; otherwise it stays.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Close,
    CleanUp,
    Discard(PathBuf, BranchChoice),
    Trash(PathBuf),
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

fn chip_text(count: usize) -> String {
    plural(count, "leftover")
}

/// "N leftovers" in the tab strip. True when clicked.
pub(crate) fn chip(ui: &mut egui::Ui, leftovers: &[Leftover]) -> bool {
    quiet_button(ui, &chip_text(leftovers.len()), true, Size::Compact)
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
        RowState::Unreadable => ("unreadable", brand::ORANGE),
        RowState::Failed(_) => ("failed", brand::RED),
        RowState::Ready(Safety::Safe) => ("clean", brand::GREEN),
        RowState::Ready(Safety::Unknown(_)) => ("unknown", brand::ORANGE),
        RowState::Ready(Safety::Unsafe(risks)) => (risk_pill(risks), brand::ORANGE),
    }
}

fn risk_pill(risks: &[Risk]) -> &'static str {
    match risks.first() {
        Some(Risk::Uncommitted { .. }) => "dirty",
        Some(Risk::Unpushed { .. }) => "unpushed",
        Some(Risk::Running { .. }) => "running",
        Some(Risk::RecentSession { .. }) => "recent",
        None => "unsafe",
    }
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
                    if !row.leftover.recorded && row.state != RowState::Unreadable {
                        pill(ui, "unrecorded", brand::ORANGE);
                    }
                });
            });
            let discardable = Cleanup::is_discardable(row);
            let loss = discardable || matches!(row.state, RowState::Failed(_));
            let color = if loss { brand::RED } else { brand::MUTED };
            if let Some(text) = row_text(row) {
                ui.label(RichText::new(text).size(12.0).color(color));
            }
            if discardable {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let path = row.leftover.path.clone();
                    if row.state == RowState::Unreadable {
                        if danger_button(ui, "Discard", Size::Compact)
                            .on_hover_text(UNREADABLE_DISCARD)
                            .clicked()
                        {
                            *answer = Some(Answer::Trash(path));
                        }
                    } else if row.leftover.recorded {
                        if danger_button(ui, "Discard, delete branch", Size::Compact).clicked() {
                            *answer = Some(Answer::Discard(path.clone(), BranchChoice::Delete));
                        }
                        if quiet_button(ui, "Discard, keep branch", true, Size::Compact).clicked() {
                            *answer = Some(Answer::Discard(path, BranchChoice::Keep));
                        }
                    } else if danger_button(ui, "Discard", Size::Compact)
                        .on_hover_text(UNRECORDED_DISCARD)
                        .clicked()
                    {
                        *answer = Some(Answer::Discard(path, BranchChoice::Delete));
                    }
                });
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
        DialogWidth::Medium,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new(
                    "No tab owns these. Clean up removes only the ones proven safe: nothing \
                     uncommitted, every commit also on a remote or merged, no process in the \
                     folder, no agent session in the last day. Any other needs its own Discard, \
                     and what it would lose is listed on its row. An unrecorded tag only says \
                     this Tidewatch did not make the worktree.",
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
            footer_row(ui, &[("Esc", "closes")], |ui| {
                let n = cleanup.clean_count();
                let label = if n == 0 { "Clean up".to_owned() } else { format!("Clean up ({n})") };
                let blocker = cleanup.clean_up_blocker();
                let clean_up = primary_button(ui, &label, blocker.is_none(), Size::Regular);
                if let Some(reason) = blocker {
                    clean_up.on_disabled_hover_text(reason);
                } else if clean_up.clicked() {
                    answer = Some(Answer::CleanUp);
                }
                if quiet_button(ui, "Close", true, Size::Regular).clicked() {
                    answer = Some(Answer::Close);
                }
            });
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

    #[test]
    fn the_chip_counts_leftovers_in_the_plural() {
        let cases = [(1, "1 leftover"), (2, "2 leftovers"), (19, "19 leftovers")];
        for (count, want) in cases {
            assert_eq!(chip_text(count), want);
        }
    }

    use egui::{Rect, pos2, vec2};

    fn leftover(name: &str) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: Some(format!("agent/{name}")),
            repo: Some(PathBuf::from("/work/web")),
            recorded: true,
        }
    }

    #[test]
    fn the_tip_lists_each_worktree_with_its_branch_when_known() {
        let bare = Leftover {
            path: PathBuf::from("/x/by-hand"),
            branch: None,
            repo: None,
            recorded: false,
        };
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
        cleanup.inspected(&leftover("a").path, Safety::Safe);
        cleanup.inspected(
            &leftover("b").path,
            Safety::Unsafe(vec![Risk::Uncommitted { files: 2 }, Risk::Unpushed { commits: 3 }]),
        );
        cleanup.removed(&leftover("c").path, Err("locked".into()));
        let ctx = crate::fonts::test_context();
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
