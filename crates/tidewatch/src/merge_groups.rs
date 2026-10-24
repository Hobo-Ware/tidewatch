//! Folding the tab groups of one repo into a single group, in a window that already holds them: a
//! window restarted for an update keeps the groups it had, and a group made for a linked worktree
//! by an older version is one of them. Which groups belong together is decided from `Main`, what
//! git says each group's folder belongs to (looked up off the UI thread); the plan and the fold
//! itself are pure and generic over the pty, so they are tested without one.
//!
//! A fold touches no terminal: every tab keeps its id, folder, pty, agent and marks. Only the main
//! tab of a folded group is special: a main shell that is idle goes (a fresh one is the repo's own
//! main), one that runs a command stays as an ordinary shell tab of the merged group.

use std::path::{Path, PathBuf};

use tidewatch_core::session::{SavedTab, SavedWindow};

use crate::pane::Pane;
use crate::repo::{Repo, Workspace};
use crate::single_instance::workspace_key;
use crate::tabs::Tabs;

/// The repo a group's folder belongs to, as git says: a linked worktree and a subfolder belong to
/// the main repo that holds their git dir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Main {
    pub(crate) repo: Repo,
    /// `repo.root` canonicalised, which is what two groups are compared by.
    pub(crate) key: PathBuf,
    /// The group is already at the repo's root.
    pub(crate) at_home: bool,
}

/// What the plan needs of one group of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    /// `None` for the Other group, a folder in no repo, or one git could not be asked about.
    pub(crate) main: Option<Main>,
    /// The group's main shell runs a command.
    pub(crate) main_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Home {
    /// The group at this position, which is at the repo's root.
    Existing(usize),
    /// No group is at the repo's root: one is made, after the others.
    Create(Repo),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) group: usize,
    /// Its main shell stays as a tab of the merged group instead of going.
    pub(crate) keep_main: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Merge {
    pub(crate) home: Home,
    /// In group order: their tabs follow the home's own, in this order.
    pub(crate) sources: Vec<Source>,
}

/// `Main` of every group, for the groups' folders. Runs git, so never on the UI thread.
pub(crate) fn resolve(groups: &[(PathBuf, bool)]) -> Vec<Option<Main>> {
    groups
        .iter()
        .map(|(dir, other)| {
            let repo =
                dir.is_dir().then(|| Repo::discover_main(dir)).flatten().filter(|_| !other)?;
            let key = workspace_key(&repo.root);
            Some(Main { at_home: key == workspace_key(dir), key, repo })
        })
        .collect()
}

pub(crate) fn facts<T>(
    mains: Vec<Option<Main>>,
    tabs: &Tabs<Pane<T>>,
    busy: impl Fn(&Pane<T>) -> bool,
) -> Vec<Facts> {
    mains
        .into_iter()
        .enumerate()
        .map(|(group, main)| {
            let main_tab = tabs.in_group(group).find(|(_, tab)| tab.pinned);
            Facts { main, main_busy: main_tab.is_some_and(|(_, tab)| busy(&tab.pane)) }
        })
        .collect()
}

/// The folds that make one group per repo. Groups of the same repo become the one at its root, else
/// a new one for the root; a group that is the only one of its repo and at its root is left alone.
pub(crate) fn plan(groups: &[Facts]) -> Vec<Merge> {
    let mut merges = Vec::new();
    let mut seen: Vec<&Path> = Vec::new();
    for (at, group) in groups.iter().enumerate() {
        let Some(main) = &group.main else { continue };
        if seen.contains(&main.key.as_path()) {
            continue;
        }
        seen.push(&main.key);
        let kin: Vec<usize> = (at..groups.len())
            .filter(|other| groups[*other].main.as_ref().is_some_and(|m| m.key == main.key))
            .collect();
        let home = kin
            .iter()
            .copied()
            .find(|other| groups[*other].main.as_ref().is_some_and(|main| main.at_home));
        let sources: Vec<Source> = kin
            .into_iter()
            .filter(|other| Some(*other) != home)
            .map(|group| Source { group, keep_main: groups[group].main_busy })
            .collect();
        if !sources.is_empty() {
            let home = home.map_or_else(|| Home::Create(main.repo.clone()), Home::Existing);
            merges.push(Merge { home, sources });
        }
    }
    merges
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// Main tabs that went, for the window to forget.
    pub(crate) dropped: Vec<u64>,
    /// The roots of the groups that were made.
    pub(crate) created: Vec<PathBuf>,
    /// One per folded group, with just its main tab, so a saved session no longer holds the tabs
    /// that now live in the merged group.
    pub(crate) retired: Vec<SavedWindow>,
}

/// Carries out `merges` on the window's tabs and workspaces, which are parallel (group n of `tabs`
/// is `workspaces[n]`). `spawn_main` makes the main shell of a group that is created.
pub(crate) fn apply<T>(
    tabs: &mut Tabs<Pane<T>>,
    workspaces: &mut Vec<Workspace>,
    merges: &[Merge],
    mut spawn_main: impl FnMut(&Path) -> T,
) -> Applied {
    let mut applied = Applied::default();
    let mains: Vec<Option<u64>> = (0..workspaces.len()).map(|group| tabs.main_id(group)).collect();
    for merge in merges {
        let home = match &merge.home {
            Home::Existing(group) => mains[*group],
            Home::Create(repo) => {
                let group = tabs.add_group(Pane::shell(spawn_main(&repo.root)));
                workspaces.push(Workspace::Repo(repo.clone()));
                applied.created.push(repo.root.clone());
                tabs.main_id(group)
            }
        };
        for source in &merge.sources {
            let ids = home.zip(mains[source.group]);
            let groups = ids.and_then(|(home, from)| tabs.group_of(home).zip(tabs.group_of(from)));
            let Some((target, from)) = groups else { continue };
            let title = source.keep_main.then(|| folder_name(workspaces[from].dir()));
            applied.dropped.extend(tabs.merge_group(from, target, title.as_deref()));
            applied.retired.push(only_main(&workspaces.remove(from)));
        }
    }
    applied
}

fn folder_name(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn only_main(workspace: &Workspace) -> SavedWindow {
    SavedWindow {
        dir: workspace.dir().to_path_buf(),
        other: false,
        active: 0,
        focus_mode: false,
        tabs: vec![SavedTab::Main],
        parked: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
