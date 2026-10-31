//! What the leftover worktrees dialog knows: one row per worktree no tab owns, what removing it
//! would lose, and the removals that Clean up and Discard turn into. Pure. Clean up takes only the
//! worktrees proven safe to lose nothing; every other row shows why, and only its own Discard,
//! chosen by the user for that row, removes it.

use std::path::{Path, PathBuf};

use tidewatch_core::worktree::{BranchChoice, BranchOutcome, Leftover, RemoveMode, Worktree};
use tidewatch_core::worktree_safety::Safety;

use crate::review::plural;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Inspecting,
    Ready(Safety),
    /// Git cannot read the folder, so nothing in it is known. Its Discard moves it to the Trash.
    Unreadable,
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
    pub(crate) spare_unique: bool,
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
                    None => RowState::Unreadable,
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
                    spare_unique: false,
                })
            })
            .collect()
    }

    /// The explicit discard of one worktree that was not proven safe. `branch` is the user's
    /// choice: only `Delete` can lose commits.
    pub(crate) fn discard_removal(&self, path: &Path, branch: BranchChoice) -> Option<Removal> {
        let row = self.rows.iter().find(|row| row.leftover.path == path)?;
        let RowState::Ready(safety) = &row.state else { return None };
        let spare_unique = !row.leftover.recorded;
        (!safety.is_safe())
            .then(|| {
                Some(Removal {
                    worktree: row.leftover.worktree()?,
                    branch,
                    mode: RemoveMode::Discard,
                    spare_unique,
                })
            })
            .flatten()
    }

    /// Whether the row has something to show the user and a Discard to offer.
    pub(crate) fn is_discardable(row: &Row) -> bool {
        match &row.state {
            RowState::Ready(safety) => !safety.is_safe(),
            RowState::Unreadable => true,
            _ => false,
        }
    }

    /// The folder to move to the Trash for the Discard of a row git cannot read, and the worktree
    /// the registry names for it, when it does.
    pub(crate) fn trash_target(&self, path: &Path) -> Option<(PathBuf, Option<Worktree>)> {
        let row = self.rows.iter().find(|row| row.leftover.path == path)?;
        (row.state == RowState::Unreadable).then(|| {
            (row.leftover.path.clone(), row.leftover.worktree().filter(|_| row.leftover.recorded))
        })
    }

    /// A folder the check could not read turned out to be one git cannot read at all.
    pub(crate) fn unreadable(&mut self, path: &Path) {
        if let Some(row) = self
            .row_mut(path)
            .filter(|row| matches!(row.state, RowState::Inspecting | RowState::Ready(_)))
        {
            row.state = RowState::Unreadable;
        }
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

    /// Why Clean up cannot be pressed now, for its tooltip. `None` when it has worktrees to remove.
    pub(crate) fn clean_up_blocker(&self) -> Option<&'static str> {
        if self.busy() {
            Some("Still looking at the worktrees.")
        } else if self.clean_count() > 0 {
            None
        } else {
            Some(NOTHING_SAFE)
        }
    }
}

const NOTHING_SAFE: &str = "Nothing here is proven safe to remove. A row that lists what it would lose has its own Discard.";

/// What the row says about the worktree, in the close dialog's words.
pub(crate) fn status_text(state: &RowState) -> String {
    match state {
        RowState::Inspecting => "looking...".to_owned(),
        RowState::Unreadable => {
            "git can't read this folder; Discard moves it to the Trash".to_owned()
        }
        RowState::Removing => "removing...".to_owned(),
        RowState::Failed(message) => format!("could not remove: {message}"),
        RowState::Ready(Safety::Safe) => "clean, nothing would be lost".to_owned(),
        RowState::Ready(safety) => safety.reasons().join(", "),
    }
}

/// How long the folder has sat there, for the row's right edge: `under an hour`, `5 hours`,
/// `3 days`.
pub(crate) fn age_text(now_ms: i64, modified_ms: i64) -> String {
    let minutes = (now_ms - modified_ms).max(0) / 60_000;
    match minutes {
        0..60 => "under an hour".to_owned(),
        60..2880 => plural(usize::try_from(minutes / 60).unwrap_or(0), "hour"),
        _ => plural(usize::try_from(minutes / 1440).unwrap_or(0), "day"),
    }
}

