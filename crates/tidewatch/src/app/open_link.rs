//! The Open a repo dialog in the app: opening it, looking a pasted link up and cloning on job
//! threads, and opening the repo that results.

use std::path::PathBuf;

use tidewatch_core::github_clone::{self, CloneError, Landing};
use tidewatch_core::paths::home_dir;
use tidewatch_core::tools::Tools;

use super::Tidewatch;
use crate::open_link::{self, OpenLink, Phase, Planned, Verdict};
use crate::open_link_view::{self, Outcome};
use crate::repo::Workspace;
use crate::welcome;

pub(super) enum LinkDone {
    Planned(Planned),
    Cloned { target: PathBuf, result: Result<(), CloneError> },
}

impl Tidewatch {
    pub(super) fn open_repo_dialog(&mut self, link: &str) {
        if self.open_link.is_some() {
            return;
        }
        let mut known: Vec<PathBuf> = self
            .workspaces
            .iter()
            .filter(|workspace| matches!(workspace, Workspace::Repo(_)))
            .map(|workspace| workspace.dir().to_path_buf())
            .collect();
        for dir in welcome::known_repo_dirs(self.paths.as_ref()) {
            if !known.contains(&dir) {
                known.push(dir);
            }
        }
        let last = (self.welcome.is_none()).then(|| self.workspace()).and_then(|workspace| {
            matches!(workspace, Workspace::Repo(_)).then(|| workspace.dir().to_path_buf())
        });
        let mut dialog = OpenLink::open(known, last.as_deref(), &self.home_dir());
        link.clone_into(&mut dialog.link);
        self.open_link = Some(dialog);
    }

    fn home_dir(&self) -> PathBuf {
        self.paths
            .as_ref()
            .and_then(|paths| paths.root().parent())
            .map(PathBuf::from)
            .or_else(home_dir)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub(super) fn open_pasted_link(&mut self, ctx: &egui::Context) {
        if self.welcome.is_none() || self.open_link.is_some() || ctx.text_edit_focused() {
            return;
        }
        let link = ctx.input(|i| open_link::pasted_link(&i.events));
        if let Some(link) = link {
            self.open_repo_dialog(&link);
        }
    }

    pub(super) fn show_open_link(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.open_link.take() else { return };
        if let Some(question) = dialog.question() {
            self.link_jobs.spawn(ctx, move || {
                let landing = github_clone::plan(
                    &Tools::system(),
                    &question.repo,
                    &question.parent,
                    &question.known,
                );
                LinkDone::Planned(Planned { key: question.key, landing })
            });
        }
        let home = self.paths.as_ref().and_then(|paths| paths.root().parent());
        match open_link_view::show(ctx, &mut dialog, home) {
            None => self.open_link = Some(dialog),
            Some(Outcome::Close) => {
                if let Phase::Cloning(cloning) = &dialog.phase {
                    cloning.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            Some(Outcome::Browse) => self.pick_repo(ctx),
            Some(Outcome::Back) => {
                dialog.retry();
                self.open_link = Some(dialog);
            }
            Some(Outcome::Go) => {
                self.go_open_link(&mut dialog, ctx);
                if self.open_link.is_none() && !matches!(dialog.phase, Phase::Form) {
                    self.open_link = Some(dialog);
                }
            }
        }
    }

    fn go_open_link(&mut self, dialog: &mut OpenLink, ctx: &egui::Context) {
        match dialog.verdict() {
            Verdict::Ready(_, Landing::Open(dir)) => self.open_workspace(dir, ctx),
            Verdict::Ready(repo, Landing::Clone { target }) => {
                let cloning = dialog.start(repo.clone(), target.clone());
                let url = self.dev_link.as_ref().and_then(|dev| dev.remote.clone());
                let repaint = ctx.clone();
                self.link_jobs.spawn(ctx, move || {
                    let mut progress = |line: &str| {
                        if let Ok(mut shown) = cloning.line.lock() {
                            line.clone_into(&mut shown);
                        }
                        repaint.request_repaint();
                    };
                    let tools = Tools::system();
                    let result = match url {
                        Some(url) => github_clone::clone_with_gh_fallback(
                            &tools,
                            &url,
                            &repo,
                            &target,
                            &cloning.cancel,
                            &mut progress,
                        ),
                        None => github_clone::clone_repo(
                            &tools,
                            &repo,
                            &target,
                            &cloning.cancel,
                            &mut progress,
                        ),
                    };
                    LinkDone::Cloned { target, result }
                });
            }
            Verdict::Idle
            | Verdict::NotALink
            | Verdict::Checking(_)
            | Verdict::Ready(_, Landing::Refuse(_)) => {}
        }
    }

    pub(super) fn drain_link_jobs(&mut self, ctx: &egui::Context) {
        for done in self.link_jobs.drain() {
            match done {
                LinkDone::Planned(planned) => {
                    if let Some(dialog) = &mut self.open_link {
                        dialog.answered(planned);
                    }
                }
                LinkDone::Cloned { target, result } => self.cloned(target, result, ctx),
            }
        }
    }

    fn cloned(&mut self, target: PathBuf, result: Result<(), CloneError>, ctx: &egui::Context) {
        let Some(dialog) = &mut self.open_link else { return };
        let Phase::Cloning(cloning) = &dialog.phase else { return };
        let slug = cloning.repo.slug();
        match result {
            Ok(()) => {
                self.open_link = None;
                self.open_workspace(target, ctx);
                self.show_toast(format!("Cloned {slug}"), ctx);
            }
            Err(CloneError::Cancelled) => self.open_link = None,
            Err(error) => dialog.phase = Phase::Failed(error.to_string()),
        }
    }

    pub(super) fn drive_dev_link(&mut self, ctx: &egui::Context) {
        let Some(dev) = self.dev_link.clone() else { return };
        if !dev.opened {
            self.open_repo_dialog(&dev.link);
            if let (Some(dialog), Some(parent)) = (&mut self.open_link, &dev.parent) {
                dialog.parent.clone_from(parent);
            }
            if let (Some(dialog), Some(line)) = (&mut self.open_link, &dev.fake)
                && let Some(repo) = dialog.repo()
            {
                let target = github_clone::target_of(&dialog.parent_path(), &repo);
                let cloning = dialog.start(repo, target);
                if let Ok(mut shown) = cloning.line.lock() {
                    line.clone_into(&mut shown);
                }
            }
            if let Some(dev) = &mut self.dev_link {
                dev.opened = true;
            }
            return;
        }
        let ready = self
            .open_link
            .as_ref()
            .is_some_and(|dialog| matches!(dialog.verdict(), Verdict::Ready(..)));
        if dev.go && ready {
            if let Some(mut dialog) = self.open_link.take() {
                self.go_open_link(&mut dialog, ctx);
                self.open_link = Some(dialog);
            }
            if let Some(dev) = &mut self.dev_link {
                dev.go = false;
            }
        }
    }
}
