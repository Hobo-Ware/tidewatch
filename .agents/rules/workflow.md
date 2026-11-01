---
trigger: always_on
description: 'How work happens here: PLAN.md and LEDGER.md protocol, one phase per branch and PR, checkpoints, deviations, AI-first conventions.'
---

# Workflow - plan and ledger protocol

Tidewatch is built mostly by AI agents, for people who run AI agents. The repo is set up so
an agent with an empty context can pick up the work, do one phase correctly, and leave the
next agent a clean trail. These rules make that possible.

## The four sources of truth

| File | What it holds | Who edits it |
| --- | --- | --- |
| `PLAN.md` | Architecture, decisions D1..Dn, phases with runnable checks, "Phase 0 findings", "Deviations log" | Agents add findings and agreed deviations; decisions change only with the human |
| `LEDGER.md` | What got done, newest first, with measured numbers | Agents, after every milestone |
| `docs/research/` | Raw research reports the plan was built from | Read-only after planning; add a new file instead of rewriting one |
| `docs/concept.html` | The clickable concept mockup | Update when a shipped behavior changes the concept |

Do not re-derive what these files already settle. Read them instead of re-exploring the
sibling repos.

## Start of every session

1. Read the `LEDGER.md` status block and the current phase in `PLAN.md`.
2. Read the rule files the phase touches (routing table in `CLAUDE.md`).
3. Before writing code, restate in a few lines: the phase goal, its steps, its runnable check
   and its exit criteria. List what you need from the human.

## One phase at a time

- **One phase = one branch = one PR.** Branch name `phase-<n>-<slug>`, in its own worktree.
- Work step by step, tests first where practical. Stop for acknowledgement between steps when
  the human asked for that pace.
- Run the phase's runnable check **exactly as written** and paste the real, trimmed output.
  Never report a phase done with a failing check or a red gate (fmt, clippy, tests).
- End with a checkpoint report: Done / Verified / Exit criteria (ticked) / Needs you /
  Deviations / PR link. Then stop for sign-off before the next phase.
- Phases 1 and 2 happen in `../stdusk` and `../kelp`. There, that repo's rules, ledger and
  commit format apply. Log the work in that repo's `LEDGER.md` and add a one-line pointer in
  this one.

## Decisions and deviations

- Decisions marked `DECIDED` in `PLAN.md` are constraints. Do not redesign them.
- When the plan and reality disagree, **stop and tell the human**. Once agreed, write the
  change in the "Deviations log" of `PLAN.md` **before** committing code that depends on it.
- A Phase 0 finding that breaks a decision reopens that decision with the human. It is never
  silently absorbed.

## Ledger entries

Add an entry after each milestone, newest first, under a `## YYYY-MM-DD` heading:

```
- **<version or phase>: <what changed, in user terms>.** How it works in one or two
  sentences. Numbers: tests passing, timings before -> after, sizes. Anything left open.
```

Keep the `## Status` block at the top current: current milestone, what is done, what is next.
Numbers come from a real run, never from memory.

## AI-first conventions

- **Every rule file has frontmatter** (`trigger`, `globs`, `description`) so any agent tool can
  route to it. A new area of code gets a rule file and a row in the `CLAUDE.md` routing table.
- **The module map in `docs/layout.md` stays true.** A new or moved module adds or fixes its
  line in the same commit.
- **Verify, do not assume.** Agent CLIs change often. Before coding against a CLI flag, hook or
  output format, check the installed CLI's `--help` or a real run, and note the version you
  checked in the code's test or in `PLAN.md`.
- **Dogfood.** Once a phase makes Tidewatch usable for a task, use it to build the next phase
  and log what got in the way in `LEDGER.md`.
- **Name the next move.** Any message that stops for the human ends with one line:
  `Next: <exactly what the human runs or says>`.

## Git and GitHub

- Conventional Commits, clean author line (no `Co-Authored-By`, no "Generated with"), plain
  hyphens only. Commit or push only when asked.
- Review feedback is folded into the commit it belongs to (`git commit --fixup=<sha>`, then a
  non-interactive autosquash), never a standalone "address review" commit. Push with
  `--force-with-lease`.
- Reply to every review thread, then resolve it. Keep the PR body in sync with the branch.
