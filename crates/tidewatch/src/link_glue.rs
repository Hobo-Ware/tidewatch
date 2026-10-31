//! The app's side of the links of each agent tab: the board is read from `links.json` on a `Jobs`
//! thread when the tab appears, an agent's report arrives already saved, and what Tidewatch finds
//! (links in the agent's chat messages, the tab's own pull request) is merged in memory at once and
//! into the file on a thread. A pull request or issue link that is not merged, closed or gone is
//! looked up with `gh` on a thread, once for the link whichever tabs hold it: when it is first seen,
//! two seconds after a webhook delivery names it, and otherwise every 30 minutes (every 5 while the
//! tab in front holds a link to a repo no forwarder delivers for); a repo `gh` cannot read or a
//! pull request that is gone stops the looking. The tab's own pull request takes its state from the
//! watch instead. `TIDEWATCH_FAKE_LINKS=1` gives every agent tab the mockup's links and writes
//! nothing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use tidewatch_core::github_hook::Hit;
use tidewatch_core::link_board::{Access, Kind, Link, LinkBoard, Status};
use tidewatch_core::link_check::{self, Lookup};
use tidewatch_core::link_store;
use tidewatch_core::paths::links_file_beside;
use tidewatch_core::tools::Tools;

use crate::job_glue::Facts;
use crate::jobs::Jobs;
use crate::link_model::{FAKE_LIVE_REPO, OwnPr, Reach, check_due, fake_board, pr_link};

enum Done {
    Loaded { tab: u64, board: Option<Box<LinkBoard>> },
    Saved,
    Looked { key: String, lookup: Lookup, at_ms: i64 },
}

struct Tab {
    board: LinkBoard,
    file: PathBuf,
    loaded: bool,
    own_pr: Option<(u64, String, Option<String>, tidewatch_core::link_board::LinkState, Status)>,
}

/// What the app knows of the lookups of one link, whichever tab holds it.
#[derive(Default)]
struct Looks {
    at_ms: Option<i64>,
    in_flight: bool,
    news: bool,
}

pub(crate) struct Host {
    tabs: HashMap<u64, Tab>,
    jobs: Jobs<Done>,
    tools: Tools,
    writable: bool,
    fake: bool,
    told: Vec<(u64, Link)>,
    looks: HashMap<String, Looks>,
    front: Option<u64>,
    live: HashSet<String>,
    alerts: HashMap<u64, u64>,
}

impl Host {
    pub(crate) fn new(writable: bool, fake: bool, tools: Tools) -> Self {
        Self {
            tabs: HashMap::new(),
            jobs: Jobs::new(),
            tools,
            writable: writable && !fake,
            fake,
            told: Vec::new(),
            looks: HashMap::new(),
            front: None,
            live: if fake { HashSet::from([FAKE_LIVE_REPO.to_owned()]) } else { HashSet::new() },
            alerts: HashMap::new(),
        }
    }

    /// The tab in front and the repos (`owner/name` in lower case) a forwarder delivers for.
    pub(crate) fn set_world(&mut self, front: Option<u64>, live: &HashSet<String>) {
        self.front = front;
        if !self.fake && self.live != *live {
            self.live.clone_from(live);
        }
    }

    pub(crate) fn live(&self) -> &HashSet<String> {
        &self.live
    }

    /// How many times a link of the tab began to need action, since the app started; the Progress
    /// switch shows a dot while this is not what the panel last showed.
    pub(crate) fn alerts(&self, tab: u64) -> Option<u64> {
        self.alerts.get(&tab).copied()
    }

    /// GitHub says something happened to these pull requests and issues of `slug`: every link to
    /// one, in every tab, is looked up soon.
    pub(crate) fn news(&mut self, slug: &str, hit: &Hit, ctx: &egui::Context) {
        let mut named = false;
        for tab in self.tabs.values() {
            for link in &tab.board.links {
                let names = match (link.kind(), link.number()) {
                    (Kind::Pr, Some(number)) => hit.numbers.contains(&number),
                    (Kind::Issue, Some(number)) => hit.issues.contains(&number),
                    _ => false,
                };
                if names && link.repo().is_some_and(|repo| repo.eq_ignore_ascii_case(slug)) {
                    self.looks.entry(link.key()).or_default().news = true;
                    named = true;
                }
            }
        }
        if named {
            ctx.request_repaint();
        }
    }

