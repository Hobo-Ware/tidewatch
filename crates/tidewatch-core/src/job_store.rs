//! A tab's jobs on disk: `<tab state>/jobs.json`, so they come back with the tab.

use std::path::Path;

use crate::fsutil::{read_json, save_json};
use crate::job::JobBoard;

/// The saved board. A missing or unreadable file is `None`.
#[must_use]
pub fn load(file: &Path) -> Option<JobBoard> {
    read_json(file)
}

pub fn save(file: &Path, board: &JobBoard) -> std::io::Result<()> {
    save_json(file, board)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Report;

    #[test]
    fn a_saved_board_loads_back_and_a_missing_or_broken_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("jobs.json");
        assert_eq!(load(&file), None);
        let mut board = JobBoard::default();
        board.report(&Report { done: Some(4.0), total: Some(9.0), ..Report::named("orders") }, 7);
        save(&file, &board).unwrap();
        assert_eq!(load(&file), Some(board));
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load(&file), None);
    }
}
