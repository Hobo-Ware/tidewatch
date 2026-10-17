//! The scheduled items of every tab's ledger, in one file (`.state/schedule.toml`), so the app can
//! list what is coming up across tabs and reopen the session of a tab that is closed by then.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::progress::{Ledger, Mark, Scheduled, When};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    #[default]
    Open,
    /// The tab was closed; its items stay so the session can be reopened on the day.
    Closed,
}

/// One open scheduled item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub when: When,
    pub what: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// What one tab has scheduled, and how to get back to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub repo: PathBuf,
    pub tab: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub presence: Presence,
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
}

/// Who a ledger belongs to, for [`Entry::of`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub repo: PathBuf,
    pub tab: String,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
}

impl Entry {
    /// The open items of `ledger`: scheduled lines that are not done or skipped.
    #[must_use]
    pub fn of(owner: Owner, ledger: &Ledger, presence: Presence) -> Self {
        let items = ledger
            .scheduled
            .iter()
            .filter(|item| !matches!(item.mark, Mark::Done | Mark::Skipped))
            .map(Item::from)
            .collect();
        Self {
            repo: owner.repo,
            tab: owner.tab,
            session_id: owner.session_id,
            profile_id: owner.profile_id,
            goal: ledger.goal.clone(),
            presence,
            items,
        }
    }

    fn is(&self, repo: &Path, tab: &str) -> bool {
        self.repo == repo && self.tab == tab
    }
}

impl From<&Scheduled> for Item {
    fn from(item: &Scheduled) -> Self {
        Self { when: item.when.clone(), what: item.what.clone(), why: item.why.clone() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    #[serde(default, rename = "tab")]
    pub tabs: Vec<Entry>,
}

impl Index {
    /// A missing or unreadable file is an empty index.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); starting empty", file.display());
            Self::default()
        })
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }

    /// Replaces the tab's entry, or drops it when it has no open item left.
    pub fn set(&mut self, entry: Entry) {
        self.tabs.retain(|existing| !existing.is(&entry.repo, &entry.tab));
        if !entry.items.is_empty() {
            self.tabs.push(entry);
        }
    }

    /// The tab was closed: its items stay.
    pub fn close(&mut self, repo: &Path, tab: &str) {
        for entry in self.tabs.iter_mut().filter(|entry| entry.is(repo, tab)) {
            entry.presence = Presence::Closed;
        }
    }

    pub fn remove(&mut self, repo: &Path, tab: &str) {
        self.tabs.retain(|entry| !entry.is(repo, tab));
    }
}

enum Change {
    Set(Entry),
    Close { repo: PathBuf, tab: String },
    Remove { repo: PathBuf, tab: String },
}

/// Applies changes to the index file one after the other on a background thread, so the UI thread
/// never waits on the disk. Dropping it waits for the changes still queued.
pub struct Writer {
    sender: Option<Sender<Change>>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    #[must_use]
    pub fn new(file: PathBuf) -> Self {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || apply_all(&file, &receiver));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    pub fn set(&self, entry: Entry) {
        self.send(Change::Set(entry));
    }

    pub fn close(&self, repo: PathBuf, tab: String) {
        self.send(Change::Close { repo, tab });
    }

    pub fn remove(&self, repo: PathBuf, tab: String) {
        self.send(Change::Remove { repo, tab });
    }

