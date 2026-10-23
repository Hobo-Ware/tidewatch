//! What the merged-PR card knows about its worktree: the safety check's state and the lines the
//! card shows for it (pure, table-tested). The check itself is `tidewatch_core::worktree_cleanup`.

use tidewatch_core::worktree_cleanup::Report;

const MOST_LISTED: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Cleanup {
    #[default]
    Unchecked,
    Checking,
    Checked(Report),
    Failed(String),
}

impl Cleanup {
    pub(crate) fn wants_check(&self) -> bool {
        matches!(self, Self::Unchecked)
    }
}

fn capped(items: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = items.iter().take(MOST_LISTED).cloned().collect();
    if items.len() > MOST_LISTED {
        lines.push(format!("and {} more", items.len() - MOST_LISTED));
    }
    lines
}

/// The ignored files the removal would lose, for the card's log box.
pub(crate) fn ignored_lines(report: &Report) -> Vec<String> {
    capped(&report.ignored)
}

/// Why the worktree is kept: each blocker in words, then the files behind it.
pub(crate) fn blocker_lines(report: &Report) -> Vec<String> {
    let mut lines: Vec<String> = report.blockers().iter().map(ToString::to_string).collect();
    let files: Vec<String> = report
        .changed
        .iter()
        .map(|path| format!("  modified  {path}"))
        .chain(report.untracked.iter().map(|path| format!("  untracked  {path}")))
        .collect();
    lines.extend(capped(&files));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report { local_commits: Some(0), ..Report::default() }
    }

    #[test]
    fn only_an_unchecked_card_starts_a_check() {
        assert!(Cleanup::Unchecked.wants_check());
        assert!(!Cleanup::Checking.wants_check());
        assert!(!Cleanup::Checked(report()).wants_check());
        assert!(!Cleanup::Failed("x".into()).wants_check());
    }

    #[test]
    fn long_lists_are_cut_with_a_count_of_the_rest() {
        let names: Vec<String> = (0..11).map(|n| format!("f{n}.log")).collect();
        let lines = ignored_lines(&Report { ignored: names, ..report() });
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[7], "f7.log");
        assert_eq!(lines[8], "and 3 more");
        let few = ignored_lines(&Report { ignored: vec!["a.log".into()], ..report() });
        assert_eq!(few, ["a.log"]);
    }

    #[test]
    fn blockers_come_with_the_files_behind_them() {
        let blocked = Report {
            changed: vec!["src/lib.rs".into()],
            untracked: vec!["notes.md".into()],
            local_commits: Some(2),
            ..Report::default()
        };
        assert_eq!(
            blocker_lines(&blocked),
            [
                "1 file with uncommitted changes",
                "1 untracked file",
                "2 commits that the pull request does not have",
                "  modified  src/lib.rs",
                "  untracked  notes.md",
            ]
        );
        assert_eq!(blocker_lines(&report()), Vec::<String>::new());
    }
}
