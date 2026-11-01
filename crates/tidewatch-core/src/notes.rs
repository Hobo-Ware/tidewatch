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

use crate::fsutil::save_json_pretty;

/// When a thread was sent to the agent, and how many comments it had then.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    /// Unix seconds.
    pub at: i64,
    pub comments: usize,
}

/// Where a thread stands with the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteState {
    Resolved,
    /// Not sent yet, or the user wrote again after it was sent.
    Queued,
    /// Unix seconds.
    Sent(i64),
    /// The agent answered a sent thread.
    Replied,
    /// The agent wrote the thread itself (`report_finding`); there is nothing to send.
    FromAgent,
}

/// `12` for one line, `12-15` for a block of lines.
#[must_use]
pub fn line_label(anchor: &Anchor) -> String {
    match anchor.span {
        0 => anchor.line_hint.to_string(),
        _ => format!("{}-{}", anchor.line_hint, anchor.last_line_hint()),
    }
}

impl NoteState {
    /// Whether the thread still needs something from the user or the agent: a draft to send, or a
    /// sent thread the agent has not answered. Resolved, answered and agent-written threads do not.
    #[must_use]
    pub fn is_waiting(self) -> bool {
        matches!(self, Self::Queued | Self::Sent(_))
    }
}

/// `:12`, `:12-15` for a block, or nothing for a note on the whole file (line 0).
#[must_use]
pub fn line_suffix(anchor: &Anchor) -> String {
    match anchor.line_hint {
        0 => String::new(),
        _ => format!(":{}", line_label(anchor)),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    sent: BTreeMap<u64, Sent>,
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

    /// Atomic, so a crash never leaves half a file.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        save_json_pretty(file, self)
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
        self.sent.remove(&id);
    }

    /// Records that the threads `ids` were sent to the agent at `at` (unix seconds). Unknown ids are
    /// skipped.
    pub fn mark_sent(&mut self, ids: &[u64], at: i64) {
        for thread in self.threads.iter().filter(|t| ids.contains(&t.id)) {
            self.sent.insert(thread.id, Sent { at, comments: thread.comments.len() });
        }
    }

    pub fn unmark_sent(&mut self, ids: &[u64]) {
        for id in ids {
            self.sent.remove(id);
        }
    }

    #[must_use]
    pub fn sent(&self, id: u64) -> Option<Sent> {
        self.sent.get(&id).copied()
    }

    /// Where `thread` stands. `agent` is the id the agent signs its replies with.
    #[must_use]
    pub fn state(&self, thread: &Thread, agent: &str) -> NoteState {
        if thread.resolved {
            return NoteState::Resolved;
        }
        let by_agent = thread.comments.last().is_some_and(|c| c.author == agent);
        let sent = self.sent(thread.id);
        match (by_agent, sent) {
            (true, Some(_)) => NoteState::Replied,
            (true, None) => NoteState::FromAgent,
            (false, Some(mark)) if thread.comments.len() <= mark.comments => {
                NoteState::Sent(mark.at)
            }
            (false, _) => NoteState::Queued,
        }
    }

    /// The open threads still to send, oldest first.
    #[must_use]
    pub fn pending(&self, agent: &str) -> Vec<u64> {
        self.threads
            .iter()
            .filter(|t| self.state(t, agent) == NoteState::Queued)
            .map(|t| t.id)
            .collect()
    }

    /// The threads that still wait on someone (see [`NoteState::is_waiting`]), oldest first.
    pub fn waiting<'a>(&'a self, agent: &'a str) -> impl Iterator<Item = &'a Thread> + 'a {
        self.threads.iter().filter(move |t| self.state(t, agent).is_waiting())
    }

    /// The local thread made from the pull request thread whose first comment has GitHub id `root`.
    /// The id outlives a deleted thread, so deleting an imported thread keeps it deleted.
    #[must_use]
    pub fn pr_thread(&self, root: u64) -> Option<u64> {
        self.pr_roots.get(&root).copied()
    }

    /// GitHub's id of the first comment of the pull request thread that `thread` was imported from or
    /// posted as.
    #[must_use]
    pub fn pr_root_of(&self, thread: u64) -> Option<u64> {
        self.pr_roots.iter().find(|(_, local)| **local == thread).map(|(root, _)| *root)
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
            span: 0,
            block: vec![],
        }
    }

    #[test]
    fn the_line_label_is_a_line_or_a_range() {
        let mut block = anchor(12);
        let cases = [(0, "12"), (1, "12-13"), (5, "12-17")];
        for (span, want) in cases {
            block.span = span;
            assert_eq!(line_label(&block), want, "span {span}");
        }
    }

    #[test]
    fn the_line_suffix_is_empty_for_a_whole_file_note() {
        let mut block = anchor(12);
        block.span = 3;
        let mut file = anchor(0);
        file.span = 3;
        assert_eq!(line_suffix(&anchor(7)), ":7");
        assert_eq!(line_suffix(&block), ":12-15");
        assert_eq!(line_suffix(&file), "");
    }

    #[test]
    fn a_block_note_survives_a_save_and_a_note_from_before_blocks_loads_as_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut block = anchor(5);
        block.span = 2;
        block.block = vec!["line 6".into(), "line 7".into()];
        let mut notes = Notes::default();
        notes.add("a.rs", None, block.clone(), comment("block"));
        notes.save(&file).unwrap();
        assert_eq!(Notes::load(&file).threads[0].anchor, block);
        let old = r#"{"threads":[{"id":1,"path":"a.rs","commit":null,"anchor":{"side":"New","line_hint":3,"text":"x","before":[],"after":[]},"resolved":false,"comments":[]}]}"#;
        std::fs::write(&file, old).unwrap();
        let loaded = Notes::load(&file);
        assert_eq!((loaded.threads[0].anchor.span, loaded.threads[0].anchor.block.len()), (0, 0));
    }

    #[test]
    fn the_states_that_wait_are_the_draft_and_the_sent() {
        let cases = [
            (NoteState::Queued, true),
            (NoteState::Sent(1), true),
            (NoteState::Replied, false),
            (NoteState::Resolved, false),
            (NoteState::FromAgent, false),
        ];
        for (state, want) in cases {
            assert_eq!(state.is_waiting(), want, "{state:?}");
        }
    }

    #[test]
    fn only_a_draft_or_an_unanswered_sent_thread_is_waiting() {
        let mut notes = Notes::default();
        let draft = notes.add("a.rs", None, anchor(1), comment("draft"));
        let sent = notes.add("a.rs", None, anchor(2), comment("sent"));
        let replied = notes.add("a.rs", None, anchor(3), comment("replied"));
        let resolved = notes.add("a.rs", None, anchor(4), comment("resolved"));
        let again = notes.add("a.rs", None, anchor(5), comment("written again after sending"));
        let finding = notes.add(
            "a.rs",
            None,
            anchor(6),
            Comment { author: "claude".into(), ..comment("found") },
        );
        notes.mark_sent(&[sent, replied, resolved, again], 10);
        notes.reply(replied, Comment { author: "claude".into(), ..comment("done") });
        notes.set_resolved(resolved, true);
        notes.reply(again, comment("one more thing"));
        let cases = [
            (draft, true),
            (sent, true),
            (replied, false),
            (resolved, false),
            (again, true),
            (finding, false),
        ];
        let waiting: Vec<u64> = notes.waiting("claude").map(|t| t.id).collect();
        for (id, want) in cases {
            assert_eq!(waiting.contains(&id), want, "thread {id}");
        }
        assert_eq!(waiting.len(), 3);
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

    fn thread_state(notes: &Notes, id: u64) -> NoteState {
        let thread = notes.threads.iter().find(|t| t.id == id).unwrap();
        notes.state(thread, "claude")
    }

    #[test]
    fn a_new_note_is_queued_until_it_is_sent_and_a_later_comment_queues_it_again() {
        let mut notes = Notes::default();
        let id = notes.add("a.rs", None, anchor(1), comment("rename"));
        assert_eq!(thread_state(&notes, id), NoteState::Queued);
        assert_eq!(notes.pending("claude"), [id]);
        notes.mark_sent(&[id], 1_790_000_100);
        assert_eq!(thread_state(&notes, id), NoteState::Sent(1_790_000_100));
        assert_eq!(notes.pending("claude"), Vec::<u64>::new());
        notes.reply(id, comment("and add a test"));
        assert_eq!(thread_state(&notes, id), NoteState::Queued);
    }

    #[test]
    fn taking_the_sent_mark_back_queues_the_note_again() {
        let mut notes = Notes::default();
        let (a, b) = (
            notes.add("a.rs", None, anchor(1), comment("rename")),
            notes.add("b.rs", None, anchor(2), comment("split")),
        );
        notes.mark_sent(&[a, b], 5);
        assert_eq!(notes.pending("claude"), Vec::<u64>::new());
        notes.unmark_sent(&[a, 99]);
        assert_eq!(notes.pending("claude"), [a]);
        assert_eq!(thread_state(&notes, b), NoteState::Sent(5));
    }

    #[test]
    fn an_agent_reply_makes_a_sent_note_replied_and_an_agent_finding_is_not_pending() {
        let mut notes = Notes::default();
        let id = notes.add("a.rs", None, anchor(1), comment("rename"));
        notes.mark_sent(&[id], 5);
        let agent = Comment { author: "claude".into(), time: 6, body: "done".into() };
        notes.reply(id, agent.clone());
        assert_eq!(thread_state(&notes, id), NoteState::Replied);
        let finding = notes.add("b.rs", None, anchor(2), agent);
        assert_eq!(thread_state(&notes, finding), NoteState::FromAgent);
        assert_eq!(notes.pending("claude"), Vec::<u64>::new());
    }

    #[test]
    fn resolved_notes_are_not_pending_and_deleting_forgets_the_mark() {
        let mut notes = Notes::default();
        let id = notes.add("a.rs", None, anchor(1), comment("rename"));
        notes.mark_sent(&[id], 5);
        notes.set_resolved(id, true);
        assert_eq!(thread_state(&notes, id), NoteState::Resolved);
        notes.delete(id);
        assert_eq!(notes.sent(id), None);
        let mut other = Notes::default();
        other.mark_sent(&[42], 1);
        assert_eq!(other, Notes::default());
    }

    #[test]
    fn sent_marks_survive_the_file_and_an_old_file_without_them_loads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut notes = sample();
        let id = notes.threads[0].id;
        notes.mark_sent(&[id], 77);
        notes.save(&file).unwrap();
        assert_eq!(Notes::load(&file).sent(id), Some(Sent { at: 77, comments: 2 }));
        let mut value = serde_json::to_value(&notes).unwrap();
        value.as_object_mut().unwrap().remove("sent");
        std::fs::write(&file, value.to_string()).unwrap();
        assert_eq!(Notes::load(&file).sent(id), None);
    }

    #[test]
    fn a_linked_thread_knows_its_pull_request_root() {
        let mut notes = sample();
        let id = notes.threads[0].id;
        assert_eq!(notes.pr_root_of(id), None);
        notes.link_pr_thread(900, id);
        assert_eq!(notes.pr_root_of(id), Some(900));
        assert_eq!(notes.pr_thread(900), Some(id));
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