    fn send(&self, change: Change) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(change);
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn apply_all(file: &Path, receiver: &Receiver<Change>) {
    while let Ok(first) = receiver.recv() {
        let mut index = Index::load(file);
        for change in std::iter::once(first).chain(receiver.try_iter()) {
            match change {
                Change::Set(entry) => index.set(entry),
                Change::Close { repo, tab } => index.close(&repo, &tab),
                Change::Remove { repo, tab } => index.remove(&repo, &tab),
            }
        }
        if let Err(e) = index.save(file) {
            eprintln!("tidewatch: could not save {}: {e}", file.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civil_date::Date;

    fn owner(tab: &str) -> Owner {
        Owner {
            repo: "/work/web".into(),
            tab: tab.into(),
            session_id: Some("0c2cbc96-1111-4222-8333-444455556666".into()),
            profile_id: Some("builder".into()),
        }
    }

    fn ledger(scheduled: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\nShip it\n\n## Scheduled\n{scheduled}"))
    }

    fn entry(tab: &str, scheduled: &str) -> Entry {
        Entry::of(owner(tab), &ledger(scheduled), Presence::Open)
    }

    const TWO: &str = "- [ ] 2026-11-01 -- Switch it off -- the milestone\n- [ ] on: the partner replies -- Delete the endpoint\n";

    #[test]
    fn an_entry_keeps_the_open_items_with_their_when_and_leaves_out_done_and_skipped() {
        let entry = entry(
            "otter",
            "- [ ] 2026-11-01 -- A -- why a\n- [x] 2026-11-02 -- B\n- [-] ~2026-11-03 -- C\n- [!] you -- D\n- [~] parked: later -- E\n",
        );
        let whens: Vec<_> = entry.items.iter().map(|item| item.when.clone()).collect();
        assert_eq!(
            whens,
            [When::Date(Date::new(2026, 11, 1).unwrap()), When::You, When::Parked("later".into())]
        );
        assert_eq!(entry.items[0].why.as_deref(), Some("why a"));
        assert_eq!(entry.goal, "Ship it");
        assert_eq!(entry.session_id.as_deref(), Some("0c2cbc96-1111-4222-8333-444455556666"));
    }

    #[test]
    fn the_file_round_trips_every_kind_of_when() {
        let mut index = Index::default();
        index.set(entry(
            "otter",
            "- [ ] 2026-11-01 -- A\n- [ ] ~2026-11-07 -- B\n- [ ] on: x happens -- C\n- [ ] parked: until y -- D -- why d\n- [ ] you -- E\n- [ ] someday -- F\n",
        ));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state").join("schedule.toml");
        index.save(&file).unwrap();
        assert_eq!(Index::load(&file), index);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("[[tab]]") && text.contains("[[tab.item]]"), "{text}");
        assert!(
            text.contains("when = \"~2026-11-07\"") && text.contains("when = \"you\""),
            "{text}"
        );
    }

    #[test]
    fn a_bad_or_missing_file_is_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("schedule.toml");
        assert_eq!(Index::load(&file), Index::default());
        for text in ["[[", "tab = 3\n", "[[tab]]\nrepo = 1\n"] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(Index::load(&file), Index::default(), "{text:?}");
        }
    }

    #[test]
    fn set_replaces_a_tabs_entry_and_drops_it_when_nothing_is_left() {
        let mut index = Index::default();
        index.set(entry("otter", TWO));
        index.set(entry("heron", TWO));
        assert_eq!(index.tabs.len(), 2);
        index.set(entry("otter", "- [ ] you -- Only this\n"));
        assert_eq!(index.tabs.len(), 2);
        assert_eq!(index.tabs.iter().find(|e| e.tab == "otter").unwrap().items.len(), 1);
        index.set(entry("otter", "- [x] 2026-11-01 -- Done now\n"));
        assert_eq!(index.tabs.iter().map(|e| e.tab.as_str()).collect::<Vec<_>>(), ["heron"]);
    }

    #[test]
    fn closing_keeps_the_items_and_marks_the_tab_closed() {
        let mut index = Index::default();
        index.set(entry("otter", TWO));
        index.close(Path::new("/work/web"), "otter");
        index.close(Path::new("/work/web"), "nobody");
        index.close(Path::new("/other"), "otter");
        assert_eq!(index.tabs[0].presence, Presence::Closed);
        assert_eq!(index.tabs[0].items.len(), 2);
        index.set(entry("otter", TWO));
        assert_eq!(index.tabs[0].presence, Presence::Open, "a tab opened again is open");
        index.remove(Path::new("/work/web"), "otter");
        assert_eq!(index, Index::default());
    }

    #[test]
    fn the_writer_applies_changes_in_order_and_drop_waits_for_them() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("schedule.toml");
        let writer = Writer::new(file.clone());
        writer.set(entry("otter", TWO));
        writer.set(entry("heron", TWO));
        writer.close("/work/web".into(), "otter".into());
        writer.remove("/work/web".into(), "heron".into());
        drop(writer);
        let index = Index::load(&file);
        assert_eq!(index.tabs.len(), 1);
        assert_eq!(
            (index.tabs[0].tab.as_str(), index.tabs[0].presence),
            ("otter", Presence::Closed)
        );
    }
}
