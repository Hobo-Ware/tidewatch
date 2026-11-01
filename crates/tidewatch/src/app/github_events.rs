//! The window's side of GitHub's live events: the repos the window holds to the forwarder host each
//! frame, and what GitHub says about a pull request or branch to the tabs that watch it.

use std::path::PathBuf;

use super::Tidewatch;
use crate::repo::Workspace;
use crate::wake::request_wake;

impl Tidewatch {
    pub(super) fn drive_github_events(&mut self, ctx: &egui::Context, now: f64) {
        let repos: Vec<PathBuf> = self
            .workspaces
            .iter()
            .filter_map(|workspace| match workspace {
                Workspace::Repo(repo) => Some(repo.root.clone()),
                Workspace::Other { .. } => None,
            })
            .collect();
        let listener = self.hooks.listener();
        let driven = self.github_events.drive(&repos, listener.as_deref());
        for news in driven.news {
            self.pr.news(&news.repos, &news.hit, now);
            self.link_live.news(&news.slug, &news.hit, ctx);
        }
        let front = self.tabs.items().get(self.tabs.active()).map(|tab| tab.id);
        self.link_live.set_world(front, &self.github_events.live_slugs());
        if let Some(wait) = driven.wake_in {
            request_wake(ctx, wait);
        }
    }
}
