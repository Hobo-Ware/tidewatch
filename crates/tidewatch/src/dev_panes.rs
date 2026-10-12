//! Dev-only switches for the screenshot runs, read once at startup.
//!
//! - `TIDEWATCH_DEV_SCOPE=whole`: start the graph on Whole repo.
//! - `TIDEWATCH_DEV_PICK=<row>`: select that graph row (0 is the newest commit) once history loads.
//! - `TIDEWATCH_DEV_NOTE=<text>`: write a note on the first added line of the first diff shown, once
//!   per run, through the same notes store the diff widget uses.

use kelp_core::diff::{Body, FileDiff, LineKind};
use kelp_core::review::{Anchor, Comment, Side};
use kelp_ui::diff::DiffAction;
use tidewatch_core::history::Scope;

pub(crate) struct DevPanes {
    pub(crate) scope: Option<Scope>,
    pub(crate) pick_row: Option<usize>,
    pub(crate) note: Option<String>,
}

impl DevPanes {
    pub(crate) fn from_env() -> Self {
        Self::parse(
            std::env::var("TIDEWATCH_DEV_SCOPE").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_PICK").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_NOTE").ok(),
        )
    }

    fn parse(scope: Option<&str>, pick: Option<&str>, note: Option<String>) -> Self {
        Self {
            scope: (scope == Some("whole")).then_some(Scope::Whole),
            pick_row: pick.and_then(|row| row.parse().ok()),
            note: note.filter(|text| !text.is_empty()),
        }
    }
}

/// The action the diff widget would report for a note on the first added line of `diff`.
pub(crate) fn note_on_first_added_line(
    diff: &FileDiff,
    text: &str,
    now: i64,
) -> Option<DiffAction> {
    let new_text = diff.new_text.as_deref()?;
    let lines: Vec<&str> = new_text.lines().collect();
    let Body::Text(hunks) = &diff.body else { return None };
    let number = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .find(|line| line.kind == LineKind::Added)
        .and_then(|line| line.new)?;
    let index = usize::try_from(number).ok()?.checked_sub(1).filter(|i| *i < lines.len())?;
    Some(DiffAction::AddThread {
        anchor: Anchor::at(&lines, index, Side::New),
        comment: Comment { author: "You".into(), time: now, body: text.to_owned() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::diff;

    #[test]
    fn parses_scope_pick_and_note() {
        let dev = DevPanes::parse(Some("whole"), Some("2"), Some("use the helper".into()));
        assert_eq!(dev.scope, Some(Scope::Whole));
        assert_eq!(dev.pick_row, Some(2));
        assert_eq!(dev.note.as_deref(), Some("use the helper"));
    }

    #[test]
    fn anything_else_leaves_the_defaults() {
        let dev = DevPanes::parse(Some("current"), Some("two"), Some(String::new()));
        assert_eq!((dev.scope, dev.pick_row, dev.note), (None, None, None));
        let dev = DevPanes::parse(None, None, None);
        assert_eq!((dev.scope, dev.pick_row, dev.note), (None, None, None));
    }

    #[test]
    fn the_note_anchors_on_the_first_added_line_with_its_neighbors() {
        let diff = diff::build("a.rs", Some(b"one\ntwo\nthree\n"), Some(b"one\ntwo\nNEW\nthree\n"));
        let Some(DiffAction::AddThread { anchor, comment }) =
            note_on_first_added_line(&diff, "rename this", 7)
        else {
            panic!("expected a new thread")
        };
        assert_eq!((anchor.side, anchor.line_hint, anchor.text.as_str()), (Side::New, 3, "NEW"));
        assert_eq!(anchor.before, ["one", "two"]);
        assert_eq!(anchor.after, ["three"]);
        assert_eq!(
            (comment.author.as_str(), comment.time, comment.body.as_str()),
            ("You", 7, "rename this")
        );
    }

    #[test]
    fn a_diff_with_nothing_added_or_no_text_gets_no_note() {
        let removed = diff::build("a.rs", Some(b"one\ntwo\n"), Some(b"one\n"));
        assert!(note_on_first_added_line(&removed, "x", 0).is_none());
        let binary = diff::build("a.bin", Some(&[0, 159, 146]), Some(&[0, 1, 2]));
        assert!(note_on_first_added_line(&binary, "x", 0).is_none());
    }
}
