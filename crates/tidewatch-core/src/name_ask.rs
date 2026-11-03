//! Asking a tab for a name (the marker keeps the ask to once): the answer to a tab's first prompt
//! carries `ASK` unless it already named itself, so the owner never has to name it by hand.

use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;

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

/// The ask for this prompt, with its marker written first; `None` once the tab was asked or named
/// itself, or when the marker could not be written, so a failing disk never repeats the ask.
#[must_use]
pub fn line_due(notes_file: &Path) -> Option<&'static str> {
    (!was_asked(notes_file) && mark_asked(notes_file).is_ok()).then_some(ASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_prompt_gets_the_ask_with_its_marker_and_no_later_one_does() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("tab").join("notes.json");
        std::fs::create_dir_all(notes.parent().unwrap()).unwrap();
        assert!(!was_asked(&notes));
        assert_eq!(line_due(&notes), Some(ASK));
        assert!(was_asked(&notes));
        assert_eq!(line_due(&notes), None);
    }

    #[test]
    fn a_marker_that_cannot_be_written_means_no_ask() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("blocked"), "a file, not a folder").unwrap();
        let notes = dir.path().join("blocked").join("notes.json");
        assert_eq!(line_due(&notes), None);
    }

    #[test]
    fn the_ask_names_the_tool_and_the_rule() {
        for needle in ["set_name", "kebab-case", "2 to 4 word"] {
            assert!(ASK.contains(needle), "{needle}");
        }
    }
}
