# Tidewatch - ledger

A running log of what got done, newest first. The plan lives in [PLAN.md](PLAN.md).

## Status

- **Current milestone:** Phase 4, worktree tabs (branch `phase-4-worktrees`, local, not pushed). Phase 3 skeleton is the base. Decisions D1 to D13 confirmed by the owner.
- **Done:** concept, plan, repo, stdusk library crates (stdusk PR #20, merged), the Phase 3 app skeleton, the Phase 4 worktree manager and worktree tabs.
- **Next:** review Phase 4, then Phase 2 (`kelp-ui`) or Phase 5. Left from Phase 4: repo switching between several repos, a way to remove leftover worktrees. Owner: `codex login` for the Codex live checks.

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
