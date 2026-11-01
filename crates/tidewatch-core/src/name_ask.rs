//! Asking a tab for a name (pure rules, plus the marker that keeps the ask to once): the answer to a
//! tab's next prompt carries `ASK` once its saved report has a goal and it has not named itself.

use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;
use crate::progress::Ledger;

pub const ASK: &str = "Also call set_name once with a 2 to 4 word kebab-case name for this task, such as fix-https-validation.";

const MARKER: &str = "name-asked";

/// The marker file beside a tab's notes. It exists once the tab was asked, or named itself.
#[must_use]
pub fn marker_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(MARKER)
}

#[must_use]
pub fn was_asked(notes_file: &Path) -> bool {
    marker_beside(notes_file).exists()
}

pub fn mark_asked(notes_file: &Path) -> std::io::Result<()> {
    write_atomic(&marker_beside(notes_file), "")
}

/// Whether the tab is asked on this prompt: its report states a goal and it was never asked.
#[must_use]
pub fn due(ledger: Option<&Ledger>, asked: bool) -> bool {
    !asked && ledger.is_some_and(|ledger| !ledger.goal.trim().is_empty())
}

/// The ask for this prompt, with its marker written first; `None` when the tab is not due or the
/// marker could not be written, so a failing disk never repeats the ask.
#[must_use]
pub fn line_due(notes_file: &Path, ledger: Option<&Ledger>) -> Option<&'static str> {
    (due(ledger, was_asked(notes_file)) && mark_asked(notes_file).is_ok()).then_some(ASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(goal: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\n{goal}\n\n## Status\nstate: working\n"))
    }

    #[test]
    fn the_ask_is_due_only_with_a_goal_and_never_after_it_was_made() {
        let cases = [
            ("a goal", Some(ledger("Ship it")), false, true),
            ("no report", None, false, false),
            ("a blank goal", Some(ledger("   ")), false, false),
            ("an empty report", Some(Ledger::default()), false, false),
            ("already asked", Some(ledger("Ship it")), true, false),
        ];
        for (what, ledger, asked, want) in cases {
            assert_eq!(due(ledger.as_ref(), asked), want, "{what}");
        }
    }

    #[test]
    fn the_marker_is_written_before_the_line_and_the_line_comes_once() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("tab").join("notes.json");
        std::fs::create_dir_all(notes.parent().unwrap()).unwrap();
        let report = ledger("Ship it");
        assert_eq!(line_due(&notes, None), None);
        assert!(!was_asked(&notes), "no goal, no marker");
        assert_eq!(line_due(&notes, Some(&report)), Some(ASK));
        assert!(was_asked(&notes));
        assert_eq!(line_due(&notes, Some(&report)), None);
    }

    #[test]
    fn a_marker_that_cannot_be_written_means_no_ask() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("blocked"), "a file, not a folder").unwrap();
        let notes = dir.path().join("blocked").join("notes.json");
        assert_eq!(line_due(&notes, Some(&ledger("Ship it"))), None);
    }

    #[test]
    fn the_ask_names_the_tool_and_the_rule() {
        for needle in ["set_name", "kebab-case", "2 to 4 word"] {
            assert!(ASK.contains(needle), "{needle}");
        }
    }
}
