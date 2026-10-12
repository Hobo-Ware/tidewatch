# Tidewatch - ledger

A running log of what got done, newest first. The plan lives in [PLAN.md](PLAN.md).

## Status

- **Current milestone:** Phase 5, graph, diff and notes panes (branch `phase-5-panes`, local, not pushed, based on `main` after Phase 4). Decisions D1 to D13 confirmed by the owner.
- **Done:** concept, plan, repo, stdusk library crates (stdusk PR #20, merged), `kelp-ui` (Kelp, merged), the Phase 3 app skeleton, the Phase 4 worktree manager and worktree tabs, the Phase 5 panes, the Phase 9 core half (workflows, runner, test command; branch `phase-9-core`, app half separate).
- **Next:** review Phase 5, then Phase 6 (agents and profiles). Left from Phase 4: repo switching between several repos, a way to remove leftover worktrees. Left from Phase 5: see the entry below. Owner: `codex login` for the Codex live checks.

## 2026-10-13

- **Phase 9, core half: workflows run as a state machine, every gate has a test.** `tidewatch-core` gets `workflows.rs` (`Workflow`, `Stage`, `Action`, `Gate`; `~/.config/tidewatch/workflows.toml`, bad file -> built-in defaults with a stderr note, seeded on first launch with a missing agent swapped for an installed one), `runner.rs` (a pure `Runner`: `RunEvent` in, `Command` out), `prompt.rs` (stage prompt placeholders) and `test_command.rs` (`<repo>/.tidewatch.toml` `test = "..."`, else cargo, npm/pnpm/yarn/bun, deno, go, pytest, rspec, mix or make; `run_in` runs it for real). Built-in workflows from the concept: Ship a feature (Work -> Review -> Refactor -> Hand off), Fix a bug (Reproduce -> Fix -> Review), Review a PR (Review -> Review -> Hand off). Gates are checked by Tidewatch: Tests, TestPasses and SameBehavior need exit 0, TestFails needs a non-zero exit, NoBlocking counts the stage's blocking findings, Approve waits for the user. A failed gate with `on_fail` loops back while under `max_loops` (default 2), then asks (D11). A stop without `stage_done` asks pass, retry or stop (D6). The run saves to `.state/<repo-dir>/<tab>/stages.json` and `resume()` repeats the command it was waiting on. Numbers: 344 tests passing (90 app, 210 core unit, 13 history, 14 listener, 1 push guard, 16 worktree), up from 267; 77 new, all in core. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` all exit 0. The scripted Ship a feature run (fake test runner, first review reports a blocking finding) loops once, passes, reaches Hand off and waits; every step of it is saved, reloaded and resumed in the test. Not verified: no real agent was run through a workflow (the app half wires the runner to tabs, the listener and the test command), and `SameBehavior` is "tests pass" only (the prompt asks the agent to keep behavior). Open: a stale `Stop` hook from the previous stage's agent must be dropped by the app once the next stage started; a hung test command has no timeout; workflows list alphabetically by id (BTreeMap), so Ship a feature is last.
- **Phase 5: the bottom half of layout B works.** The History pane (bottom left) draws Kelp's commit graph for the active tab only: a worktree tab shows its branch and the commits under it, the main tab its checked-out branch. The header names the tab, says how far it is ahead of or behind the default branch (`2 ahead · 1 behind main`, from `origin/HEAD`, else `main`, else `master`) and has a This worktree / Whole repo switch. A live "Uncommitted changes" row sits on top. The Diff pane (bottom right) shows the selected commit or the working tree: header, file list, Unified/Split, jump arrows, word highlights, all from `kelp-ui`. Threads written in the diff are notes to the tab's agent in `~/.tidewatch/.state/<repo-dir>/<tab>/notes.json` (atomic write on a background thread, flushed on quit); nothing is sent to the agent yet (Phase 8). Double-clicking a branch chip checks it out only in the main tab and only when the tree is clean; a worktree tab shows a toast instead. A non-git folder (Other) now has no panes and a full-height terminal. Refresh is event-driven: Kelp's file watcher runs only for the visible tab, and history, status and diff reload on background threads, so an agent's commit shows up without input. Numbers: 159 tests passing (90 app, 40 core unit, 13 history round trips, 16 worktree round trips), up from 83. Runnable check on a throwaway repo and temp home (git 2.55.0): a worktree tab opened by the app, two commits and an uncommitted edit made behind its back appeared in the graph and the Diff pane; a note written on the main tab was in `notes.json` and showed again, on the same line, after a restart; idle CPU time stayed at 0:00.42 over 10 s (0.0%), RSS 182 MB in a debug build. On Kelp's own repo (242 commits, Whole repo) the panes filled within the 6 s capture wait. Not verified: the checkout toast and a real double-click (the screenshot harness cannot click; the rule and the git outcomes have tests). Open: Kelp's thread card is a fixed 640 pt wide and clips in a narrow pane, and says "Saved in this repo only" (Kelp wording); `kelp_ui::fonts::ensure_fallback` replaces the whole font set, so Tidewatch triggers it once at startup (non-Latin glyphs then rely on stdusk's fallbacks); commit summaries are read lazily for visible rows on the UI thread (cheap object reads, no git process); Alt+Up/Down also reaches the shell; Tidewatch tabs are not restored, so notes of a closed tab stay on disk but are not shown again; no avatars (offline store); no Overview, focus mode or send-to-agent yet.

## 2026-10-13

- **Phase 7, core half: agents can now report to Tidewatch, and their status folds into one value.** `tidewatch-core` gets a loopback listener (`listener.rs`, `127.0.0.1:0`, started once by the app) with per-tab bearer tokens: `POST /hook/<agent>` takes hook payloads, `POST /mcp` is the Tidewatch MCP server (`mcp.rs`: `initialize`, `ping`, `tools/list`, `tools/call`, notifications answered 202, `GET` 405). A missing or unknown token is 401 on every path. Tools: `stage_done`, `report_finding` (events to the app), `list_notes`, `reply_to_note` (read and write the tab's `notes.json` through `notes.rs`, then an event). Bad arguments are JSON-RPC -32602, bad JSON -32700, and nothing panics on input. `status.rs` holds `AgentStatus`, per-CLI hook parsing and a pure `resolve` over three layers (hooks, terminal signals, process facts), with a small `StatusTracker` around it. HTTP is hand-rolled on `std::net` plus `httparse` (already in `Cargo.lock` through kelp): keep-alive, chunked bodies, `Expect: 100-continue`, 16 KiB head and 2 MiB body caps, 64 connections. `tiny_http` (0.12.0) was rejected: maintained forks advertise CVE fixes, so it is not the safe default, and the glue it would save is about the same size. The only new dependency is `httparse 1.10.1`. Tokens are 16 bytes from `/dev/urandom`.
  Status rules (my design, PLAN.md only says "decays"): a dead process wins (exit 0 Done, else Failed with the code); a hook state wins over terminal and process layers; `Working` with no output for `STATUS_QUIET_SECS` (8) becomes `NeedsYou(Quiet)`; a permission or question prompt flips back to `Working` once the terminal prints again 2 s after the hook (no hook reports the answer); an idle notice never turns `Done` back amber; with no hook state, progress, bell, title activity and quiet time decide.
  Verified live (claude 2.1.296 haiku, gemini 0.46.0, copilot 1.0.91, throwaway repos, `cargo run -p tidewatch-core --example live_check`, `launch_command` wiring, Auto edit level, never YOLO): **Claude** hook events arrived on the tab's token (UserPromptSubmit at 4.0 s, Stop at 8.0 s with the last message), `stage_done` with "hello" arrived at 6.9 s with no permission prompt, exit 0. At the Ask level with a `touch` request the PermissionRequest hook arrived at 6.2 s and the command stayed blocked. **Gemini** SessionStart, BeforeAgent, AfterAgent and SessionEnd arrived plus `stage_done` "hello" at 7.2 s. **Copilot** `stage_done` "hello" arrived at 9.8 s; no hook events, as Copilot has no hook wiring in v0. Hook payloads captured from the real CLIs (redacted) are the fixtures in the `status.rs` tests: Claude UserPromptSubmit, PermissionRequest, Notification (permission_prompt), Stop; Gemini SessionStart, BeforeAgent, Notification (ToolPermission), AfterAgent, SessionEnd; Copilot event types from real `events.jsonl` files.
  Findings: Claude's http `SessionStart` hook never fired, in `-p` and in an interactive run, so the first Claude status comes from `UserPromptSubmit` or the lower layers (the Phase 0 "recheck" is settled). No hook reports a permission being answered, hence the decay by output. Claude also probes `server/discover`; the -32601 answer is fine. Copilot's `assistant.turn_end` fires after every model round, so its Done is a final `assistant.message` (no tool requests) or `session.task_complete`, and its `permission.requested` / `permission.completed` pair gives NeedsYou and back; `parse_hook("copilot", ...)` reads `events.jsonl` lines, for the Phase 8 tailer. Numbers: 202 tests passing (45 app, 126 core unit, 14 listener integration, 1 push guard, 16 worktree), up from 139; this branch carries the notes store commit from `phase-5-panes` (and the kelp dependency commit under it), because `reply_to_note` needs `notes.rs`; both merge as no-ops when Phase 5 lands.
  Not verified: Codex (no `codex login`; its hooks are assumed to use Claude's event names), the `StopFailure` payload fields (`error`, `error_details`, from the docs, no real capture), a Gemini idle or question notice. Open for the app half: `reply_to_note` writes `notes.json` behind the app's in-memory copy, so the app must reload the file on `NoteReply` (or its next save overwrites the reply); `Finding` carries file, line and text only, the app builds the note's anchor from the diff; the listener closes the connection after a 401; `SettingUp` has no percentage (PLAN.md's pseudo-signature has `SettingUp(u8)`).

## 2026-10-12

- **Phase 6, core half: launch an agent with the right flags, pushing blocked.** `tidewatch-core` now holds the
  agent table (Claude, Codex experimental, Gemini, Copilot; five levels each, plus `[agent.<id>]` overrides and
  custom agents in `profiles.toml`), `profiles.toml` load/save with first-launch seeding from the agents on PATH
  (Builder = Auto edit, Reviewer = Read only, a YOLO builder per verified agent, never-push on except that YOLO
  builder), `launch_command` and the default `house-style.md`. YOLO is refused unless the tab is a worktree tab.
  The never-push guard is the D7 env setting appended after any existing `GIT_CONFIG_COUNT` entries plus a
  `git-remote-tidewatch-no-push` helper on PATH. Verified live (claude 2.1.296, gemini 0.46.0, copilot 1.0.91,
  throwaway repos, a local MCP and hook listener): hook payloads arrived with the bearer header for Claude and
  Gemini, the `tidewatch` MCP tool was called without a prompt by all three, and the house style produced the
  Done / Needs you / Next block in all three. Push guard against a real bare remote (git 2.55.0): plain,
  `--no-verify`, explicit-URL, new-branch and `--force` pushes all exit 128 with the Tidewatch message, fetch
  works, the remote kept its 1 ref, and a push from a normal shell still lands. Numbers: 139 tests passing
  (45 app, 77 core unit, 1 push guard, 16 worktree round trip), up from 83. Findings: Claude `--permission-mode plan`
  blocks the Tidewatch MCP tools ("Cannot call mcp__tidewatch__stage_done while in plan mode"), so a Read only
  reviewer cannot call `stage_done` or `report_finding`; `--permission-mode dontAsk --allowedTools=mcp__tidewatch__*`
  allowed reads and the MCP call and denied writes, an override candidate for Read only (Bash is unchecked).
  Copilot still shows the folder-trust prompt with `--add-dir`, so the caller must answer it or pre-trust the
  worktree. Codex hooks, MCP and `developer_instructions` are encoded from the binary's config keys, not run
  (no `codex login`). Left for the UI half: new-tab dialog with command preview, settings editor, wiring
  `launch_command` into tabs (writing the MCP file, the Gemini settings and `info/exclude` lines, the helper dir),
  a PATH that includes Homebrew when the app starts from Finder, and the hook listener itself (Phase 7).
- **Phase 4: Cmd+T makes a worktree tab, closing it removes the worktree.** In a git repo, Cmd+T or "+"
  runs `git worktree add --lock -b agent/<adjective-noun>` under `~/.tidewatch/<repo>-<hash>/<name>` on a
  background thread and starts the shell inside it. Closing asks "Keep branch" or "Delete branch"; a
  worktree with uncommitted changes also offers "Keep worktree" and an explicit "Discard" (plain remove
  is refused), and unmerged commits are called out before a branch delete. In a folder that is not a git
  repo the window is in the Other group: Cmd+T opens a plain shell, no worktree. The main tab stays a
  plain shell in the real checkout. The repo chip shows the repo name. On startup a background job runs
  `git worktree repair` for stored worktrees whose repo moved and counts leftovers (worktree folders with
  no open tab), shown as "N leftover" with a path list on hover. Real-git findings (git 2.55.0) are in
  PLAN.md: a failed `add` can leave its branch or whole worktree behind, so add rolls both back, and
  `rev-list --all` hides unmerged commits while a worktree is checked out. Numbers: 83 tests passing
  (45 app, 22 core unit, 16 core round-trip against throwaway repos), up from 22. Runnable check on a
  throwaway repo and temp home: the worktree and `agent/*` branch appeared in `git worktree list`
  (locked); closing with "Delete branch" left no folder, no worktree entry and no branch; two earlier
  worktrees showed as "2 leftover". Open: no repo switching (one repo per window), no UI to remove
  leftovers, worktrees stay on disk when the window quits, `worktrees.toml` is read-modify-write
  without a lock when two windows run, Tidewatch tabs are not restored across launches yet.
- **Decisions confirmed one by one.** D1 to D13 reviewed with the owner. Changed on review: D4 (comments are notes to
  the agent and PR comments), D6 (MCP only), D7 (`pushInsteadOf` guard, Tidewatch delivers the push and PR), D10
  (Linux too, Ubuntu CI job added). Added: D12 (Reader view), D13 (house-style file). Verified live: Claude
  `--append-system-prompt-file`; Codex 0.162.1 flags from `--help`.
- **Phase 3: first running app.** `tidewatch [path]` opens a window for the git repo with a
  pinned main shell tab, Cmd+T / "+" for more shell tabs, stdusk's tab strip and terminal,
  and placeholder History and Diff panels (layout B, 45% / 55%). `TIDEWATCH_SCREENSHOT`
  harness works. Numbers: 22 tests passing (18 app, 4 core). Stdusk crates are path
  dependencies on the Phase 1 worktree. CI is not verified (path dependencies need the
  sibling checkout there).

## 2026-10-05

- Repo created. Concept mockup saved to `docs/concept.html`. Decided so far: name Tidewatch,
  its own repo, agent worktrees in `~/.tidewatch/<repo>/<tab>`, layout B with focus mode,
  stdusk's terminal engine shared through a library crate in the stdusk repo.
- Plan written (`PLAN.md`, 11 decisions, Phases 0 to 10) from three research reports in
  `docs/research/`. Agent rules ported from stdusk and Kelp into `.agents/rules/`, plus
  `workflow.md` for the plan and ledger protocol. Concept corrected: Codex `--full-auto` is
  deprecated, Copilot `--allow-tool write` confirmed valid.
