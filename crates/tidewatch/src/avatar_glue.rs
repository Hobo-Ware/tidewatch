//! The app's side of agent avatars: each agent tab's avatar is read from `avatar.json` when the tab
//! appears, replaced when the agent calls `set_avatar`, and cleared by Use letter.
//!
//! An avatar that fails the readability check when it loads (see `avatar_check.rs`) shows the letter,
//! its file is removed and the tab is queued once for a Redraw ask.
//!
//! `TIDEWATCH_FAKE_AVATARS=1` gives the agent tabs the mockup icons in turn, one of them too dark to
//! read so its tab shows the letter, and writes nothing. `TIDEWATCH_FAKE_AVATARS=icons` gives them
//! icons from the bundled Lucide set in different colors instead.

use std::collections::HashMap;
use std::sync::Arc;

use tidewatch_core::avatar::{Avatar, clean};
use tidewatch_core::avatar_check;
use tidewatch_core::avatar_icons;
use tidewatch_core::avatar_store;
use tidewatch_core::paths::avatar_file_beside;

use crate::job_glue::Facts;
use crate::jobs::Jobs;

pub(crate) const FAKE: [(&str, &str); 7] = [
    (
        r##"<svg viewBox="0 0 24 24"><path d="M7 11V8a5 5 0 0 1 10 0v3h-2V8a3 3 0 0 0-6 0v3z" fill="#e6e8ec"/><path d="M6 11h12v9H6z M12 13.5a1.6 1.6 0 1 0 0.01 0z" fill="#ff6b6b"/></svg>"##,
        "Securing the login flow: a lock.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M5 7h11a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z" fill="#f2b84b"/><path d="M9 9.5l5 2.5-5 2.5z" fill="#141413"/></svg>"##,
        "Syncing the media library: a video.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M7 11V8a5 5 0 0 1 10 0v3h1v10H6V11z" fill="#2a2f3a"/><circle cx="12" cy="15" r="1.6" fill="#8a1c1c"/></svg>"##,
        "Locking down the vault: a dark lock nobody can read.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M4 6h4v4H4z M10 6h4v4h-4z" fill="#b49cff"/><path d="M16 6h4v4h-4z M4 17l5-3 4 2 7-4v2l-7 4-4-2-5 3z" fill="#f2b84b"/></svg>"##,
        "Tuning the recommendations: tiles and a rising line.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M4 6h7v9H4z" fill="#ff6b6b"/><path d="M13 7h7v1.6h-7z M13 10.2h7v1.6h-7z M13 13.4h5v1.6h-5z M4 17.5h16v1.5H4z" fill="#e6e8ec"/></svg>"##,
        "Writing the season recap: a poster and its lines.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M4 20V14h3v6z M10 20V11h3v9z M16 20V8h3v12z" fill="#4cd27a"/><path d="M4 10l5-4 4 3 6-5 1 1.4-7 5.6-4-3-4.2 3.4z" fill="#e6e8ec"/></svg>"##,
        "Charting profit and spend: bars and a trend line.",
    ),
    (
        r##"<svg viewBox="0 0 24 24"><path d="M4 5h16v11H10l-4 3v-3H4z" fill="#ff6b6b"/><path d="M8 10.5l1.4-1.4 2 2 4.2-4.2 1.4 1.4-5.6 5.6z" fill="#ffffff"/></svg>"##,
        "Checking the farewell worker: a speech bubble with a tick.",
    ),
];

pub(crate) const FAKE_ICONS: [(&str, &str, &str); 7] = [
    ("music", "teal", "Syncing the playlists: a music note."),
    ("shield", "blue", "Fixing the login flow: a shield."),
    ("refresh-cw", "green", "Retrying the media sync: two arrows in a loop."),
    ("database", "amber", "Migrating the orders table: a database."),
    ("bug", "red", "Chasing the flaky test: a bug."),
    ("rocket", "orange", "Shipping the release: a rocket."),
    ("book-open", "purple", "Writing the guide: an open book."),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Off,
    Mockups,
    Icons,
}

pub(crate) fn fake_mode_of(value: Option<&str>) -> Fake {
    match value {
        None | Some("" | "0") => Fake::Off,
        Some("icons") => Fake::Icons,
        Some(_) => Fake::Mockups,
    }
}

fn fake_mode() -> Fake {
    fake_mode_of(std::env::var("TIDEWATCH_FAKE_AVATARS").ok().as_deref())
}

pub(crate) fn fake_requested() -> bool {
    fake_mode() != Fake::Off
}

/// The `n`th stand-in icon of the bundled set, in its color, through the same checks a real avatar
/// passes.
pub(crate) fn fake_icon_face(n: usize) -> Option<Arc<Avatar>> {
    let (icon, color, reason) = FAKE_ICONS[n % FAKE_ICONS.len()];
    let svg = avatar_icons::svg(icon, &avatar_icons::color_hex(color)?)?;
    clean(&svg, reason).ok().filter(|avatar| avatar_check::check(&avatar.svg).is_ok()).map(Arc::new)
}

/// The `n`th stand-in icon through the same checks a real avatar passes: for the mockups `None`
/// for the dark one.
pub(crate) fn fake_face(n: usize) -> Option<Arc<Avatar>> {
    if fake_mode() == Fake::Icons {
        return fake_icon_face(n);
    }
    let (svg, reason) = FAKE[n % FAKE.len()];
    clean(svg, reason).ok().filter(|avatar| avatar_check::check(&avatar.svg).is_ok()).map(Arc::new)
}

pub(crate) const MEESEEKS_SVG: &str = include_str!("../../../assets/meeseeks.svg");

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
    Unreadable { tab: u64 },
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
    unreadable: Vec<u64>,
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
            unreadable: Vec::new(),
        }
    }

    pub(crate) fn from_env(writable: bool) -> Self {
        Self::new(writable, fake_requested())
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
            match done {
                Done::Loaded { tab, avatar }
                    if matches!(self.tabs.get(&tab), Some(Slot::Loading)) =>
                {
                    self.tabs.insert(tab, Slot::Known(avatar.map(|avatar| Arc::new(*avatar))));
                }
                Done::Unreadable { tab } if matches!(self.tabs.get(&tab), Some(Slot::Loading)) => {
                    self.tabs.insert(tab, Slot::Known(None));
                    if self.writable {
                        self.unreadable.push(tab);
                    }
                }
                _ => {}
            }
        }
        for facts in tabs {
            if self.tabs.contains_key(&facts.id) {
                continue;
            }
            if self.fake {
                let avatar = fake_face(self.fake_next);
                self.fake_next += 1;
                self.tabs.insert(facts.id, Slot::Known(avatar));
            } else {
                let (id, notes, writable) = (facts.id, facts.notes_file, self.writable);
                let file = avatar_file_beside(notes);
                self.jobs.spawn(ctx, move || match avatar_store::load(&file) {
                    Some(avatar) if avatar_check::check(&avatar.svg).is_err() => {
                        if writable {
                            let _ = avatar_store::clear(&file);
                        }
                        Done::Unreadable { tab: id }
                    }
                    found => Done::Loaded { tab: id, avatar: found.map(Box::new) },
                });
                self.tabs.insert(facts.id, Slot::Loading);
            }
        }
    }

    /// The tabs whose saved avatar failed the check since the last call, each once: they show the
    /// letter and are to be asked to redraw.
    pub(crate) fn take_unreadable(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.unreadable)
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
