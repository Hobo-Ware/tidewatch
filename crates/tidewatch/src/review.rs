//! The review of a tab as plain data. Notes never send by themselves: the open notes that were not
//! sent yet are the tab's draft review, across every file; a sent review is read back from the
//! pasted block in the transcript and shown with where each of its notes stands. Everything is
//! derived from the notes file, so nothing here is stored.

use tidewatch_core::notes::{NoteState, Notes, line_suffix};

use crate::note_send::ReviewGate;

const TEXT_MAX: usize = 160;
const LOCAL_AUTHOR: &str = "You";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    pub(crate) files: Vec<FileNotes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileNotes {
    pub(crate) path: String,
    pub(crate) notes: Vec<DraftNote>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DraftNote {
    pub(crate) id: u64,
    /// `:12`, `:12-15` or empty for a note on the whole file.
    pub(crate) lines: String,
    pub(crate) text: String,
    /// The pull request reviewer who wrote it, when it was pulled in rather than written here.
    pub(crate) reviewer: Option<String>,
}

impl Draft {
    /// The notes waiting to go to `agent`, grouped by file in the order the files first appear.
    /// `me` is the name the user's own notes are signed with.
    pub(crate) fn of(notes: &Notes, agent: &str, me: &str) -> Self {
        let mut files: Vec<FileNotes> = Vec::new();
        for id in notes.pending(agent) {
            let Some(thread) = notes.threads.iter().find(|thread| thread.id == id) else {
                continue;
            };
            let first_author = thread.comments.first().map(|comment| comment.author.as_str());
            let reviewer = first_author
                .filter(|author| {
                    notes.pr_root_of(id).is_some() && *author != me && *author != LOCAL_AUTHOR
                })
                .map(str::to_owned);
            let note = DraftNote {
                id,
                lines: line_suffix(&thread.anchor),
                text: cut(thread.comments.last().map_or("", |comment| comment.body.as_str())),
                reviewer,
            };
            match files.iter_mut().find(|file| file.path == thread.path) {
                Some(file) => file.notes.push(note),
                None => files.push(FileNotes { path: thread.path.clone(), notes: vec![note] }),
            }
        }
        Self { files }
    }

    pub(crate) fn count(&self) -> usize {
        self.files.iter().map(|file| file.notes.len()).sum()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(crate) fn ids(&self) -> Vec<u64> {
        self.files.iter().flat_map(|file| file.notes.iter().map(|note| note.id)).collect()
    }

    /// `3 notes · 2 files`.
    pub(crate) fn summary(&self) -> String {
        let (notes, files) = (self.count(), self.files.len());
        format!("{} · {}", plural(notes, "note"), plural(files, "file"))
    }
}

/// What sending the draft review does right now, for the card above the chat box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SendState {
    Ready,
    /// The agent is busy: sending holds the review until it is idle.
    WhenIdle,
    /// Held, waiting for the agent to be idle.
    Held,
    Refused(&'static str),
}

impl SendState {
    pub(crate) fn of(gate: ReviewGate, held: bool) -> Self {
        match (gate, held) {
            (ReviewGate::Refused(reason), _) => Self::Refused(reason),
            (_, true) => Self::Held,
            (ReviewGate::WhenIdle, false) => Self::WhenIdle,
            (ReviewGate::Now, false) => Self::Ready,
        }
    }
}

/// What the Reader needs to know about the tab's review.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReviewFacts<'a> {
    pub(crate) draft: &'a Draft,
    pub(crate) send: SendState,
    pub(crate) notes: &'a Notes,
    /// The id the agent signs its replies with.
    pub(crate) agent_id: &'a str,
    /// What the agent is called to the user.
    pub(crate) profile: &'a str,
    pub(crate) working: bool,
    /// No dialog is over the window, so the card may take Cmd+Enter.
    pub(crate) keys_free: bool,
}

/// The chip in the file view's header and the status bar: `Review: 3 notes`.
pub(crate) fn chip_text(count: usize) -> String {
    format!("Review: {}", plural(count, "note"))
}

pub(crate) fn plural(count: usize, word: &str) -> String {
    if count == 1 { format!("1 {word}") } else { format!("{count} {word}s") }
}

fn cut(body: &str) -> String {
    let text = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= TEXT_MAX {
        return text;
    }
    let head: String = text.chars().take(TEXT_MAX).collect();
    format!("{}...", head.trim_end())
}

/// Where one note of a sent review stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    Answered,
    /// The agent is busy and this is the first note it has not answered.
    Working,
    Waiting,
    /// Closed by the user, answered or not.
    Resolved,
}

