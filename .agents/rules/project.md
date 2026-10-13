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
  src/test_command.rs   per-repo test command: <repo>/.tidewatch.toml `test` (and `test_timeout_secs`, default 10 min), else detected (cargo, npm/pnpm/yarn/bun, deno, go, pytest, rspec, mix, make); run_in, run_in_timeout (kills the process group)
  src/runner.rs         Runner: pure workflow state machine (RunEvent in, Command out), gates decided by Tidewatch, loops, safety net, stages.json save/load/resume
  src/prompt.rs         stage prompt templates: {task} {base} {branch} {previous_summary} {findings} {test_output} (pure)
  src/launch.rs         launch_command: program, args, env; YOLO refusal, never-push guard (D7), hooks/MCP/house-style wiring per CLI
  src/status.rs         AgentStatus and the pure fold of hook events, terminal signals and process facts (D5); per-CLI hook payload parsing; next_change (when a decay fires)
  src/events.rs         TabEvent and TabMessage (with the generation its token registered with): what the listener sends the app, plus the Repaint callback type
  src/listener.rs       127.0.0.1:0 HTTP listener on std::net: per-tab bearer tokens, POST /hook/<agent>, POST /mcp, events over a channel
  src/mcp.rs            Tidewatch MCP tools (D6): stage_done, report_finding, list_notes, reply_to_note over JSON-RPC 2.0
  src/house_style.rs    default house-style.md (D13), ensure never overwrites, derived Copilot instructions file
  src/path_env.rs       PATH with the standard install dirs in front, for Finder launches (pure)
  src/prepare.rs        gets a tab ready to start (or to resume): house style, folder trust for the worktree (D14), guard helper, MCP and Gemini files, task flags, setup-then-agent shell line, banner_line (a stage divider printed before the agent)
  src/fsutil.rs         atomic file write shared by the settings writers
  src/jsonc_edit.rs     text-level edits of JSON with comments (scan, insert, remove, replace) that keep every untouched byte (crate-private, pure)
  src/trust.rs          D14 folder trust, pure: per-CLI config text in, text with the folder trusted (or not) out, idempotent, refuses unparsable files; TrustState per installed CLI
  src/consent.rs        D14 consent state machine (Undecided, Granted, Declined; Yes, No, Apply, Undo -> effects) and trust.toml: the answer and the entries Tidewatch added
  src/trust_apply.rs    D14 disk half: each CLI's config file (CLAUDE_CONFIG_DIR, COPILOT_HOME, CODEX_HOME honored), inspect, trust the root, trust a worktree, revoke; atomic write, `.tidewatch-bak` backup, race check
  src/names.rs          adjective-noun names for new worktrees (pure)
  src/registry.rs       .state/worktrees.toml: the worktrees Tidewatch made, for repair
  src/restore.rs        D11 restore: pure plan (saved session + Disk -> one action per tab), repair_moved, resume_command, restore_workflow, codex_session_id
  src/session_id.rs     the random UUID passed to `--session-id` so the Reader finds a tab's transcript by name
  src/session.rs        .state/session.toml (D11): Session, SavedWindow, SavedTab (main, shell, agent); load (bad file -> empty), atomic save
  src/worktree.rs       add (locked, agent/<name>), inspect, remove, list, repair, leftovers (shells out to git)
  src/notes.rs          per-tab review threads in .state/<repo-dir>/<tab>/notes.json: load, atomic save, background writer, PR import links, sent marks and note state (queued, sent, replied) (D4)
  src/note_paste.rs     the text pasted into an agent to hand it notes: path:line, quoted line, text, id, closing reply_to_note ask; control characters stripped (pure)
  src/end_block.rs      D13 parser: the Done / Needs you / Next block of an agent's last message (pure)
  src/tools.rs          Tools (PATH for git and gh), GhError and the gh runner shared by delivery and pr
  src/delivery.rs       D7 hand-off: DeliveryPlan (commits ahead of base, proposed PR text), deliver = git push -u then gh pr create
  src/pr.rs             D4 PR comments: find the open PR (gh pr view), pull review comments into notes threads (idempotent merge), post a note or reply
  src/pr_lines.rs       which lines of a PR diff take a review comment: git diff base...head parsed into hunk ranges (parse is pure, load shells out)
  src/rfc3339.rs        RFC 3339 timestamp to unix millis (pure, crate-private)
  src/transcript.rs     D12 Reader model: Entry/Role/ToolCall, per-CLI transcript parsers (claude, gemini, copilot verified; codex unverified), incremental Tail
  src/transcript_locate.rs  where each CLI keeps a tab's session file: Locator by cwd and session id (claude folder encoding verified), gemini, copilot, codex (unverified)
  src/history.rs        branch-scoped graph, ahead/behind, commit and working-tree files and diffs, file watcher, via kelp-core
  tests/worktree_roundtrip.rs  real throwaway repos: add, lock, dirty refusal, delete branch, moved repo
  tests/listener_http.rs  listener on a random port with a raw HTTP client: 401s, hooks, MCP, keep-alive, chunked, size caps
  examples/live_check.rs  by hand: run a real claude, gemini or copilot headless through the listener and print the events
  tests/push_guard.rs   real bare remote: plain, --no-verify, explicit-url and forced pushes refused, fetch works
  tests/common/mod.rs   test helpers: throwaway repos with a bare remote, a fake gh that logs its calls (the real gh never runs)
  tests/delivery_roundtrip.rs  real bare remote + fake gh: plan, base detection, push, PR args, each typed failure
  tests/pr_lines_git.rs real throwaway repo: the branch diff against the remote base says which lines take a comment
  tests/pr_gh.rs        fake gh: pr view, comments fetch and post argument shapes, GitHub-shaped JSON fixtures
  tests/fixtures/       redacted real transcripts (claude, gemini, copilot), hand-written codex, GitHub-shaped PR comments
  tests/history_roundtrip.rs   real throwaway repos: worktree branch history, whole repo, ahead/behind, diffs, watcher
  tests/session_restore.rs     real repos + fake home: moved repo repaired then planned, removed worktree, missing session file
