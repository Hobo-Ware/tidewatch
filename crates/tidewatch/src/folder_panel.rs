//! The panel of an agent tab that runs in a folder of its own, not in a worktree: what the folder
//! holds (the repos one level down, or the files changed in the last hour), looked up on a `Jobs`
//! thread and refreshed while the tab shows, and the repo the panel reads, which is the owner's pick,
//! else the repo the agent edited last, else the one touched last.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::Context;
use tidewatch_core::folder_repos::{self, FolderRepo, Scan, Target};

use crate::jobs::Jobs;
use crate::pane::{Body, Pane};
use crate::repo::Workspace;
use crate::tab_view::ViewSpec;
use crate::tabs::Tab;

const REFRESH: Duration = Duration::from_secs(3);

/// The folder a tab's panel is about: an agent tab with no worktree of its own, not the main tab,
/// in the folder it was launched in, else the folder of its group.
pub(crate) fn folder_of<T>(workspace: &Workspace, tab: &Tab<Pane<T>>) -> Option<PathBuf> {
    let pane = &tab.pane;
    if tab.pinned || pane.worktree.is_some() || !matches!(pane.body, Body::Shell(_)) {
        return None;
    }
    let agent = pane.agent.as_ref()?;
    let own = agent.launched.as_ref().and_then(|launched| launched.dir.clone());
    Some(own.unwrap_or_else(|| workspace.dir().to_path_buf()))
}

pub(crate) struct Scanned {
    tab: u64,
    scan: Scan,
}

struct Entry {
    scan: Option<Scan>,
    asked_at: Option<Instant>,
    inflight: bool,
}

pub(crate) struct Folders {
    jobs: Jobs<Scanned>,
    tabs: HashMap<u64, Entry>,
    pub(crate) dev_picker_open: bool,
}

/// What the panel of a folder tab reads.
pub(crate) enum Shown<'a> {
    /// A repo's panel; `repos` is the folder's list for the picker, empty when the folder is itself
    /// inside the repo.
    Repo {
        root: &'a Path,
        repos: &'a [FolderRepo],
    },
    Folder(&'a Scan),
}

fn unix_ms() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

impl Folders {
    pub(crate) fn new() -> Self {
        Self {
            jobs: Jobs::new(),
            tabs: HashMap::new(),
            dev_picker_open: std::env::var_os("TIDEWATCH_DEV_FOLDER_PICKER").is_some(),
        }
    }

    pub(crate) fn drain(&mut self) {
        for Scanned { tab, scan } in self.jobs.drain() {
            let Some(entry) = self.tabs.get_mut(&tab) else { continue };
            entry.inflight = false;
            if entry.scan.as_ref().is_none_or(|old| old.folder == scan.folder) {
                entry.scan = Some(scan);
            }
        }
    }

    pub(crate) fn keep(&mut self, open: &HashSet<u64>) {
        self.tabs.retain(|tab, _| open.contains(tab));
    }

    /// Starts a look at `folder` for `tab` when there is none yet, the folder changed, or the last
    /// one is `REFRESH` old, and asks for the frame that will want the next one.
    pub(crate) fn look(&mut self, ctx: &Context, tab: u64, folder: &Path) {
        let entry =
            self.tabs.entry(tab).or_insert(Entry { scan: None, asked_at: None, inflight: false });
        if entry.scan.as_ref().is_some_and(|scan| scan.folder != folder) {
            entry.scan = None;
            entry.asked_at = None;
        }
        let due = entry.asked_at.is_none_or(|at| at.elapsed() >= REFRESH);
        if due && !entry.inflight {
            entry.inflight = true;
            entry.asked_at = Some(Instant::now());
            let folder = folder.to_path_buf();
            self.jobs
                .spawn(ctx, move || Scanned { tab, scan: folder_repos::scan(&folder, unix_ms()) });
        }
        ctx.request_repaint_after(REFRESH);
    }

    /// The view spec of a folder tab whose folder holds a repo to show, started with the look.
    pub(crate) fn spec<T>(
        &mut self,
        ctx: &Context,
        workspace: &Workspace,
        tab: &Tab<Pane<T>>,
    ) -> Option<ViewSpec> {
        let folder = folder_of(workspace, tab)?;
        self.look(ctx, tab.id, &folder);
        let agent = tab.pane.agent.as_ref();
        let hand = agent.and_then(|agent| agent.folder_pick.as_deref());
        let edits = agent.map_or(&[][..], |agent| agent.reader.edited_files());
        match self.shown(tab.id, hand, edits)? {
            Shown::Repo { root, .. } => Some(ViewSpec::of_repo(tab.id, root)),
            Shown::Folder(_) => None,
        }
    }

    pub(crate) fn scan(&self, tab: u64) -> Option<&Scan> {
        self.tabs.get(&tab)?.scan.as_ref()
    }

    /// What the panel of `tab` reads, once the folder has been looked at.
    pub(crate) fn shown(
        &self,
        tab: u64,
        hand: Option<&Path>,
        edits: &[PathBuf],
    ) -> Option<Shown<'_>> {
        let scan = self.scan(tab)?;
        Some(match folder_repos::target(scan, hand, edits) {
            Target::Inside(root) => Shown::Repo { root, repos: &[] },
            Target::Repo(repo) => Shown::Repo { root: &repo.root, repos: &scan.repos },
            Target::Folder => Shown::Folder(scan),
        })
    }
}

#[cfg(test)]
mod tests;
