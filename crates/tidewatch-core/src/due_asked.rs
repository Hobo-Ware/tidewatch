//! The due items a tab was already asked to start today, kept in
//! `.state/<repo-dir>/<tab>/due-asked.json` so a restart does not ask again.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::civil_date::Date;
use crate::fsutil::save_json_pretty;

#[derive(Serialize, Deserialize)]
struct Saved {
    day: Date,
    #[serde(default)]
    asked: Vec<String>,
}

/// The items asked on `today`. A file from an earlier day, a missing one and an unreadable one all
/// read as none.
#[must_use]
pub fn load(file: &Path, today: Date) -> Vec<String> {
    let Ok(bytes) = std::fs::read(file) else { return Vec::new() };
    match serde_json::from_slice::<Saved>(&bytes) {
        Ok(saved) if saved.day == today => saved.asked,
        _ => Vec::new(),
    }
}

pub fn save(file: &Path, today: Date, asked: &[String]) -> std::io::Result<()> {
    let saved = Saved { day: today, asked: asked.to_vec() };
    save_json_pretty(file, &saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u8) -> Date {
        Date::new(2026, 10, d).unwrap()
    }

    #[test]
    fn what_was_asked_today_is_read_back_and_an_old_day_or_a_bad_file_reads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("due-asked.json");
        assert_eq!(load(&file, day(26)), Vec::<String>::new());
        let asked = vec!["Switch off the old webhook".to_owned()];
        save(&file, day(26), &asked).unwrap();
        assert_eq!(load(&file, day(26)), asked);
        assert_eq!(load(&file, day(27)), Vec::<String>::new());
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(load(&file, day(26)), Vec::<String>::new());
    }
}