crates/tidewatch/       the egui app
  src/main.rs           argument parsing, workspace discovery, eframe start
  src/app.rs            the eframe::App loop: chip, tab strip, terminal, card bands, History and Diff panes, toast, shortcuts, job results
  src/tabs.rs           tab list and its Action reducer (pure, generic over the pane)
  src/pane.rs           what a tab holds: shell or agent (with its AgentRun), creating, removing, failed; opening results, close policy (pure, generic)
  src/jobs.rs           background thread runner: result channel plus a repaint
  src/worktree_jobs.rs  open a tab (worktree, listener registration and prepared agent launch), inspect, remove and startup repair as jobs
  src/agent_run.rs      one agent tab's live state: StatusTracker, terminal snapshot in, status change out, last MCP report (pure)
  src/agent_events.rs   starts the Tidewatch listener once (bind failure: logged, status falls back to layers 2 and 3), drains it, findings -> notes
  src/agent_frame.rs    per-frame agent work: route listener messages to tabs, feed terminal signals, notify, say when the next frame is due (generic, pure)
  src/status_view.rs    status -> tone, label, color; CLI logo mapping; the tab's top-edge status bar (animated only while working)
  src/notify.rs         desktop notifications (osascript, notify-send) off the UI thread, per-tab per-kind rate limit, window-away rule
  src/overview.rs       Overview rows and groups from the agent tabs (needs you first), the workflow stage of a row, elapsed format (pure)
  src/overview_view.rs  the Overview screen (Cmd+0) and the tab strip's icon button with a needs-you badge
  src/catalog.rs        loaded profiles, the agent table built from them, which CLIs were found and their versions
  src/agent_jobs.rs     load or seed profiles.toml and probe each agent CLI as jobs
  src/new_tab.rs        new-tab dialog state: picks (profiles, workflows in a repo, shell), permission ladder and YOLO gating, command preview, request (pure)
  src/new_tab_workflow.rs  the dialog's workflow side: list order, stage preview, missing agents, the request (pure)
  src/new_tab_dialog.rs the dialog itself: profile list, ladder, task, preview; Enter starts, Esc cancels
  src/list_row.rs       one selectable list row, shared by the dialog and the profile editor
  src/profile_editor.rs profile editor state: drafts, text in and out of fields, validation, profiles to save (pure)
  src/settings.rs       what the settings view holds: the profile and workflow editors and which page shows (pure)
  src/settings_view.rs  the settings view, Profiles and Workflows pages (replaces the terminal area; Cmd+, or the gear)
  src/workflow_editor.rs  workflow editor state: drafts per workflow and stage, problems found as you type, workflows to save (pure)
  src/workflow_editor_view.rs  the Workflows page: list, stage chips, the stage's fields, problems under them
  src/close_dialog.rs   keep/delete branch dialog: choices and wording (pure) plus the modal
  src/layout.rs         terminal/panel split, grid size, tab width, digit keys (pure)
  src/tab_view.rs       per-tab pane state: loaded history, selection, open diff, notes, watcher; wants -> jobs -> answers (stale ones dropped)
  src/pane_jobs.rs      history, status, files, diff, notes, watcher and checkout as jobs
  src/history_pane.rs   History pane: header (branch, ahead/behind, This worktree / Whole repo), kelp-ui graph
  src/diff_pane.rs      Diff pane: header (Send all open, Deliver when idle), file list, toolbar (PR chip, Pull PR comments), notes strip, kelp-ui diff body with notes
  src/note_actions.rs   kelp-ui DiffAction -> the tab's notes (pure)
  src/checkout.rs       double-click checkout rule: main tab only, clean tree only (pure plan, git outcome)
  src/toast.rs          one timed message, fade on egui's clock (pure)
  src/fonts.rs          stdusk font set with kelp-ui's faces and semibold family merged in (kelp_ui::fonts::extend), CJK fallback
  src/repo.rs           git root and label; Workspace = Repo or the Other group
  src/dev_screenshot.rs TIDEWATCH_SCREENSHOT harness
  src/dev_worktree.rs   TIDEWATCH_DEV_WORKTREE switch: open a worktree tab at startup for captures
  src/dev_panes.rs      TIDEWATCH_DEV_SCOPE / _PICK / _NOTE switches: graph scope, selected row, a note for captures
  src/dev_view.rs       TIDEWATCH_DEV_OVERVIEW / _FOCUS / _STATUS_LOG / _KEYS switches: start on a view, log status changes, answer a prompt
  src/dev_agent.rs      TIDEWATCH_DEV_AGENT / _WORKFLOW / _RESUME / _DIALOG / _SETTINGS / _TASK / _LEVEL switches: start a profile or workflow, resume a saved run, open the dialog or the editor
  src/dev_workflow.rs   TIDEWATCH_DEV_ASK / _OPEN_STAGE switches: answer a workflow run's question, open a stage under the stepper
  src/dev_flow.rs       TIDEWATCH_DEV_GH_DIR / _AUTO / _SEND / _DELIVER / _PR switches: a fake gh first on PATH, then do once what a click would
  src/flow.rs           Flow: one TabFlow per agent worktree tab (deliver-when-idle, delivery card, PR state), decide() turns frame facts into jobs, finished jobs come back as Out
  src/flow_jobs.rs      send notes, probe/build/deliver a PR, find the PR, load its diff lines, pull its comments and post a note as jobs; notes-file jobs take NOTES_TURN
  src/flow_glue.rs      seams between Flow and the app: tab facts, paste and Enter into a pty, the Diff pane's actions, the flow's dialogs, dev switch driver
  src/note_send.rs      send gate (never paste into a waiting agent), AutoDelivery (deliver when idle, never while Working or NeedsYou), the Enter queue (pure)
  src/notes_strip.rs    the open notes under the Diff toolbar: state (queued, sent <ago>, replied), Send, Post to PR, jump to file
  src/delivery_card.rs  state of the push-and-open-PR card: probe, build, edit, deliver, fail with a plain fix, succeed (pure)
  src/delivery_view.rs  the compact card above the panes and the editable dialog (Cmd+Enter approves, Esc cancels)
  src/pr_state.rs       PR lookup state (focus and after a delivery, never per frame), the chip, whether a note may be posted (pure)
  src/post_confirm.rs   the confirm dialog before a note goes to GitHub: PR number and repo, exact text, Enter posts, Esc cancels
  src/reader.rs         one agent tab's Reader: the view choice (Reader or raw terminal), feed state, rows; `shown()` decides what the tab draws and why (pure)
  src/reader_rows.rs    transcript entries to view rows (a tool call and its result merge into one row, a divider where a workflow stage starts), capped at 400 rows (pure)
  src/reader_feed.rs    the thread that finds the transcript (retry schedule) and tails it; woken by pty output and hooks, backs off while quiet
  src/reader_scroll.rs  autoscroll state (follow until the user scrolls up) and row height cache and estimate (pure)
  src/reader_keys.rs    the permission answers verified per CLI and the bytes that give them (Claude: 1 and Esc)
  src/reader_view.rs    draws the Reader: markdown rows, collapsed tool lines, permission strip, reply box, jump to latest
  src/reader_glue.rs    seams: start feeds, drain, Cmd+Shift+T toggle, draw in the terminal area, type replies and answers into the pty
  src/dev_reader.rs     TIDEWATCH_DEV_READER / _READER_KEY / _ANSWER / _REPLY switches
  src/end_block_view.rs the Done / Needs you / Next card on the tab (expandable) and its one-line form for the Overview
  src/workflow_run.rs   one workflow tab's run: the Runner, Exec (launch a stage, run the tests) out, events in, generation per launch, stages.json saved after every event, held status, ask (pure)
  src/workflow_ask.rs   the question a run asks and its buttons per reason (Approve, Retry, Reject with feedback, Stop), button to event (pure)
  src/stepper.rs        the stepper's data: a pill per stage with state and loop counter, the gate, a stage's summary and findings, the position label (pure)
  src/stepper_view.rs   draws the header of a workflow tab: stepper, a clicked stage, the ask card, the stopped line
  src/workflow_jobs.rs  open (worktree, base, test command and timeout), launch a stage (fresh token, prepare, divider), run the tests as jobs
  src/workflow_glue.rs  Host: open a workflow or saved-run tab, carry out what a run asks, apply finished jobs, drop a replaced agent's messages (generic over the pty)
