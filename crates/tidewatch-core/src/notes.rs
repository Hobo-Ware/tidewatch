//! Review threads for one tab, stored in `.state/<repo-dir>/<tab>/notes.json` (PLAN.md, D4).
//!
//! A thread is a note to the tab's agent. The thread type is Kelp's, so the diff widget draws and
//! edits it as is; the file is Tidewatch's own and Kelp's `comments.json` is never touched.
//! Loading and saving are blocking calls: [`NotesWriter`] keeps saves off the UI thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use kelp_core::review::{Anchor, Comment, Thread};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    pr_roots: BTreeMap<u64, u64>,
    #[serde(default)]
    pr_seen: BTreeSet<u64>,
}

impl Notes {
    /// A missing or unreadable file is an empty set: a bad state file must never stop the app.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let Ok(bytes) = std::fs::read(file) else {
            return Self::default();
        };
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(mut notes) => {
                let newest = notes.threads.iter().map(|t| t.id).max().unwrap_or(0);
                notes.next_id = notes.next_id.max(newest);
                notes
            }
            Err(e) => {
                eprintln!(
                    "tidewatch: {} is unreadable ({e}); starting with no notes",
                    file.display()
                );
                Self::default()
            }
        }
    }

    /// Writes a sibling temp file and renames it over the real one, so a crash never leaves half a
    /// file.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, file)
    }

    pub fn add(
        &mut self,
        path: &str,
        commit: Option<String>,
        anchor: Anchor,
        comment: Comment,
    ) -> u64 {
        self.next_id += 1;
        self.threads.push(Thread {
            id: self.next_id,
            path: path.to_owned(),
            commit,
            anchor,
            resolved: false,
            comments: vec![comment],
        });
        self.next_id
    }

    /// A reply reopens a resolved thread.
    pub fn reply(&mut self, id: u64, comment: Comment) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.comments.push(comment);
            thread.resolved = false;
        }
    }

    pub fn set_resolved(&mut self, id: u64, resolved: bool) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.resolved = resolved;
        }
    }

    pub fn delete(&mut self, id: u64) {
        self.threads.retain(|t| t.id != id);
    }

    /// The local thread made from the pull request thread whose first comment has GitHub id `root`.
    /// The id outlives a deleted thread, so deleting an imported thread keeps it deleted.
    #[must_use]
    pub fn pr_thread(&self, root: u64) -> Option<u64> {
        self.pr_roots.get(&root).copied()
    }

    pub fn link_pr_thread(&mut self, root: u64, thread: u64) {
        self.pr_roots.insert(root, thread);
    }

    /// Whether the pull request comment with GitHub id `comment` was already imported.
    #[must_use]
    pub fn pr_comment_seen(&self, comment: u64) -> bool {
        self.pr_seen.contains(&comment)
    }

    pub fn mark_pr_comment_seen(&mut self, comment: u64) {
        self.pr_seen.insert(comment);
    }

    #[must_use]
    pub fn open_count(&self) -> usize {
        self.threads.iter().filter(|t| !t.resolved).count()
    }
}

/// Saves notes on one background thread. A burst of edits collapses into the last snapshot per
/// file, and dropping the writer waits for the pending writes, so quitting never loses a note.
pub struct NotesWriter {
    sender: Option<Sender<(PathBuf, Notes)>>,
    worker: Option<JoinHandle<()>>,
}

impl NotesWriter {
    #[must_use]
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || write_loop(&receiver));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    pub fn save(&self, file: PathBuf, notes: Notes) {
        if let Some(sender) = &self.sender {
            let _ = sender.send((file, notes));
        }
    }
}

