//! What the comment guard tells a tab, kept in the tab's state folder: the flags it sent
//! (`comment-flags.jsonl`, one JSON line per flagged comment), the digest of the last branch sweep it
//! reported (so the same report is never sent twice) and the per-tab off switch
//! (`comment-guard-off`). The hook process appends, the app reads.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FLAGS_FILE: &str = "comment-flags.jsonl";
const SWEPT_FILE: &str = "comment-guard-swept";
const OFF_FILE: &str = "comment-guard-off";
const KEEP_RECORDS: usize = 200;
const TRIM_AT_BYTES: u64 = 96 * 1024;

/// Where a flag was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// A Write, Edit or MultiEdit the agent just made.
    Edit,
    /// The branch diff at the end of a turn.
    Sweep,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub at_ms: u64,
    pub origin: Origin,
    pub file: String,
    pub line: usize,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabFiles {
    dir: PathBuf,
}

impl TabFiles {
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf() }
    }

    #[must_use]
    pub fn flags_file(&self) -> PathBuf {
        self.dir.join(FLAGS_FILE)
    }

    #[must_use]
    pub fn is_off(&self) -> bool {
        self.dir.join(OFF_FILE).exists()
    }

    pub fn set_off(&self, off: bool) -> std::io::Result<()> {
        let file = self.dir.join(OFF_FILE);
        if off {
            std::fs::create_dir_all(&self.dir)?;
            std::fs::write(file, "")
        } else {
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        }
    }

    /// Adds the records as whole lines in one write. The file is cut back to its newest records once
    /// it passes 96 KB.
    pub fn append(&self, records: &[Record]) -> std::io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let file = self.flags_file();
        let mut text = String::new();
        for record in records {
            text.push_str(&serde_json::to_string(record).map_err(std::io::Error::other)?);
            text.push('\n');
        }
        let mut open = std::fs::OpenOptions::new().create(true).append(true).open(&file)?;
        open.write_all(text.as_bytes())?;
        if open.metadata().is_ok_and(|meta| meta.len() > TRIM_AT_BYTES) {
            let newest = Self::read(&file);
            let kept = &newest[newest.len().saturating_sub(KEEP_RECORDS)..];
            let mut cut = String::new();
            for record in kept {
                cut.push_str(&serde_json::to_string(record).map_err(std::io::Error::other)?);
                cut.push('\n');
            }
            crate::fsutil::write_atomic(&file, &cut)?;
        }
        Ok(())
    }

    fn read(file: &Path) -> Vec<Record> {
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// The newest 200 records, oldest first. A missing file or a bad line is skipped.
    #[must_use]
    pub fn load(&self) -> Vec<Record> {
        let all = Self::read(&self.flags_file());
        all[all.len().saturating_sub(KEEP_RECORDS)..].to_vec()
    }

    /// The digest of the last sweep report that was sent, if the report is still the same.
    #[must_use]
    pub fn swept(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join(SWEPT_FILE)).ok().map(|text| text.trim().to_owned())
    }

    pub fn set_swept(&self, digest: Option<&str>) -> std::io::Result<()> {
        let file = self.dir.join(SWEPT_FILE);
        match digest {
            Some(digest) => {
                std::fs::create_dir_all(&self.dir)?;
                std::fs::write(file, digest)
            }
            None => match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests;