docs/                   concept mockup, research reports
packaging/              Info.plist, reference Homebrew cask (tidewatch.rb), .desktop file, release README
scripts/                bundle-macos.sh (universal ad-hoc signed Tidewatch.app + zip), package-linux.sh (tarball + AppImage), make-icon.sh (icon from assets/icon.svg)
assets/                 icon.svg (placeholder), Tidewatch.icns, icon-512.png
.github/workflows/      ci.yml (fmt, clippy, test on macOS and Ubuntu), release.yml (v* tags: bundle, package, GitHub Release, tap behind PUBLISH_TAP)
```

Update this block in the same commit that adds or moves a module.

## Commands

```
cargo run -p tidewatch -- [path]       # open the git repo containing path (default: cwd)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
scripts/bundle-macos.sh                # universal Tidewatch.app and zip in target/dist, ad-hoc signed; never launch the result from a check
scripts/package-linux.sh               # x86_64 Linux only: tarball and AppImage in target/dist
scripts/make-icon.sh                   # rebuild the icon files from assets/icon.svg (inkscape, pngquant, oxipng)
TIDEWATCH_SCREENSHOT=/tmp/x.png cargo run -p tidewatch -- <repo>   # render, save PNG, quit
```

Screenshot runs open in the background: no Dock icon, never take keyboard focus from the
developer's active window. `TIDEWATCH_BACKGROUND=1` does the same for any other dev or live-check
run. Agents must use one of the two for every window they open.
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
`TIDEWATCH_DEV_GH_DIR=<dir>` puts `<dir>` first on the PATH `git` and `gh` run with, for a fake `gh` script (the real GitHub API must
never be reached from a check). `TIDEWATCH_DEV_AUTO=1` turns Deliver when idle on, `TIDEWATCH_DEV_SEND=one|all` sends the first or every
unsent note, `TIDEWATCH_DEV_DELIVER=open|approve` opens the delivery dialog (and presses Push and open PR), `TIDEWATCH_DEV_PR=pull|ask|post`
pulls the PR's comments or opens (and confirms) the post dialog for the first note, each once, as soon as the active tab is ready.
`TIDEWATCH_DEV_OVERVIEW=1` and `TIDEWATCH_DEV_FOCUS=1` start on the Overview or in focus mode; `TIDEWATCH_DEV_STATUS_LOG=1`
prints every hook and status change to stderr with a wall-clock time in ms; `TIDEWATCH_DEV_KEYS='\e[B\r'` (after
`TIDEWATCH_DEV_KEYS_AFTER` seconds, default 8) types into the newest agent tab once, for Claude's folder-trust prompt.
`TIDEWATCH_DEV_WORKFLOW=<id>` starts a workflow in a new tab (task from `TIDEWATCH_DEV_TASK`), `TIDEWATCH_DEV_DIALOG=workflow:<id>` opens the dialog on it,
`TIDEWATCH_DEV_RESUME=<worktree name>` opens the tab of a saved run (`.state/<repo-dir>/<name>/stages.json`) and carries on, `TIDEWATCH_DEV_ASK=approve|retry|stop|reject:<reason>`
answers the next question a run asks, `TIDEWATCH_DEV_OPEN_STAGE=<n>` opens stage n under the stepper, `TIDEWATCH_DEV_SETTINGS=workflows` opens the workflow editor.
`TIDEWATCH_DEV_READER=raw` starts agent tabs on the raw terminal, `TIDEWATCH_DEV_READER_KEY=<s>` presses Cmd+Shift+T after that many
seconds through the real key path, `TIDEWATCH_DEV_ANSWER=allow|deny` clicks a permission button once, `TIDEWATCH_DEV_REPLY=<text>`
(`_REPLY_AFTER=<s>`) sends a reply from the Reader's box. Run Claude checks with `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1` when the
shell that starts Tidewatch is itself a Claude session: the child otherwise writes no transcript and the Reader falls back to the terminal.
Keys: Cmd+T new tab (the dialog: pick a profile or Shell, permission level, task; Enter starts, Esc cancels), Cmd+W
close the active tab (not the pinned main tab; a worktree tab asks keep/delete branch first), Cmd+1..9 select,
Cmd+Shift+T flip the active agent tab between the Reader and the raw terminal, Cmd+, the profile editor, Cmd+0 the Overview (Esc or Cmd+0 closes it), Cmd+. focus mode (hides History and Diff).
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

Releases follow `packaging/README.md` (bump `Cargo.toml`, tag `v<version>`). The tap job stays off until the
repository variable `PUBLISH_TAP` is `true`, which waits for the repo to go public. Once there is a public page
and a Homebrew cask, the same upkeep as Kelp applies: a user-visible change updates `README.md` and the page in the same branch, removed
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
