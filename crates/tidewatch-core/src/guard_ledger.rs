//! The ledger guard: asks the agent to call `update_progress` when a turn ends with no ledger saved
//! in it, or with a ledger saved in it that still says `state: working`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::guard_run::GuardEnv;
use crate::paths::progress_file_beside;
use crate::progress::{Ledger, State};

const TURN_FILE: &str = "ledger-turn";
const ASKED_FILE: &str = "ledger-asked";
const NOTES_FILE: &str = "notes.json";
const TAIL_BYTES: u64 = 512 * 1024;

pub const MESSAGE: &str = "LEDGER GUARD - the Tidewatch ledger is missing or was not updated this turn. Call `update_progress` now with the whole ledger, and put every dated or later item in Scheduled (`YYYY-MM-DD`, `~YYYY-MM-DD`, `on: <event>`, `parked: <until what>`, `you`). A ledger file of your own does not count.";

pub const FINAL_STATE_MESSAGE: &str = "LEDGER GUARD - the turn is ending but your Tidewatch ledger still says `state: working`. Call `update_progress` once more with the final state: `waiting` (for the user or an event), `blocked` or `done`, and move every Next item that is not happening now into Scheduled.";

/// What the guard asks of the agent at the end of a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Ledger,
    FinalState,
}

impl Ask {
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Ledger => MESSAGE,
            Self::FinalState => FINAL_STATE_MESSAGE,
        }
    }
}

/// What the Stop decision needs to know, read once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub stop_hook_active: bool,
    /// `None` when the transcript could not tell.
    pub used_tools: Option<bool>,
    /// Unix milliseconds the tab's `progress.md` was last written; `None` when there is none.
    pub ledger_written_ms: Option<u64>,
    /// The saved ledger's `state:` is `working`.
    pub ledger_working: bool,
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

/// What to send the agent back to the ledger for, if anything.
#[must_use]
pub fn should_ask(facts: &Facts) -> Option<Ask> {
    if facts.stop_hook_active
        || facts.used_tools == Some(false)
        || facts.asked_turn_ms == Some(facts.turn_key())
    {
        return None;
    }
    match (facts.ledger_written_ms, facts.turn_started_ms) {
        (None, _) => Some(Ask::Ledger),
        (Some(written), Some(started)) if written < started => Some(Ask::Ledger),
        (Some(_), Some(_)) => facts.ledger_working.then_some(Ask::FinalState),
        (Some(_), None) => None,
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

fn ledger_says_working(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .is_ok_and(|text| Ledger::parse(&text).status.state == Some(State::Working))
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
        ledger_working: ledger_says_working(&ledger_file(tab)),
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
    let Some(ask) = should_ask(&facts) else { return 0 };
    let _ = std::fs::write(tab.join(ASKED_FILE), facts.turn_key().to_string());
    eprintln!("{}", ask.message());
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
            ledger_working: false,
            turn_started_ms: Some(200),
            asked_turn_ms: None,
        };
        change(&mut facts);
        facts
    }

    fn working(change: impl FnOnce(&mut Facts)) -> Facts {
        facts(|f| {
            f.ledger_written_ms = Some(300);
            f.ledger_working = true;
            change(f);
        })
    }

    impl Facts {
        fn with_no_turn_or_ledger(self) -> Self {
            Self { turn_started_ms: None, ledger_written_ms: None, ..self }
        }
    }

    #[test]
    fn the_agent_is_asked_when_the_ledger_is_missing_older_than_the_turn_or_still_working() {
        use Ask::{FinalState, Ledger};
        let cases = [
            (facts(|_| {}), Some(Ledger)),
            (facts(|f| f.ledger_written_ms = None), Some(Ledger)),
            (facts(|f| f.ledger_written_ms = Some(200)), None),
            (facts(|f| f.ledger_written_ms = Some(300)), None),
            (facts(|f| f.used_tools = None), Some(Ledger)),
            (facts(|f| f.used_tools = Some(false)), None),
            (facts(|f| f.stop_hook_active = true), None),
            (facts(|f| f.asked_turn_ms = Some(200)), None),
            (facts(|f| f.asked_turn_ms = Some(150)), Some(Ledger)),
            (facts(|f| f.turn_started_ms = None), None),
            (facts(|f| (f.turn_started_ms, f.ledger_written_ms) = (None, None)), Some(Ledger)),
            (facts(|f| f.asked_turn_ms = Some(0)).with_no_turn_or_ledger(), None),
            (
                facts(|f| (f.ledger_written_ms, f.ledger_working) = (Some(300), true)),
                Some(FinalState),
            ),
            (
                facts(|f| (f.ledger_written_ms, f.ledger_working) = (Some(200), true)),
                Some(FinalState),
            ),
            (facts(|f| f.ledger_working = true), Some(Ledger)),
            (facts(|f| (f.ledger_written_ms, f.ledger_working) = (Some(300), false)), None),
            (working(|f| f.stop_hook_active = true), None),
            (working(|f| f.asked_turn_ms = Some(200)), None),
            (working(|f| f.used_tools = Some(false)), None),
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
    fn the_final_state_ask_names_the_tool_and_the_three_states() {
        assert!(FINAL_STATE_MESSAGE.is_ascii() && !FINAL_STATE_MESSAGE.contains('\n'));
        for needle in
            ["update_progress", "state: working", "waiting", "blocked", "done", "Scheduled"]
        {
            assert!(FINAL_STATE_MESSAGE.contains(needle), "{needle}");
        }
        assert_eq!(Ask::FinalState.message(), FINAL_STATE_MESSAGE);
    }

    #[test]
    fn the_ledger_file_decides_whether_it_still_says_working() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("progress.md");
        let cases = [
            ("# Goal\ng\n\n## Status\nstate: working\n", true),
            ("# Goal\ng\n\n## Status\nstate: waiting\n", false),
            ("# Goal\ng\n\n## Status\nstate: done\n", false),
            ("# Goal\ng\n", false),
        ];
        for (text, want) in cases {
            std::fs::write(&file, text).unwrap();
            assert_eq!(ledger_says_working(&file), want, "{text:?}");
        }
        assert!(!ledger_says_working(&dir.path().join("none.md")));
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
