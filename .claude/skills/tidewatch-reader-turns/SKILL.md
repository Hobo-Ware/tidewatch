---
name: tidewatch-reader-turns
description: Use when a Reader answer hides under an "N updates" chip.
category: native-app
---

The Reader shows one answer per turn. It starts after the last call that did real work, pulled back to the newest text with the Done / Needs you / Next block before that point (0.1.67; the 0.1.66 tag never got a build). Texts before it fold under the "N updates" chip. 0.1.65 took the newest end-block text outright, so a full answer, a report call, then a short reply with its own block folded the full answer (owner screenshot, 2026-11-04). Test both shapes: answer, edit, short reply; and answer, report, short reply with a block.

- Grouping lives in `crates/tidewatch/src/reader_turns.rs`. Before 0.1.65 the answer began after the last call that did real work, so a full answer followed by a ledger edit and a short reply folded away.
- Test a grouping change with that shape: full answer, one work call, short reply. The full answer must be the shown text.
- Confirm the change in the built app with a Reader screenshot of that turn. The switches are in `docs/dev-switches.md`.
- The agent side (write the answer in the closing message, after the last tool call) is in `tidewatch-tools`.
