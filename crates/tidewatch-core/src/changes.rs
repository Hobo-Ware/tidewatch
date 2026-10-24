//! The uncommitted files of a checkout in the three groups the Changes tab lists: staged, changed
//! (tracked files edited and not staged) and new (untracked). The tab only views them.

use std::collections::HashSet;

use kelp_core::commit::{ChangeKind, FileChange};
use kelp_core::status::WorkingStatus;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Groups {
    pub staged: Vec<FileChange>,
    pub changed: Vec<FileChange>,
    pub new_files: Vec<FileChange>,
}

impl Groups {
    #[must_use]
    pub fn of(status: &WorkingStatus) -> Self {
        let untracked: HashSet<&str> = status.untracked.iter().map(String::as_str).collect();
        let mut changed: Vec<FileChange> = status
            .unstaged
            .iter()
            .filter(|change| !untracked.contains(change.path.as_str()))
            .cloned()
            .collect();
        changed.extend(
            status
                .conflicted
                .iter()
                .map(|path| FileChange { path: path.clone(), kind: ChangeKind::Modified }),
        );
        changed.sort_by(|a, b| a.path.cmp(&b.path));
        let mut new_files: Vec<FileChange> = status
            .untracked
            .iter()
            .map(|path| FileChange { path: path.clone(), kind: ChangeKind::Added })
            .collect();
        new_files.sort_by(|a, b| a.path.cmp(&b.path));
        Self { staged: status.staged.clone(), changed, new_files }
    }

    #[must_use]
    pub fn ordered(&self) -> Vec<FileChange> {
        let mut seen = HashSet::new();
        self.staged
            .iter()
            .chain(&self.changed)
            .chain(&self.new_files)
            .filter(|change| seen.insert(change.path.as_str()))
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn files(&self) -> usize {
        self.staged
            .iter()
            .chain(&self.changed)
            .chain(&self.new_files)
            .map(|change| change.path.as_str())
            .collect::<HashSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::status::parse_porcelain;

    fn paths(list: &[FileChange]) -> Vec<(&str, ChangeKind)> {
        list.iter().map(|change| (change.path.as_str(), change.kind)).collect()
    }

    const OUT: &str = "MM both.rs\0M  staged.rs\0 M edited.rs\0A  made.rs\0 D gone.rs\0?? notes.md\0?? a/new.txt\0UU clash.rs\0";

    #[test]
    fn staged_changed_and_new_files_land_in_their_own_group() {
        let groups = Groups::of(&parse_porcelain(OUT));
        assert_eq!(
            paths(&groups.staged),
            [
                ("both.rs", ChangeKind::Modified),
                ("made.rs", ChangeKind::Added),
                ("staged.rs", ChangeKind::Modified)
            ]
        );
        assert_eq!(
            paths(&groups.changed),
            [
                ("both.rs", ChangeKind::Modified),
                ("clash.rs", ChangeKind::Modified),
                ("edited.rs", ChangeKind::Modified),
                ("gone.rs", ChangeKind::Deleted)
            ]
        );
        assert_eq!(
            paths(&groups.new_files),
            [("a/new.txt", ChangeKind::Added), ("notes.md", ChangeKind::Added)]
        );
    }

    #[test]
    fn a_file_staged_and_edited_again_counts_once() {
        let groups = Groups::of(&parse_porcelain(OUT));
        assert_eq!(groups.staged.len() + groups.changed.len() + groups.new_files.len(), 9);
        assert_eq!(groups.files(), 8);
    }

    #[test]
    fn the_ordered_list_follows_the_groups_and_names_each_file_once() {
        let groups = Groups::of(&parse_porcelain(OUT));
        let ordered = groups.ordered();
        let order: Vec<&str> = ordered.iter().map(|change| change.path.as_str()).collect();
        assert_eq!(
            order,
            [
                "both.rs",
                "made.rs",
                "staged.rs",
                "clash.rs",
                "edited.rs",
                "gone.rs",
                "a/new.txt",
                "notes.md"
            ]
        );
    }

    #[test]
    fn a_clean_tree_has_no_files_and_no_groups() {
        let groups = Groups::of(&parse_porcelain(""));
        assert_eq!(groups, Groups::default());
        assert_eq!(groups.files(), 0);
    }

    #[test]
    fn the_count_matches_what_the_graph_row_counts() {
        let status = parse_porcelain(OUT);
        assert_eq!(Groups::of(&status).files(), status.all().len());
    }
}
