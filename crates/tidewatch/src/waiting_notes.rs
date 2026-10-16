//! How many of a tab's notes wait on someone, per commit and per file, for the badges in the
//! timeline. What waits is `NoteState::is_waiting`; a commit that is not a full object id is skipped.

use std::collections::HashMap;

use kelp_core::ObjectId;
use tidewatch_core::notes::Notes;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Waiting {
    commits: HashMap<ObjectId, usize>,
    uncommitted: usize,
    files: HashMap<(Option<ObjectId>, String), usize>,
}

impl Waiting {
    pub(crate) fn of(notes: &Notes, agent: &str) -> Self {
        let mut waiting = Self::default();
        for thread in notes.waiting(agent) {
            let commit = match thread.commit.as_deref() {
                None => None,
                Some(hex) => match ObjectId::from_hex(hex.as_bytes()) {
                    Ok(id) => Some(id),
                    Err(_) => continue,
                },
            };
            match commit {
                Some(id) => *waiting.commits.entry(id).or_default() += 1,
                None => waiting.uncommitted += 1,
            }
            *waiting.files.entry((commit, thread.path.clone())).or_default() += 1;
        }
        waiting
    }

    pub(crate) fn commits(&self) -> &HashMap<ObjectId, usize> {
        &self.commits
    }

    pub(crate) fn uncommitted(&self) -> Option<usize> {
        Some(self.uncommitted).filter(|count| *count > 0)
    }

    /// The count on `path` of `commit`, `None` for the uncommitted changes.
    pub(crate) fn file(&self, commit: Option<ObjectId>, path: &str) -> usize {
        self.files.get(&(commit, path.to_owned())).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    const A: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432";
    const B: &str = "1111111111111111111111111111111111111111";

    fn id(hex: &str) -> ObjectId {
        ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn add(notes: &mut Notes, path: &str, commit: Option<&str>) -> u64 {
        let anchor = Anchor {
            side: Side::New,
            line_hint: 1,
            text: String::new(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        };
        let comment = Comment { author: "You".into(), time: 1, body: "n".into() };
        notes.add(path, commit.map(str::to_owned), anchor, comment)
    }

    #[test]
    fn counts_split_by_commit_the_uncommitted_row_and_file() {
        let mut notes = Notes::default();
        add(&mut notes, "a.rs", Some(A));
        add(&mut notes, "a.rs", Some(A));
        add(&mut notes, "b.rs", Some(A));
        add(&mut notes, "a.rs", Some(B));
        add(&mut notes, "a.rs", None);
        let waiting = Waiting::of(&notes, "claude");
        assert_eq!(waiting.commits().get(&id(A)), Some(&3));
        assert_eq!(waiting.commits().get(&id(B)), Some(&1));
        assert_eq!(waiting.uncommitted(), Some(1));
        let files = [
            (Some(id(A)), "a.rs", 2),
            (Some(id(A)), "b.rs", 1),
            (Some(id(B)), "a.rs", 1),
            (Some(id(B)), "b.rs", 0),
            (None, "a.rs", 1),
            (None, "b.rs", 0),
        ];
        for (commit, path, want) in files {
            assert_eq!(waiting.file(commit, path), want, "{commit:?} {path}");
        }
    }

    #[test]
    fn resolved_and_answered_notes_are_not_counted_and_nothing_gives_none() {
        let mut notes = Notes::default();
        let resolved = add(&mut notes, "a.rs", Some(A));
        let answered = add(&mut notes, "a.rs", Some(A));
        notes.set_resolved(resolved, true);
        notes.mark_sent(&[answered], 5);
        let reply = Comment { author: "claude".into(), time: 6, body: "done".into() };
        notes.reply(answered, reply);
        let waiting = Waiting::of(&notes, "claude");
        assert_eq!(waiting, Waiting::default());
        assert_eq!(waiting.uncommitted(), None);
    }

    #[test]
    fn a_commit_that_is_not_an_object_id_is_skipped_not_counted_anywhere() {
        let mut notes = Notes::default();
        for bad in ["", "abc123", "not a commit", "zz8e7d6c5b4a39281706f5e4d3c2b1a098765432"] {
            add(&mut notes, "a.rs", Some(bad));
        }
        add(&mut notes, "a.rs", Some(A));
        let waiting = Waiting::of(&notes, "claude");
        assert_eq!(waiting.commits().len(), 1);
        assert_eq!(waiting.uncommitted(), None);
        assert_eq!(waiting.file(None, "a.rs"), 0);
    }
}
