//! The app's side of the links of each agent tab: the board is read from `links.json` on a `Jobs`
//! thread when the tab appears, an agent's report arrives already saved, and what Tidewatch finds
//! (links in the agent's chat messages, the tab's own pull request) is merged in memory at once and
//! into the file on a thread. The state of a pull request or issue link that is not merged or closed
//! is looked up with `gh` on a thread when the link is first seen and then at most every 30 minutes
//! while the tab is open; the tab's own pull request takes its state from the watch instead.
//! `TIDEWATCH_FAKE_LINKS=1` gives every agent tab the mockup's links and writes nothing.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::link_board::{Link, LinkBoard, LinkState};
use tidewatch_core::link_check;
use tidewatch_core::link_store;
use tidewatch_core::paths::links_file_beside;
use tidewatch_core::tools::Tools;

use crate::job_glue::Facts;
use crate::jobs::Jobs;
use crate::link_model::{OwnPr, check_due, fake_board, pr_link};

enum Done {
    Loaded { tab: u64, board: Option<Box<LinkBoard>> },
    Saved,
    Checked { tab: u64, url: String, state: LinkState },
}

struct Tab {
    board: LinkBoard,
    file: PathBuf,
    loaded: bool,
    own_pr: Option<(u64, String, Option<String>, LinkState)>,
    checked: HashMap<String, i64>,
}

pub(crate) struct Host {
    tabs: HashMap<u64, Tab>,
    jobs: Jobs<Done>,
    tools: Tools,
    writable: bool,
    fake: bool,
}

impl Host {
    pub(crate) fn new(writable: bool, fake: bool, tools: Tools) -> Self {
        Self { tabs: HashMap::new(), jobs: Jobs::new(), tools, writable: writable && !fake, fake }
    }

    pub(crate) fn from_env(writable: bool) -> Self {
        let fake =
            std::env::var_os("TIDEWATCH_FAKE_LINKS").is_some_and(|v| !v.is_empty() && v != "0");
        Self::new(writable, fake, Tools::system())
    }

    pub(crate) fn board(&self, tab: u64) -> Option<&LinkBoard> {
        self.tabs.get(&tab).map(|tab| &tab.board).filter(|board| !board.is_empty())
    }

    /// Reads the file of each new tab, takes what the reads found and starts the lookups that are
    /// due.
    pub(crate) fn tick(&mut self, tabs: &[Facts<'_>], ctx: &egui::Context, now_ms: i64) {
        self.tabs.retain(|id, _| tabs.iter().any(|facts| facts.id == *id));
        for done in self.jobs.drain() {
            match done {
                Done::Loaded { tab, board } => {
                    let Some(tab) = self.tabs.get_mut(&tab) else { continue };
                    if let Some(saved) = board {
                        let mut merged = *saved;
                        merged.absorb_all(&tab.board.links);
                        tab.board = merged;
                    }
                    tab.loaded = true;
                }
                Done::Checked { tab, url, state } => self.state_found(tab, &url, state, ctx),
                Done::Saved => {}
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
                checked: HashMap::new(),
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
        self.check_due(ctx, now_ms);
    }

    fn check_due(&mut self, ctx: &egui::Context, now_ms: i64) {
        if !self.writable {
            return;
        }
        for (id, tab) in &mut self.tabs {
            if !tab.loaded {
                continue;
            }
            let own = tab.own_pr.as_ref().filter(|own| !own.3.is_unknown()).map(|own| &own.1);
            let due: Vec<String> = tab
                .board
                .links
                .iter()
                .filter(|link| own != Some(&link.url))
                .filter(|link| check_due(link, tab.checked.get(&link.url).copied(), now_ms))
                .map(|link| link.url.clone())
                .collect();
            for url in due {
                tab.checked.insert(url.clone(), now_ms);
                let (tools, tab_id) = (self.tools.clone(), *id);
                let dir = tab.file.parent().map(PathBuf::from).unwrap_or_default();
                self.jobs.spawn(ctx, move || {
                    let state = link_check::state_of(&tools, &dir, &url);
                    Done::Checked { tab: tab_id, url, state }
                });
            }
        }
    }

    fn state_found(&mut self, tab: u64, url: &str, state: LinkState, ctx: &egui::Context) {
        if state.is_unknown() {
            return;
        }
        let Some(link) =
            self.tabs.get(&tab).and_then(|tab| tab.board.links.iter().find(|l| l.url == url))
        else {
            return;
        };
        let known = Link { state, ..link.clone() };
        self.found(tab, vec![known], ctx);
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
    pub(crate) fn own_pr(&mut self, tab: u64, pr: &OwnPr<'_>, now_ms: i64, ctx: &egui::Context) {
        let Some(known) = self.tabs.get_mut(&tab) else { return };
        let seen = (pr.number, pr.url.to_owned(), pr.title.map(str::to_owned), pr.state);
        if known.own_pr.as_ref() == Some(&seen) {
            return;
        }
        known.own_pr = Some(seen);
        self.found(tab, vec![pr_link(pr, now_ms)], ctx);
    }
}

#[cfg(test)]
mod tests;
