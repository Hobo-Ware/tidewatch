//! A tab's report on disk: `<tab state>/progress.json` holds the typed report, and `progress.md`
//! beside it holds the same report as the canonical text of [`Ledger::to_markdown`]. A tab that
//! saved only `progress.md` (an older Tidewatch, or a ledger written by hand) still loads from it.

use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;
use crate::progress::Ledger;
use crate::progress_report::ProgressReport;

const REPORT_FILE: &str = "progress.json";
const MARKDOWN_TOLD_FILE: &str = "progress-markdown-told";

/// The typed report beside the tab's `progress.md`.
#[must_use]
pub fn report_file(file: &Path) -> PathBuf {
    file.with_file_name(REPORT_FILE)
}

fn markdown_told_file(file: &Path) -> PathBuf {
    file.with_file_name(MARKDOWN_TOLD_FILE)
}

/// The text of the saved ledger, `None` when there is none.
#[must_use]
pub fn read_text(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok().filter(|text| !text.trim().is_empty())
}

fn read_report(file: &Path) -> Option<ProgressReport> {
    let text = std::fs::read_to_string(report_file(file)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The saved report: the typed one, else the markdown ledger mapped onto the typed model. A
/// missing or empty file is `None`.
#[must_use]
pub fn load_report(file: &Path) -> Option<ProgressReport> {
    read_report(file)
        .or_else(|| read_text(file).map(|text| ProgressReport::from_ledger(&Ledger::parse(&text))))
}

/// The saved ledger as the screens read it. A markdown ledger that was never saved as a report is
/// read as written; a file that is not a ledger is read as far as it goes.
#[must_use]
pub fn load(file: &Path) -> Option<Ledger> {
    match read_report(file) {
        Some(report) => Some(report.to_ledger()),
        None => read_text(file).map(|text| Ledger::parse(&text)),
    }
}

pub fn save_report(file: &Path, report: &ProgressReport) -> std::io::Result<()> {
    write_atomic(&report_file(file), &report.to_json())?;
    write_atomic(file, &report.to_ledger().to_markdown())
}

/// Saves a ledger written in markdown: the report it maps to is the one stored, and the text the
/// agent wrote stays in `progress.md`.
pub fn save(file: &Path, ledger: &Ledger) -> std::io::Result<()> {
    write_atomic(&report_file(file), &ProgressReport::from_ledger(ledger).to_json())?;
    write_atomic(file, &ledger.to_markdown())
}

/// Whether the agent was already told to prefer the typed fields.
#[must_use]
pub fn markdown_was_told(file: &Path) -> bool {
    markdown_told_file(file).exists()
}

pub fn mark_markdown_told(file: &Path) -> std::io::Result<()> {
    write_atomic(&markdown_told_file(file), "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_ledger_loads_back_and_a_missing_or_empty_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("progress.md");
        assert_eq!(load(&file), None);
        assert_eq!(load_report(&file), None);
        let ledger = Ledger::parse("# Goal\nShip it\n\n## Next\n- go\n");
        save(&file, &ledger).unwrap();
        let loaded = load(&file).unwrap();
        assert_eq!(loaded.goal, "Ship it");
        assert_eq!(read_text(&file).as_deref(), Some("# Goal\nShip it\n\n## Next\n- go\n"));
        std::fs::remove_file(report_file(&file)).unwrap();
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

    #[test]
    fn a_markdown_only_file_from_an_older_version_loads_as_written_and_maps_to_a_report() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let old = "# Goal\nShip it\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the test\n\n## Checklist\n### 1. Understand\n- [x] read -- done\n### 2. Build\n- [~] write\n### 3. PR\n- [ ] open\n### 4. Done\n- [ ] verify\n\n## Next\n- run it\n";
        std::fs::write(&file, old).unwrap();
        let ledger = load(&file).unwrap();
        assert_eq!(ledger.next.len(), 1, "an old ledger keeps what only the old format had");
        assert_eq!(ledger, Ledger::parse(old));
        let report = load_report(&file).unwrap();
        assert_eq!(report.now, "writing the test");
        let kinds: Vec<_> = report.stages.iter().map(|stage| stage.kind.word()).collect();
        assert_eq!(kinds, ["understand", "build", "ship", "verify"]);
    }

    #[test]
    fn the_typed_report_beside_the_markdown_wins_and_both_files_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        std::fs::write(&file, "# Goal\nold goal\n").unwrap();
        let report = ProgressReport::from_ledger(&Ledger::parse("# Goal\nnew goal\n"));
        save_report(&file, &report).unwrap();
        assert!(report_file(&file).exists());
        assert_eq!(load(&file).unwrap().goal, "new goal");
        assert_eq!(load_report(&file), Some(report));
        assert!(read_text(&file).unwrap().contains("new goal"));
    }

    #[test]
    fn the_told_marker_is_per_tab_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        assert!(!markdown_was_told(&file));
        mark_markdown_told(&file).unwrap();
        assert!(markdown_was_told(&file));
    }
}
