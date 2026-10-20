//! The app's side of agent avatars: each agent tab's avatar is read from `avatar.json` when the tab
//! appears, replaced when the agent calls `set_avatar`, and cleared by Use letter.
//!
//! `TIDEWATCH_FAKE_AVATARS=1` gives the agent tabs the six mockup icons in turn and writes nothing.

use std::collections::HashMap;
use std::sync::Arc;

use tidewatch_core::avatar::{Avatar, clean};
use tidewatch_core::avatar_store;
use tidewatch_core::paths::avatar_file_beside;

use crate::job_glue::Facts;
use crate::jobs::Jobs;

pub(crate) const FAKE: [(&str, &str); 6] = [
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#123b38"/><ellipse cx="32" cy="20" rx="16" ry="6" fill="none" stroke="#2dd4bf" stroke-width="3"/><path d="M16 20v24c0 3.3 7.2 6 16 6s16-2.7 16-6V20" fill="none" stroke="#2dd4bf" stroke-width="3"/><path d="M16 36c5 4 11-4 16 0s11 4 16 0" fill="none" stroke="#99f6e4" stroke-width="3"/></svg>"##,
        "Filling 58k old orders: a table slowly filling with water.",
    ),
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#3a2a14"/><circle cx="32" cy="32" r="10" fill="none" stroke="#fb923c" stroke-width="3"/><path d="M32 12v7M32 45v7M12 32h7M45 32h7M18 18l5 5M41 41l5 5M18 46l5-5M41 23l5-5" stroke="#fb923c" stroke-width="3" stroke-linecap="round"/></svg>"##,
        "Fixing the build: a gear that needs a turn.",
    ),
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#2a2140"/><circle cx="28" cy="28" r="11" fill="none" stroke="#c4b5fd" stroke-width="3"/><path d="M36 36l12 12" stroke="#c4b5fd" stroke-width="4" stroke-linecap="round"/></svg>"##,
        "Reviewing the login code: a magnifier.",
    ),
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#3a1830"/><rect x="12" y="22" width="14" height="14" rx="3" fill="none" stroke="#f472b6" stroke-width="3"/><rect x="38" y="28" width="14" height="14" rx="3" fill="#f472b6"/><path d="M28 29h8M33 25l4 4-4 4" stroke="#f472b6" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>"##,
        "Moving the billing tables: a box going across.",
    ),
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#16301f"/><path d="M26 14h12M28 14v14L18 46c-1.5 3 .5 5 3.5 5h21c3 0 5-2 3.5-5L36 28V14" fill="none" stroke="#4ade80" stroke-width="3" stroke-linejoin="round"/><path d="M22 40h20" stroke="#86efac" stroke-width="3"/></svg>"##,
        "Chasing a flaky test: a flask.",
    ),
    (
        r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#16263f"/><path d="M20 12h18l8 8v32H20z" fill="none" stroke="#60a5fa" stroke-width="3" stroke-linejoin="round"/><path d="M26 30h14M26 37h14M26 44h9" stroke="#93c5fd" stroke-width="3" stroke-linecap="round"/></svg>"##,
        "Sweeping the docs: a page.",
    ),
];

const MEESEEKS_SVG: &str = include_str!("../../../assets/meeseeks.svg");

pub(crate) fn meeseeks_avatar() -> Arc<Avatar> {
    static FACE: std::sync::OnceLock<Arc<Avatar>> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        Arc::new(Avatar {
            svg: MEESEEKS_SVG.to_owned(),
            reason: "Caaan do! Every Meeseeks wears this face.".to_owned(),
        })
    })
    .clone()
}

pub(crate) enum Done {
    Loaded { tab: u64, avatar: Option<Box<Avatar>> },
    Cleared,
}

enum Slot {
    Loading,
    Known(Option<Arc<Avatar>>),
}

pub(crate) struct Host {
    tabs: HashMap<u64, Slot>,
    jobs: Jobs<Done>,
    writable: bool,
    fake: bool,
    fake_next: usize,
    fixed: Vec<u64>,
}

impl Host {
    pub(crate) fn new(writable: bool, fake: bool) -> Self {
        Self {
            tabs: HashMap::new(),
            jobs: Jobs::new(),
            writable: writable && !fake,
            fake,
            fake_next: 0,
            fixed: Vec::new(),
        }
    }

    pub(crate) fn from_env(writable: bool) -> Self {
        let fake =
            std::env::var_os("TIDEWATCH_FAKE_AVATARS").is_some_and(|v| !v.is_empty() && v != "0");
        Self::new(writable, fake)
    }

    pub(crate) fn get(&self, tab: u64) -> Option<&Arc<Avatar>> {
        match self.tabs.get(&tab)? {
            Slot::Known(avatar) => avatar.as_ref(),
            Slot::Loading => None,
        }
    }

    pub(crate) fn fixed_faces(&mut self, meeseeks: &[u64], ctx: &egui::Context) {
        for gone in self.fixed.iter().filter(|id| !meeseeks.contains(id)) {
            self.tabs.remove(gone);
        }
        self.fixed.retain(|id| meeseeks.contains(id));
        for id in meeseeks {
            if !self.fixed.contains(id) {
                self.fixed.push(*id);
            }
            self.tabs.insert(*id, Slot::Known(Some(meeseeks_avatar())));
        }
        crate::agent_avatar::lock(ctx, self.fixed.clone());
    }

    pub(crate) fn tick(&mut self, tabs: &[Facts<'_>], ctx: &egui::Context) {
        self.tabs.retain(|id, _| tabs.iter().any(|facts| facts.id == *id));
        for done in self.jobs.drain() {
            if let Done::Loaded { tab, avatar } = done
                && matches!(self.tabs.get(&tab), Some(Slot::Loading))
            {
                self.tabs.insert(tab, Slot::Known(avatar.map(|avatar| Arc::new(*avatar))));
            }
        }
        for facts in tabs {
            if self.tabs.contains_key(&facts.id) {
                continue;
            }
            if self.fake {
                let (svg, reason) = FAKE[self.fake_next % FAKE.len()];
                self.fake_next += 1;
                let avatar = clean(svg, reason).ok().map(Arc::new);
                self.tabs.insert(facts.id, Slot::Known(avatar));
            } else {
                let (id, file) = (facts.id, avatar_file_beside(facts.notes_file));
                self.jobs.spawn(ctx, move || Done::Loaded {
                    tab: id,
                    avatar: avatar_store::load(&file).map(Box::new),
                });
                self.tabs.insert(facts.id, Slot::Loading);
            }
        }
    }

    /// The agent called `set_avatar`; the file is saved already.
    pub(crate) fn arrived(&mut self, tab: u64, avatar: Avatar) {
        if self.tabs.contains_key(&tab) && !self.fixed.contains(&tab) {
            self.tabs.insert(tab, Slot::Known(Some(Arc::new(avatar))));
        }
    }

    /// Use letter: the avatar goes, from the tab and from disk.
    pub(crate) fn clear(&mut self, tab: u64, notes_file: &std::path::Path, ctx: &egui::Context) {
        if !self.tabs.contains_key(&tab) || self.fixed.contains(&tab) {
            return;
        }
        self.tabs.insert(tab, Slot::Known(None));
        if self.writable {
            let file = avatar_file_beside(notes_file);
            self.jobs.spawn(ctx, move || {
                let _ = avatar_store::clear(&file);
                Done::Cleared
            });
        }
    }
}

#[cfg(test)]
mod tests;
