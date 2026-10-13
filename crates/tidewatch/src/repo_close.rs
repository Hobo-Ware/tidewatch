//! Closing a repo one tab at a time: which tab to close next, whether to wait for the answer to the
//! last one, and when to stop because a tab stayed.

use std::path::PathBuf;

use crate::pane::{ClosePolicy, Pane};
use crate::tabs::Tabs;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// The tab asked about last is still being closed (its worktree is going).
    Wait,
    Close(usize),
    /// Only the main tab is left.
    Done,
    /// A tab is busy and cannot be asked yet.
    Busy(String),
    /// The tab asked about last is still there: it was kept or its removal failed.
    Stop,
}

pub(crate) struct RepoClose {
    pub(crate) dir: PathBuf,
    asked: Option<u64>,
}

impl RepoClose {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir, asked: None }
    }

    /// The next move for the repo's tab group, called while no close dialog is open.
    pub(crate) fn next<T>(&mut self, group: usize, tabs: &Tabs<Pane<T>>) -> Step {
        let Some((index, tab)) = tabs.in_group(group).find(|(_, tab)| !tab.pinned) else {
            return Step::Done;
        };
        let policy = tab.pane.close_policy();
        if self.asked.is_some_and(|id| tabs.index_of(id).is_some()) {
            return if policy == ClosePolicy::Wait { Step::Wait } else { Step::Stop };
        }
        if policy == ClosePolicy::Wait {
            return Step::Busy(tab.title.clone());
        }
        self.asked = Some(tab.id);
        Step::Close(index)
    }
}

#[cfg(test)]
mod tests {
    use tidewatch_core::worktree::Worktree;

    use super::*;
    use crate::pane::Body;
    use std::path::PathBuf;

    fn worktree_pane(body: Body<u32>) -> Pane<u32> {
        let worktree = Worktree {
            repo: PathBuf::from("/work/web"),
            path: PathBuf::from("/home/.tidewatch/web-abc123/calm-comet"),
            branch: "agent/calm-comet".into(),
        };
        Pane { body, worktree: Some(worktree), agent: None }
    }

    fn repo_with(tabs: Vec<(&str, Pane<u32>)>) -> Tabs<Pane<u32>> {
        let mut all = Tabs::new(Pane::shell(0));
        for (title, pane) in tabs {
            all.push_in(0, title.into(), pane);
        }
        all
    }

    #[test]
    fn a_repo_with_only_its_main_tab_is_done() {
        let tabs = repo_with(vec![]);
        assert_eq!(RepoClose::new("/work/web".into()).next(0, &tabs), Step::Done);
    }

    #[test]
    fn it_asks_for_the_first_tab_that_is_not_pinned_and_waits_for_its_answer() {
        let mut tabs = repo_with(vec![("a", worktree_pane(Body::Shell(1))), ("b", Pane::shell(2))]);
        let mut closing = RepoClose::new("/work/web".into());
        assert_eq!(closing.next(0, &tabs), Step::Close(1));
        assert_eq!(closing.next(0, &tabs), Step::Stop, "it is still there: kept or failed");
        let asked = tabs.items()[1].id;
        tabs.get_mut(asked).unwrap().pane.begin_remove();
        assert_eq!(closing.next(0, &tabs), Step::Wait, "its worktree is being removed");
        tabs.remove_id(asked);
        assert_eq!(closing.next(0, &tabs), Step::Close(1), "the next tab moved up");
        tabs.remove_id(tabs.items()[1].id);
        assert_eq!(closing.next(0, &tabs), Step::Done);
    }

    #[test]
    fn a_tab_that_is_still_being_made_keeps_the_repo_open() {
        let tabs = repo_with(vec![("calm-comet", Pane::creating())]);
        let got = RepoClose::new("/work/web".into()).next(0, &tabs);
        assert_eq!(got, Step::Busy("calm-comet".into()));
    }

    #[test]
    fn other_repos_tabs_are_not_asked_about() {
        let mut tabs = repo_with(vec![]);
        let group = tabs.add_group(Pane::shell(5));
        tabs.push_in(0, "web work".into(), Pane::shell(6));
        tabs.push_in(group, "api work".into(), Pane::shell(7));
        let mut closing = RepoClose::new("/work/api".into());
        assert_eq!(closing.next(group, &tabs), Step::Close(3));
    }
}
