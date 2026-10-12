# Tidewatch - ledger

A running log of what got done, newest first. The plan lives in [PLAN.md](PLAN.md).

## Status

- **Current milestone:** planning. `PLAN.md` decisions D1 to D11 under review; next is Phase 0 (verification spikes).
- **Done:** concept and mockup (`docs/concept.html`), repo created.

## 2026-10-12

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
