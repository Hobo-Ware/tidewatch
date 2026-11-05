---
name: tidewatch-question-menu
description: Use when a Claude question card or Allow once misfires.
category: native-app
---

Tidewatch types an answer into Claude's question menu only while that question is on screen. Look for the menu again right before the first key. Never close a Claude question's card before its menu has been seen, and never type into a question that was already answered.

- The card's gone timer (2 s) starts only after the menu was seen on screen once: `MenuWatch` in `crates/tidewatch/src/reader.rs`, `close_gone_menu` in `reader_glue.rs`. Claude draws its menu only when the tool runs, so a slow Bash call before it can hold the menu back 40 s or more, while the transcript names the question at once.
- A permission prompt behind a Claude question is that question, never Allow once or Deny (`Rows::claude_asks_last`). Allow once types `1` into Claude's menu and picks an option.
- A digit sent after the menu has closed stays in Claude's empty input box and goes out with the next message.
- Claude sends a `PermissionRequest` for `AskUserQuestion` even in bypass (YOLO) mode. Release that held permission when the tool finishes, matched by `tool_use_id`. Matching by tool name and input fails, because `PostToolUse` adds `answers` and `annotations` to the input.
- Keys, and Esc followed by a message, go only to a question that still waits. A second answer to the same question row is refused.
- Test a close-timing fix on an `Origin::Claude` pane. Break each fix on purpose and confirm its test fails before the commit.
- Check a key-path change in a live `claude` pty: capture the screen bytes and read the input box.
- To find a stray digit, grep the owner's transcripts for a user line that starts with a digit and then a letter (`1Yes`). Read the menu answer just before it in the tab's `panel-log.jsonl` under `~/.tidewatch/.state/`.
- Status: the stray-digit fix shipped in 0.1.55 (PR 322) and is not yet checked in a live tab. The close-timing fix is PR #338 (2026-11-02). Check it is on main with `grep -n MenuWatch crates/tidewatch/src/reader.rs`.
