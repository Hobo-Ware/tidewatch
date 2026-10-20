//! `.state/meeseeks.toml`: every Meeseeks that poofed, newest last, for the Overview's Poofed today.

use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::civil_date::Date;
use crate::fsutil::write_atomic;

const MAX_ENTRIES: usize = 500;

static APPENDING: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poofed {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub duration_secs: u64,
    pub date: Date,
}

impl Poofed {
    /// `PR #214` from a pull request address, else nothing.
    #[must_use]
    pub fn pr_label(&self) -> Option<String> {
        let number = self.pr_url.as_deref()?.rsplit('/').next()?;
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("PR #{number}"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    #[serde(default, rename = "poofed")]
    pub entries: Vec<Poofed>,
}

impl Log {
    /// A missing or unreadable file is an empty log.
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); starting empty", file.display());
            Self::default()
        })
    }

    /// The entries of `day`, the latest first.
    #[must_use]
    pub fn on(&self, day: Date) -> Vec<&Poofed> {
        self.entries.iter().rev().filter(|entry| entry.date == day).collect()
    }
}

/// Adds one entry to the file, dropping the oldest past 500. Safe to call from several threads.
pub fn append(file: &Path, entry: Poofed) -> std::io::Result<()> {
    let _one_at_a_time = APPENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut log = Log::load(file);
    log.entries.push(entry);
    let extra = log.entries.len().saturating_sub(MAX_ENTRIES);
    log.entries.drain(..extra);
    let text = toml::to_string(&log).map_err(std::io::Error::other)?;
    write_atomic(file, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u8) -> Date {
        Date::new(2026, 10, d).unwrap()
    }

    fn entry(name: &str, date: Date, url: Option<&str>) -> Poofed {
        Poofed {
            name: name.into(),
            summary: format!("did {name}"),
            pr_url: url.map(str::to_owned),
            duration_secs: 360,
            date,
        }
    }

    #[test]
    fn an_entry_keeps_its_five_facts_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state").join("meeseeks.toml");
        let first = entry("promo-test", day(20), Some("https://github.com/acme/web/pull/213"));
        append(&file, first.clone()).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        for part in ["promo-test", "did promo-test", "pull/213", "360", "2026-10-20"] {
            assert!(text.contains(part), "{part} in {text}");
        }
        assert_eq!(Log::load(&file).entries, [first]);
    }

    #[test]
    fn today_lists_the_latest_first_and_leaves_other_days_out() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("meeseeks.toml");
        for (name, date) in [("old", day(19)), ("a", day(20)), ("b", day(20))] {
            append(&file, entry(name, date, None)).unwrap();
        }
        let log = Log::load(&file);
        let names: Vec<&str> = log.on(day(20)).iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(log.on(day(21)), Vec::<&Poofed>::new());
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_log_and_the_next_poof_starts_a_clean_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("meeseeks.toml");
        assert_eq!(Log::load(&file), Log::default());
        std::fs::write(&file, "[[poofed]]\nname = 3").unwrap();
        assert_eq!(Log::load(&file), Log::default());
        append(&file, entry("a", day(20), None)).unwrap();
        assert_eq!(Log::load(&file).entries.len(), 1);
    }

    #[test]
    fn the_log_keeps_the_newest_five_hundred() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("meeseeks.toml");
        let log = Log {
            entries: (0..MAX_ENTRIES).map(|n| entry(&n.to_string(), day(20), None)).collect(),
        };
        write_atomic(&file, &toml::to_string(&log).unwrap()).unwrap();
        append(&file, entry("last", day(20), None)).unwrap();
        let loaded = Log::load(&file);
        assert_eq!(loaded.entries.len(), MAX_ENTRIES);
        assert_eq!(loaded.entries[0].name, "1");
        assert_eq!(loaded.entries.last().unwrap().name, "last");
    }

    #[test]
    fn a_pull_request_address_names_its_number_and_anything_else_names_nothing() {
        let cases = [
            (Some("https://github.com/acme/web/pull/214"), Some("PR #214")),
            (Some("https://github.com/acme/web/pull/"), None),
            (Some("https://example.test/not-a-number"), None),
            (None, None),
        ];
        for (url, want) in cases {
            assert_eq!(entry("x", day(20), url).pr_label().as_deref(), want, "{url:?}");
        }
    }

    #[test]
    fn several_threads_appending_lose_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = std::sync::Arc::new(dir.path().join("meeseeks.toml"));
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let file = file.clone();
                std::thread::spawn(move || {
                    append(&file, entry(&n.to_string(), day(20), None)).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(Log::load(&file).entries.len(), 8);
    }
}
