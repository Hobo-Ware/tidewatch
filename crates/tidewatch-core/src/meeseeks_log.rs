//! `.state/meeseeks.toml`: every Meeseeks that poofed, newest last, for the Overview's Poofed today
//! and for the summary tab it leaves behind.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::civil_date::Date;
use crate::fsutil::write_atomic;
use crate::kind::Kind;
use crate::meeseeks::CheckLine;

const MAX_ENTRIES: usize = 500;

static APPENDING: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Poofed {
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub duration_secs: u64,
    pub date: Date,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    /// The repo's main checkout, which holds the branch that outlives the worktree.
    #[serde(default, skip_serializing_if = "path_is_empty")]
    pub repo: PathBuf,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default)]
    pub ended_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckLine>,
    /// The Done and Next lines of the agent's last turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    /// The stages of the ledger the agent kept, as it left them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<StageLine>,
    /// An old record has none and reads as an edit.
    #[serde(default, skip_serializing_if = "is_edit")]
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StageLine {
    pub name: String,
    pub done: u32,
    pub total: u32,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde passes a reference")]
fn is_edit(kind: &Kind) -> bool {
    *kind == Kind::Edit
}

fn path_is_empty(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

/// `PR #213` for a pull request address that ends in its number.
#[must_use]
pub fn pr_label_of(url: &str) -> Option<String> {
    let number = url.rsplit('/').next()?;
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("PR #{number}"))
}

impl Poofed {
    /// The five facts the Overview line needs; the rest of the summary is left empty.
    #[must_use]
    pub fn bare(
        name: &str,
        summary: &str,
        pr_url: Option<&str>,
        duration_secs: u64,
        date: Date,
    ) -> Self {
        Self {
            name: name.to_owned(),
            summary: summary.to_owned(),
            pr_url: pr_url.map(str::to_owned),
            duration_secs,
            date,
            task: String::new(),
            repo: PathBuf::new(),
            branch: String::new(),
            ended_ms: 0,
            checks: Vec::new(),
            done: Vec::new(),
            next: Vec::new(),
            stages: Vec::new(),
            kind: Kind::Edit,
        }
    }

    #[must_use]
    pub fn pr_label(&self) -> Option<String> {
        pr_label_of(self.pr_url.as_deref()?)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    #[serde(default, rename = "poofed")]
    pub entries: Vec<Poofed>,
}

impl Log {
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); starting empty", file.display());
            Self::default()
        })
    }

    #[must_use]
    pub fn on(&self, day: Date) -> Vec<&Poofed> {
        self.entries.iter().rev().filter(|entry| entry.date == day).collect()
    }
}

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
        Poofed::bare(name, &format!("did {name}"), url, 360, date)
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
    fn a_file_from_before_the_summary_tab_still_loads_with_the_new_facts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("meeseeks.toml");
        std::fs::write(
            &file,
            "[[poofed]]\nname = \"old\"\nsummary = \"did old\"\nduration_secs = 60\ndate = \"2026-10-20\"\n",
        )
        .unwrap();
        assert_eq!(Log::load(&file).entries, [Poofed::bare("old", "did old", None, 60, day(20))]);
    }

    #[test]
    fn the_whole_summary_survives_the_file() {
        use crate::meeseeks::{Step, Verdict};
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("meeseeks.toml");
        let mut full = entry("promo-test", day(20), Some("https://github.com/acme/web/pull/213"));
        full.task = "Add the promo test".into();
        full.repo = PathBuf::from("/code/web");
        full.branch = "agent/promo-test".into();
        full.ended_ms = 1_790_000_000_000;
        full.checks = vec![
            CheckLine { step: Step::Change, verdict: Verdict::Passed, detail: "2 commits".into() },
            CheckLine { step: Step::HandOff, verdict: Verdict::Skipped, detail: String::new() },
        ];
        full.done = vec!["Added `promo.test.ts`".into()];
        full.next = vec!["Review it".into()];
        full.stages = vec![StageLine { name: "Build".into(), done: 3, total: 3 }];
        append(&file, entry("older", day(20), None)).unwrap();
        append(&file, full.clone()).unwrap();
        assert_eq!(Log::load(&file).entries[1], full);
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
