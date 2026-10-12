//! Maps what the diff widget reports (a new thread, a reply, resolve, delete) onto the tab's notes.

use kelp_ui::diff::DiffAction;
use tidewatch_core::notes::Notes;

/// `commit` is the selected commit's id, or `None` while the working tree is shown. Returns
/// whether the notes changed and need saving.
pub(crate) fn apply(
    notes: &mut Notes,
    path: &str,
    commit: Option<&str>,
    actions: Vec<DiffAction>,
) -> bool {
    let mut changed = false;
    for action in actions {
        match action {
            DiffAction::AddThread { anchor, comment } => {
                notes.add(path, commit.map(str::to_owned), anchor, comment);
            }
            DiffAction::Reply { thread, comment } => notes.reply(thread, comment),
            DiffAction::SetResolved { thread, resolved } => notes.set_resolved(thread, resolved),
            DiffAction::DeleteThread(thread) => notes.delete(thread),
            DiffAction::HunkButton { .. } => continue,
        }
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    fn anchor() -> Anchor {
        Anchor {
            side: Side::New,
            line_hint: 4,
            text: "let x = 1;".into(),
            before: vec![],
            after: vec![],
        }
    }

    fn comment(body: &str) -> Comment {
        Comment { author: "You".into(), time: 1, body: body.into() }
    }

    fn add(body: &str) -> DiffAction {
        DiffAction::AddThread { anchor: anchor(), comment: comment(body) }
    }

    #[test]
    fn a_new_thread_lands_on_the_file_and_commit() {
        let mut notes = Notes::default();
        assert!(apply(&mut notes, "src/a.rs", Some("abc"), vec![add("rename")]));
        let thread = &notes.threads[0];
        assert_eq!((thread.path.as_str(), thread.commit.as_deref()), ("src/a.rs", Some("abc")));
        assert_eq!(thread.comments[0].body, "rename");
    }

    #[test]
    fn a_thread_written_on_the_working_tree_has_no_commit() {
        let mut notes = Notes::default();
        apply(&mut notes, "src/a.rs", None, vec![add("rename")]);
        assert_eq!(notes.threads[0].commit, None);
    }

    #[test]
    fn reply_resolve_and_delete_edit_the_thread() {
        let mut notes = Notes::default();
        apply(&mut notes, "a.rs", None, vec![add("first")]);
        let id = notes.threads[0].id;
        let edits = vec![
            DiffAction::Reply { thread: id, comment: comment("second") },
            DiffAction::SetResolved { thread: id, resolved: true },
        ];
        assert!(apply(&mut notes, "a.rs", None, edits));
        assert_eq!(notes.threads[0].comments.len(), 2);
        assert!(notes.threads[0].resolved);
        assert!(apply(&mut notes, "a.rs", None, vec![DiffAction::DeleteThread(id)]));
        assert_eq!(notes.threads, []);
    }

    #[test]
    fn no_actions_and_hunk_buttons_change_nothing() {
        let mut notes = Notes::default();
        let hunk = DiffAction::HunkButton { hunk: 0, button: 0, shift: false };
        assert!(!apply(&mut notes, "a.rs", None, vec![]));
        assert!(!apply(&mut notes, "a.rs", None, vec![hunk]));
        assert_eq!(notes, Notes::default());
    }
}