impl Standing {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Working => "working on it",
            Self::Waiting => "waiting",
            Self::Resolved => "resolved",
        }
    }

    fn is_answered(self) -> bool {
        matches!(self, Self::Answered | Self::Resolved)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SentNote {
    pub(crate) id: u64,
    pub(crate) path: String,
    pub(crate) lines: String,
    pub(crate) text: String,
    pub(crate) reply: Option<String>,
    pub(crate) standing: Standing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SentReview {
    /// How many notes the review was sent with.
    pub(crate) sent: usize,
    /// The ones still there to show.
    pub(crate) notes: Vec<SentNote>,
}

impl SentReview {
    /// The notes `ids` that still exist, in the order they were sent. `agent_working` says whether
    /// the agent is busy now, which marks the first unanswered note as the one it is on.
    pub(crate) fn of(ids: &[u64], notes: &Notes, agent: &str, agent_working: bool) -> Self {
        let mut busy = agent_working;
        let notes = ids
            .iter()
            .filter_map(|id| notes.threads.iter().find(|thread| thread.id == *id))
            .map(|thread| {
                let state = notes.state(thread, agent);
                let standing = match state {
                    NoteState::Resolved => Standing::Resolved,
                    NoteState::Replied => Standing::Answered,
                    NoteState::Queued | NoteState::Sent(_) | NoteState::FromAgent => {
                        if std::mem::take(&mut busy) {
                            Standing::Working
                        } else {
                            Standing::Waiting
                        }
                    }
                };
                let reply = (standing == Standing::Answered)
                    .then(|| thread.comments.last())
                    .flatten()
                    .map(|comment| comment.body.trim().to_owned());
                let asked = thread.comments.iter().rev().find(|comment| comment.author != agent);
                SentNote {
                    id: thread.id,
                    path: thread.path.clone(),
                    lines: line_suffix(&thread.anchor),
                    text: cut(asked.map_or("", |comment| comment.body.as_str())),
                    reply,
                    standing,
                }
            })
            .collect();
        Self { sent: ids.len(), notes }
    }

    pub(crate) fn answered(&self) -> usize {
        self.notes.iter().filter(|note| note.standing.is_answered()).count()
    }

    /// `3 notes`.
    pub(crate) fn notes_label(&self) -> String {
        plural(self.sent, "note")
    }

    /// `2 of 3 answered`.
    pub(crate) fn progress(&self) -> String {
        format!("{} of {} answered", self.answered(), self.notes.len())
    }

    /// The notes the agent answered that are still open: what Resolve answered closes.
    pub(crate) fn resolvable(&self) -> Vec<u64> {
        self.notes
            .iter()
            .filter(|note| note.standing == Standing::Answered)
            .map(|note| note.id)
            .collect()
    }

    /// The note Open in the file view goes to: the first one still waiting, else the first.
    pub(crate) fn first_open(&self) -> Option<&SentNote> {
        self.notes.iter().find(|note| !note.standing.is_answered()).or_else(|| self.notes.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    fn anchor(line: u32) -> Anchor {
        Anchor {
            side: Side::New,
            line_hint: line,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        }
    }

    fn comment(author: &str, body: &str) -> Comment {
        Comment { author: author.into(), time: 1, body: body.into() }
    }

    fn written(notes: &mut Notes, path: &str, line: u32, body: &str) -> u64 {
        notes.add(path, None, anchor(line), comment("vlad", body))
    }

    #[test]
    fn the_draft_gathers_unsent_open_notes_by_file_in_order_of_appearance() {
        let mut notes = Notes::default();
        written(&mut notes, "src/a.rs", 3, "first");
        written(&mut notes, "src/b.rs", 9, "second");
        written(&mut notes, "src/a.rs", 20, "third");
        let sent = written(&mut notes, "src/c.rs", 1, "already gone");
        notes.mark_sent(&[sent], 5);
        let resolved = written(&mut notes, "src/d.rs", 1, "closed");
        notes.set_resolved(resolved, true);
        notes.add("src/e.rs", None, anchor(2), comment("claude", "finding"));
        let draft = Draft::of(&notes, "claude", "vlad");
        let shape: Vec<_> = draft
            .files
            .iter()
            .map(|file| {
                (
                    file.path.as_str(),
                    file.notes.iter().map(|n| n.lines.as_str()).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(shape, [("src/a.rs", vec![":3", ":20"]), ("src/b.rs", vec![":9"])]);
        assert_eq!((draft.count(), draft.ids().len()), (3, 3));
        assert_eq!(draft.summary(), "3 notes \u{b7} 2 files");
    }

    #[test]
    fn a_block_note_shows_its_range_in_the_draft_and_in_the_sent_review() {
        let mut notes = Notes::default();
        let mut block = anchor(12);
        block.span = 3;
        let id = notes.add("src/a.rs", None, block, comment("vlad", "extract"));
        notes.add("src/b.rs", None, anchor(0), comment("vlad", "whole file"));
        let draft = Draft::of(&notes, "claude", "vlad");
        let lines: Vec<&str> = draft.files.iter().map(|f| f.notes[0].lines.as_str()).collect();
        assert_eq!(lines, [":12-15", ""]);
        notes.mark_sent(&[id], 5);
        let sent = SentReview::of(&[id], &notes, "claude", false);
        assert_eq!(sent.notes[0].lines, ":12-15");
    }

    #[test]
    fn no_open_notes_is_no_draft() {
        let draft = Draft::of(&Notes::default(), "claude", "vlad");
        assert_eq!(draft, Draft::default());
        assert_eq!(draft.count(), 0);
        assert_eq!(draft.summary(), "0 notes \u{b7} 0 files");
    }

    #[test]
    fn a_note_written_again_after_it_was_sent_is_back_in_the_draft() {
        let mut notes = Notes::default();
        let id = written(&mut notes, "a.rs", 1, "rename");
        notes.mark_sent(&[id], 5);
        assert_eq!(Draft::of(&notes, "claude", "vlad"), Draft::default());
        notes.reply(id, comment("vlad", "and add a test"));
        let draft = Draft::of(&notes, "claude", "vlad");
        assert_eq!(draft.files[0].notes[0].text, "and add a test");
    }

    #[test]
    fn a_pulled_comment_carries_its_reviewers_name_and_the_users_own_do_not() {
        let mut notes = Notes::default();
        let pulled = notes.add("a.rs", None, anchor(1), comment("rectifyer", "guard this"));
        let mine_on_pr = notes.add("a.rs", None, anchor(2), comment("vlad", "posted by me"));
        let signed_you = notes.add("a.rs", None, anchor(3), comment("You", "no git name"));
        let local = notes.add("a.rs", None, anchor(4), comment("rectifyer", "not on a PR"));
        notes.link_pr_thread(900, pulled);
        notes.link_pr_thread(901, mine_on_pr);
        notes.link_pr_thread(902, signed_you);
        let draft = Draft::of(&notes, "claude", "vlad");
        let marks: Vec<_> =
            draft.files[0].notes.iter().map(|n| (n.id, n.reviewer.clone())).collect();
        assert_eq!(
            marks,
            [
                (pulled, Some("rectifyer".to_owned())),
                (mine_on_pr, None),
                (signed_you, None),
                (local, None),
            ]
        );
    }

    #[test]
    fn a_long_note_is_flattened_and_cut() {
        assert_eq!(cut("one\n  two\n\nthree"), "one two three");
        let long = "word ".repeat(80);
        let text = cut(&long);
        assert!(text.ends_with("..."));
        assert!(text.chars().count() <= TEXT_MAX + 3);
        assert!(text.starts_with("word word word"));
    }

    #[test]
    fn a_held_review_is_held_a_busy_agent_holds_it_and_no_agent_refuses_it() {
        let cases = [
            (ReviewGate::Now, false, SendState::Ready),
            (ReviewGate::WhenIdle, false, SendState::WhenIdle),
            (ReviewGate::WhenIdle, true, SendState::Held),
            (ReviewGate::Now, true, SendState::Held),
            (ReviewGate::Refused("no"), false, SendState::Refused("no")),
            (ReviewGate::Refused("no"), true, SendState::Refused("no")),
        ];
        for (gate, held, want) in cases {
            assert_eq!(SendState::of(gate, held), want, "{gate:?} held {held}");
        }
    }

    #[test]
    fn words_agree_with_their_count() {
        assert_eq!(chip_text(1), "Review: 1 note");
        assert_eq!(chip_text(3), "Review: 3 notes");
        assert_eq!(plural(0, "file"), "0 files");
    }

    fn sent(notes: &mut Notes, specs: &[(&str, u32, &str)]) -> Vec<u64> {
        let ids: Vec<u64> = specs.iter().map(|(p, l, b)| written(notes, p, *l, b)).collect();
        notes.mark_sent(&ids, 10);
        ids
    }

    #[test]
    fn a_sent_review_says_which_notes_are_answered_and_which_one_the_agent_is_on() {
        let mut notes = Notes::default();
        let ids = sent(&mut notes, &[("a.rs", 1, "one"), ("a.rs", 2, "two"), ("b.rs", 3, "three")]);
        notes.reply(ids[0], comment("claude", "moved it"));
        let working = SentReview::of(&ids, &notes, "claude", true);
        let standing: Vec<_> = working.notes.iter().map(|n| n.standing).collect();
        assert_eq!(standing, [Standing::Answered, Standing::Working, Standing::Waiting]);
        assert_eq!(working.notes[0].reply.as_deref(), Some("moved it"));
        assert_eq!(working.notes[1].reply, None);
        assert_eq!(working.progress(), "1 of 3 answered");
        assert_eq!(working.notes_label(), "3 notes");
        let idle = SentReview::of(&ids, &notes, "claude", false);
        let standing: Vec<_> = idle.notes.iter().map(|n| n.standing).collect();
        assert_eq!(standing, [Standing::Answered, Standing::Waiting, Standing::Waiting]);
    }

    #[test]
    fn a_resolved_note_counts_as_answered_and_resolve_answered_closes_only_the_replied_ones() {
        let mut notes = Notes::default();
        let ids = sent(&mut notes, &[("a.rs", 1, "one"), ("a.rs", 2, "two"), ("b.rs", 3, "three")]);
        notes.reply(ids[0], comment("claude", "done"));
        notes.set_resolved(ids[1], true);
        let review = SentReview::of(&ids, &notes, "claude", false);
        assert_eq!(review.answered(), 2);
        assert_eq!(review.resolvable(), [ids[0]]);
        assert_eq!(review.first_open().map(|n| n.id), Some(ids[2]));
    }

    #[test]
    fn deleted_notes_drop_out_and_an_answered_review_opens_its_first_note() {
        let mut notes = Notes::default();
        let ids = sent(&mut notes, &[("a.rs", 1, "one"), ("b.rs", 2, "two")]);
        notes.reply(ids[0], comment("claude", "done"));
        notes.reply(ids[1], comment("claude", "done too"));
        notes.delete(ids[0]);
        let review = SentReview::of(&ids, &notes, "claude", false);
        assert_eq!(review.notes.len(), 1);
        assert_eq!(review.first_open().map(|n| n.id), Some(ids[1]));
        assert_eq!(review.notes_label(), "2 notes");
        let both = SentReview::of(&[ids[0], 77], &notes, "claude", false);
        assert_eq!((both.notes.len(), both.first_open()), (0, None));
        assert_eq!(both.progress(), "0 of 0 answered");
    }

    #[test]
    fn the_note_text_is_what_the_user_asked_not_what_the_agent_answered() {
        let mut notes = Notes::default();
        let ids = sent(&mut notes, &[("a.rs", 1, "rename it")]);
        notes.reply(ids[0], comment("claude", "renamed"));
        let review = SentReview::of(&ids, &notes, "claude", false);
        assert_eq!(review.notes[0].text, "rename it");
        assert_eq!(review.notes[0].reply.as_deref(), Some("renamed"));
    }

    #[test]
    fn each_standing_has_its_own_word() {
        let words: Vec<_> =
            [Standing::Answered, Standing::Working, Standing::Waiting, Standing::Resolved]
                .map(Standing::label)
                .to_vec();
        assert_eq!(words, ["answered", "working on it", "waiting", "resolved"]);
    }
}
