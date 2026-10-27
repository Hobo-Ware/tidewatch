//! The permission level a Claude Code session was last running at, read from its transcript. The
//! CLI records the live mode as `permissionMode`, on a `permission-mode` record each time it
//! changes and on every user record, so Shift+Tab inside the agent shows up here even though
//! Tidewatch never sees the keystroke. Mode names checked against claude 2.1.310 transcripts.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::agents::Level;

const CHUNK_BYTES: u64 = 64 * 1024;
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// The level Tidewatch runs a Claude `permissionMode` at. `None` for a mode it has no level for.
#[must_use]
pub fn claude_level(mode: &str) -> Option<Level> {
    match mode {
        "bypassPermissions" => Some(Level::Yolo),
        "auto" => Some(Level::Auto),
        "acceptEdits" => Some(Level::Edits),
        "default" => Some(Level::Ask),
        "plan" | "dontAsk" => Some(Level::Plan),
        _ => None,
    }
}

/// The mode the newest record of the file names, looking back from the end through at most 8 MiB.
#[must_use]
pub fn last_claude_mode(file: &Path) -> Option<String> {
    let mut file = File::open(file).ok()?;
    let len = file.metadata().ok()?.len();
    let floor = len.saturating_sub(MAX_SCAN_BYTES);
    let mut end = len;
    let mut carry: Vec<u8> = Vec::new();
    while end > floor {
        let start = end.saturating_sub(CHUNK_BYTES).max(floor);
        let mut buf = vec![0; usize::try_from(end - start).ok()?];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buf).ok()?;
        buf.extend_from_slice(&carry);
        let whole_lines = if start == 0 {
            carry.clear();
            &buf[..]
        } else {
            let cut = buf.iter().position(|byte| *byte == b'\n').map_or(buf.len(), |at| at + 1);
            carry = buf[..cut].to_vec();
            &buf[cut.min(buf.len())..]
        };
        if let Some(mode) = whole_lines.rsplit(|byte| *byte == b'\n').find_map(mode_of_line) {
            return Some(mode);
        }
        end = start;
    }
    None
}

/// The level the session was last running at, when its transcript says.
#[must_use]
pub fn last_claude_level(file: &Path) -> Option<Level> {
    claude_level(&last_claude_mode(file)?)
}

fn mode_of_line(line: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.contains("\"permissionMode\"") {
        return None;
    }
    let record: Value = serde_json::from_str(text).ok()?;
    if record["isSidechain"].as_bool() == Some(true) {
        return None;
    }
    record["permissionMode"].as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: &str = include_str!("../tests/fixtures/claude_modes.jsonl");

    fn file_with(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn every_claude_mode_maps_to_the_level_that_launches_it() {
        let cases = [
            ("bypassPermissions", Some(Level::Yolo)),
            ("auto", Some(Level::Auto)),
            ("acceptEdits", Some(Level::Edits)),
            ("default", Some(Level::Ask)),
            ("plan", Some(Level::Plan)),
            ("dontAsk", Some(Level::Plan)),
            ("somethingNew", None),
            ("", None),
        ];
        for (mode, want) in cases {
            assert_eq!(claude_level(mode), want, "{mode:?}");
        }
    }

    #[test]
    fn the_last_record_naming_a_mode_wins_and_prose_and_subagents_do_not_count() {
        let (_dir, path) = file_with(MODES);
        assert_eq!(last_claude_mode(&path).as_deref(), Some("auto"));
        assert_eq!(last_claude_level(&path), Some(Level::Auto));
    }

    #[test]
    fn a_user_record_after_a_mode_change_names_the_mode_it_ran_at() {
        let text = format!(
            "{MODES}{}\n",
            r#"{"type":"user","isSidechain":false,"permissionMode":"default","message":{"role":"user","content":"later"}}"#
        );
        let (_dir, path) = file_with(&text);
        assert_eq!(last_claude_level(&path), Some(Level::Ask));
    }

    #[test]
    fn a_file_with_no_mode_or_no_file_gives_nothing() {
        let (_dir, path) = file_with("{\"type\":\"mode\",\"mode\":\"normal\"}\nnot json\n");
        assert_eq!(last_claude_mode(&path), None);
        assert_eq!(last_claude_mode(&path.with_file_name("missing.jsonl")), None);
        let (_dir, empty) = file_with("");
        assert_eq!(last_claude_mode(&empty), None);
    }

    #[test]
    fn a_mode_far_back_behind_long_lines_is_still_found() {
        let filler = format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":\"{}\"}}}}\n",
            "x".repeat(150_000)
        );
        let text = format!(
            "{}\n{}{filler}{filler}",
            r#"{"type":"permission-mode","permissionMode":"acceptEdits"}"#, filler
        );
        let (_dir, path) = file_with(&text);
        assert_eq!(last_claude_level(&path), Some(Level::Edits));
    }

    #[test]
    fn a_mode_beyond_the_scan_limit_is_not_read() {
        let filler = format!("{{\"type\":\"assistant\",\"pad\":\"{}\"}}\n", "x".repeat(1_000_000));
        let mut text = String::from("{\"type\":\"permission-mode\",\"permissionMode\":\"auto\"}\n");
        for _ in 0..9 {
            text.push_str(&filler);
        }
        let (_dir, path) = file_with(&text);
        assert_eq!(last_claude_mode(&path), None);
    }

    #[test]
    fn the_scan_agrees_with_a_whole_file_read_at_every_chunk_alignment() {
        for pad in [0, 1, 7, 65_535, 65_536, 65_537, 131_071] {
            let text = format!(
                "{}\n{}\n{}\n",
                r#"{"type":"permission-mode","permissionMode":"plan"}"#,
                "y".repeat(pad),
                r#"{"type":"permission-mode","permissionMode":"auto"}"#
            );
            let (_dir, path) = file_with(&text);
            assert_eq!(last_claude_mode(&path).as_deref(), Some("auto"), "pad {pad}");
            let text = format!(
                "{}\n{}\n",
                r#"{"type":"permission-mode","permissionMode":"plan"}"#,
                "y".repeat(pad)
            );
            let (_dir, path) = file_with(&text);
            assert_eq!(last_claude_mode(&path).as_deref(), Some("plan"), "pad {pad}");
        }
    }
}
