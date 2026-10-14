//! Moving between files and between rows of the tree from the keyboard: pure index steps.

use kelp_core::commit::FileChange;
use kelp_ui::graph::Selection;

/// The file `delta` places from `current`, never wrapping. `None` when there is nowhere to go.
pub(crate) fn step_file<'a>(
    files: &'a [FileChange],
    current: Option<&str>,
    delta: isize,
) -> Option<&'a str> {
    let at = current.and_then(|path| files.iter().position(|f| f.path == path));
    let target = match at {
        Some(at) => at.checked_add_signed(delta)?,
        None => 0,
    };
    let next = files.get(target)?;
    (at != Some(target)).then_some(next.path.as_str())
}

/// The row `delta` places from `current` in the tree's order (uncommitted changes first, then the
/// commits newest first), never wrapping. `None` when there is nowhere to go.
pub(crate) fn step_row(
    current: Option<Selection>,
    has_wip: bool,
    commits: usize,
    delta: isize,
) -> Option<Selection> {
    let offset = usize::from(has_wip);
    let at = match current? {
        Selection::Wip => 0,
        Selection::Commit(row) => row + offset,
    };
    let target = at.checked_add_signed(delta).filter(|target| *target < commits + offset)?;
    if target == at {
        return None;
    }
    Some(if has_wip && target == 0 { Selection::Wip } else { Selection::Commit(target - offset) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::commit::ChangeKind;

    fn files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange { path: (*p).to_owned(), kind: ChangeKind::Modified })
            .collect()
    }

    #[test]
    fn files_step_one_at_a_time_and_stop_at_the_ends() {
        let list = files(&["a", "b", "c"]);
        assert_eq!(step_file(&list, Some("a"), 1), Some("b"));
        assert_eq!(step_file(&list, Some("b"), 1), Some("c"));
        assert_eq!(step_file(&list, Some("c"), 1), None);
        assert_eq!(step_file(&list, Some("b"), -1), Some("a"));
        assert_eq!(step_file(&list, Some("a"), -1), None);
    }

    #[test]
    fn a_file_that_is_not_in_the_list_steps_to_the_first() {
        let list = files(&["a", "b"]);
        assert_eq!(step_file(&list, Some("gone"), 1), Some("a"));
        assert_eq!(step_file(&list, None, -1), Some("a"));
        assert_eq!(step_file(&[], None, 1), None);
    }

    #[test]
    fn rows_run_from_the_uncommitted_changes_down_the_commits() {
        assert_eq!(step_row(Some(Selection::Wip), true, 3, 1), Some(Selection::Commit(0)));
        assert_eq!(step_row(Some(Selection::Commit(0)), true, 3, -1), Some(Selection::Wip));
        assert_eq!(step_row(Some(Selection::Commit(1)), true, 3, 1), Some(Selection::Commit(2)));
        assert_eq!(step_row(Some(Selection::Commit(2)), true, 3, 1), None);
        assert_eq!(step_row(Some(Selection::Wip), true, 3, -1), None);
    }

    #[test]
    fn without_uncommitted_changes_the_first_commit_is_the_top() {
        assert_eq!(step_row(Some(Selection::Commit(0)), false, 2, -1), None);
        assert_eq!(step_row(Some(Selection::Commit(0)), false, 2, 1), Some(Selection::Commit(1)));
        assert_eq!(step_row(None, false, 2, 1), None);
    }
}
