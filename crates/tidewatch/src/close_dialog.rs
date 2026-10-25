//! The confirm dialog shown when a worktree tab closes: keep or delete the branch, and what to do
//! with uncommitted changes. The choices and wording are pure; `show` only draws them.

use crate::brand;
use crate::notice::key_hint;
use crate::screen_kit::{
    Size, danger_button, dialog_frame, footer_row, primary_button, quiet_button,
};
use egui::{Id, Key, Modifiers, RichText};
use tidewatch_core::prefs::CloseEnter;
use tidewatch_core::worktree::{BranchChoice, Inspection, RemoveMode, Worktree};

const DIALOG_WIDTH: f32 = 540.0;

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

/// The button Enter presses, but only when no uncommitted change would be thrown away.
pub(crate) fn enter_choice(inspection: &Inspection, enter: CloseEnter) -> Option<Choice> {
    let index = match enter {
        CloseEnter::KeepBranch => 0,
        CloseEnter::DeleteBranch => 1,
    };
    (inspection.changed == 0).then(|| choices(inspection)[index])
}

pub(crate) fn show(
    ctx: &egui::Context,
    worktree: &Worktree,
    inspection: &Inspection,
    enter: CloseEnter,
) -> Option<Answer> {
    let mut answer = None;
    let modal =
        dialog_frame(ctx, Id::new("close_worktree"), &heading(worktree), DIALOG_WIDTH, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new("This removes the worktree folder. The branch can stay or go.")
                    .color(brand::MUTED),
            );
            for line in warnings(inspection) {
                ui.label(RichText::new(line).color(brand::RED.gamma_multiply(0.8)));
            }
            ui.add_space(6.0);
            footer_row(ui, "", |ui| {
                let keyed = enter_choice(inspection, enter);
                for choice in choices(inspection).into_iter().rev() {
                    let text = label(choice);
                    let response = match choice {
                        Choice::Remove { branch: BranchChoice::Delete, .. } => {
                            danger_button(ui, text, Size::Regular)
                        }
                        _ if keyed == Some(choice) => primary_button(ui, text, true, Size::Regular),
                        _ => quiet_button(ui, text, true, Size::Regular),
                    };
                    if response.clicked() {
                        answer = Some(Answer::Pick(choice));
                    }
                }
                if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
            key_hint(
                ui,
                match enter_choice(inspection, enter) {
                    Some(Choice::Remove { branch: BranchChoice::Delete, .. }) => {
                        "Enter deletes the branch  -  Esc cancels"
                    }
                    Some(_) => "Enter keeps the branch  -  Esc cancels",
                    None => "Esc cancels  -  nothing is discarded until you pick it",
                },
            );
        });
    if answer.is_none()
        && let Some(choice) = enter_choice(inspection, enter)
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
    use egui::Event;

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
    fn enter_presses_the_button_the_setting_names_only_when_no_change_is_lost() {
        let remove = |branch| Choice::Remove { branch, mode: RemoveMode::Safe };
        assert_eq!(
            enter_choice(&clean(), CloseEnter::DeleteBranch),
            Some(remove(BranchChoice::Delete))
        );
        assert_eq!(
            enter_choice(&clean(), CloseEnter::KeepBranch),
            Some(remove(BranchChoice::Keep))
        );
        for enter in CloseEnter::ALL {
            assert_eq!(enter_choice(&dirty(), enter), None);
        }
    }

    fn shown(inspection: &Inspection, enter: CloseEnter, press_enter: bool) -> Option<Answer> {
        let worktree =
            Worktree { repo: "/r".into(), path: "/w".into(), branch: "agent/brave-otter".into() };
        let key = Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut answer = None;
        for events in [vec![], if press_enter { vec![key] } else { vec![] }] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(900.0, 600.0),
                )),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let _ = ctx.run_ui(raw, |_| answer = show(&ctx, &worktree, inspection, enter));
        }
        answer
    }

    #[test]
    fn the_dialog_opens_and_waits_whatever_enter_is_set_to_press() {
        for enter in CloseEnter::ALL {
            assert_eq!(shown(&clean(), enter, false), None);
        }
    }

    #[test]
    fn enter_answers_with_the_button_the_setting_names() {
        let remove = |branch| Answer::Pick(Choice::Remove { branch, mode: RemoveMode::Safe });
        assert_eq!(
            shown(&clean(), CloseEnter::DeleteBranch, true),
            Some(remove(BranchChoice::Delete))
        );
        assert_eq!(shown(&clean(), CloseEnter::KeepBranch, true), Some(remove(BranchChoice::Keep)));
        for enter in CloseEnter::ALL {
            assert_eq!(shown(&dirty(), enter, true), None);
        }
    }

    #[test]
    fn warnings_name_changes_and_unmerged_commits() {
        assert_eq!(warnings(&clean()), Vec::<String>::new());
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
