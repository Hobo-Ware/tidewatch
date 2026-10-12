//! The confirm dialog shown when a worktree tab closes: keep or delete the branch, and what to do
//! with uncommitted changes. The choices and wording are pure; `show` only draws them.

use egui::{Id, Key, Modifiers, RichText};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, overlay_frame};
use tidewatch_core::worktree::{BranchChoice, Inspection, RemoveMode, Worktree};

const DIALOG_WIDTH: f32 = 440.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    KeepWorktree,
    Remove { branch: BranchChoice, mode: RemoveMode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Cancel,
    Pick(Choice),
}

/// Safest first. A clean worktree only asks about the branch; one with changes also offers to keep
/// the worktree, and removing it takes an explicit discard.
pub(crate) fn choices(inspection: &Inspection) -> Vec<Choice> {
    let remove = |branch, mode| Choice::Remove { branch, mode };
    if inspection.changed == 0 {
        vec![
            remove(BranchChoice::Keep, RemoveMode::Safe),
            remove(BranchChoice::Delete, RemoveMode::Safe),
        ]
    } else {
        vec![
            Choice::KeepWorktree,
            remove(BranchChoice::Keep, RemoveMode::Discard),
            remove(BranchChoice::Delete, RemoveMode::Discard),
        ]
    }
}

pub(crate) fn label(choice: Choice) -> &'static str {
    match choice {
        Choice::KeepWorktree => "Keep worktree",
        Choice::Remove { branch: BranchChoice::Keep, mode: RemoveMode::Safe } => "Keep branch",
        Choice::Remove { branch: BranchChoice::Delete, mode: RemoveMode::Safe } => "Delete branch",
        Choice::Remove { branch: BranchChoice::Keep, mode: RemoveMode::Discard } => {
            "Discard, keep branch"
        }
        Choice::Remove { branch: BranchChoice::Delete, mode: RemoveMode::Discard } => {
            "Discard, delete branch"
        }
    }
}

pub(crate) fn heading(worktree: &Worktree) -> String {
    format!("Close {}?", worktree.branch)
}

/// The warning lines: what would be lost, most serious first.
pub(crate) fn warnings(inspection: &Inspection) -> Vec<String> {
    let mut lines = Vec::new();
    if inspection.changed > 0 {
        let n = inspection.changed;
        let files = if n == 1 { "file has" } else { "files have" };
        lines.push(format!("{n} {files} uncommitted changes. Discarding removes them for good."));
    }
    if inspection.unmerged > 0 {
        let n = inspection.unmerged;
        let commits = if n == 1 { "commit exists" } else { "commits exist" };
        lines.push(format!("{n} {commits} only on this branch. Deleting the branch loses them."));
    }
    lines
}

/// Enter takes the safe default, but only when nothing would be thrown away.
pub(crate) fn enter_choice(inspection: &Inspection) -> Option<Choice> {
    (inspection.changed == 0).then(|| choices(inspection)[0])
}

pub(crate) fn show(
    ctx: &egui::Context,
    worktree: &Worktree,
    inspection: &Inspection,
) -> Option<Answer> {
    let mut answer = None;
    let modal =
        egui::Modal::new(Id::new("close_worktree")).frame(overlay_frame()).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(RichText::new(heading(worktree)).strong().size(15.0).color(colors::fg()));
            ui.label(
                RichText::new("This removes the worktree folder. The branch can stay or go.")
                    .color(colors::dim()),
            );
            for line in warnings(inspection) {
                ui.label(RichText::new(line).color(colors::red()));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if action_button(ui, "Cancel", false).clicked() {
                    answer = Some(Answer::Cancel);
                }
                for (i, choice) in choices(inspection).into_iter().enumerate() {
                    let primary = i == 0 && inspection.changed == 0;
                    if action_button(ui, label(choice), primary).clicked() {
                        answer = Some(Answer::Pick(choice));
                    }
                }
            });
        });
    if answer.is_none()
        && let Some(choice) = enter_choice(inspection)
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
    {
        answer = Some(Answer::Pick(choice));
    }
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Inspection {
        Inspection { changed: 0, unmerged: 0 }
    }

    fn dirty() -> Inspection {
        Inspection { changed: 2, unmerged: 0 }
    }

    #[test]
    fn a_clean_worktree_only_asks_about_the_branch() {
        assert_eq!(
            choices(&clean()),
            [
                Choice::Remove { branch: BranchChoice::Keep, mode: RemoveMode::Safe },
                Choice::Remove { branch: BranchChoice::Delete, mode: RemoveMode::Safe },
            ]
        );
    }

    #[test]
    fn a_dirty_worktree_never_offers_a_safe_remove() {
        let all = choices(&dirty());
        assert_eq!(all[0], Choice::KeepWorktree);
        assert!(
            all[1..].iter().all(|c| matches!(c, Choice::Remove { mode: RemoveMode::Discard, .. }))
        );
    }

    #[test]
    fn every_choice_has_its_own_label() {
        let mut labels: Vec<&str> =
            choices(&clean()).into_iter().chain(choices(&dirty())).map(label).collect();
        let before = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), before);
    }

    #[test]
    fn enter_is_the_safe_default_only_when_nothing_is_lost() {
        assert_eq!(
            enter_choice(&clean()),
            Some(Choice::Remove { branch: BranchChoice::Keep, mode: RemoveMode::Safe })
        );
        assert_eq!(enter_choice(&dirty()), None);
    }

    #[test]
    fn warnings_name_changes_and_unmerged_commits() {
        assert!(warnings(&clean()).is_empty());
        let both = warnings(&Inspection { changed: 1, unmerged: 3 });
        assert_eq!(both.len(), 2);
        assert!(both[0].starts_with("1 file has uncommitted changes"));
        assert!(both[1].starts_with("3 commits exist only on this branch"));
        assert!(
            warnings(&Inspection { changed: 0, unmerged: 1 })[0].starts_with("1 commit exists")
        );
    }

    #[test]
    fn heading_names_the_branch() {
        let worktree =
            Worktree { repo: "/r".into(), path: "/w".into(), branch: "agent/brave-otter".into() };
        assert_eq!(heading(&worktree), "Close agent/brave-otter?");
    }
}
