//! What the leftover worktrees dialog knows: one row per worktree no tab owns, what removing it
//! would lose, and the removals that Clean up and Discard turn into. Pure. The rules are the close
//! dialog's: a worktree with uncommitted changes is refused unless the user discards it
//! explicitly, and a branch with commits found nowhere else is never deleted.

use std::path::Path;

use tidewatch_core::worktree::{
    BranchChoice, BranchOutcome, Inspection, Leftover, RemoveMode, Worktree,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Inspecting,
    Ready(Inspection),
    /// The registry has no record of its repository or branch, so git cannot remove it from here.
    Unrecorded,
    Removing,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) leftover: Leftover,
    pub(crate) state: RowState,
}

/// One worktree to remove and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Removal {
    pub(crate) worktree: Worktree,
    pub(crate) branch: BranchChoice,
    pub(crate) mode: RemoveMode,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Cleanup {
    pub(crate) rows: Vec<Row>,
}

/// The branch goes with the worktree only when nothing would be lost with it.
fn branch_for(inspection: Inspection) -> BranchChoice {
    if inspection.unmerged == 0 { BranchChoice::Delete } else { BranchChoice::Keep }
}

impl Cleanup {
    /// The rows for `leftovers`, and the worktrees to look at.
    pub(crate) fn open(leftovers: &[Leftover]) -> (Self, Vec<Worktree>) {
        let mut to_inspect = Vec::new();
        let rows = leftovers
            .iter()
            .map(|leftover| {
                let state = match leftover.worktree() {
                    Some(worktree) => {
                        to_inspect.push(worktree);
                        RowState::Inspecting
                    }
                    None => RowState::Unrecorded,
                };
                Row { leftover: leftover.clone(), state }
            })
            .collect();
        (Self { rows }, to_inspect)
    }

    fn row_mut(&mut self, path: &Path) -> Option<&mut Row> {
        self.rows.iter_mut().find(|row| row.leftover.path == path)
    }

    pub(crate) fn inspected(&mut self, path: &Path, inspection: Inspection) {
        if let Some(row) = self.row_mut(path).filter(|row| row.state == RowState::Inspecting) {
            row.state = RowState::Ready(inspection);
        }
    }

    fn removal(row: &Row, mode: RemoveMode) -> Option<Removal> {
        let RowState::Ready(inspection) = row.state else { return None };
        Some(Removal { worktree: row.leftover.worktree()?, branch: branch_for(inspection), mode })
    }

    /// What Clean up removes: every inspected worktree with nothing uncommitted in it.
    pub(crate) fn clean_removals(&self) -> Vec<Removal> {
        self.rows
            .iter()
            .filter(|row| matches!(row.state, RowState::Ready(i) if i.changed == 0))
            .filter_map(|row| Self::removal(row, RemoveMode::Safe))
            .collect()
    }

    /// The explicit discard of one worktree that has uncommitted changes.
    pub(crate) fn discard_removal(&self, path: &Path) -> Option<Removal> {
        let row = self.rows.iter().find(|row| row.leftover.path == path)?;
        matches!(row.state, RowState::Ready(i) if i.changed > 0)
            .then(|| Self::removal(row, RemoveMode::Discard))
            .flatten()
    }

    pub(crate) fn removing(&mut self, path: &Path) {
        if let Some(row) = self.row_mut(path) {
            row.state = RowState::Removing;
        }
    }

