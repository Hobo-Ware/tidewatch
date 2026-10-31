//! Draws the leftover worktrees: the chip in the tab strip and the dialog it opens, with what
//! each worktree would lose, Clean up for the ones proven safe and a Discard on the row of any
//! other, with the branch kept or deleted as that click says.

use std::path::PathBuf;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, Id, Layout, Rect, RichText, Sense, UiBuilder, pos2, vec2};
use tidewatch_core::worktree::{BranchChoice, Leftover};
use tidewatch_core::worktree_safety::Safety;

use crate::brand;
use crate::leftovers::{Cleanup, Row, RowState, Summary, right_text, shown_branch, summary};
use crate::reader_view::ellipsized;
use crate::review::plural;
use crate::screen_kit::{
    ChipPart, DialogWidth, Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, TagFill, body, danger_button,
    dialog_pinned, footer_row, heading, mono, paint_pill_card, paint_tag, primary_button,
    quiet_button, status_chip,
};
use crate::settings_ui::disclosure;

const ROW_H: f32 = 36.0;
const BUTTONS_H: f32 = 36.0;
const ROW_PAD: f32 = 12.0;
const ROW_GAP: f32 = 4.0;
const DOT_R: f32 = 4.0;
const DOT_GAP: f32 = 8.0;
const RULES: &str = "No tab owns these. Clean up removes only the ones proven safe: nothing uncommitted, every commit also on a remote or merged, no process in the folder, no agent session in the last day. Any other needs its own Discard, and what it would lose is listed on its row. An unrecorded tag only says this Tidewatch did not make the worktree.";
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
    let parts = [ChipPart { text: &chip_text(leftovers.len()), ink: brand::SOFT }];
    let lines = (brand::FIELD_LINE, brand::MUTED);
    status_chip(ui, &parts, brand::MUTED, lines, egui::Sense::click())
        .on_hover_text(tip(leftovers))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn name_of(row: &Row) -> String {
    row.leftover.path.file_name().map_or_else(
        || row.leftover.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn dot_of(state: &RowState) -> Color32 {
    match state {
        RowState::Inspecting | RowState::Removing => brand::MUTED,
        RowState::Failed(_) => brand::RED,
        RowState::Ready(Safety::Safe) => brand::GREEN,
        RowState::Unreadable | RowState::Ready(_) => brand::ORANGE,
    }
}

fn right_ink(state: &RowState) -> Color32 {
    match state {
        RowState::Failed(_) => brand::RED,
        _ => brand::MUTED,
    }
}

fn row_ui(ui: &mut egui::Ui, row: &Row, now_ms: i64, answer: &mut Option<Answer>) {
    let discardable = Cleanup::is_discardable(row);
    let height = if discardable { ROW_H + BUTTONS_H } else { ROW_H };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mid = rect.top() + ROW_H / 2.0;
    let name = name_of(row);
    let painter = ui.painter();
    paint_pill_card(painter, rect);
    painter.circle_filled(pos2(rect.left() + ROW_PAD + DOT_R, mid), DOT_R, dot_of(&row.state));
    let name_at = pos2(rect.left() + ROW_PAD + 2.0 * DOT_R + DOT_GAP, mid);
    let mut left =
        painter.text(name_at, Align2::LEFT_CENTER, &name, mono(13.0), brand::TEXT).right();
    if let Some(branch) = shown_branch(row, &name) {
        let at = pos2(left + DOT_GAP, mid);
        left = painter.text(at, Align2::LEFT_CENTER, branch, body(TEXT_TAG), brand::MUTED).right();
    }
    if !row.leftover.recorded && row.state != RowState::Unreadable {
        let tag = paint_tag(
            painter,
            (left + DOT_GAP, mid),
            "unrecorded",
            brand::ORANGE,
            TagFill::Tint(brand::ORANGE.gamma_multiply(0.14)),
        );
        left = tag.right();
    }
    let full = right_text(row, now_ms);
    let room = rect.right() - ROW_PAD - left - 2.0 * DOT_GAP;
    let font = body(TEXT_SECONDARY);
    let galley = ellipsized(painter, &full, font, right_ink(&row.state), room);
    let cut = galley.text() != full;
    let right = pos2(rect.right() - ROW_PAD - galley.size().x, mid - galley.size().y / 2.0);
    painter.galley(right, galley, right_ink(&row.state));
    if cut {
        response.on_hover_text(full);
    }
    if discardable {
        let strip = Rect::from_min_max(
            pos2(rect.left() + ROW_PAD, rect.top() + ROW_H - 4.0),
            pos2(rect.right() - ROW_PAD, rect.bottom() - 8.0),
        );
        let layout = Layout::right_to_left(Align::Center);
        let mut buttons = ui.new_child(UiBuilder::new().max_rect(strip).layout(layout));
        buttons.spacing_mut().item_spacing.x = 8.0;
        discard_buttons(&mut buttons, row, answer);
    }
    ui.add_space(ROW_GAP);
}

fn discard_buttons(ui: &mut egui::Ui, row: &Row, answer: &mut Option<Answer>) {
    let path = row.leftover.path.clone();
    if row.state == RowState::Unreadable {
        if danger_button(ui, "Discard", Size::Compact).on_hover_text(UNREADABLE_DISCARD).clicked() {
            *answer = Some(Answer::Trash(path));
        }
    } else if row.leftover.recorded {
        if quiet_button(ui, "Discard, keep branch", true, Size::Compact).clicked() {
            *answer = Some(Answer::Discard(path.clone(), BranchChoice::Keep));
        }
        if danger_button(ui, "Discard, delete branch", Size::Compact).clicked() {
            *answer = Some(Answer::Discard(path, BranchChoice::Delete));
        }
    } else if danger_button(ui, "Discard", Size::Compact)
        .on_hover_text(UNRECORDED_DISCARD)
        .clicked()
    {
        *answer = Some(Answer::Discard(path, BranchChoice::Delete));
    }
}

fn summary_job(summary: &Summary) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(&summary.lead, 0.0, TextFormat::simple(heading(TEXT_BODY), brand::TEXT));
    if !summary.rest.is_empty() {
        let rest = format!(" {}", summary.rest);
        job.append(&rest, 0.0, TextFormat::simple(body(TEXT_BODY), brand::SOFT));
    }
    job
}

pub(crate) fn dialog(ctx: &egui::Context, cleanup: &Cleanup, now_ms: i64) -> Option<Answer> {
    let mut answer = None;
    let modal = dialog_pinned(
        ctx,
        Id::new("leftover_worktrees"),
        "Leftover worktrees",
        DialogWidth::Medium,
        &mut answer,
        |ui, answer| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(egui::Label::new(summary_job(&summary(cleanup))).wrap());
            ui.add_space(6.0);
            for row in &cleanup.rows {
                row_ui(ui, row, now_ms, answer);
            }
            ui.add_space(2.0);
            disclosure(ui, "leftover_rules", "What counts as safe", |ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(RULES).size(TEXT_SECONDARY).color(brand::MUTED));
            });
        },
        |ui, answer| {
            footer_row(ui, &[("Esc", "closes")], |ui| {
                let n = cleanup.clean_count();
                let label = if n == 0 { "Clean up".to_owned() } else { format!("Clean up {n}") };
                let blocker = cleanup.clean_up_blocker();
                let clean_up = primary_button(ui, &label, blocker.is_none(), Size::Regular);
                if let Some(reason) = blocker {
                    clean_up.on_disabled_hover_text(reason);
                } else if clean_up.clicked() {
                    *answer = Some(Answer::CleanUp);
                }
                if quiet_button(ui, "Close", true, Size::Regular).clicked() {
                    *answer = Some(Answer::Close);
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
    use tidewatch_core::worktree_safety::Risk;

    fn leftover(name: &str) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: Some(format!("agent/{name}")),
            repo: Some(PathBuf::from("/work/web")),
            recorded: true,
            modified_ms: None,
        }
    }

    #[test]
    fn the_tip_lists_each_worktree_with_its_branch_when_known() {
        let bare = Leftover {
            path: PathBuf::from("/x/by-hand"),
            branch: None,
            repo: None,
            recorded: false,
            modified_ms: None,
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
            frame(&ctx, |ui| answer = dialog(ui.ctx(), &cleanup, 0));
            assert_eq!(answer, None);
        }
        let (empty, _) = Cleanup::open(&[]);
        let mut answer = None;
        frame(&ctx, |ui| answer = dialog(ui.ctx(), &empty, 0));
        assert_eq!(answer, None);
    }

    #[test]
    fn the_chip_renders_and_is_not_clicked_by_itself() {
        let ctx = egui::Context::default();
        let mut clicked = true;
        frame(&ctx, |ui| clicked = chip(ui, &[leftover("a")]));
        assert!(!clicked);
    }

    #[test]
    fn a_long_list_scrolls_between_a_header_and_a_footer_that_stay_on_screen() {
        let list: Vec<Leftover> = (0..24).map(|n| leftover(&format!("worktree-{n}"))).collect();
        let (mut cleanup, _) = Cleanup::open(&list);
        for item in &list {
            cleanup.inspected(&item.path, Safety::Safe);
        }
        let ctx = crate::fonts::test_context();
        let mut out = None;
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 640.0))),
                focused: true,
                ..Default::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                let _ = dialog(ui.ctx(), &cleanup, 0);
            }));
        }
        let texts = crate::screen_kit::painted_texts(&out.unwrap());
        let at = |name: &str| texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect);
        assert!(at("Leftover worktrees").unwrap().top() >= 0.0);
        assert!(at("Clean up 24").unwrap().bottom() <= 640.0);
        assert!(at("Close").unwrap().bottom() <= 640.0);
    }

    #[test]
    fn a_row_is_one_line_a_dot_a_name_and_the_age_and_a_row_that_is_not_safe_keeps_its_buttons() {
        let mut safe = leftover("calm-comet");
        safe.modified_ms = Some(0);
        let list = [safe.clone(), leftover("jolly-beacon")];
        let (mut cleanup, _) = Cleanup::open(&list);
        cleanup.inspected(&list[0].path, Safety::Safe);
        cleanup.inspected(&list[1].path, Safety::Unsafe(vec![Risk::Unpushed { commits: 2 }]));
        let ctx = crate::fonts::test_context();
        let mut out = None;
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                focused: true,
                ..Default::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                let _ = dialog(ui.ctx(), &cleanup, 3 * 86_400_000);
            }));
        }
        let texts = crate::screen_kit::painted_texts(&out.unwrap());
        let at = |name: &str| texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect);
        let (name, age) = (at("calm-comet").unwrap(), at("3 days").unwrap());
        assert!((name.center().y - age.center().y).abs() < 2.0, "{name:?} {age:?}");
        assert_eq!(at("agent/calm-comet"), None, "the branch is the name again");
        let reason = at("2 commits found nowhere else (on no remote branch, not merged)").unwrap();
        let discard = at("Discard, delete branch").unwrap();
        assert!(discard.top() > reason.bottom(), "{discard:?} {reason:?}");
        assert!(at("Discard, keep branch").is_some());
        assert!(at("Clean up 1").is_some());
        assert!(at("What counts as safe").is_some());
        assert!(at("1 worktree").is_none());
    }
}
