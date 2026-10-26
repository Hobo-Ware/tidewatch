//! The ledger guard: asks the agent to call `update_progress` when a turn ends with no ledger saved in it.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::guard_run::GuardEnv;
use crate::paths::progress_file_beside;

const TURN_FILE: &str = "ledger-turn";
const ASKED_FILE: &str = "ledger-asked";
const NOTES_FILE: &str = "notes.json";
const TAIL_BYTES: u64 = 512 * 1024;

pub const MESSAGE: &str = "LEDGER GUARD - the Tidewatch ledger is missing or was not updated this turn. Call `update_progress` now with the whole ledger, and put every dated or later item in Scheduled (`YYYY-MM-DD`, `~YYYY-MM-DD`, `on: <event>`, `parked: <until what>`, `you`). A ledger file of your own does not count.";

/// What the Stop decision needs to know, read once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub stop_hook_active: bool,
    /// `None` when the transcript could not tell.
    pub used_tools: Option<bool>,
    /// Unix milliseconds the tab's `progress.md` was last written; `None` when there is none.
    pub ledger_written_ms: Option<u64>,
    /// Unix milliseconds the turn began, when it was noted.
    pub turn_started_ms: Option<u64>,
    /// The turn the agent was already asked about.
    pub asked_turn_ms: Option<u64>,
}

impl Facts {
    fn turn_key(&self) -> u64 {
        self.turn_started_ms.unwrap_or(0)
    }
}

/// Whether to send the agent back to the ledger.
#[must_use]
pub fn should_ask(facts: &Facts) -> bool {
    if facts.stop_hook_active
        || facts.used_tools == Some(false)
        || facts.asked_turn_ms == Some(facts.turn_key())
    {
        return false;
    }
    match (facts.ledger_written_ms, facts.turn_started_ms) {
        (None, _) => true,
        (Some(written), Some(started)) => written < started,
        (Some(_), None) => false,
    }
}

/// Whether the last turn of a Claude Code transcript (JSON lines) called a tool. Reads backwards to
/// the last line the user typed; `None` when no such line is in `tail`.
#[must_use]
pub fn used_tools(tail: &str) -> Option<bool> {
    let mut tools = false;
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let blocks = entry.pointer("/message/content").and_then(Value::as_array);
        let has = |kind: &str| {
            blocks.is_some_and(|blocks| {
                blocks.iter().any(|block| block.get("type").and_then(Value::as_str) == Some(kind))
            })
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("assistant") => tools |= has("tool_use"),
            Some("user")
                if !has("tool_result")
                    && entry.get("isMeta").and_then(Value::as_bool) != Some(true) =>
            {
                return Some(tools);
            }
            _ => {}
        }
    }
    None
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn read_ms(file: &Path) -> Option<u64> {
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

fn modified_ms(file: &Path) -> Option<u64> {
    let since = file.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_millis() as u64)
}

fn tail_of(file: &Path) -> Option<String> {
    let mut open = std::fs::File::open(file).ok()?;
    let len = open.metadata().ok()?.len();
    open.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).ok()?;
    let mut bytes = Vec::new();
    open.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn ledger_file(tab: &Path) -> PathBuf {
    progress_file_beside(&tab.join(NOTES_FILE))
}

/// Notes that a turn began now.
pub fn note_turn(tab: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(tab)?;
    std::fs::write(tab.join(TURN_FILE), now_ms().to_string())
}

fn facts_of(stdin: &str, tab: &Path) -> Option<Facts> {
    let payload: Value = serde_json::from_str(stdin).ok()?;
    let transcript = payload.get("transcript_path").and_then(Value::as_str);
    Some(Facts {
        stop_hook_active: payload.get("stop_hook_active").and_then(Value::as_bool) == Some(true),
        used_tools: transcript.and_then(|path| used_tools(&tail_of(Path::new(path))?)),
        ledger_written_ms: modified_ms(&ledger_file(tab)),
        turn_started_ms: read_ms(&tab.join(TURN_FILE)),
        asked_turn_ms: read_ms(&tab.join(ASKED_FILE)),
    })
}

