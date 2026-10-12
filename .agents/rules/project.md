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
  src/paths.rs          ~/.tidewatch layout (D9), state dirs, per-tab notes and stages files, ~/.config/tidewatch files (pure)
  src/agents.rs         agent table: claude, codex, gemini, copilot, five permission levels to flags, version probe, [agent.<id>] overrides
  src/profiles.rs       Profile, profiles.toml load/save (bad file -> defaults), first-launch seeding
  src/workflows.rs      Workflow, Stage, Gate, Action; workflows.toml load/save/seed (bad file -> defaults), the built-in Ship a feature, Fix a bug, Review a PR
  src/test_command.rs   per-repo test command: <repo>/.tidewatch.toml `test`, else detected (cargo, npm/pnpm/yarn/bun, deno, go, pytest, rspec, mix, make); run_in
  src/runner.rs         Runner: pure workflow state machine (RunEvent in, Command out), gates decided by Tidewatch, loops, safety net, stages.json save/load/resume
  src/prompt.rs         stage prompt templates: {task} {base} {branch} {previous_summary} {findings} {test_output} (pure)
  src/launch.rs         launch_command: program, args, env; YOLO refusal, never-push guard (D7), hooks/MCP/house-style wiring per CLI
  src/status.rs         AgentStatus and the pure fold of hook events, terminal signals and process facts (D5); per-CLI hook payload parsing
  src/events.rs         TabEvent and TabMessage: what the listener sends the app, plus the Repaint callback type
  src/listener.rs       127.0.0.1:0 HTTP listener on std::net: per-tab bearer tokens, POST /hook/<agent>, POST /mcp, events over a channel
  src/mcp.rs            Tidewatch MCP tools (D6): stage_done, report_finding, list_notes, reply_to_note over JSON-RPC 2.0
  src/house_style.rs    default house-style.md (D13), ensure never overwrites, derived Copilot instructions file
  src/path_env.rs       PATH with the standard install dirs in front, for Finder launches (pure)
  src/prepare.rs        gets a tab ready to start: house style, guard helper, MCP and Gemini files, task flags, setup-then-agent shell line
  src/fsutil.rs         atomic file write shared by the settings writers
  src/names.rs          adjective-noun names for new worktrees (pure)
  src/registry.rs       .state/worktrees.toml: the worktrees Tidewatch made, for repair
  src/worktree.rs       add (locked, agent/<name>), inspect, remove, list, repair, leftovers (shells out to git)
  src/notes.rs          per-tab review threads in .state/<repo-dir>/<tab>/notes.json: load, atomic save, background writer (D4)
  src/history.rs        branch-scoped graph, ahead/behind, commit and working-tree files and diffs, file watcher, via kelp-core
  tests/worktree_roundtrip.rs  real throwaway repos: add, lock, dirty refusal, delete branch, moved repo
  tests/listener_http.rs  listener on a random port with a raw HTTP client: 401s, hooks, MCP, keep-alive, chunked, size caps
  examples/live_check.rs  by hand: run a real claude, gemini or copilot headless through the listener and print the events
  tests/push_guard.rs   real bare remote: plain, --no-verify, explicit-url and forced pushes refused, fetch works
  tests/history_roundtrip.rs   real throwaway repos: worktree branch history, whole repo, ahead/behind, diffs, watcher
