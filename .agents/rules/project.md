---
trigger: always_on
description: 'Tidewatch overview: what it is, sibling repos, planned layout, commands, performance rules, showcase and release upkeep, commit standards.'
---

# Tidewatch - project rules

A native app for macOS and Linux in Rust (egui/eframe 0.35) for running AI coding agents in git worktrees,
one repo at a time. Each tab is a fresh worktree with an agent in it. It reuses Kelp (graph,
diff, review) and stdusk (terminal engine, tabs, quake window) through library crates in
those repos, as git dependencies. The plan and decisions live in `PLAN.md`; progress, with
measured numbers, in `LEDGER.md`. The concept mockup is `docs/concept.html`.

## Sibling repos

- Kelp: `../kelp` (github.com/Hobo-Ware/kelp). Rules: `../kelp/.agents/rules/project.md`.
- stdusk: `../stdusk` (github.com/Hobo-Ware/stdusk). Rules: `../stdusk/.agents/rules/`.

Behavior Tidewatch shares with them (quake window, tabs, updates, settings, review, graph)
matches the sibling unless `PLAN.md` says otherwise. Fix a shared bug in the shared crate,
in the sibling repo, never in a local copy.

## Layout

```
crates/tidewatch-core/  worktrees, profiles, permission levels, workflows, agent status, no UI
  src/paths.rs          ~/.tidewatch layout (D9), state dirs (pure)
  src/names.rs          adjective-noun names for new worktrees (pure)
  src/registry.rs       .state/worktrees.toml: the worktrees Tidewatch made, for repair
  src/worktree.rs       add (locked, agent/<name>), inspect, remove, list, repair, leftovers (shells out to git)
  tests/worktree_roundtrip.rs  real throwaway repos: add, lock, dirty refusal, delete branch, moved repo
crates/tidewatch/       the egui app
  src/main.rs           argument parsing, workspace discovery, eframe start
  src/app.rs            the eframe::App loop: chip, tab strip, terminal, placeholder panels, shortcuts, job results
  src/tabs.rs           tab list and its Action reducer (pure, generic over the pane)
  src/pane.rs           what a tab holds: shell, creating, removing, failed; close policy (pure, generic)
  src/jobs.rs           background thread runner: result channel plus a repaint
  src/worktree_jobs.rs  worktree create, inspect, remove and startup repair as jobs
  src/close_dialog.rs   keep/delete branch dialog: choices and wording (pure) plus the modal
  src/layout.rs         terminal/panel split, grid size, tab width, digit keys (pure)
  src/panels.rs         History and Diff placeholder panels (Phase 5 replaces them)
  src/repo.rs           git root and label; Workspace = Repo or the Other group
  src/dev_screenshot.rs TIDEWATCH_SCREENSHOT harness
  src/dev_worktree.rs   TIDEWATCH_DEV_WORKTREE switch: open a worktree tab at startup for captures
docs/                   concept mockup, research reports
```

Update this block in the same commit that adds or moves a module.

## Commands

```
cargo run -p tidewatch -- [path]       # open the git repo containing path (default: cwd)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
TIDEWATCH_SCREENSHOT=/tmp/x.png cargo run -p tidewatch -- <repo>   # render, save PNG, quit
```

`TIDEWATCH_SCREENSHOT_WAIT=<seconds>` waits longer before the capture (default 1.5s).
`TIDEWATCH_DEV_WORKTREE=open|ask|delete` opens one worktree tab at startup; `ask` also shows its close
dialog, `delete` closes it with "Delete branch". Use a throwaway repo, and a temp `HOME` (with
`ZDOTDIR` set to it) so worktrees land in a temp `~/.tidewatch`, never in the real home.
Keys: Cmd+T new tab (a worktree tab in a git repo, a plain shell in the Other group), Cmd+W close the
active tab (not the pinned main tab; a worktree tab asks keep/delete branch first), Cmd+1..9 select.
The sibling crates are git dependencies pinned by commit (`rev`), not by tag (PLAN.md,
Deviations log). To pick up a sibling change, merge it there, then bump `rev`.

Check the real exit code of a build (`cargo build 2>build.log; echo $?`), never a piped
`tail`. Verify UI changes with the screenshot switch instead of asking for a manual check.

## Performance rules

- Idle CPU stays at 0%: repaint only on input, pty output or finished background work.
  Animations run only while something is happening.
- Never block the UI thread on git, the network, disk-heavy work or a child process.
- The graph draws only visible rows; per-row work is O(visible lanes).
- Many agents at once is the normal case: a hidden tab costs no paint time.

## Showcase page and releases

Once there is a public page and a Homebrew cask (later phases), the same upkeep as Kelp
applies: a user-visible change updates `README.md` and the page in the same branch, removed
behavior comes off, screenshots come from the screenshot switch on a demo repo (never a
private or work repo) and are compressed (pngquant + oxipng, PSNR checked).

## Commit standards

- Conventional Commits: `feat: ...`, `fix: ...`, `docs: ...`, `chore: ...`, optional scope.
- Clean author line: no `Co-Authored-By` trailer, no "Generated with" footer.
- Plain hyphens only in commits and GitHub prose, no em or en dashes.
- Commit or push only when asked. One logical change per commit.

## Quick checklist

- [ ] Read `LEDGER.md` + the current phase in `PLAN.md`; follow `workflow.md`.
- [ ] Tests, clippy and fmt pass.
- [ ] UI change verified with a screenshot.
- [ ] `LEDGER.md` updated; layout block above still true.
