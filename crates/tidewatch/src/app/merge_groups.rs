//! One tab group per repo, however the window came by its groups. A window restarted for an update
//! keeps the groups the old version made, so whenever the set of groups changes (and once at the
//! start) git is asked, on a job thread, which repo each group's folder belongs to, and the groups
//! of one repo are folded into it (`crate::merge_groups`). No terminal restarts.

use std::path::PathBuf;

use stdusk_core::terminal::{CmdState, PtyTerm};

use super::Tidewatch;
use super::workspaces::remembered;
use crate::handoff;
use crate::jobs::Jobs;
use crate::merge_groups::{self, Main};
use crate::pane::{Body, Pane};
use crate::pty::spawn_shell;
use crate::repo::Workspace;

pub(super) struct Host {
    jobs: Jobs<Looked>,
    looking: bool,
    examined: Option<Vec<PathBuf>>,
}

impl Host {
    pub(super) fn new() -> Self {
        Self { jobs: Jobs::new(), looking: false, examined: None }
    }
}

pub(super) struct Looked {
    dirs: Vec<PathBuf>,
    mains: Vec<Option<Main>>,
}

fn runs_a_command(pane: &Pane<PtyTerm>) -> bool {
    let Body::Shell(term) = &pane.body else { return false };
    let foreground = term.foreground_pgid().zip(term.shell_pid()).is_some_and(|(fg, sh)| fg != sh);
    term.cmd_state() == CmdState::Running || foreground
}

impl Tidewatch {
    pub(super) fn drive_group_merge(&mut self, ctx: &egui::Context) {
        for looked in self.group_merge.jobs.drain() {
            self.group_merge.looking = false;
            self.fold_groups(looked, ctx);
        }
        if self.group_merge.looking || !self.can_fold_groups() {
            return;
        }
        let seen = self.group_merge.examined.iter().flatten().map(PathBuf::as_path);
        if self.workspaces.iter().map(Workspace::dir).eq(seen) {
            return;
        }
        let groups: Vec<(PathBuf, bool)> = self
            .workspaces
            .iter()
            .map(|workspace| {
                (workspace.dir().to_path_buf(), matches!(workspace, Workspace::Other { .. }))
            })
            .collect();
        self.group_merge.looking = true;
        self.group_merge.jobs.spawn(ctx, move || Looked {
            mains: merge_groups::resolve(&groups),
            dirs: groups.into_iter().map(|(dir, _)| dir).collect(),
        });
    }

    fn can_fold_groups(&self) -> bool {
        self.welcome.is_none()
            && self.restore.settled()
            && self.closing.is_none()
            && self.repo_close.is_none()
            && handoff::blocker(&self.tabs).is_none()
    }

    fn fold_groups(&mut self, looked: Looked, ctx: &egui::Context) {
        let current = self.workspaces.iter().map(Workspace::dir);
        if !current.eq(looked.dirs.iter().map(PathBuf::as_path)) || !self.can_fold_groups() {
            return;
        }
        let facts = merge_groups::facts(looked.mains, &self.tabs, runs_a_command);
        let merges = merge_groups::plan(&facts);
        if !merges.is_empty() {
            let applied =
                merge_groups::apply(&mut self.tabs, &mut self.workspaces, &merges, |dir| {
                    spawn_shell(ctx, dir)
                });
            for id in applied.dropped {
                self.forget_tab(id);
            }
            for window in applied.retired {
                self.saver.retire(window);
            }
            for dir in &applied.created {
                self.bases.refresh(ctx, dir, self.paths.as_ref());
            }
            self.last_used.record(&remembered(&self.workspaces, &self.tabs));
            ctx.request_repaint();
        }
        let dirs = self.workspaces.iter().map(|workspace| workspace.dir().to_path_buf());
        self.group_merge.examined = Some(dirs.collect());
    }
}