crates/tidewatch/       the egui app
  src/main.rs           argument parsing, workspace discovery, eframe start
  src/app.rs            the eframe::App loop: chip, tab strip, terminal, History and Diff panes, toast, shortcuts, job results
  src/tabs.rs           tab list and its Action reducer (pure, generic over the pane)
  src/pane.rs           what a tab holds: shell or agent, creating, removing, failed; opening results, close policy (pure, generic)
  src/jobs.rs           background thread runner: result channel plus a repaint
  src/worktree_jobs.rs  open a tab (worktree and prepared agent launch), inspect, remove and startup repair as jobs
  src/catalog.rs        loaded profiles, the agent table built from them, which CLIs were found and their versions
  src/agent_jobs.rs     load or seed profiles.toml and probe each agent CLI as jobs
  src/new_tab.rs        new-tab dialog state: picks, permission ladder and YOLO gating, command preview, request (pure)
  src/new_tab_dialog.rs the dialog itself: profile list, ladder, task, preview; Enter starts, Esc cancels
  src/list_row.rs       one selectable list row, shared by the dialog and the profile editor
  src/profile_editor.rs profile editor state: drafts, text in and out of fields, validation, profiles to save (pure)
  src/settings_view.rs  the profile editor view (replaces the terminal area; Cmd+, or the gear)
  src/close_dialog.rs   keep/delete branch dialog: choices and wording (pure) plus the modal
  src/layout.rs         terminal/panel split, grid size, tab width, digit keys (pure)
  src/tab_view.rs       per-tab pane state: loaded history, selection, open diff, notes, watcher; wants -> jobs -> answers (stale ones dropped)
  src/pane_jobs.rs      history, status, files, diff, notes, watcher and checkout as jobs
  src/history_pane.rs   History pane: header (branch, ahead/behind, This worktree / Whole repo), kelp-ui graph
  src/diff_pane.rs      Diff pane: header, file list, toolbar, kelp-ui diff body with notes
  src/note_actions.rs   kelp-ui DiffAction -> the tab's notes (pure)
  src/checkout.rs       double-click checkout rule: main tab only, clean tree only (pure plan, git outcome)
  src/toast.rs          one timed message, fade on egui's clock (pure)
  src/fonts.rs          stdusk font set plus the semibold family kelp-ui draws with
  src/repo.rs           git root and label; Workspace = Repo or the Other group
  src/dev_screenshot.rs TIDEWATCH_SCREENSHOT harness
  src/dev_worktree.rs   TIDEWATCH_DEV_WORKTREE switch: open a worktree tab at startup for captures
  src/dev_panes.rs      TIDEWATCH_DEV_SCOPE / _PICK / _NOTE switches: graph scope, selected row, a note for captures
  src/dev_agent.rs      TIDEWATCH_DEV_AGENT / _DIALOG / _SETTINGS / _TASK / _LEVEL switches: start a profile, open the dialog or the editor
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
`TIDEWATCH_DEV_SCOPE=whole` starts the graph on Whole repo, `TIDEWATCH_DEV_PICK=<row>` selects a graph row,
`TIDEWATCH_DEV_NOTE=<text>` writes a note on the first added line of the first diff shown (saved like a real one).
Notes live in `<home>/.tidewatch/.state/<repo-dir>/<tab>/notes.json`; the main tab's key is `main`.
`TIDEWATCH_HOME=<dir>` keeps `.tidewatch` and `.config/tidewatch` in `<dir>` while `HOME` stays real, so agent
CLIs keep their login. `TIDEWATCH_DEV_AGENT=<profile id>` starts that profile in a new tab at startup (task from
`TIDEWATCH_DEV_TASK`, level from `TIDEWATCH_DEV_LEVEL`; a level the group refuses, such as YOLO in Other, is
reported on stderr and nothing starts). `TIDEWATCH_DEV_DIALOG=<profile id>|1` opens the new-tab dialog and
`TIDEWATCH_DEV_SETTINGS=1` the profile editor, for captures (use `TIDEWATCH_SCREENSHOT_WAIT=6`: profiles load and
versions are probed in the background). Check an agent tab with `ps -o args= -p $(pgrep -n claude)`.
Keys: Cmd+T new tab (the dialog: pick a profile or Shell, permission level, task; Enter starts, Esc cancels), Cmd+W
close the active tab (not the pinned main tab; a worktree tab asks keep/delete branch first), Cmd+1..9 select,
Cmd+, the profile editor.
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
