//! The confirm dialog shown when a worktree tab closes: keep or delete the branch, and what to do
//! with uncommitted changes. The choices and wording are pure; `show` only draws them.

use crate::brand;
use crate::notice::key_hint;
use crate::screen_kit::{
    Size, danger_button, dialog_frame, footer_row, primary_button, quiet_button,
};
use egui::{Event, Id, Key, Modifiers, RichText};
use tidewatch_core::prefs::CloseEnter;
use tidewatch_core::worktree::{BranchChoice, RemoveMode, Worktree};
use tidewatch_core::worktree_safety::{Risk, Safety};

const DIALOG_WIDTH: f32 = 540.0;
const ARM_AFTER_SECS: f64 = 0.4;
const CHECKING_LINE: &str = "Checking whether anything would be lost...";

pub(crate) enum Closing {
    Inspecting { tab: u64, worktree: Worktree },
    Asking { tab: u64, worktree: Worktree, safety: Safety },
}

impl Closing {
    pub(crate) fn land(
        closing: &mut Option<Self>,
        tab: u64,
        checked: &Worktree,
        safety: Safety,
    ) -> bool {
        match closing.take() {
            Some(Self::Inspecting { tab: waiting, worktree })
                if waiting == tab && worktree == *checked =>
            {
                *closing = Some(Self::Asking { tab, worktree, safety });
                true
            }
            other => {
                *closing = other;
                false
            }
        }
    }
}

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

