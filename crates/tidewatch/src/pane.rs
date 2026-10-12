//! What a tab holds: a shell, or the transient states of a worktree tab while git works.
//! Generic over the shell type so the transitions are testable without a pty.

use std::path::Path;

use tidewatch_core::worktree::Worktree;

use crate::tabs::Tabs;

pub(crate) const CREATING_TITLE: &str = "new worktree";
pub(crate) const FAILED_TITLE: &str = "failed";

pub(crate) enum Body<T> {
    Shell(T),
    Creating,
    Removing,
    Failed(String),
}

pub(crate) struct Pane<T> {
    pub(crate) body: Body<T>,
    pub(crate) worktree: Option<Worktree>,
}

/// What closing a tab does first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClosePolicy {
    Now,
    Confirm(Worktree),
    Wait,
}

impl<T> Pane<T> {
    pub(crate) fn shell(shell: T) -> Self {
        Self { body: Body::Shell(shell), worktree: None }
    }

    pub(crate) fn creating() -> Self {
        Self { body: Body::Creating, worktree: None }
    }

    pub(crate) fn close_policy(&self) -> ClosePolicy {
        match (&self.body, &self.worktree) {
            (Body::Creating | Body::Removing, _) => ClosePolicy::Wait,
            (_, Some(worktree)) => ClosePolicy::Confirm(worktree.clone()),
            (_, None) => ClosePolicy::Now,
        }
    }

    /// Hands back the shell, so the caller can stop it off the UI thread, and the worktree to
    /// remove. `None` while there is nothing to remove or a job is already running.
    pub(crate) fn begin_remove(&mut self) -> Option<(Option<T>, Worktree)> {
        let worktree = self.worktree.clone()?;
        match std::mem::replace(&mut self.body, Body::Removing) {
            Body::Shell(shell) => Some((Some(shell), worktree)),
            Body::Failed(_) => Some((None, worktree)),
            other @ (Body::Creating | Body::Removing) => {
                self.body = other;
                None
            }
        }
    }

    pub(crate) fn remove_failed(&mut self, message: String) {
        self.body = Body::Failed(message);
    }
}

/// Applies a finished worktree creation to its tab: the shell starts inside the new worktree and
/// the tab takes its name, or the tab shows git's error and nothing else is left behind.
pub(crate) fn finish_create<T>(
    tabs: &mut Tabs<Pane<T>>,
    id: u64,
    result: Result<Worktree, String>,
    spawn: impl FnOnce(&Path) -> T,
) {
    let Some(tab) = tabs.get_mut(id) else { return };
    if !matches!(tab.pane.body, Body::Creating) {
        return;
    }
    match result {
        Ok(worktree) => {
            worktree.name().clone_into(&mut tab.title);
            tab.pane = Pane { body: Body::Shell(spawn(&worktree.path)), worktree: Some(worktree) };
        }
        Err(message) => {
            FAILED_TITLE.clone_into(&mut tab.title);
            tab.pane.body = Body::Failed(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn shell_of(pane: &Pane<u32>) -> Option<u32> {
        match pane.body {
            Body::Shell(n) => Some(n),
            _ => None,
        }
    }

    fn worktree() -> Worktree {
        Worktree {
            repo: PathBuf::from("/work/web"),
            path: PathBuf::from("/home/.tidewatch/web-abc123/brave-otter"),
            branch: "agent/brave-otter".into(),
        }
    }

    fn tabs_with_creating() -> (Tabs<Pane<u32>>, u64) {
        let mut tabs = Tabs::new(Pane::shell(0));
        let id = tabs.push(CREATING_TITLE.into(), Pane::creating());
        (tabs, id)
    }

    #[test]
    fn created_worktree_starts_the_shell_inside_it_and_names_the_tab() {
        let (mut tabs, id) = tabs_with_creating();
        let mut started_in = None;
        finish_create(&mut tabs, id, Ok(worktree()), |path| {
            started_in = Some(path.to_path_buf());
            5
        });
        assert_eq!(started_in, Some(worktree().path));
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, "brave-otter");
        assert_eq!(shell_of(&tab.pane), Some(5));
        assert_eq!(tab.pane.worktree, Some(worktree()));
    }

    #[test]
    fn failed_creation_shows_the_error_and_keeps_no_worktree() {
        let (mut tabs, id) = tabs_with_creating();
        finish_create(&mut tabs, id, Err("disk full".into()), |_| unreachable!());
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, FAILED_TITLE);
        assert!(matches!(&tab.pane.body, Body::Failed(m) if m == "disk full"));
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Now);
    }

    #[test]
    fn a_result_for_a_closed_tab_or_a_settled_tab_is_ignored() {
        let (mut tabs, id) = tabs_with_creating();
        finish_create(&mut tabs, 99, Ok(worktree()), |_| unreachable!());
        finish_create(&mut tabs, id, Err("x".into()), |_| unreachable!());
        finish_create(&mut tabs, id, Ok(worktree()), |_| unreachable!());
        assert!(matches!(tabs.get_mut(id).unwrap().pane.body, Body::Failed(_)));
    }

    #[test]
    fn close_policy_by_tab_kind() {
        let plain = Pane::shell(1);
        assert_eq!(plain.close_policy(), ClosePolicy::Now);
        assert_eq!(Pane::<u32>::creating().close_policy(), ClosePolicy::Wait);
        let agent = Pane { body: Body::Shell(1), worktree: Some(worktree()) };
        assert_eq!(agent.close_policy(), ClosePolicy::Confirm(worktree()));
        let removing = Pane::<u32> { body: Body::Removing, worktree: Some(worktree()) };
        assert_eq!(removing.close_policy(), ClosePolicy::Wait);
        let failed = Pane::<u32> { body: Body::Failed("x".into()), worktree: Some(worktree()) };
        assert_eq!(failed.close_policy(), ClosePolicy::Confirm(worktree()));
    }

    #[test]
    fn begin_remove_hands_over_the_shell_once() {
        let mut pane = Pane { body: Body::Shell(3), worktree: Some(worktree()) };
        assert_eq!(pane.begin_remove(), Some((Some(3), worktree())));
        assert!(matches!(pane.body, Body::Removing));
        assert_eq!(pane.begin_remove(), None);
        assert!(matches!(pane.body, Body::Removing));
    }

    #[test]
    fn a_plain_shell_has_nothing_to_remove() {
        let mut pane = Pane::shell(3);
        assert_eq!(pane.begin_remove(), None);
        assert_eq!(shell_of(&pane), Some(3));
    }

    #[test]
    fn a_failed_removal_can_be_retried() {
        let mut pane = Pane { body: Body::Shell(3), worktree: Some(worktree()) };
        pane.begin_remove();
        pane.remove_failed("locked".into());
        assert!(matches!(&pane.body, Body::Failed(m) if m == "locked"));
        assert_eq!(pane.begin_remove(), Some((None, worktree())));
    }
}