impl Default for NotesWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NotesWriter {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_loop(receiver: &Receiver<(PathBuf, Notes)>) {
    while let Ok(first) = receiver.recv() {
        let mut latest: Vec<(PathBuf, Notes)> = vec![first];
        for (file, notes) in receiver.try_iter() {
            match latest.iter_mut().find(|(f, _)| *f == file) {
                Some(slot) => slot.1 = notes,
                None => latest.push((file, notes)),
            }
        }
        for (file, notes) in latest {
            if let Err(e) = notes.save(&file) {
                eprintln!("tidewatch: cannot save {}: {e}", file.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::Side;

    fn anchor(line: u32) -> Anchor {
        Anchor {
            side: Side::New,
            line_hint: line,
            text: format!("line {line}"),
            before: vec!["above".into()],
            after: vec!["below".into()],
        }
    }

    fn comment(body: &str) -> Comment {
        Comment { author: "You".into(), time: 1_790_000_000, body: body.into() }
    }

    fn sample() -> Notes {
        let mut notes = Notes::default();
        let first =
            notes.add("src/a.rs", Some("abc123".into()), anchor(3), comment("use formatDate"));
        notes.add("src/b.rs", None, anchor(9), comment("rename this"));
        notes.reply(first, comment("and add a test"));
        notes
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("web-abc123").join("brave-otter").join("notes.json");
        let notes = sample();
        notes.save(&file).unwrap();
        assert_eq!(Notes::load(&file), notes);
        assert!(!file.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Notes::load(&dir.path().join("none.json")), Notes::default());
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        for text in ["{ not json", "", "[1, 2]", "{\"threads\": 7}"] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(Notes::load(&file), Notes::default(), "content {text:?}");
        }
    }

    #[test]
    fn an_empty_object_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(Notes::load(&file), Notes::default());
    }

    #[test]
    fn ids_stay_unique_after_deleting_the_newest_and_reloading() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut notes = sample();
        let newest = notes.threads.last().unwrap().id;
        notes.delete(newest);
        notes.save(&file).unwrap();
        let mut loaded = Notes::load(&file);
        let id = loaded.add("c.rs", None, anchor(1), comment("x"));
        assert!(id > newest);
    }

    #[test]
    fn next_id_recovers_from_a_file_that_lost_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("next_id");
        std::fs::write(&file, value.to_string()).unwrap();
        let mut loaded = Notes::load(&file);
        let max = loaded.threads.iter().map(|t| t.id).max().unwrap();
        assert_eq!(loaded.add("c.rs", None, anchor(1), comment("x")), max + 1);
    }

    #[test]
    fn reply_reopens_a_resolved_thread_and_appends() {
        let mut notes = sample();
        let id = notes.threads[0].id;
        notes.set_resolved(id, true);
        assert_eq!(notes.open_count(), 1);
        notes.reply(id, comment("one more thing"));
        assert!(!notes.threads[0].resolved);
        assert_eq!(notes.threads[0].comments.len(), 3);
        assert_eq!(notes.open_count(), 2);
    }

    #[test]
    fn unknown_thread_ids_change_nothing() {
        let mut notes = sample();
        let before = notes.clone();
        notes.reply(99, comment("x"));
        notes.set_resolved(99, true);
        notes.delete(99);
        assert_eq!(notes, before);
    }

    #[test]
    fn delete_removes_only_that_thread() {
        let mut notes = sample();
        let id = notes.threads[0].id;
        notes.delete(id);
        assert_eq!(notes.threads.len(), 1);
        assert_eq!(notes.threads[0].path, "src/b.rs");
    }

    #[test]
    fn writer_keeps_the_last_snapshot_per_file_and_flushes_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a").join("notes.json");
        let b = dir.path().join("b").join("notes.json");
        let mut notes = Notes::default();
        let writer = NotesWriter::new();
        for n in 1..=20 {
            notes.add("f.rs", None, anchor(n), comment("c"));
            writer.save(a.clone(), notes.clone());
        }
        writer.save(b.clone(), sample());
        drop(writer);
        assert_eq!(Notes::load(&a), notes);
        assert_eq!(Notes::load(&a).threads.len(), 20);
        assert_eq!(Notes::load(&b), sample());
    }
}
