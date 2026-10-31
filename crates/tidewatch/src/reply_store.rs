//! A tab's reply box on disk: the messages sent before (for Up in an empty box) and the draft that
//! was in the box, so a restart brings both back. The draft is written once it has stood still for
//! `SETTLE_SECS`, so typing never writes per key and a send that empties the box removes the file.

use std::path::{Path, PathBuf};

use tidewatch_core::message_history;
use tidewatch_core::paths::{reply_draft_file_beside, reply_history_file_beside};
use tidewatch_core::reply_draft::{self, Draft};

pub(crate) const SETTLE_SECS: f64 = 0.8;

#[derive(Debug, Default)]
pub(crate) struct ReplyStore {
    files: Option<(PathBuf, PathBuf)>,
    saved: Draft,
    pending: Option<(Draft, f64)>,
}

impl ReplyStore {
    pub(crate) fn is_attached(&self) -> bool {
        self.files.is_some()
    }

    /// Reads the files beside the tab's `notes_file`: the sent messages and the saved draft.
    pub(crate) fn attach(&mut self, notes_file: &Path) -> (Vec<String>, Option<Draft>) {
        let history = reply_history_file_beside(notes_file);
        let draft = reply_draft_file_beside(notes_file);
        let entries = message_history::load(&history);
        let saved = reply_draft::load(&draft);
        self.saved = saved.clone().unwrap_or_default();
        self.files = Some((history, draft));
        (entries, saved)
    }

    pub(crate) fn save_history(&self, entries: &[String]) {
        if let Some((history, _)) = &self.files {
            let _ = message_history::save(history, entries);
        }
    }

    /// Called every frame with the box as it is. Writes the draft once it stood still, and says when
    /// to look again if a change is waiting.
    pub(crate) fn tick(&mut self, text: &str, pictures: &[PathBuf], now: f64) -> Option<f64> {
        let (_, file) = self.files.as_ref()?;
        if text == self.saved.text && pictures == self.saved.pictures.as_slice() {
            self.pending = None;
            return None;
        }
        let same = self
            .pending
            .as_ref()
            .is_some_and(|(draft, _)| draft.text == text && draft.pictures == pictures);
        if !same {
            let draft = Draft { text: text.to_owned(), pictures: pictures.to_vec() };
            self.pending = Some((draft, now));
            return Some(SETTLE_SECS);
        }
        let since = self.pending.as_ref().map_or(now, |(_, since)| *since);
        if now - since < SETTLE_SECS {
            return Some(SETTLE_SECS - (now - since));
        }
        if let Some((draft, _)) = self.pending.take() {
            let _ = reply_draft::save(file, &draft);
            self.saved = draft;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(dir: &Path) -> PathBuf {
        dir.join("tab").join("notes.json")
    }

    #[test]
    fn a_draft_is_written_only_after_it_stood_still_and_comes_back_on_attach() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ReplyStore::default();
        assert_eq!(store.attach(&notes(dir.path())), (Vec::new(), None));
        assert_eq!(store.tick("che", &[], 0.0), Some(SETTLE_SECS));
        assert_eq!(store.tick("check the totals", &[], 0.5), Some(SETTLE_SECS));
        let file = reply_draft_file_beside(&notes(dir.path()));
        assert!(!file.exists(), "typing is not saved per key");
        assert!(store.tick("check the totals", &[], 0.5 + SETTLE_SECS - 0.1).is_some());
        assert!(!file.exists());
        assert_eq!(store.tick("check the totals", &[], 0.5 + SETTLE_SECS), None);
        let mut again = ReplyStore::default();
        let (_, draft) = again.attach(&notes(dir.path()));
        assert_eq!(draft.map(|draft| draft.text), Some("check the totals".to_owned()));
    }

    #[test]
    fn an_unchanged_box_writes_nothing_and_an_emptied_one_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ReplyStore::default();
        store.attach(&notes(dir.path()));
        let file = reply_draft_file_beside(&notes(dir.path()));
        assert_eq!(store.tick("", &[], 0.0), None);
        assert!(!file.exists());
        store.tick("half a thought", &[], 1.0);
        store.tick("half a thought", &[], 1.0 + SETTLE_SECS);
        assert!(file.exists());
        store.tick("", &[], 10.0);
        assert_eq!(store.tick("", &[], 10.0 + SETTLE_SECS), None);
        assert!(!file.exists());
    }

    #[test]
    fn the_sent_messages_are_saved_and_loaded_for_the_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ReplyStore::default();
        assert!(!store.is_attached());
        store.attach(&notes(dir.path()));
        store.save_history(&["one".to_owned(), "two".to_owned()]);
        let mut again = ReplyStore::default();
        let (entries, _) = again.attach(&notes(dir.path()));
        assert_eq!(entries, ["one", "two"]);
    }

    #[test]
    fn a_store_with_no_files_never_asks_to_be_looked_at_again() {
        assert_eq!(ReplyStore::default().tick("typed", &[], 0.0), None);
    }
}
