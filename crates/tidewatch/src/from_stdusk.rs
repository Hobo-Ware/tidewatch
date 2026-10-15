//! Moving tabs from stdusk: stdusk hands its live panes over a socket and each one becomes a shell
//! tab here, in the group of the git repo its folder is in, still running what it ran.
//! Until the ACK inside [`receive`] nothing changed: a failure adopts nothing and stdusk keeps its
//! window.

use std::path::{Path, PathBuf};

use stdusk_core::handoff::export::Export;
use stdusk_core::terminal::PtyTerm;

use crate::handoff::confirm_terms;
use crate::pty;
use crate::repo::Workspace;
use crate::single_instance::workspace_key;

pub(crate) type Incoming = stdusk_core::handoff::Incoming<Export>;

const NAMELESS: &str = "shell";

/// A pane that came over and runs here now, with where it belongs.
pub(crate) struct Taken {
    pub(crate) term: PtyTerm,
    pub(crate) name: String,
    pub(crate) focused: bool,
    /// The repo of its folder, or the folder when it is in none.
    pub(crate) workspace: Workspace,
    pub(crate) key: PathBuf,
}

/// The tab title: what stdusk called the pane, else the name of its folder.
pub(crate) fn name_of(title: Option<&str>, cwd: Option<&Path>) -> String {
    let title = title.map(str::trim).filter(|title| !title.is_empty());
    let folder = cwd.and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned());
    title.map(str::to_owned).or(folder).unwrap_or_else(|| NAMELESS.to_owned())
}

/// What the placement needs to know of one pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Landing {
    pub(crate) key: PathBuf,
    pub(crate) focused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The group at this position among the window's groups.
    Existing(usize),
    /// The nth entry of [`Plan::new_groups`].
    New(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// A group to make, as the position of the first pane that needs it. Groups are made in this
    /// order, one per folder.
    pub(crate) new_groups: Vec<usize>,
    /// Where each pane goes, in pane order.
    pub(crate) targets: Vec<Target>,
    /// The pane that held focus in stdusk, which becomes the active tab.
    pub(crate) focus: Option<usize>,
}

/// `held` is the key of every group the window has, in order; empty while it shows the welcome
/// screen, which the first new group replaces.
pub(crate) fn plan(panes: &[Landing], held: &[PathBuf]) -> Plan {
    let mut new_groups: Vec<usize> = Vec::new();
    let mut made: Vec<&Path> = Vec::new();
    let mut targets = Vec::with_capacity(panes.len());
    for (at, pane) in panes.iter().enumerate() {
        let key = pane.key.as_path();
        let target = match held.iter().position(|held| held == key) {
            Some(group) => Target::Existing(group),
            None => Target::New(made.iter().position(|made| *made == key).unwrap_or_else(|| {
                made.push(key);
                new_groups.push(at);
                made.len() - 1
            })),
        };
        targets.push(target);
    }
    Plan { new_groups, targets, focus: panes.iter().position(|pane| pane.focused) }
}

/// Takes the panes stdusk offers on `socket` and makes each a live terminal here. `fallback` is the
/// folder of a pane that does not say where it is. Blocks until stdusk has sent everything, so it
/// belongs on a job thread.
pub(crate) fn receive(
    ctx: &egui::Context,
    socket: &Path,
    fallback: &Path,
) -> Result<Vec<Taken>, String> {
    let mut incoming = Incoming::receive(socket)
        .map_err(|e| format!("stdusk did not hand its tabs over ({e})"))?;
    let export = incoming.session().clone();
    let received = incoming.take_panes();
    let mut taken = Vec::with_capacity(received.len());
    for (entry, (meta, fd)) in export.panes.iter().zip(received) {
        let cwd = entry.cwd.clone().or_else(|| meta.cwd.clone()).map(PathBuf::from);
        let dir = cwd.as_deref().filter(|cwd| cwd.is_dir()).unwrap_or(fallback);
        let term = pty::adopt(ctx, &meta, fd, dir)
            .map_err(|e| format!("a terminal of stdusk could not be taken over ({e})"))?;
        let workspace = Workspace::open(dir);
        let key = workspace_key(workspace.dir());
        let name = name_of(entry.title.as_deref(), cwd.as_deref());
        taken.push(Taken { term, name, focused: entry.focused, workspace, key });
    }
    confirm_terms(&incoming, taken.iter_mut().map(|pane| &mut pane.term))
        .map_err(|e| format!("stdusk did not get the confirmation ({e})"))?;
    Ok(taken)
}

#[cfg(test)]
mod tests;
