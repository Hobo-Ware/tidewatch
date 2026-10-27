//! The app's side of the links of each agent tab: the board is read from `links.json` on a `Jobs`
//! thread when the tab appears, an agent's report arrives already saved, and what Tidewatch finds
//! (links in the agent's chat messages, the tab's own pull request) is merged in memory at once and
//! into the file on a thread. Nothing here polls: it is fed by data the app already reads.
//! `TIDEWATCH_FAKE_LINKS=1` gives every agent tab the mockup's links and writes nothing.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::link_board::{Link, LinkBoard};
use tidewatch_core::link_store;
use tidewatch_core::paths::links_file_beside;

use crate::job_glue::Facts;
use crate::jobs::Jobs;
use crate::link_model::{fake_board, pr_link};

enum Done {
    Loaded { tab: u64, board: Option<Box<LinkBoard>> },
    Saved,
}

struct Tab {
    board: LinkBoard,
    file: PathBuf,
    loaded: bool,
    own_pr: Option<(u64, String, Option<String>)>,
}

pub(crate) struct Host {
    tabs: HashMap<u64, Tab>,
    jobs: Jobs<Done>,
    writable: bool,
    fake: bool,
}

impl Host {
    pub(crate) fn new(writable: bool, fake: bool) -> Self {
        Self { tabs: HashMap::new(), jobs: Jobs::new(), writable: writable && !fake, fake }
    }

    pub(crate) fn from_env(writable: bool) -> Self {
        let fake =
            std::env::var_os("TIDEWATCH_FAKE_LINKS").is_some_and(|v| !v.is_empty() && v != "0");
        Self::new(writable, fake)
    }

    pub(crate) fn board(&self, tab: u64) -> Option<&LinkBoard> {
        self.tabs.get(&tab).map(|tab| &tab.board).filter(|board| !board.is_empty())
    }

    /// Reads the file of each new tab and takes what the reads found.
    pub(crate) fn tick(&mut self, tabs: &[Facts<'_>], ctx: &egui::Context, now_ms: i64) {
        self.tabs.retain(|id, _| tabs.iter().any(|facts| facts.id == *id));
        for done in self.jobs.drain() {
            if let Done::Loaded { tab, board } = done
                && let Some(tab) = self.tabs.get_mut(&tab)
            {
                if let Some(saved) = board {
                    let mut merged = *saved;
                    merged.absorb_all(&tab.board.links);
                    tab.board = merged;
                }
                tab.loaded = true;
            }
        }
        for facts in tabs {
            if self.tabs.contains_key(&facts.id) {
                continue;
            }
            let file = links_file_beside(facts.notes_file);
            let mut tab = Tab {
                board: LinkBoard::default(),
                file: file.clone(),
                loaded: false,
                own_pr: None,
            };
            if self.fake {
                tab.board = fake_board(now_ms);
                tab.loaded = true;
            } else {
                let id = facts.id;
                self.jobs.spawn(ctx, move || Done::Loaded {
                    tab: id,
                    board: link_store::load(&file).map(Box::new),
                });
            }
            self.tabs.insert(facts.id, tab);
        }
    }

    /// The agent's report brought these links; they are saved already.
    pub(crate) fn arrived(&mut self, tab: u64, links: &[Link]) {
        if self.fake {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(&tab) {
            tab.board.absorb_all(links);
        }
    }

    /// Tidewatch found these links: they are in the board now and saved on a thread.
    pub(crate) fn found(&mut self, tab: u64, links: Vec<Link>, ctx: &egui::Context) {
        if self.fake || links.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get_mut(&tab) else { return };
        if !tab.board.absorb_all(&links) || !self.writable {
            return;
        }
        let file = tab.file.clone();
        self.jobs.spawn(ctx, move || {
            let _ = link_store::merge(&file, &links);
            Done::Saved
        });
    }

    /// The tab's own pull request, as the lookup or the watch knows it. A repeat of what was last
    /// seen does nothing.
    pub(crate) fn own_pr(
        &mut self,
        tab: u64,
        (number, url, title): (u64, &str, Option<&str>),
        now_ms: i64,
        ctx: &egui::Context,
    ) {
        let Some(known) = self.tabs.get_mut(&tab) else { return };
        let seen = (number, url.to_owned(), title.map(str::to_owned));
        if known.own_pr.as_ref() == Some(&seen) {
            return;
        }
        known.own_pr = Some(seen);
        self.found(tab, vec![pr_link(number, url, title, now_ms)], ctx);
    }
}

#[cfg(test)]
mod tests;
