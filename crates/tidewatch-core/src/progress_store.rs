//! A tab's ledger on disk: `<tab state>/progress.md`, in the canonical text of [`Ledger::to_markdown`].

use std::path::Path;

use crate::fsutil::write_atomic;
use crate::progress::Ledger;

/// The text of the saved ledger, `None` when there is none.
#[must_use]
pub fn read_text(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok().filter(|text| !text.trim().is_empty())
}

/// The saved ledger. A missing or empty file is `None`; a file that is not a ledger is read as far
/// as it goes.
#[must_use]
pub fn load(file: &Path) -> Option<Ledger> {
    read_text(file).map(|text| Ledger::parse(&text))
}

pub fn save(file: &Path, ledger: &Ledger) -> std::io::Result<()> {
    write_atomic(file, &ledger.to_markdown())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_ledger_loads_back_and_a_missing_or_empty_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("progress.md");
        assert_eq!(load(&file), None);
        let ledger = Ledger::parse("# Goal\nShip it\n\n## Next\n- go\n");
        save(&file, &ledger).unwrap();
        assert_eq!(load(&file), Some(ledger.clone()));
        assert_eq!(read_text(&file).as_deref(), Some("# Goal\nShip it\n\n## Next\n- go\n"));
        std::fs::write(&file, "  \n").unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn a_file_that_is_not_a_ledger_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        std::fs::write(&file, "just some words\n").unwrap();
        let ledger = load(&file).unwrap();
        assert_eq!(ledger.goal, "");
        assert_eq!(ledger.extra.len(), 1);
    }
}