/// What the right edge of a row says: the age of a worktree proven safe, what the others would
/// lose or why they cannot be read.
pub(crate) fn right_text(row: &Row, now_ms: i64) -> String {
    let age = row.leftover.modified_ms.map(|at| age_text(now_ms, at));
    match (&row.state, row.leftover.recorded, age) {
        (RowState::Ready(Safety::Safe), true, age) => age.unwrap_or_default(),
        (RowState::Ready(Safety::Safe), false, Some(age)) => {
            format!("nothing to lose \u{b7} {age}")
        }
        (RowState::Ready(Safety::Safe), false, None) => "nothing to lose".to_owned(),
        (state, _, _) => status_text(state),
    }
}

/// The branch a row shows after its name: none when it is the `agent/<name>` Tidewatch gave it.
pub(crate) fn shown_branch(row: &Row, name: &str) -> Option<String> {
    row.leftover.branch.clone().filter(|branch| *branch != format!("agent/{name}"))
}

/// The line over the list: how many worktrees no tab owns, and how many of them are safe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) lead: String,
    pub(crate) rest: String,
}

pub(crate) fn summary(cleanup: &Cleanup) -> Summary {
    let total = cleanup.rows.len();
    let safe = cleanup.clean_count();
    let lead = format!("{} no tab owns.", plural(total, "worktree"));
    let rest = if total == 0 {
        String::new()
    } else if cleanup.busy() {
        "Looking at what each one would lose...".to_owned()
    } else if safe == total && total == 1 {
        "It is safe to remove: nothing uncommitted, nothing unpushed.".to_owned()
    } else if safe == total {
        format!("All {total} are safe to remove: nothing uncommitted, nothing unpushed.")
    } else if safe == 0 {
        "None is proven safe to remove. Each row says what it would lose.".to_owned()
    } else {
        let verb = if safe == 1 { "is" } else { "are" };
        format!("{safe} {verb} safe to remove. The others say what they would lose.")
    };
    Summary { lead, rest }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::worktree_safety::Risk;

    fn leftover(name: &str, recorded: bool) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: Some(format!("agent/{name}")),
            repo: Some(PathBuf::from("/work/web")),
            recorded,
            modified_ms: None,
        }
    }

    fn unreadable(name: &str) -> Leftover {
        Leftover {
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: None,
            repo: None,
            recorded: false,
            modified_ms: None,
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

    fn opened_unreadable(names: &[&str]) -> Cleanup {
        let leftovers: Vec<Leftover> = names.iter().map(|n| unreadable(n)).collect();
        Cleanup::open(&leftovers).0
    }

    #[test]
    fn opening_asks_to_inspect_everything_git_can_remove_recorded_or_not() {
        let (cleanup, to_inspect) = Cleanup::open(&[
            leftover("calm-comet", true),
            leftover("by-hand", false),
            unreadable("broken"),
        ]);
        assert_eq!(to_inspect.len(), 2);
        assert_eq!(to_inspect[0].branch, "agent/calm-comet");
        assert_eq!(to_inspect[1].branch, "agent/by-hand");
        assert_eq!(cleanup.rows[0].state, RowState::Inspecting);
        assert_eq!(cleanup.rows[1].state, RowState::Inspecting);
        assert_eq!(cleanup.rows[2].state, RowState::Unreadable);
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
            ("by-hand-proven", false, Some(Safety::Safe)),
            ("by-hand-dirty", false, Some(unsafe_with(Risk::Uncommitted { files: 1 }))),
        ]);
        let removals = cleanup.clean_removals();
        let branches: Vec<&str> = removals.iter().map(|r| r.worktree.branch.as_str()).collect();
        assert_eq!(branches, ["agent/proven", "agent/by-hand-proven"]);
        assert!(
            removals.iter().all(|r| (r.mode, r.branch) == (RemoveMode::Safe, BranchChoice::Delete))
        );
        assert_eq!(cleanup.clean_count(), 2);
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
    fn an_unrecorded_row_gets_a_discard_that_spares_a_branch_with_commits_found_nowhere_else() {
        let cleanup = opened(&[
            ("dirty", false, Some(unsafe_with(Risk::Uncommitted { files: 2 }))),
            ("unpushed", false, Some(unsafe_with(Risk::Unpushed { commits: 2 }))),
            ("unknown", false, Some(Safety::Unknown("no git".into()))),
            ("ours", true, Some(unsafe_with(Risk::Uncommitted { files: 2 }))),
        ]);
        for row in &cleanup.rows {
            assert!(Cleanup::is_discardable(row), "{}", row.leftover.path.display());
            for asked in [BranchChoice::Keep, BranchChoice::Delete] {
                let removal =
                    cleanup.discard_removal(&row.leftover.path, asked).expect("a discard");
                assert_eq!((removal.mode, removal.branch), (RemoveMode::Discard, asked));
                assert_eq!(removal.spare_unique, !row.leftover.recorded);
            }
        }
    }

    #[test]
    fn a_folder_git_cannot_read_has_a_discard_that_moves_it_to_the_trash() {
        let cleanup = opened_unreadable(&["broken"]);
        let path = cleanup.rows[0].leftover.path.clone();
        assert!(Cleanup::is_discardable(&cleanup.rows[0]));
        assert_eq!(cleanup.trash_target(&path), Some((path.clone(), None)));
        assert_eq!(cleanup.discard_removal(&path, BranchChoice::Keep), None);
        assert_eq!(cleanup.trash_target(Path::new("/nowhere")), None);
        assert_eq!(cleanup.clean_up_blocker(), Some(NOTHING_SAFE));
        let ready = opened(&[("x", false, Some(Safety::Safe))]);
        assert_eq!(ready.trash_target(&ready.rows[0].leftover.path), None);
    }

    #[test]
    fn a_recorded_row_git_turns_out_not_to_read_becomes_unreadable_and_keeps_its_worktree() {
        let mut cleanup = opened(&[
            ("dead", true, Some(Safety::Unknown("git status failed".into()))),
            ("dirty", true, Some(unsafe_with(Risk::Uncommitted { files: 1 }))),
        ]);
        let (dead, dirty) =
            (cleanup.rows[0].leftover.path.clone(), cleanup.rows[1].leftover.path.clone());
        assert_eq!(cleanup.trash_target(&dead), None);
        cleanup.unreadable(&dead);
        assert_eq!(cleanup.rows[0].state, RowState::Unreadable);
        let (path, known) = cleanup.trash_target(&dead).expect("a trash target");
        assert_eq!((path, known.map(|w| w.branch)), (dead.clone(), Some("agent/dead".to_owned())));
        assert_eq!(cleanup.discard_removal(&dead, BranchChoice::Delete), None);
        cleanup.removing(&dirty);
        cleanup.unreadable(&dirty);
        assert_eq!(cleanup.rows[1].state, RowState::Removing, "a row that moved on is left alone");
    }

    #[test]
    fn the_right_edge_lists_what_an_unrecorded_worktree_would_lose() {
        let cases: [(&str, bool, Safety, Option<&str>); 6] = [
            ("recorded and clean says nothing", true, Safety::Safe, Some("")),
            ("unrecorded and clean", false, Safety::Safe, Some("nothing to lose")),
            (
                "unrecorded and dirty",
                false,
                unsafe_with(Risk::Uncommitted { files: 4 }),
                Some("4 files with uncommitted changes"),
            ),
            (
                "unrecorded with unpushed commits",
                false,
                unsafe_with(Risk::Unpushed { commits: 2 }),
                Some("2 commits found nowhere else (on no remote branch, not merged)"),
            ),
            (
                "unrecorded and in use",
                false,
                Safety::Unsafe(vec![
                    Risk::Running { pid: 7, command: "claude".into() },
                    Risk::RecentSession { minutes: 20 },
                ]),
                Some("claude (pid 7) is running in it, an agent session wrote in it 20 min ago"),
            ),
            (
                "unrecorded and unreadable status",
                false,
                Safety::Unknown("git status failed".into()),
                Some("could not check it: git status failed"),
            ),
        ];
        for (what, recorded, safety, want) in cases {
            let cleanup = opened(&[("x", recorded, Some(safety))]);
            assert_eq!(Some(right_text(&cleanup.rows[0], 0).as_str()), want, "{what}");
        }
        let looking = opened(&[("x", false, None)]);
        assert_eq!(right_text(&looking.rows[0], 0), "looking...");
        let broken = opened_unreadable(&["x"]);
        assert_eq!(
            right_text(&broken.rows[0], 0),
            "git can't read this folder; Discard moves it to the Trash"
        );
    }

    #[test]
    fn a_safe_row_says_how_long_it_has_sat_there() {
        const HOUR: i64 = 3_600_000;
        let cases = [
            (30 * 60_000, "under an hour"),
            (HOUR, "1 hour"),
            (5 * HOUR, "5 hours"),
            (47 * HOUR, "47 hours"),
            (48 * HOUR, "2 days"),
            (72 * HOUR, "3 days"),
            (-HOUR, "under an hour"),
        ];
        for (idle, want) in cases {
            assert_eq!(age_text(1_000_000_000, 1_000_000_000 - idle), want, "{idle}");
        }
        let mut cleanup =
            opened(&[("old", true, Some(Safety::Safe)), ("by-hand", false, Some(Safety::Safe))]);
        for row in &mut cleanup.rows {
            row.leftover.modified_ms = Some(1_000_000_000 - 72 * HOUR);
        }
        assert_eq!(right_text(&cleanup.rows[0], 1_000_000_000), "3 days");
        assert_eq!(right_text(&cleanup.rows[1], 1_000_000_000), "nothing to lose \u{b7} 3 days");
    }

    #[test]
    fn the_branch_is_shown_only_when_it_is_not_agent_slash_the_name() {
        let cleanup = opened(&[("calm-comet", true, None)]);
        let row = &cleanup.rows[0];
        assert_eq!(shown_branch(row, "calm-comet"), None);
        assert_eq!(shown_branch(row, "other"), Some("agent/calm-comet".to_owned()));
        let bare = opened_unreadable(&["x"]);
        assert_eq!(shown_branch(&bare.rows[0], "x"), None);
    }

    #[test]
    fn the_summary_says_how_many_are_safe() {
        let safe = || Some(Safety::Safe);
        let dirty = || Some(unsafe_with(Risk::Uncommitted { files: 1 }));
        let cases = [
            (opened(&[]), "0 worktrees no tab owns.", ""),
            (
                opened(&[("a", true, safe())]),
                "1 worktree no tab owns.",
                "It is safe to remove: nothing uncommitted, nothing unpushed.",
            ),
            (
                opened(&[("a", true, safe()), ("b", true, safe())]),
                "2 worktrees no tab owns.",
                "All 2 are safe to remove: nothing uncommitted, nothing unpushed.",
            ),
            (
                opened(&[("a", true, safe()), ("b", true, dirty()), ("c", true, dirty())]),
                "3 worktrees no tab owns.",
                "1 is safe to remove. The others say what they would lose.",
            ),
            (
                opened(&[("a", true, dirty())]),
                "1 worktree no tab owns.",
                "None is proven safe to remove. Each row says what it would lose.",
            ),
            (
                opened(&[("a", true, safe()), ("b", true, None)]),
                "2 worktrees no tab owns.",
                "Looking at what each one would lose...",
            ),
        ];
        for (cleanup, lead, rest) in cases {
            assert_eq!(summary(&cleanup), Summary { lead: lead.into(), rest: rest.into() });
        }
    }

    #[test]
    fn clean_up_says_why_it_has_nothing_to_do() {
        let cases: [(&str, Cleanup, Option<&str>); 5] = [
            ("a proven row", opened(&[("a", true, Some(Safety::Safe))]), None),
            ("only unreadable rows", opened_unreadable(&["a", "b"]), Some(NOTHING_SAFE)),
            (
                "only rows with something to lose",
                opened(&[("a", true, Some(unsafe_with(Risk::Uncommitted { files: 1 })))]),
                Some(NOTHING_SAFE),
            ),
            (
                "an unrecorded one next to one with something to lose",
                opened(&[
                    ("a", false, Some(unsafe_with(Risk::Uncommitted { files: 1 }))),
                    ("b", true, Some(unsafe_with(Risk::Unpushed { commits: 1 }))),
                ]),
                Some(NOTHING_SAFE),
            ),
            (
                "a row still being looked at next to a proven one",
                opened(&[("a", true, Some(Safety::Safe)), ("b", true, None)]),
                Some("Still looking at the worktrees."),
            ),
        ];
        for (what, cleanup, want) in cases {
            assert_eq!(cleanup.clean_up_blocker(), want, "{what}");
        }
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
