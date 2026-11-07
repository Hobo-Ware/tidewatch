//! A tab's report on disk: `<tab state>/progress.json` holds the typed report. A tab that saved
//! only the markdown `progress.md` of an older Tidewatch has no report.

use std::path::Path;

use crate::fsutil::write_atomic;
use crate::progress::Ledger;
use crate::progress_report::ProgressReport;

/// The saved report, `None` when there is none or the file is not one.
#[must_use]
pub fn load_report(file: &Path) -> Option<ProgressReport> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(&text).ok()
}

/// The saved report as the screens read it.
#[must_use]
pub fn load(file: &Path) -> Option<Ledger> {
    load_report(file).map(|report| report.to_ledger())
}

pub fn save_report(file: &Path, report: &ProgressReport) -> std::io::Result<()> {
    write_atomic(file, &report.to_json())
}

/// Saves a ledger built in code: the report it maps to is the one stored.
pub fn save(file: &Path, ledger: &Ledger) -> std::io::Result<()> {
    save_report(file, &ProgressReport::from_ledger(ledger))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_report_loads_back_and_a_missing_or_empty_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("progress.json");
        assert_eq!(load(&file), None);
        assert_eq!(load_report(&file), None);
        save(&file, &Ledger::parse("# Goal\nShip it\n\n## Next\n- go\n")).unwrap();
        assert_eq!(load(&file).unwrap().goal, "Ship it");
        std::fs::write(&file, "  \n").unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn a_markdown_ledger_from_an_older_version_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.json");
        std::fs::write(&file, "# Goal\nShip it\n\n## Status\nstate: working\n").unwrap();
        assert_eq!(load(&file), None);
        assert_eq!(load_report(&file), None);
        let old = dir.path().join("progress.md");
        std::fs::write(&old, "# Goal\nShip it\n").unwrap();
        assert_eq!(load_report(&old), None);
    }

    #[test]
    fn saving_a_report_replaces_the_one_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.json");
        save(&file, &Ledger::parse("# Goal\nold goal\n")).unwrap();
        let report = ProgressReport::from_ledger(&Ledger::parse("# Goal\nnew goal\n"));
        save_report(&file, &report).unwrap();
        assert_eq!(load(&file).unwrap().goal, "new goal");
        assert_eq!(load_report(&file), Some(report));
    }
}
