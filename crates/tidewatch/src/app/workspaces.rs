//! The repos of the window: which one is on show, what the session file and the next launch are
//! told about them, and opening another one when a launch hands over its folder.

use std::path::{Path, PathBuf};

use tidewatch_core::last_workspace::Remembered;

use super::Tidewatch;
use crate::pane::Pane;
use crate::pty::spawn_shell;
use crate::repo::Workspace;
use crate::session_save::WindowView;
use crate::single_instance::workspace_key;
use crate::tabs::{Action, Tabs};

/// One view per repo of the window, for the session file.
pub(super) fn window_views(
    workspaces: &[Workspace],
    focus: bool,
) -> impl Iterator<Item = WindowView<'_>> + Clone {
    workspaces
        .iter()
        .enumerate()
        .map(move |(group, workspace)| WindowView::of(group, workspace, focus))
}

/// What a launch with no path reopens: the repo on show, then the others that have tabs open
/// beyond their main shell.
pub(super) fn remembered<T>(workspaces: &[Workspace], tabs: &Tabs<T>) -> Remembered {
    let front = tabs.active_group();
    let busy = |group: usize| tabs.in_group(group).any(|(_, tab)| !tab.pinned);
    Remembered {
        dir: workspaces[front].dir().to_path_buf(),
        also: workspaces
            .iter()
            .enumerate()
            .filter(|(group, _)| *group != front && busy(*group))
            .map(|(_, workspace)| workspace.dir().to_path_buf())
            .collect(),
    }
}

impl Tidewatch {
    /// The repo on show.
    pub(super) fn workspace(&self) -> &Workspace {
        &self.workspaces[self.tabs.active_group()]
    }

    pub(super) fn workspace_of_tab(&self, tab: u64) -> &Workspace {
        &self.workspaces[self.tabs.group_of(tab).unwrap_or_else(|| self.tabs.active_group())]
    }

    pub(super) fn group_of_dir(&self, dir: &Path) -> Option<usize> {
        let key = workspace_key(dir);
        self.workspaces.iter().position(|workspace| workspace_key(workspace.dir()) == key)
    }

    /// Another launch handed over a folder: the repo already open for it comes forward, and any
    /// other is looked up (git, off the UI thread) and then added as a repo of this window.
    pub(super) fn open_workspace(&mut self, path: PathBuf, ctx: &egui::Context) {
        if let Some(group) = self.group_of_dir(&path) {
            self.dispatch(Action::ShowGroup(group), ctx);
            return;
        }
        self.workspace_jobs.spawn(ctx, move || Workspace::open(&path));
    }

    /// Adds a repo with its own pinned main shell and its saved tabs, and brings it to the front.
    /// A folder inside a repo that is open already only brings that one forward.
    pub(super) fn adopt_workspace(&mut self, workspace: Workspace, ctx: &egui::Context) {
        if let Some(group) = self.group_of_dir(workspace.dir()) {
            self.dispatch(Action::ShowGroup(group), ctx);
            return;
        }
        let group = self.add_workspace(workspace, ctx);
        self.dispatch(Action::ShowGroup(group), ctx);
    }

    /// Adds a repo as a group of the window, replacing the welcome screen when that is what shows,
    /// and returns its position. What is in front stays.
    pub(super) fn add_workspace(&mut self, workspace: Workspace, ctx: &egui::Context) -> usize {
        let mut group = self.tabs.add_group(Pane::shell(spawn_shell(ctx, workspace.dir())));
        if self.welcome.take().is_some() {
            for id in self.tabs.remove_group(0) {
                self.forget_tab(id);
            }
            self.workspaces.remove(0);
            group -= 1;
        }
        self.restore.enqueue(group, &workspace);
        self.bases.refresh(ctx, workspace.dir());
        self.workspaces.push(workspace);
        if let Some(paths) = &self.paths {
            self.restore.begin(ctx, paths, &self.catalog);
        }
        group
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other(dir: &str) -> Workspace {
        Workspace::Other { dir: PathBuf::from(dir), label: dir.into() }
    }

    #[test]
    fn the_repo_on_show_comes_first_and_a_repo_with_only_its_main_shell_is_not_kept() {
        let workspaces = [other("/a"), other("/b"), other("/c")];
        let mut tabs = Tabs::new(0);
        let b = tabs.add_group(1);
        let c = tabs.add_group(2);
        tabs.push_in(0, "work".into(), 3);
        tabs.push_in(c, "work".into(), 4);
        let got = remembered(&workspaces, &tabs);
        assert_eq!(got.dir, PathBuf::from("/a"));
        assert_eq!(got.also, [PathBuf::from("/c")]);
        tabs.apply(Action::ShowGroup(b), || unreachable!());
        let got = remembered(&workspaces, &tabs);
        assert_eq!(got.dir, PathBuf::from("/b"));
        assert_eq!(got.also, [PathBuf::from("/a"), PathBuf::from("/c")]);
    }
}