    /// A finished removal leaves the list, a failed one stays with git's reason. Returns whether
    /// the worktree is gone.
    pub(crate) fn removed(&mut self, path: &Path, result: Result<BranchOutcome, String>) -> bool {
        match result {
            Ok(_) => {
                self.rows.retain(|row| row.leftover.path != path);
                true
            }
            Err(message) => {
                if let Some(row) = self.row_mut(path) {
                    row.state = RowState::Failed(message);
                }
                false
            }
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.rows.iter().any(|row| matches!(row.state, RowState::Inspecting | RowState::Removing))
    }

    pub(crate) fn clean_count(&self) -> usize {
        self.clean_removals().len()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What the row says about the worktree, in the close dialog's words.
pub(crate) fn status_text(state: &RowState) -> String {
    match state {
        RowState::Inspecting => "looking...".to_owned(),
        RowState::Unrecorded => "not made by this Tidewatch, remove it with git".to_owned(),
        RowState::Removing => "removing...".to_owned(),
        RowState::Failed(message) => format!("could not remove: {message}"),
        RowState::Ready(i) => {
            let mut parts = Vec::new();
            if i.changed > 0 {
                parts.push(format!(
                    "{} with uncommitted changes",
                    plural(i.changed, "file", "files")
                ));
            }
            if i.unmerged > 0 {
                parts.push(format!(
                    "{} only on its branch (the branch is kept)",
                    plural(i.unmerged, "commit", "commits")
                ));
            }
            if parts.is_empty() { "clean".to_owned() } else { parts.join(", ") }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn leftover(name: &str, recorded: bool) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: recorded.then(|| format!("agent/{name}")),
            repo: recorded.then(|| PathBuf::from("/work/web")),
        }
    }

    fn ready(changed: usize, unmerged: usize) -> RowState {
        RowState::Ready(Inspection { changed, unmerged })
    }

    type Found = Option<(usize, usize)>;

    fn opened(list: &[(&str, bool, Found)]) -> Cleanup {
        let leftovers: Vec<Leftover> = list.iter().map(|(n, rec, _)| leftover(n, *rec)).collect();
        let (mut cleanup, _) = Cleanup::open(&leftovers);
        for ((name, _, found), _) in list.iter().zip(&leftovers) {
            if let Some((changed, unmerged)) = found {
                cleanup.inspected(
                    &leftover(name, true).path,
                    Inspection { changed: *changed, unmerged: *unmerged },
                );
            }
        }
        cleanup
    }

    #[test]
    fn opening_asks_to_inspect_only_what_git_can_remove() {
        let (cleanup, to_inspect) =
            Cleanup::open(&[leftover("calm-comet", true), leftover("by-hand", false)]);
        assert_eq!(to_inspect.len(), 1);
        assert_eq!(to_inspect[0].branch, "agent/calm-comet");
        assert_eq!(cleanup.rows[0].state, RowState::Inspecting);
        assert_eq!(cleanup.rows[1].state, RowState::Unrecorded);
        assert!(cleanup.busy());
    }

    #[test]
    fn clean_up_takes_the_clean_ones_and_never_a_dirty_one() {
        let cleanup = opened(&[
            ("clean", true, Some((0, 0))),
            ("dirty", true, Some((3, 0))),
            ("still-looking", true, None),
            ("by-hand", false, None),
        ]);
        let removals = cleanup.clean_removals();
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].worktree.branch, "agent/clean");
        assert_eq!(removals[0].mode, RemoveMode::Safe);
        assert_eq!(cleanup.clean_count(), 1);
    }

    #[test]
    fn a_branch_with_commits_found_nowhere_else_is_kept_and_an_empty_one_goes() {
        let cleanup = opened(&[("empty", true, Some((0, 0))), ("work", true, Some((0, 4)))]);
        let removals = cleanup.clean_removals();
        assert_eq!(removals[0].branch, BranchChoice::Delete);
        assert_eq!(removals[1].branch, BranchChoice::Keep);
    }

    #[test]
    fn discarding_is_a_separate_explicit_removal_for_a_dirty_worktree_only() {
        let cleanup = opened(&[("dirty", true, Some((2, 1))), ("clean", true, Some((0, 0)))]);
        let dirty = cleanup.rows[0].leftover.path.clone();
        let clean = cleanup.rows[1].leftover.path.clone();
        let removal = cleanup.discard_removal(&dirty).expect("a discard");
        assert_eq!((removal.mode, removal.branch), (RemoveMode::Discard, BranchChoice::Keep));
        assert_eq!(cleanup.discard_removal(&clean), None, "a clean one needs no discard");
        assert_eq!(cleanup.discard_removal(Path::new("/nowhere")), None);
    }

    #[test]
    fn a_removed_row_leaves_and_a_failed_one_stays_with_the_reason() {
        let mut cleanup = opened(&[("a", true, Some((0, 0))), ("b", true, Some((0, 0)))]);
        let (a, b) = (cleanup.rows[0].leftover.path.clone(), cleanup.rows[1].leftover.path.clone());
        cleanup.removing(&a);
        cleanup.removing(&b);
        assert!(cleanup.busy());
        assert!(cleanup.removed(&a, Ok(BranchOutcome::Deleted)));
        assert!(!cleanup.removed(&b, Err("worktree is locked".into())));
        assert_eq!(cleanup.rows.len(), 1);
        assert_eq!(cleanup.rows[0].leftover.path, b);
        assert_eq!(cleanup.rows[0].state, RowState::Failed("worktree is locked".into()));
        assert!(!cleanup.busy());
        assert_eq!(cleanup.clean_count(), 0, "a failed row is not retried by itself");
    }

    #[test]
    fn a_late_inspection_never_overwrites_a_row_that_moved_on() {
        let mut cleanup = opened(&[("a", true, Some((0, 0)))]);
        let a = cleanup.rows[0].leftover.path.clone();
        cleanup.removing(&a);
        cleanup.inspected(&a, Inspection { changed: 9, unmerged: 9 });
        assert_eq!(cleanup.rows[0].state, RowState::Removing);
    }

    #[test]
    fn the_status_text_names_changes_and_unmerged_commits() {
        assert_eq!(status_text(&ready(0, 0)), "clean");
        assert_eq!(status_text(&ready(1, 0)), "1 file with uncommitted changes");
        assert_eq!(
            status_text(&ready(2, 3)),
            "2 files with uncommitted changes, 3 commits only on its branch (the branch is kept)"
        );
        assert_eq!(status_text(&ready(0, 1)), "1 commit only on its branch (the branch is kept)");
        assert_eq!(status_text(&RowState::Failed("locked".into())), "could not remove: locked");
    }
}
