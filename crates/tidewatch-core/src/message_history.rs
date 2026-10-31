//! The messages the owner sent one tab, newest last, kept in `reply-history.json` beside the tab's
//! notes so Up in an empty reply box finds them again after a restart. At most `KEPT` are kept and a
//! message sent twice in a row is one entry.

use std::path::Path;

use crate::fsutil::write_atomic;

pub const KEPT: usize = 100;

/// The saved messages, oldest first. A missing or unreadable file is no history.
#[must_use]
pub fn load(file: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let mut entries: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    entries.retain(|entry| !entry.trim().is_empty());
    let extra = entries.len().saturating_sub(KEPT);
    entries.drain(..extra);
    entries
}

/// Adds `text` as the newest entry unless it is blank or the same as the newest, and cuts the oldest
/// past `KEPT`. Returns whether `entries` changed.
pub fn remember(entries: &mut Vec<String>, text: &str) -> bool {
    if text.trim().is_empty() || entries.last().is_some_and(|last| last == text) {
        return false;
    }
    entries.push(text.to_owned());
    let extra = entries.len().saturating_sub(KEPT);
    entries.drain(..extra);
    true
}

/// Writes `entries` to `file`.
///
/// # Errors
/// When the file cannot be written.
pub fn save(file: &Path, entries: &[String]) -> std::io::Result<()> {
    let text = serde_json::to_string(entries).map_err(std::io::Error::other)?;
    write_atomic(file, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_message_and_a_repeat_of_the_newest_are_not_remembered() {
        let mut entries = Vec::new();
        let steps = [("fix it", true), ("  \n", false), ("fix it", false), ("then ship", true)];
        for (text, changed) in steps {
            assert_eq!(remember(&mut entries, text), changed, "{text:?}");
        }
        assert_eq!(entries, ["fix it", "then ship"]);
        assert!(remember(&mut entries, "fix it"), "an older message may come again");
    }

    #[test]
    fn only_the_newest_hundred_are_kept() {
        let mut entries = Vec::new();
        for n in 0..KEPT + 7 {
            remember(&mut entries, &format!("message {n}"));
        }
        assert_eq!(entries.len(), KEPT);
        assert_eq!(entries.first().map(String::as_str), Some("message 7"));
        assert_eq!(entries.last().map(String::as_str), Some("message 106"));
    }

    #[test]
    fn what_is_saved_comes_back_in_order_and_a_bad_file_is_no_history() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("reply-history.json");
        assert_eq!(load(&file), Vec::<String>::new());
        let entries = vec!["one".to_owned(), "two\nlines".to_owned()];
        save(&file, &entries).unwrap();
        assert_eq!(load(&file), entries);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(load(&file), Vec::<String>::new());
    }

    #[test]
    fn a_file_with_more_than_the_limit_loads_its_newest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reply-history.json");
        let many: Vec<String> = (0..KEPT + 3).map(|n| format!("m{n}")).collect();
        save(&file, &many).unwrap();
        let loaded = load(&file);
        assert_eq!(loaded.len(), KEPT);
        assert_eq!(loaded.first().map(String::as_str), Some("m3"));
    }
}