/// Safest first. A worktree proven safe only asks about the branch. One whose folder is safe but
/// whose branch holds commits found nowhere else can leave with its branch kept, and deleting the
/// branch takes an explicit discard. Anything else also offers to keep the worktree, and removing
/// it takes an explicit discard.
pub(crate) fn choices(safety: &Safety) -> Vec<Choice> {
    let remove = |branch, mode| Choice::Remove { branch, mode };
    if safety.is_safe() {
        vec![
            remove(BranchChoice::Keep, RemoveMode::Safe),
            remove(BranchChoice::Delete, RemoveMode::Safe),
        ]
    } else if safety.folder_is_safe() {
        vec![
            remove(BranchChoice::Keep, RemoveMode::Safe),
            remove(BranchChoice::Delete, RemoveMode::Discard),
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
pub(crate) fn warnings(safety: &Safety) -> Vec<String> {
    match safety {
        Safety::Safe => Vec::new(),
        Safety::Unknown(error) => vec![format!(
            "Could not check this worktree: {error}. Nothing is removed unless you discard it."
        )],
        Safety::Unsafe(risks) => risks.iter().map(risk_line).collect(),
    }
}

fn risk_line(risk: &Risk) -> String {
    match risk {
        Risk::Uncommitted { files } => {
            let have = if *files == 1 { "file has" } else { "files have" };
            format!("{files} {have} uncommitted changes. Discarding removes them for good.")
        }
        Risk::Unpushed { commits } => {
            let exist = if *commits == 1 { "commit exists" } else { "commits exist" };
            format!("{commits} {exist} only on this branch. Deleting the branch loses them.")
        }
        other => {
            format!("{}. Discarding takes the folder from under it.", capitalized(&other.text()))
        }
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// The button Enter presses: Keep branch whenever removing the folder loses nothing, Delete branch
/// only when the whole worktree is proven safe, never a discard.
pub(crate) fn enter_choice(safety: &Safety, enter: CloseEnter) -> Option<Choice> {
    let remove = |branch| Choice::Remove { branch, mode: RemoveMode::Safe };
    match enter {
        CloseEnter::KeepBranch => safety.folder_is_safe().then(|| remove(BranchChoice::Keep)),
        CloseEnter::DeleteBranch => safety.is_safe().then(|| remove(BranchChoice::Delete)),
    }
}

/// A key press that arrived before the user could see this dialog must not answer it: closing a
/// repo opens one dialog after another, and a second Enter meant for the last one would land here.
fn armed(ctx: &egui::Context, worktree: &Worktree) -> bool {
    let now = ctx.input(|i| i.time);
    let shown = Id::new(("close_worktree_shown", &worktree.path));
    let since = ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(shown, || now));
    now - since >= ARM_AFTER_SECS
}

fn forget_shown(ctx: &egui::Context, worktree: &Worktree) {
    ctx.data_mut(|data| data.remove_temp::<f64>(Id::new(("close_worktree_shown", &worktree.path))));
}

fn fresh_enter(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| {
        let fresh = input.events.iter().any(|event| {
            matches!(event, Event::Key { key: Key::Enter, pressed: true, repeat: false, .. })
        });
        input.consume_key(Modifiers::NONE, Key::Enter) && fresh
    })
}

pub(crate) fn show_closing(
    ctx: &egui::Context,
    closing: &Closing,
    enter: CloseEnter,
) -> Option<Answer> {
    match closing {
        Closing::Inspecting { worktree, .. } => show_checking(ctx, worktree),
        Closing::Asking { worktree, safety, .. } => show(ctx, worktree, safety, enter),
    }
}

fn show_checking(ctx: &egui::Context, worktree: &Worktree) -> Option<Answer> {
    let mut answer = None;
    let modal =
        dialog_frame(ctx, Id::new("close_worktree"), &heading(worktree), DIALOG_WIDTH, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(RichText::new(CHECKING_LINE).color(brand::MUTED));
            ui.add_space(6.0);
            footer_row(ui, "", |ui| {
                if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
            key_hint(ui, "Esc cancels  -  the choices appear when the check is done");
        });
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

pub(crate) fn show(
    ctx: &egui::Context,
    worktree: &Worktree,
    safety: &Safety,
    enter: CloseEnter,
) -> Option<Answer> {
    let armed = armed(ctx, worktree);
    let mut answer = None;
    let modal =
        dialog_frame(ctx, Id::new("close_worktree"), &heading(worktree), DIALOG_WIDTH, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new("This removes the worktree folder. The branch can stay or go.")
                    .color(brand::MUTED),
            );
            for line in warnings(safety) {
                ui.label(RichText::new(line).color(brand::RED.gamma_multiply(0.8)));
            }
            ui.add_space(6.0);
            footer_row(ui, "", |ui| {
                let keyed = enter_choice(safety, enter);
                for choice in choices(safety).into_iter().rev() {
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
                match enter_choice(safety, enter) {
                    Some(Choice::Remove { branch: BranchChoice::Delete, .. }) => {
                        "Enter deletes the branch  -  Esc cancels"
                    }
                    Some(_) => "Enter keeps the branch  -  Esc cancels",
                    None => "Esc cancels  -  nothing is discarded until you pick it",
                },
            );
        });
    let pressed = fresh_enter(ctx);
    if answer.is_none()
        && armed
        && pressed
        && let Some(choice) = enter_choice(safety, enter)
    {
        answer = Some(Answer::Pick(choice));
    }
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    if answer.is_some() {
        forget_shown(ctx, worktree);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Safety {
        Safety::Safe
    }

    fn dirty() -> Safety {
        Safety::Unsafe(vec![Risk::Uncommitted { files: 2 }])
    }

    fn unpushed() -> Safety {
        Safety::Unsafe(vec![Risk::Unpushed { commits: 3 }])
    }

    fn running() -> Safety {
        Safety::Unsafe(vec![Risk::Running { pid: 7, command: "zsh".into() }])
    }

    fn unknown() -> Safety {
        Safety::Unknown("git status failed".into())
    }

    fn remove(branch: BranchChoice, mode: RemoveMode) -> Choice {
        Choice::Remove { branch, mode }
    }

    #[test]
    fn a_clean_worktree_only_asks_about_the_branch() {
        assert_eq!(
            choices(&clean()),
            [
                remove(BranchChoice::Keep, RemoveMode::Safe),
                remove(BranchChoice::Delete, RemoveMode::Safe),
            ]
        );
    }

    #[test]
    fn unpushed_commits_make_deleting_the_branch_a_discard() {
        assert_eq!(
            choices(&unpushed()),
            [
                remove(BranchChoice::Keep, RemoveMode::Safe),
                remove(BranchChoice::Delete, RemoveMode::Discard),
            ]
        );
    }

    #[test]
    fn a_worktree_that_is_not_safe_never_offers_a_safe_remove() {
        for safety in [dirty(), running(), unknown()] {
            let all = choices(&safety);
            assert_eq!(all[0], Choice::KeepWorktree, "{safety:?}");
            assert!(
                all[1..]
                    .iter()
                    .all(|c| matches!(c, Choice::Remove { mode: RemoveMode::Discard, .. })),
                "{safety:?}"
            );
        }
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
    fn enter_never_presses_delete_unless_the_whole_worktree_is_proven_safe() {
        let keep = Some(remove(BranchChoice::Keep, RemoveMode::Safe));
        let delete = Some(remove(BranchChoice::Delete, RemoveMode::Safe));
        let cases = [
            (clean(), keep, delete),
            (unpushed(), keep, None),
            (dirty(), None, None),
            (running(), None, None),
            (unknown(), None, None),
        ];
        for (safety, on_keep, on_delete) in cases {
            assert_eq!(enter_choice(&safety, CloseEnter::KeepBranch), on_keep, "{safety:?}");
            assert_eq!(enter_choice(&safety, CloseEnter::DeleteBranch), on_delete, "{safety:?}");
        }
    }

    fn press(repeat: bool) -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        }
    }

    fn release() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn tap() -> Vec<Event> {
        vec![press(false), release()]
    }

    fn worktree(name: &str) -> Worktree {
        Worktree {
            repo: "/r".into(),
            path: format!("/w/{name}").into(),
            branch: format!("agent/{name}"),
        }
    }

    fn frame(
        ctx: &egui::Context,
        worktree: &Worktree,
        safety: &Safety,
        enter: CloseEnter,
        at: f64,
        events: Vec<Event>,
    ) -> Option<Answer> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 600.0),
            )),
            time: Some(at),
            events,
            focused: true,
            ..egui::RawInput::default()
        };
        let mut answer = None;
        let _ = ctx.run_ui(raw, |_| answer = show(ctx, worktree, safety, enter));
        answer
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx
    }

    fn shown(safety: &Safety, enter: CloseEnter, press_enter: bool) -> Option<Answer> {
        let ctx = context();
        let wt = worktree("brave-otter");
        let _ = frame(&ctx, &wt, safety, enter, 0.0, vec![]);
        let events = if press_enter { tap() } else { vec![] };
        frame(&ctx, &wt, safety, enter, 1.0, events)
    }

    #[test]
    fn the_dialog_opens_and_waits_whatever_enter_is_set_to_press() {
        for enter in CloseEnter::ALL {
            assert_eq!(shown(&clean(), enter, false), None);
        }
    }

    #[test]
    fn enter_answers_with_the_button_the_setting_names() {
        let answer = |branch| Answer::Pick(remove(branch, RemoveMode::Safe));
        assert_eq!(
            shown(&clean(), CloseEnter::DeleteBranch, true),
            Some(answer(BranchChoice::Delete))
        );
        assert_eq!(shown(&clean(), CloseEnter::KeepBranch, true), Some(answer(BranchChoice::Keep)));
        for enter in CloseEnter::ALL {
            assert_eq!(shown(&dirty(), enter, true), None);
            assert_eq!(shown(&unknown(), enter, true), None);
        }
    }

    #[test]
    fn enter_with_unpushed_commits_keeps_the_branch_and_never_deletes_it() {
        assert_eq!(shown(&unpushed(), CloseEnter::DeleteBranch, true), None);
        assert_eq!(
            shown(&unpushed(), CloseEnter::KeepBranch, true),
            Some(Answer::Pick(remove(BranchChoice::Keep, RemoveMode::Safe)))
        );
    }

    #[test]
    fn a_key_press_that_arrived_with_the_dialog_does_not_answer_it() {
        let ctx = context();
        let wt = worktree("next-tab");
        let enter = CloseEnter::DeleteBranch;
        assert_eq!(frame(&ctx, &wt, &clean(), enter, 5.0, tap()), None);
        assert_eq!(frame(&ctx, &wt, &clean(), enter, 5.1, tap()), None);
        assert_eq!(
            frame(&ctx, &wt, &clean(), enter, 5.6, tap()),
            Some(Answer::Pick(remove(BranchChoice::Delete, RemoveMode::Safe))),
            "a press after the dialog had time to be seen answers it"
        );
    }

    #[test]
    fn a_double_enter_answers_the_first_dialog_and_not_the_one_that_follows() {
        let ctx = context();
        let (first, second) = (worktree("first"), worktree("second"));
        let enter = CloseEnter::DeleteBranch;
        let _ = frame(&ctx, &first, &clean(), enter, 0.0, vec![]);
        let answered = frame(&ctx, &first, &clean(), enter, 1.0, tap());
        assert!(answered.is_some());
        assert_eq!(
            frame(&ctx, &second, &clean(), enter, 1.05, tap()),
            None,
            "the second press belongs to the first dialog"
        );
        assert_eq!(frame(&ctx, &second, &clean(), enter, 1.1, vec![]), None);
        assert!(
            frame(&ctx, &second, &clean(), enter, 2.0, tap()).is_some(),
            "a deliberate press on the second dialog still works"
        );
    }

    #[test]
    fn a_held_enter_never_answers_a_dialog() {
        let ctx = context();
        let wt = worktree("held");
        let enter = CloseEnter::DeleteBranch;
        let _ = frame(&ctx, &wt, &clean(), enter, 0.0, vec![press(false)]);
        assert_eq!(frame(&ctx, &wt, &clean(), enter, 3.0, vec![press(true)]), None);
    }

    #[test]
    fn warnings_name_what_would_be_lost() {
        assert_eq!(warnings(&clean()), Vec::<String>::new());
        let both = warnings(&Safety::Unsafe(vec![
            Risk::Uncommitted { files: 1 },
            Risk::Unpushed { commits: 3 },
            Risk::Running { pid: 7, command: "zsh".into() },
        ]));
        assert_eq!(both.len(), 3);
        assert!(both[0].starts_with("1 file has uncommitted changes"));
        assert!(both[1].starts_with("3 commits exist only on this branch"));
        assert_eq!(
            both[2],
            "Zsh (pid 7) is running in it. Discarding takes the folder from under it."
        );
        assert!(
            warnings(&Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }]))[0]
                .starts_with("1 commit exists")
        );
        assert!(warnings(&unknown())[0].starts_with("Could not check this worktree: git status"));
    }

    #[test]
    fn heading_names_the_branch() {
        assert_eq!(heading(&worktree("brave-otter")), "Close agent/brave-otter?");
    }

    fn escape() -> Event {
        Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn inspecting(tab: u64, name: &str) -> Closing {
        Closing::Inspecting { tab, worktree: worktree(name) }
    }

    fn frame_closing(
        ctx: &egui::Context,
        closing: Option<&Closing>,
        at: f64,
        events: Vec<Event>,
    ) -> Option<Answer> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 600.0),
            )),
            time: Some(at),
            events,
            focused: true,
            ..egui::RawInput::default()
        };
        let mut answer = None;
        let _ = ctx.run_ui(raw, |_| {
            answer = closing.and_then(|c| show_closing(ctx, c, CloseEnter::DeleteBranch));
        });
        answer
    }

    fn dialog_drawn(ctx: &egui::Context) -> bool {
        ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .any(|id| id.order == egui::Order::Foreground)
        })
    }

    #[test]
    fn the_dialog_is_drawn_while_the_check_runs_and_escape_ends_the_close() {
        let ctx = context();
        let mut closing = Some(inspecting(3, "slow"));
        assert_eq!(frame_closing(&ctx, closing.as_ref(), 0.0, vec![]), None);
        assert!(dialog_drawn(&ctx), "the checking dialog is on screen in the first frame");
        assert!(closing.is_some(), "keys are held, and a dialog shows why");

        assert_eq!(
            frame_closing(&ctx, closing.as_ref(), 1.0, vec![escape()]),
            Some(Answer::Cancel)
        );
        closing = None;
        let _ = frame_closing(&ctx, closing.as_ref(), 1.1, vec![]);
        let _ = frame_closing(&ctx, closing.as_ref(), 1.2, vec![]);
        assert!(!dialog_drawn(&ctx), "nothing is drawn once the close ended");
        assert!(closing.is_none(), "so no key is held");
    }

    #[test]
    fn enter_does_nothing_until_the_check_landed_and_then_waits_for_the_arming_delay() {
        let ctx = context();
        let mut closing = Some(inspecting(3, "slow"));
        let _ = frame_closing(&ctx, closing.as_ref(), 0.0, vec![]);
        assert_eq!(frame_closing(&ctx, closing.as_ref(), 5.0, tap()), None);

        assert!(Closing::land(&mut closing, 3, &worktree("slow"), Safety::Safe));
        assert_eq!(frame_closing(&ctx, closing.as_ref(), 5.1, vec![]), None);
        assert_eq!(
            frame_closing(&ctx, closing.as_ref(), 5.2, tap()),
            None,
            "too soon after the choices"
        );
        assert_eq!(
            frame_closing(&ctx, closing.as_ref(), 6.0, tap()),
            Some(Answer::Pick(remove(BranchChoice::Delete, RemoveMode::Safe)))
        );
    }

    #[test]
    fn a_check_that_lands_after_cancel_changes_nothing() {
        let mut closing = Some(inspecting(3, "slow"));
        drop(closing.take());
        assert!(!Closing::land(&mut closing, 3, &worktree("slow"), Safety::Safe));
        assert!(closing.is_none());
    }

    #[test]
    fn a_check_for_another_tab_or_worktree_leaves_the_waiting_close_alone() {
        let mut closing = Some(inspecting(3, "slow"));
        assert!(!Closing::land(&mut closing, 4, &worktree("slow"), Safety::Safe));
        assert!(!Closing::land(&mut closing, 3, &worktree("other"), Safety::Safe));
        assert!(matches!(closing, Some(Closing::Inspecting { tab: 3, .. })));
    }
}