/// Runs one hook. `stop` is the end of a turn, otherwise its start. Returns the exit code; the
/// request, if any, is printed to stderr.
#[must_use]
pub fn run(stop: bool, stdin: &str, env: &GuardEnv) -> i32 {
    let Some(tab) = env.tab.as_deref() else { return 0 };
    if !stop {
        let _ = note_turn(tab);
        return 0;
    }
    let Some(facts) = facts_of(stdin, tab) else { return 0 };
    if !should_ask(&facts) {
        return 0;
    }
    let _ = std::fs::write(tab.join(ASKED_FILE), facts.turn_key().to_string());
    eprintln!("{MESSAGE}");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(change: impl FnOnce(&mut Facts)) -> Facts {
        let mut facts = Facts {
            stop_hook_active: false,
            used_tools: Some(true),
            ledger_written_ms: Some(100),
            turn_started_ms: Some(200),
            asked_turn_ms: None,
        };
        change(&mut facts);
        facts
    }

    impl Facts {
        fn with_no_turn_or_ledger(self) -> Self {
            Self { turn_started_ms: None, ledger_written_ms: None, ..self }
        }
    }

    #[test]
    fn the_agent_is_asked_when_the_ledger_is_missing_or_older_than_the_turn() {
        let cases = [
            (facts(|_| {}), true),
            (facts(|f| f.ledger_written_ms = None), true),
            (facts(|f| f.ledger_written_ms = Some(200)), false),
            (facts(|f| f.ledger_written_ms = Some(300)), false),
            (facts(|f| f.used_tools = None), true),
            (facts(|f| f.used_tools = Some(false)), false),
            (facts(|f| f.stop_hook_active = true), false),
            (facts(|f| f.asked_turn_ms = Some(200)), false),
            (facts(|f| f.asked_turn_ms = Some(150)), true),
            (facts(|f| f.turn_started_ms = None), false),
            (facts(|f| (f.turn_started_ms, f.ledger_written_ms) = (None, None)), true),
            (facts(|f| f.asked_turn_ms = Some(0)).with_no_turn_or_ledger(), false),
        ];
        for (facts, want) in cases {
            assert_eq!(should_ask(&facts), want, "{facts:?}");
        }
    }

    const USER: &str = r#"{"type":"user","message":{"role":"user","content":"do it"}}"#;
    const USER_BLOCKS: &str =
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do it"}]}}"#;
    const META: &str = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"x"}}"#;
    const CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Bash","input":{}}]}}"#;
    const RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"1","content":"ok"}]}}"#;
    const SAY: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;

    #[test]
    fn a_turn_used_tools_when_a_call_sits_after_the_last_line_the_user_typed() {
        let cases = [
            (vec![USER, SAY], Some(false)),
            (vec![USER_BLOCKS, SAY], Some(false)),
            (vec![USER, CALL, RESULT, SAY], Some(true)),
            (vec![CALL, RESULT, USER, SAY], Some(false)),
            (vec![USER, CALL, RESULT, META, SAY], Some(true)),
            (vec!["not json", USER, CALL, "{}", SAY], Some(true)),
            (vec![CALL, RESULT, SAY], None),
            (vec![], None),
        ];
        for (lines, want) in cases {
            assert_eq!(used_tools(&lines.join("\n")), want, "{lines:?}");
        }
    }

    #[test]
    fn the_message_is_one_ascii_line_that_names_the_tool_and_every_when() {
        assert!(MESSAGE.is_ascii() && !MESSAGE.contains('\n'));
        for needle in [
            "update_progress",
            "Scheduled",
            "YYYY-MM-DD",
            "~YYYY-MM-DD",
            "on: <event>",
            "parked:",
            "you",
        ] {
            assert!(MESSAGE.contains(needle), "{needle}");
        }
    }
}
