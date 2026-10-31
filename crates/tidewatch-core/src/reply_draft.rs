//! The half-written message of a tab's reply box, kept in `reply-draft.json` beside the tab's notes
//! so a restart brings back the text and the files that were in it. An empty draft is no file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub pictures: Vec<PathBuf>,
}

impl Draft {
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty() && self.pictures.is_empty()
    }
}

/// The saved draft, with the files that are gone left out. None when there is nothing to bring back.
#[must_use]
pub fn load(file: &Path) -> Option<Draft> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut draft: Draft = serde_json::from_str(&text).ok()?;
    draft.pictures.retain(|path| path.exists());
    (!draft.is_blank()).then_some(draft)
}

/// Writes `draft` to `file`, or removes the file when the draft is blank.
///
/// # Errors
/// When the file cannot be written or removed.
pub fn save(file: &Path, draft: &Draft) -> std::io::Result<()> {
    if draft.is_blank() {
        return match std::fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_string(draft).map_err(std::io::Error::other)?;
    write_atomic(file, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_comes_back_with_its_text_and_the_files_that_still_exist() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("reply-draft.json");
        let kept = dir.path().join("shot.png");
        std::fs::write(&kept, b"x").unwrap();
        let draft = Draft {
            text: "check the totals".into(),
            pictures: vec![kept.clone(), dir.path().join("gone.png")],
        };
        save(&file, &draft).unwrap();
        let back = load(&file).unwrap();
        assert_eq!(back, Draft { text: "check the totals".into(), pictures: vec![kept] });
    }

    #[test]
    fn a_blank_draft_removes_the_file_and_loads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reply-draft.json");
        save(&file, &Draft { text: "x".into(), pictures: Vec::new() }).unwrap();
        assert!(file.exists());
        save(&file, &Draft { text: "  ".into(), pictures: Vec::new() }).unwrap();
        assert!(!file.exists());
        save(&file, &Draft::default()).unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn a_broken_file_is_no_draft() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reply-draft.json");
        std::fs::write(&file, "{").unwrap();
        assert_eq!(load(&file), None);
    }
}
