//! What the leftover worktrees dialog knows: one row per worktree no tab owns, what removing it
//! would lose, and the removals that Clean up and Discard turn into. Pure. Clean up takes only the
//! worktrees proven safe to lose nothing; every other row shows why, and only its own Discard,
//! chosen by the user for that row, removes it.

use std::path::Path;

use tidewatch_core::worktree::{BranchChoice, BranchOutcome, Leftover, RemoveMode, Worktree};
use tidewatch_core::worktree_safety::Safety;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Inspecting,
    Ready(Safety),
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

    pub(crate) fn inspected(&mut self, path: &Path, safety: Safety) {
        if let Some(row) = self.row_mut(path).filter(|row| row.state == RowState::Inspecting) {
            row.state = RowState::Ready(safety);
        }
    }

    /// What Clean up removes: the worktrees proven safe, and their branches with `git branch -d`.
    pub(crate) fn clean_removals(&self) -> Vec<Removal> {
        self.rows
            .iter()
            .filter(|row| row.state == RowState::Ready(Safety::Safe))
            .filter_map(|row| {
                Some(Removal {
                    worktree: row.leftover.worktree()?,
                    branch: BranchChoice::Delete,
                    mode: RemoveMode::Safe,
                })
            })
            .collect()
    }

    /// The explicit discard of one worktree that was not proven safe. `branch` is the user's
    /// choice: only `Delete` can lose commits.
    pub(crate) fn discard_removal(&self, path: &Path, branch: BranchChoice) -> Option<Removal> {
        let row = self.rows.iter().find(|row| row.leftover.path == path)?;
        let RowState::Ready(safety) = &row.state else { return None };
        (!safety.is_safe())
            .then(|| {
                Some(Removal {
                    worktree: row.leftover.worktree()?,
                    branch,
                    mode: RemoveMode::Discard,
                })
            })
            .flatten()
    }

    /// Whether the row has something to show the user and a Discard to offer.
    pub(crate) fn is_discardable(row: &Row) -> bool {
        matches!(&row.state, RowState::Ready(safety) if !safety.is_safe())
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

/// What the row says about the worktree, in the close dialog's words.
pub(crate) fn status_text(state: &RowState) -> String {
    match state {
        RowState::Inspecting => "looking...".to_owned(),
        RowState::Unrecorded => "not made by this Tidewatch, remove it with git".to_owned(),
        RowState::Removing => "removing...".to_owned(),
        RowState::Failed(message) => format!("could not remove: {message}"),
        RowState::Ready(Safety::Safe) => "clean, nothing would be lost".to_owned(),
        RowState::Ready(safety) => safety.reasons().join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::worktree_safety::Risk;

    fn leftover(name: &str, recorded: bool) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: recorded.then(|| format!("agent/{name}")),
            repo: recorded.then(|| PathBuf::from("/work/web")),
        }
    }

    fn unsafe_with(risk: Risk) -> Safety {
        Safety::Unsafe(vec![risk])
    }

    fn opened(list: &[(&str, bool, Option<Safety>)]) -> Cleanup {
        let leftovers: Vec<Leftover> = list.iter().map(|(n, rec, _)| leftover(n, *rec)).collect();
        let (mut cleanup, _) = Cleanup::open(&leftovers);
        for (name, _, found) in list {
            if let Some(safety) = found {
                cleanup.inspected(&leftover(name, true).path, safety.clone());
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
    fn clean_up_takes_only_what_was_proven_safe() {
        let cleanup = opened(&[
            ("proven", true, Some(Safety::Safe)),
            ("dirty", true, Some(unsafe_with(Risk::Uncommitted { files: 3 }))),
            ("unpushed", true, Some(unsafe_with(Risk::Unpushed { commits: 2 }))),
            (
                "running",
                true,
                Some(unsafe_with(Risk::Running { pid: 7, command: "claude".into() })),
            ),
            ("recent", true, Some(unsafe_with(Risk::RecentSession { minutes: 20 }))),
            ("git-error", true, Some(Safety::Unknown("git status failed".into()))),
            ("still-looking", true, None),
            ("by-hand", false, None),
        ]);
        let removals = cleanup.clean_removals();
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].worktree.branch, "agent/proven");
        assert_eq!(
            (removals[0].mode, removals[0].branch),
            (RemoveMode::Safe, BranchChoice::Delete)
        );
        assert_eq!(cleanup.clean_count(), 1);
    }

    #[test]
    fn a_discard_is_one_rows_own_removal_for_anything_not_proven_safe() {
        let cleanup = opened(&[
            ("dirty", true, Some(unsafe_with(Risk::Uncommitted { files: 2 }))),
            ("unknown", true, Some(Safety::Unknown("no git".into()))),
            ("proven", true, Some(Safety::Safe)),
        ]);
        let dirty = cleanup.rows[0].leftover.path.clone();
        let unknown = cleanup.rows[1].leftover.path.clone();
        let proven = cleanup.rows[2].leftover.path.clone();
        for branch in [BranchChoice::Keep, BranchChoice::Delete] {
            let removal = cleanup.discard_removal(&dirty, branch).expect("a discard");
            assert_eq!((removal.mode, removal.branch), (RemoveMode::Discard, branch));
        }
        assert!(cleanup.discard_removal(&unknown, BranchChoice::Keep).is_some());
        assert_eq!(cleanup.discard_removal(&proven, BranchChoice::Delete), None);
        assert_eq!(cleanup.discard_removal(Path::new("/nowhere"), BranchChoice::Keep), None);
        assert!(Cleanup::is_discardable(&cleanup.rows[0]));
        assert!(!Cleanup::is_discardable(&cleanup.rows[2]));
    }

    #[test]
    fn a_removed_row_leaves_and_a_failed_one_stays_with_the_reason() {
        let mut cleanup =
            opened(&[("a", true, Some(Safety::Safe)), ("b", true, Some(Safety::Safe))]);
        let (a, b) = (cleanup.rows[0].leftover.path.clone(), cleanup.rows[1].leftover.path.clone());
        cleanup.removing(&a);
        cleanup.removing(&b);
        assert!(cleanup.busy());
        assert!(cleanup.removed(&a, Ok(BranchOutcome::Kept)));
        assert!(!cleanup.removed(&b, Err("worktree is locked".into())));
        assert_eq!(cleanup.rows.len(), 1);
        assert_eq!(cleanup.rows[0].leftover.path, b);
        assert_eq!(cleanup.rows[0].state, RowState::Failed("worktree is locked".into()));
        assert!(!cleanup.busy());
        assert_eq!(cleanup.clean_count(), 0, "a failed row is not retried by itself");
    }

    #[test]
    fn a_late_inspection_never_overwrites_a_row_that_moved_on() {
        let mut cleanup = opened(&[("a", true, Some(Safety::Safe))]);
        let a = cleanup.rows[0].leftover.path.clone();
        cleanup.removing(&a);
        cleanup.inspected(&a, unsafe_with(Risk::Uncommitted { files: 9 }));
        assert_eq!(cleanup.rows[0].state, RowState::Removing);
    }

    #[test]
    fn the_status_text_names_every_reason() {
        let both = Safety::Unsafe(vec![
            Risk::Uncommitted { files: 2 },
            Risk::Unpushed { commits: 3 },
            Risk::Running { pid: 9, command: "claude".into() },
        ]);
        assert_eq!(status_text(&RowState::Ready(Safety::Safe)), "clean, nothing would be lost");
        assert_eq!(
            status_text(&RowState::Ready(both)),
            "2 files with uncommitted changes, 3 commits found nowhere else (on no remote branch, not merged), claude (pid 9) is running in it"
        );
        assert_eq!(
            status_text(&RowState::Ready(Safety::Unknown("git status failed".into()))),
            "could not check it: git status failed"
        );
        assert_eq!(status_text(&RowState::Failed("locked".into())), "could not remove: locked");
    }
}