    pub(crate) fn take_told(&mut self) -> Vec<(u64, Link)> {
        std::mem::take(&mut self.told)
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
                Done::Looked { key, lookup, at_ms } => self.looked(&key, &lookup, at_ms, ctx),
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
        let mut held: HashMap<String, Held> = HashMap::new();
        for (id, tab) in self.tabs.iter().filter(|(_, tab)| tab.loaded) {
            let own = tab.own_pr.as_ref().filter(|own| !own.3.is_unknown()).map(|own| &own.1);
            for link in &tab.board.links {
                let reach = match link.repo() {
                    Some(repo) if self.live.contains(&repo) => Reach::Live,
                    _ if self.front == Some(*id) => Reach::Front,
                    _ => Reach::Back,
                };
                let entry = held.entry(link.key()).or_insert_with(|| Held {
                    link: link.clone(),
                    reach,
                    dir: tab.file.parent().map(PathBuf::from).unwrap_or_default(),
                    own_only: true,
                });
                entry.reach = entry.reach.min_by_recheck(reach);
                entry.own_only &= own == Some(&link.url);
            }
        }
        self.looks.retain(|key, looks| held.contains_key(key) || looks.in_flight);
        let mut wake: Option<i64> = None;
        for (key, held) in held {
            let looks = self.looks.entry(key.clone()).or_default();
            if held.own_only || !held.link.is_watchable() {
                looks.news = false;
                continue;
            }
            if looks.in_flight {
                continue;
            }
            if !check_due(&held.link, looks.at_ms, now_ms, held.reach, looks.news) {
                let wait = if looks.news { NEWS_GAP_MS } else { held.reach.recheck_ms() };
                wake = wake.into_iter().chain(looks.at_ms.map(|at| at + wait)).min();
                continue;
            }
            if !self.writable {
                continue;
            }
            looks.in_flight = true;
            looks.news = false;
            let (tools, url, dir) = (self.tools.clone(), held.link.url, held.dir);
            self.jobs.spawn(ctx, move || Done::Looked {
                lookup: link_check::lookup(&tools, &dir, &url, now_ms),
                key,
                at_ms: now_ms,
            });
        }
        if let Some(at) = wake {
            let wait = u64::try_from((at - now_ms).max(0)).unwrap_or(0);
            ctx.request_repaint_after(Duration::from_millis(wait + 50));
        }
    }

    fn looked(&mut self, key: &str, lookup: &Lookup, at_ms: i64, ctx: &egui::Context) {
        let looks = self.looks.entry(key.to_owned()).or_default();
        looks.in_flight = false;
        looks.at_ms = Some(at_ms);
        if matches!(lookup, Lookup::Failed) {
            return;
        }
        let holders: Vec<(u64, Link)> = self
            .tabs
            .iter()
            .flat_map(|(id, tab)| tab.board.links.iter().map(move |link| (*id, link)))
            .filter(|(_, link)| link.key() == key)
            .map(|(id, link)| (id, link.clone()))
            .collect();
        for (tab, known) in holders {
            let incoming = if let Lookup::Found { state, status } = lookup {
                Link { state: *state, status: status.clone(), ..known.clone() }
            } else {
                let access = link_check::access_of(lookup).unwrap_or(Access::Readable);
                let status = Status { access, checked_ms: Some(at_ms), ..known.status.clone() };
                Link { status, ..known.clone() }
            };
            if incoming.needs_action() && wants_a_look(&known, &incoming) {
                *self.alerts.entry(tab).or_default() += 1;
            }
            self.found(tab, vec![incoming], ctx);
        }
    }

    /// The agent's report brought these links; they are saved already.
    pub(crate) fn arrived(&mut self, id: u64, links: &[Link]) {
        if self.fake {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(&id)
            && tab.board.absorb_all(links)
        {
            self.told.extend(links.iter().map(|link| (id, link.clone())));
        }
    }

    /// Tidewatch found these links: they are in the board now and saved on a thread.
    pub(crate) fn found(&mut self, id: u64, links: Vec<Link>, ctx: &egui::Context) {
        if self.fake || links.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        let changed = tab.board.absorb_all(&links);
        if changed {
            self.told.extend(links.iter().map(|link| (id, link.clone())));
        }
        if !changed || !self.writable {
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
        let seen = (
            pr.number,
            pr.url.to_owned(),
            pr.title.map(str::to_owned),
            pr.state,
            pr.status.clone(),
        );
        if known.own_pr.as_ref() == Some(&seen) {
            return;
        }
        known.own_pr = Some(seen);
        self.found(tab, vec![pr_link(pr, now_ms)], ctx);
    }
}

/// A link the lookups of every tab share.
struct Held {
    link: Link,
    reach: Reach,
    dir: PathBuf,
    own_only: bool,
}

const NEWS_GAP_MS: i64 = crate::link_model::NEWS_GAP_MS;

/// Whether `after` needs action in a way `before` did not show: it did not need action, or what it
/// needs changed.
fn wants_a_look(before: &Link, after: &Link) -> bool {
    !before.needs_action()
        || (before.status.checks, before.status.review, before.status.conflicts)
            != (after.status.checks, after.status.review, after.status.conflicts)
}

#[cfg(test)]
mod tests;
