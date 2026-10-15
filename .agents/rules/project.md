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
  src/paths.rs          ~/.tidewatch layout (D9), state dirs, the one app socket, the handoff socket of a restart (`.state/handoff-<pid>.sock`, outside `instances/` so a toggle never reaches it), per-tab notes and stages files, ~/.config/tidewatch files (pure)
  src/agents.rs         agent table: claude, codex, gemini, copilot, five permission levels to flags, version probe, [agent.<id>] overrides
  src/profiles.rs       Profile, profiles.toml load/save (bad file -> defaults), first-launch seeding
  src/workflows.rs      Workflow, Stage, Gate, Action; workflows.toml load/save/seed (bad file -> defaults), `BUILT_IN_IDS` (the dialog's order), the original built-ins Ship a feature, Fix a bug, Review a PR
  src/builtin_workflows.rs  the six habit built-ins: Hunt a root cause, Finish with proof, Change the UI with proof, Port to a platform, Prepare a release, Refactor safely (stage prompts, gates, loops; none pushes or tags, each ends at an Approve gate)
  src/workflow_upgrade.rs  an existing workflows.toml gains the built-ins it was never offered, once: appended after the user's text, ids recorded in `workflows.offered` next to it so a deleted one stays deleted and an edited or same-named one is left alone; the original three always count as offered
  src/test_command.rs   per-repo test command: <repo>/.tidewatch.toml `test` (and `test_timeout_secs`, default 10 min), else detected (cargo, npm/pnpm/yarn/bun, deno, go, pytest, rspec, mix, make); run_in, run_in_timeout (kills the process group)
  src/runner.rs         Runner: pure workflow state machine (RunEvent in, Command out), gates decided by Tidewatch, loops, safety net, stages.json save/load/resume
  src/prompt.rs         stage prompt templates: {task} {base} {branch} {previous_summary} {findings} {test_output} (pure)
  src/propose.rs        Build workflow from this session, the core half: the prompt (a template filled from `Action::ALL`, `Gate::ALL`, the levels and `prompt::PLACEHOLDERS`, so it cannot drift), the `propose_workflow` tool schema and argument parsing, the rules a proposal passes (the editor's, plus no YOLO, no profile, known agents), the last `toml` block of a reply and its parse (`workflows.toml` or one bare workflow), `unique_name`, `id_stem` (pure)
  src/launch.rs         launch_command: program, args, env; YOLO refusal, never-push guard (D7), hooks/MCP/house-style wiring per CLI
  src/status.rs         AgentStatus and the pure fold of hook events, terminal signals and process facts (D5); per-CLI hook payload parsing; next_change (when a decay fires)
  src/events.rs         TabEvent and TabMessage (with the generation its token registered with): what the listener sends the app, plus the Repaint callback type
  src/listener.rs       127.0.0.1:0 HTTP listener on std::net: per-tab bearer tokens, POST /hook/<agent>, POST /mcp, events over a channel; `credential_of`, `adopt_tab` and `start_on_port` let a restarted window serve the old port and honour the old tokens
  src/mcp.rs            Tidewatch MCP tools (D6): stage_done, report_finding, list_notes, reply_to_note, propose_workflow (a valid workflow becomes `TabEvent::Proposal`, problems come back as a tool error to fix) over JSON-RPC 2.0
  src/house_style.rs    default house-style.md (D13), ensure never overwrites, reset puts the default text back, derived Copilot instructions file
  src/path_env.rs       PATH with the standard install dirs in front, for Finder launches (pure)
  src/prepare.rs        gets a tab ready to start (or to resume): house style, folder trust for the worktree (D14), guard helper, MCP and Gemini files, task flags, setup-then-agent shell line, banner_line (a stage divider printed before the agent), note_script (a plain shell that prints a note first)
  src/fsutil.rs         atomic file write shared by the settings writers
  src/jsonc_edit.rs     text-level edits of JSON with comments (scan, insert, remove, replace) that keep every untouched byte (crate-private, pure)
  src/trust.rs          D14 folder trust, pure: per-CLI config text in, text with the folder trusted (or not) out, idempotent, refuses unparsable files; TrustState per installed CLI
  src/consent.rs        D14 consent state machine (Undecided, Granted, Declined; Yes, No, Apply, Undo -> effects) and trust.toml: the answer and the entries Tidewatch added
  src/trust_apply.rs    D14 disk half: each CLI's config file (CLAUDE_CONFIG_DIR, COPILOT_HOME, CODEX_HOME honored), inspect, trust the root, trust a worktree, revoke; atomic write, `.tidewatch-bak` backup, race check
  src/prefs.rs          ~/.config/tidewatch/settings.toml: the app-wide switches (restore the last session at launch, default on; `resume_agents` auto, prefill or off, default auto) the [window] table (quake on, hotkey Ctrl+Alt+Grave, menu-bar item on, panels side by side or below) and the [updates] table (check, install; both on); bad file -> defaults; `update` rewrites one part from a fresh read
  src/update.rs          self-update, the pure part: the repo and cask, `Mode::pick` (only a macOS cask install updates itself, the rest is notify-only), the fake release for `TIDEWATCH_FAKE_UPDATE`
  src/instance.rs       one window for the app: one Unix socket (`.state/instances/tidewatch.sock`), `open <path>` and `adopt-stdusk <socket>` hand-offs from a second launch (any folder: the window adds it as a repo or shows it), `toggle` to every live window, stale socket takeover, guard that unlinks on exit; every command is answered `ok` or `closing`, and a launch that hears `closing` waits (50 ms polls, 10 s) for the socket and takes it over; `claim_replacing` is the same wait for a window that takes over from a closing one, and sends nothing
  src/last_workspace.rs .state/last-workspace.toml (`dir`, the repo in front when the window was used last, and `also`, the other repos that had tabs open) and `pick`, the pure choice of the folders a launch opens: command line (alone), a working folder in a repo, the remembered folders that still exist, the working folder
  src/names.rs          adjective-noun names for new worktrees (pure)
  src/registry.rs       .state/worktrees.toml: the worktrees Tidewatch made, for repair
  src/restore.rs        D11 restore: pure plan (saved session + Disk -> one action per tab, each resumed agent run or typed; a YOLO tab resumes with its YOLO launch, a YOLO workflow run is a shell), repair_moved, resume_command (`launch_command` for the same profile plus the CLI's resume args, YOLO included), restore_workflow, codex_session_id
  src/resume_mode.rs    what a saved agent tab does at launch: `decide` (resume_agents x how it ended x YOLO -> run, type only with a reason, or a shell; a YOLO tab is always typed, never run, unless resuming is off), the notes for a typed tab, `exit_is_abnormal` (pure, table-tested)
  src/run_marker.rs     `.state/running`: written when a run starts, removed by its clean quit, so a launch that finds it knows the last run crashed or was killed
  src/session_id.rs     the random UUID passed to `--session-id` so the Reader finds a tab's transcript by name
  src/session.rs        .state/session.toml (D11): Session, SavedWindow, SavedTab (main, shell, agent, with `crashed` when the agent ended abnormally); load (bad file -> empty), atomic save
  src/worktree.rs       add (locked, agent/<name>), inspect, remove, list, repair, leftovers with their repository (shells out to git)
  src/notes.rs          per-tab review threads in .state/<repo-dir>/<tab>/notes.json: load, atomic save, background writer, PR import links, sent marks and note state (queued, sent, replied) (D4)
  src/note_paste.rs     the text pasted into an agent to hand it notes: path:line, quoted line, text, id, closing reply_to_note ask; control characters stripped (pure)
  src/end_block.rs      D13 parser: the Done / Needs you / Next block of an agent's last message (pure)
  src/tools.rs          Tools (PATH for git and gh), GhError and the gh runner shared by delivery and pr
  src/delivery.rs       D7 hand-off: DeliveryPlan (commits ahead of base, proposed PR text), deliver = git push -u then gh pr create
  src/pr.rs             D4 PR comments: find the open PR (gh pr view), pull review comments into notes threads (idempotent merge), post a note or reply
  src/pr_lines.rs       which lines of a PR diff take a review comment: git diff base...head parsed into hunk ranges (parse is pure, load shells out)
  src/rfc3339.rs        RFC 3339 timestamp to unix millis (pure, crate-private)
  src/transcript.rs     D12 Reader model: Entry/Role/ToolCall, per-CLI transcript parsers (claude, gemini, copilot, codex), incremental Tail
  src/transcript_locate.rs  where each CLI keeps a tab's session file: Locator by cwd and session id (claude folder encoding verified), gemini, copilot, codex (by folder and start time)
  src/history.rs        branch-scoped graph, ahead/behind, commit and working-tree files and diffs, file watcher, via kelp-core
  tests/worktree_roundtrip.rs  real throwaway repos: add, lock, dirty refusal, delete branch, moved repo
  tests/listener_http.rs  listener on a random port with a raw HTTP client: 401s, hooks, MCP, keep-alive, chunked, size caps
  examples/live_check.rs  by hand: run a real claude, gemini or copilot headless through the listener and print the events
  tests/push_guard.rs   real bare remote: plain, --no-verify, explicit-url and forced pushes refused, fetch works
  tests/common/mod.rs   test helpers: throwaway repos with a bare remote, a fake gh that logs its calls (the real gh never runs)
  tests/delivery_roundtrip.rs  real bare remote + fake gh: plan, base detection, push, PR args, each typed failure
  tests/pr_lines_git.rs real throwaway repo: the branch diff against the remote base says which lines take a comment
  tests/pr_gh.rs        fake gh: pr view, comments fetch and post argument shapes, GitHub-shaped JSON fixtures
  tests/fixtures/       redacted real transcripts (claude, gemini, copilot, codex), a hand-written codex function_call sample, GitHub-shaped PR comments
  tests/history_roundtrip.rs   real throwaway repos: worktree branch history, whole repo, ahead/behind, diffs, watcher
  tests/session_restore.rs     real repos + fake home: moved repo repaired then planned, removed worktree, missing session file
crates/tidewatch/       the egui app
  src/main.rs           workspace discovery, the single-instance claim, the window shape from the Plan, eframe start; `--toggle` exits before any window
  src/cli.rs            `tidewatch [--no-restore] [--toggle] [path]`, `--handoff <socket>` (how a restarting window launches its replacement) and `--from-stdusk <socket>` (how stdusk launches Tidewatch to take its panes) (pure)
  src/handoff.rs        a restart that keeps every shell and agent running, the pure half: the `Handover` layout (groups, tabs, worktrees, agents with their listener credential, workflow runs; TOML through stdusk's transport), `blocker` (a tab being made or removed, a stage starting or testing), `plan` (layout plus the order the ptys travel), `rebuild` (ptys paired with live entries by position, an adoption that fails becomes a fresh shell), `boot` (adopt, normal start or give way), `successor_command` (`open -n -a` from a bundle, the executable otherwise), `capture`, `release`, `adopt` and `confirm` over stdusk's `handoff` module
  src/handoff/tests.rs  the layout, pairing, rebuild, boot decisions, the wire with pipes, and a real pty round trip in one process
  src/app/restart.rs    the window's side of Restart: refuse when blocked, listen, mark closing, launch the new window, send, release and close; any failure leaves the window open with a notice
  src/from_stdusk.rs    Move tabs to Tidewatch, the receiving half: `plan` (panes + the groups the window holds -> a group per pane, new groups, the focused pane; pure, table tests), `name_of` (tab title), `receive` (a job thread: `Incoming::<Export>`, `pty::adopt` each pane, `handoff::confirm_terms`, then the repo of each folder; any failure before the ACK adopts nothing)
  src/from_stdusk/tests.rs  the plan tables, the names, and a real pty round trip in one process (a sender thread with stdusk's `send_session`, the shells still answer)
  src/app/stdusk.rs     the window's side: start the receive on a job thread, then one shell tab per pane in the group of its repo (existing group, new group, or the folder's own Other group), the focused pane active; a failure is a notice and stdusk keeps its window
  src/last_used.rs      writes the last-workspace file (repos of the window) when the window gains focus and when it closes (not under a screenshot or background run)
  src/single_instance.rs  startup before any window: Mode from the run kind (screenshot and a background run in the real home take no part), hand the path (or stdusk's socket) to a running window and exit, `--toggle` to every live window with an exit code, workspace key
  src/app.rs            the Tidewatch struct, `new` and the eframe::App loop, which only calls the frame steps below in order
  src/app/frame.rs      the frame's steps: drain finished jobs, drive background work, read shortcuts into a FrameIntent, apply what the frame asked for, finish
  src/app/strip.rs      the tab strip: repo chip (a menu: the window's repos when there are several, a dot when an agent in another one needs you, Open repo and Close repo), leftovers chip, Overview button, the tabs, new tab button, and at the right end the update chip and the settings button
  src/app/central.rs    the central panel: Overview, settings page or the workspace (stepper, end card, delivery card, terminal, panes), view toggles
  src/app/terminal.rs   the active tab's terminal area: shell grid and input, status messages, the exit notice, the Reader or its note
  src/app/panes.rs      History and Diff panes: one view per tab, finished pane jobs, notes saving, double-click checkout
  src/app/tabs.rs       new tab dialog state, opening and closing tabs, the close dialog's answer, finished worktree jobs, tab actions
  src/app/agents.rs     profile and probe results, listener messages to tabs, findings to notes, per-frame agent status and the next wake-up
  src/app/flow.rs       workflow runs, session restore and the delivery flow, driven once per frame
  src/app/settings.rs   opening and saving settings, window preferences, shell events (hotkey, menu bar, other launches), the settings page and its actions
  src/app/palette.rs    the command palette in the app: open and toggle it, what it lists (tabs of the repo on show, other repos, profiles, workflows in a repo), run what it picked
  src/app/repos.rs      the repo menu's items: the folder picker (rfd on a job thread, one at a time) and closing the repo on show tab by tab through the close path
  src/app/workspaces.rs the repos of the window: the one on show, the views the session file is written from, what the next launch reopens, adding a repo when a launch hands over its folder (git look-up on a job thread; `add_workspace` is the part tabs from stdusk reuse)
  src/app/overlays.rs   the toast and its pill, and the dialogs drawn over the frame (close, leftovers, delivery, new tab, trust, notice)
  src/app/dev.rs        the TIDEWATCH_DEV_* drivers that need the whole app: start an agent, resume, press keys, leftovers dialog, pick and note
  src/commands.rs       the one command table: id, title, group, the chord on macOS and on Linux (none for a palette-only command such as build-workflow), the Command it runs; consume (most specific chord first), press_event, Gate (which chord works over which screen); key handling, tooltips and hints read it
  src/commands/rows.rs  the table as rows for the Shortcuts page, and (tests only) the markdown that docs/shortcuts.md must equal
  src/commands/tests.rs distinct chords per platform, no shadowing, every Action with a chord is in the table, macOS chords as shipped, Linux never takes a bare Ctrl letter, the gate
  src/repo_close.rs     the close-repo sequence: which tab next, wait for a removal, stop when a tab stays (pure)
  src/palette.rs        the palette's data: items from the table plus tabs, profiles and workflows (a row can be disabled with its reason), Kelp's fuzzy matcher, filter, selection (pure)
  src/palette_view.rs   draws the palette: search field, rows with matched letters and the chord (a disabled row dim with its reason, Enter skips it), arrows, Enter, Esc
  src/build_workflow.rs Build workflow from this session, the app half: when the command may run (`availability`, with the reason), the wait for the agent's answer (`Asked`, `Step`), the fallback that reads a `toml` block from the Reader's rows after the prompt, `workflow_from_toml` (pure)
  src/app/build_workflow.rs  runs the command (paste the prompt, queue Enter, wait), drives each wait per frame, opens a proposal as a draft in Settings, Workflows
  src/shortcuts_view.rs the Shortcuts page of Settings: the table for this platform as key caps
  src/tabs.rs           tab list and its Action reducer (pure, generic over the pane); every tab is in a group, one per repo of the window, with its own pinned main tab, and the visible group is the one the active tab is in
  src/test_exec.rs      test only: a fixture script written through cp, so no forking test thread holds it open (Linux ETXTBSY)
  src/pane.rs           what a tab holds: shell or agent (with its AgentRun and how it was launched), creating, removing, failed; opening results, close policy (pure, generic)
  src/jobs.rs           background thread runner: result channel plus a repaint
  src/pty.rs            starts the ptys: a shell in a folder (optionally with a note printed first), the user's shell running an agent's script, a shell with the agent's command typed at its first prompt and not run, or adopts one another window sent
  src/worktree_jobs.rs  open a tab (worktree, listener registration, folder trust and prepared agent launch), inspect and remove (and revoke its trust entries) as jobs
  src/agent_run.rs      one agent tab's live state: StatusTracker, terminal snapshot in, status change out, last MCP report (pure)
  src/agent_events.rs   starts the Tidewatch listener once (bind failure: logged, status falls back to layers 2 and 3) or on the old window's port for a restart, drains it, findings -> notes
  src/agent_frame.rs    per-frame agent work: route listener messages to tabs, feed terminal signals, notify, say when the next frame is due (generic, pure)
  src/status_view.rs    status -> tone, label, color; CLI logo mapping; the tab's top-edge status bar (animated only while working)
  src/notify.rs         desktop notifications (osascript, notify-send) off the UI thread, per-tab per-kind rate limit, window-away rule
  src/overview.rs       Overview rows and groups from the agent tabs (needs you first), the workflow stage of a row, elapsed format (pure)
  src/overview_view.rs  the Overview screen (Cmd+0) with the sleeping mascot when no agent runs, and the tab strip's icon button with a needs-you badge
  src/catalog.rs        loaded profiles, the agent table built from them, which CLIs were found and their versions
  src/agent_jobs.rs     load or seed profiles.toml and probe each agent CLI as jobs
  src/new_tab.rs        new-tab dialog state: the steps (what to start, agent, workflow), the filter, the permission level and YOLO gating, the request each step turns into, the warnings (pure)
  src/new_tab_workflow.rs  the dialog's workflow side: list order, missing agents, the request (no task: the tab asks for it) (pure)
  src/new_tab_dialog.rs the dialog's frame and first screen: the Agent and Workflow cards and the plain shell; A, W, S, arrows, Enter, Esc
  src/new_tab_pick.rs   the agent picker (profiles grouped by CLI, the picked one open with its five levels) and the workflow picker
  src/screen_kit.rs     headings, key caps and CLI badges shared by the dialog, the agent tab's empty state, the workflow start card and the welcome screen
  src/repo_base.rs      the branch a new worktree starts from (what the repo has checked out), looked up on a job thread
  src/profile_editor.rs profile editor state: drafts grouped by agent, text in and out of fields, validation, profiles to save, revert (pure)
  src/settings.rs       what the settings view holds: the profile, workflow and window editors and which page shows (pure)
  src/window_settings.rs  the Window section's state: quake and menu-bar switches and the Panels choice apply at once, the hotkey is recorded from a key press (`recording`), checked at once and applied when it parses (pure)
  src/updates.rs        kelp-ui's `Updater` with Tidewatch's repo and cask, the two switches, the status line (pure), saving them; never disabled, so Linux gets the notify-only pill; once an update is installed the Restart button is drawn here (kelp-ui's cannot hand terminals over) and `take_restart` tells the app
  src/updates_view.rs   draws the Updates section of General: status, Check now, the pill, the switches (the install one only in install mode)
  src/window_view.rs    draws the Window section: switches and the hotkey key-cap recorder (click, press keys; Esc cancels) with its problem or registration status
  src/settings_view.rs  the settings view: a sidebar of six pages and the page that shows (replaces the terminal area; Cmd+, or the gear; Esc closes); files shown only in a footer line or a tooltip
  src/brand.rs          the Night tide palette every view reads (ground, surfaces, text, teal, green, orange, red, the status inks that differ in lightness), the terminal theme (One Half Dark on the brand ground, ANSI palette untouched), the chrome colors handed to stdusk's `colors::set_chrome` (teal accent, red, green) and the teal egui visuals; no view hard-codes a color or reads `colors::accent()`
  src/segmented.rs      the segmented control of the window chrome (one bordered container, the active segment teal-tinted): drawn by the History card header and the agent status line (pure geometry, drawn)
  src/status_bar.rs     the 30 px bar along the bottom of the window (hidden on the welcome screen): agents working and needing you, the agent tab's `branch from base`, 3 or 4 key hints per screen read from the command table (pure counts and hint choice, drawn); `app/bar.rs` gathers its input
  src/update_chip.rs    the update chip: dark fill, 1 px border, teal dot, `<version> available` with Update (or View when notify-only), `Installing <version>`, `<version> ready` with Restart (pure spec, drawn); never Kelp's green
  src/settings_ui.rs    the shared pieces of every Settings page (re-exports `brand`): page header, section and card, rows with a control, switch, buttons, key caps
  src/settings_icons.rs the six sidebar icons drawn with the painter
  src/general_view.rs   the General page: Startup (restore, the resume mode control, open at login), Window and Updates cards
  src/login_item.rs     the Open at login row of Startup: reads the real status when the page shows and at most once a second after, never stores it; a click says what was asked, the app registers (macOS only; the row is hidden elsewhere)
  src/house_style_view.rs  the House style page: the file's path, Open in editor, Reset to default (two clicks)
  src/profiles_view.rs  the Agents and profiles page: profiles grouped by agent (version found or not installed), the editor card with pills, level cards, an Advanced disclosure and the save bar
  src/settings_ui/editor.rs  the pieces of a page that edits a list: list rows, editor card and header, option cards, save bar (Save, Revert)
  src/trust_glue.rs     D14 host: installed CLIs and their trust state, the consent state and its jobs, the grant handed to new tabs and worktree removal
  src/trust_view.rs     D14 wording per CLI (pure), the first-launch consent dialog and the Folder trust settings page
  src/workflow_editor.rs  workflow editor state: drafts per workflow and stage, problems found as you type, workflows to save, a proposed draft marked with its tab, revert to the saved workflows (pure)
  src/workflow_editor_view.rs  the Workflows page: the list of workflows, the editor card with stage pills, the stage's fields, problems under them, the note on a proposed draft and the save bar (Save, Revert)
  src/leftovers.rs      the leftover worktrees dialog's rows, what each would lose, the removals Clean up and Discard stand for (pure)
  src/leftover_jobs.rs  count what a leftover would lose, remove one (and its trust entries) as jobs
  src/leftover_glue.rs  Host: the leftover list, the chip, the dialog and its jobs
  src/leftover_view.rs  the chip in the tab strip and the dialog (Clean up for the clean ones, Discard for a dirty one)
  src/close_dialog.rs   keep/delete branch dialog: choices and wording (pure) plus the modal
  src/layout.rs         terminal/panel split (side by side: terminal left ~58% with a draggable divider, History a card on the right with a 10 px margin; `CHROME_H` 48 is the card header and the agent status line; below: terminal over History and Diff), the left column that a file's diff takes over, grid size, tab width (pure)
  src/tab_view.rs       per-tab pane state: loaded history, selection, open diff, notes, watcher, the opened commit (one at a time), the file view and the last opened file, line counts per file; wants -> jobs -> answers (stale ones dropped)
  src/pane_jobs.rs      history, status, files, diff, notes, watcher and checkout as jobs
  src/history_pane.rs   History pane: side by side a raised card (12 px corners, 1 px border) with a 48 px header (branch in mono, `N ahead of <base>` and `N behind` pills, one This branch / Whole repo segmented control) and kelp-ui's embedded graph (`GraphStyle::embedded(brand::TEAL)`: flat list, inline ref chips, teal lanes) with the open commit's card in the gap the graph opens under its row (`expanded`, `expanded_rect`, `scroll_to`); Below keeps the flat pane with the HISTORY caption and This worktree / Whole repo chips and Kelp's default graph look
  src/commit_files.rs   the open commit's files as a rounded teal card around its row and the graph's gap: meta line (short id, author, file count, +/-), per file a kind letter, dimmed folder and name, +/- counts, one scroll inside past the room (pure gap height, text inset, sums; drawing)
  src/file_view.rs      the side layout's file view in the terminal's place: Terminal chip (profile and state, goes back), path, `in <sha>`, Unified/Split, toolbar with Send all and Deliver when idle, PR row, notes strip, kelp-ui diff body
  src/file_nav.rs       the previous or next file or tree row from the keyboard (pure index steps)
  src/diff_pane.rs      Diff pane (the Below layout; its pieces also serve the file view): header (Send all open, Deliver when idle), file list, toolbar (PR chip, Pull PR comments), notes strip, kelp-ui diff body with notes
  src/note_actions.rs   kelp-ui DiffAction -> the tab's notes (pure)
  src/checkout.rs       double-click checkout rule: main tab only, clean tree only (pure plan, git outcome)
  src/toast.rs          one timed message, fade on egui's clock (pure)
  src/fonts.rs          stdusk font set with kelp-ui's faces and semibold family merged in (kelp_ui::fonts::extend), CJK fallback
  src/repo.rs           git root and label; Workspace = Repo or the Other group
  src/session_save.rs   the open tabs of one repo of the window (a tab group) as a SavedWindow (main, shell, agent with profile, level, session id, stages file; the active tab, focus mode) and a fingerprint of the same facts (pure, generic over the pane)
  src/session_writer.rs debounced writer thread for session.toml (Debounce is pure), synchronous flush for the quit path
  src/restore_jobs.rs   startup job, one per repo of the window (repair a moved repo, plan the saved window, scan leftovers owned by no saved or open tab) and the reopen job (fresh token, prepare_resume, folder trust)
  src/restore_glue.rs   Host: carry each repo's plan out in its tab group in saved order (shell, shell plus note, resume agent or type its command, resume workflow), active tab and focus, the settled flag the saver waits for (generic over the pty)
  src/mascot.rs         the mascot (a shoggoth in a porcelain smiley mask): `Pose` Peek (welcome), Asleep (empty Overview), Caught (typed resume note); art in `assets/mascot/` (`mascot-<pose>-base.svg`, `eye-<kind>.svg`, `mascot.json` with each pose's viewBox and eye boxes) bundled with `include_bytes!`, rasterized with resvg once per pixel height (cached in egui's context memory per pose, rebuilt only when the size or pixels_per_point changes), drawn as the base then each real eye; never redraw or edit the art
  src/mascot_motion.rs  the mascot's motion as pure functions of time (bob 7 units over 3.2 s, asleep breathes instead, each eye blinks every 4 to 7 s and darts 2 to 3 units on its own schedule, the mask never moves against the body) and the repaint gate `should_animate(on_screen, focused, visible)` with `next_repaint` (33 ms, `None` when still)
  src/notice.rs         a message that stays until dismissed (what a restore could not bring back)
  src/tabs/groups.rs    taking a whole group out of the tab list (`remove_group`), the groups after it move down
  src/session_glue.rs   Saver: once per frame, fingerprint changed -> queue a save (a closed repo's window rides with the next one); never under the screenshot harness or while a restore is under way; flush on quit
  src/hotkey.rs         the quake hotkey string (`Ctrl+Alt+Grave`, `F13`) to modifiers and a key, strict with a named Problem for each mistake (pure); `spell` turns a pressed key into that string and `cap_labels` the text into key caps (pure)
  src/shell_plan.rs     the background gate: Plan from the settings and Env (background, Wayland, tray support); background means no hotkey, no menu-bar item, no activation, a plain window (pure)
  src/quake.rs          the drop-down: pure show/hide state (monitor-size wait, hide on blur, 120 ms wake while hidden), geometry (half height, 2 point sliver, alpha 0), `present` turns it into viewport commands
  src/macos.rs          objc2 calls (activation policy, Spaces, window alpha, screen lookups, the SMAppService login item: status mapped from the raw value, register, unregister, open System Settings) with no-op stubs off macOS
  src/os.rs             the `Os` seam the shell talks through: RealOs (global-hotkey 0.8, tray, macos.rs) and a recorder for tests
  src/tray.rs           the menu-bar item (macOS only): Show / Hide, Overview, Settings, Quit; clicks reach the UI through a handler that wakes it
  src/shell.rs          Shell: per frame, other launches (open, toggle), hotkey presses, menu-bar clicks and live settings become viewport commands and OS calls, all through the Plan; hotkey status for settings
  src/dev_session.rs    TIDEWATCH_DEV_QUIT / _RESTART / _SELECT / _SELECT_AFTER / _SHOT / _SHOT_AFTER / _SAVE_DEBOUNCE_MS switches: close the window through the close request, select a tab, a screenshot that keeps running, a long debounce
  src/dev_screenshot.rs TIDEWATCH_SCREENSHOT harness
  src/dev_worktree.rs   TIDEWATCH_DEV_WORKTREE switch: open a worktree tab at startup for captures
  src/dev_panes.rs      TIDEWATCH_DEV_SCOPE / _PICK / _OPEN / _PRESS / _NOTE switches: graph scope, selected row, open a commit or a file, press table commands, a note for captures
  src/dev_view.rs       TIDEWATCH_DEV_OVERVIEW / _FOCUS / _STATUS_LOG / _KEYS switches: start on a view, log status changes, answer a prompt
  src/dev_agent.rs      TIDEWATCH_DEV_AGENT / _WORKFLOW / _RESUME / _DIALOG / _SETTINGS / _PALETTE / _PROPOSE / _TASK / _START / _LEVEL switches: open an agent or workflow tab, resume a saved run (through the restore path), open the dialog on a screen, the editor, the palette or a proposed workflow
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
  src/agent_ready.rs    an agent tab before its first message: the "<Profile> is ready" empty state and the composer (Enter sends, Shift+Enter new line)
  src/agent_status_line.rs  the line over an agent tab, `CHROME_H` tall: Ready or its status, profile, CLI, level, and the Reader | Terminal segmented control (pure line, drawn)
  src/reader_glue.rs    seams: start feeds, drain, Cmd+Shift+T toggle, draw in the terminal area, type replies and answers into the pty
  src/dev_reader.rs     TIDEWATCH_DEV_READER / _READER_KEY / _ANSWER / _REPLY switches
  src/end_block_view.rs the Done / Needs you / Next card on the tab (expandable) and its one-line form for the Overview
  src/workflow_run.rs   one workflow tab's run: the Runner, Exec (launch a stage, run the tests) out, events in, generation per launch, stages.json saved after every event, held status, ask (pure)
  src/workflow_ask.rs   the question a run asks and its buttons per reason (Approve, Retry, Reject with feedback, Stop), button to event (pure)
  src/stepper.rs        the stepper's data: a pill per stage with state and loop counter, the gate, a stage's summary and findings, the position label (pure)
  src/stepper_view.rs   draws the header of a workflow tab: stepper, a clicked stage, the ask card, the stopped line
  src/workflow_start_view.rs  the card a workflow tab shows until its run starts: description, task box, a card per stage, Start run (Cmd+Enter)
  src/welcome.rs        the welcome screen's Recent list: the last window's repos, then the session's, each once, with its open tabs (pure)
  src/welcome_view.rs   draws the welcome screen of a window with no repo: the peeking mascot, Open repo (Cmd+O), Recent, the command line hint
  src/workflow_jobs.rs  open (worktree, base, test command and timeout), launch a stage (fresh token, prepare, divider), run the tests as jobs
  src/workflow_glue.rs  Host: open a workflow or saved-run tab, carry out what a run asks, apply finished jobs, drop a replaced agent's messages (generic over the pty)
docs/                   concept mockup, research reports, shortcuts.md (generated from the command table, checked by a test)
site/                   the GitHub Pages showcase (tidewatch.hoboware.dev): index.html, 404.html, style.css, mascot/ (the animated mascot SVGs), shot-*.png app screenshots, icon.svg, favicon, apple-touch icon, tidewatch-og.png, CNAME, robots.txt, sitemap.xml
packaging/              Info.plist, reference Homebrew cask (tidewatch.rb), .desktop file, release README
scripts/                bundle-macos.sh (universal ad-hoc signed Tidewatch.app + zip), package-linux.sh (tarball + AppImage), make-icon.sh (icon from assets/icon.svg)
assets/                 icon.svg (the K mark: ripple prompt and wave, Night tide colors), AppIcon.icon (the same mark as Liquid Glass layers: sun, prompt, ripples, wave) compiled to Assets.car, Tidewatch.icns (older macOS), icon-512.png, tray.svg and tray.png (the menu-bar template icon)
.github/workflows/      ci.yml (fmt, clippy, test on macOS and Ubuntu), release.yml (v* tags: bundle, package, GitHub Release, tap behind PUBLISH_TAP), pages.yml (site/** on main: deploy to GitHub Pages)
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
scripts/make-icon.sh                   # rebuild the icon files from assets/icon.svg and assets/tray.svg (inkscape, pngquant, oxipng)
TIDEWATCH_SCREENSHOT=/tmp/x.png cargo run -p tidewatch -- <repo>   # render, save PNG, quit (saves no session)
cargo run -p tidewatch -- --no-restore <repo>   # start with just the main tab; the last session is not read
tidewatch --toggle                     # show or hide every running window, exit 1 if none answers (the way in on Wayland)
```

One window for the app: `tidewatch <path>` hands the path to the running window and exits (no second window); the window adds a folder it does not hold as a repo (its own tab group, main shell and saved tabs, chip menu or the next/previous repo chords to switch) and brings forward one it holds. A launch with no path reopens the repos of the last window (`dir`, `also`). To check it in the background: start a `TIDEWATCH_BACKGROUND=1` run with a short throwaway `TIDEWATCH_HOME` and `TIDEWATCH_DEV_QUIT`, wait for `.state/instances/tidewatch.sock`, run `tidewatch <other repo>` with the same home (it hands over and exits at once, so only do it once the socket exists), then read both repos in `session.toml`. A launch with no path and no git repo around it (Finder, or the `open -n` of the update Restart button, both start in `/`) opens the last-used folder; a window that is closing answers `closing` and the new launch waits for its socket, so a restart never hands its folder to a dying window. Restart to update keeps every shell and agent running: the old window sends its layout and ptys to the new one (`--handoff <socket>`, `handoff.rs`), the new one claims the app socket only after it confirmed, and on any failure the old window stays open. The pill needs a foreground window, so check it with `TIDEWATCH_DEV_RESTART=<seconds>` in a `TIDEWATCH_BACKGROUND=1` run with a short throwaway `TIDEWATCH_HOME` (the new window is started directly, inherits the environment except the dev switches that open or restart tabs, and `TIDEWATCH_DEV_QUIT` ends it): read the pty children's pids before and after (same pids, parent 1) and post a hook to the old URL and token (200, and `TIDEWATCH_DEV_STATUS_LOG=1` shows it); kill everything you started. Moving tabs from stdusk (its "Move tabs to Tidewatch") launches `tidewatch --from-stdusk <socket>`: with a window already running (same home) it is forwarded as `adopt-stdusk <socket>` (answered `ok` or `closing` like the others) and the launch exits; with none, or after a `closing` window is gone, that launch becomes the window and receives at startup. The window receives on a job thread (`from_stdusk.rs`) and makes each pane a shell tab in the group of the git repo its folder is in (an existing group, else a new one, else the folder's own Other group), the pane that had focus active; on a failure it adopts nothing, shows a notice, and stdusk keeps its window. To check it in the background: start a `TIDEWATCH_BACKGROUND=1` run with a short throwaway `TIDEWATCH_HOME`, `TIDEWATCH_DEV_STATUS_LOG=1` and a `TIDEWATCH_DEV_QUIT` long enough, in a throwaway repo, then run `cargo run -p tidewatch --example fake_stdusk -- <built tidewatch> <folder>...` (no `TIDEWATCH_BACKGROUND`, same `TIDEWATCH_HOME`; it spawns a shell per folder, the last holds focus, sends them and prints their pids and `acked`): the pids stay alive with parent 1 until the window quits, the status log shows `shell adopt-stdusk <socket>`, and `session.toml` (via `TIDEWATCH_DEV_QUIT`) has a shell tab in the right group; add `TIDEWATCH_DEV_SHOT=<png>` to see it. Kill everything you started. A background run from `/` with `last-workspace.toml` seeded in a throwaway `TIDEWATCH_HOME` shows the folder pick (read the window's `dir` in `session.toml`). With nothing remembered the window shows the welcome screen (Open repo, Cmd+O, and the repos of an old `session.toml`) instead of a shell in `/`; opening a repo from it replaces it, and it saves nothing while it shows. The socket is `<home>/.tidewatch/.state/instances/tidewatch.sock`, so a `TIDEWATCH_HOME` run never meets the real one, but a socket path must stay under about 100 bytes: use a short throwaway home such as `/tmp/x`, not one under macOS `$TMPDIR`. Quake is on by default (`Ctrl+Alt+Grave`, Settings, General, Window). **Never live-test quake show/hide, the hotkey or the menu-bar item from an agent run: they take focus. A background run (below) gets none of them by construction (`shell_plan::Plan`).** To see a background instance answer a launch without any window action: start it with `TIDEWATCH_DEV_STATUS_LOG=1`, then run `tidewatch --toggle` or `tidewatch <repo>` with the same `TIDEWATCH_HOME` and read its stderr (`shell toggle`, `shell open <repo>`).
Screenshot runs open in the background: no Dock icon, never take keyboard focus from the
developer's active window. `TIDEWATCH_BACKGROUND=1` does the same for any other dev or live-check
run. Agents must use one of the two for every window they open.
`TIDEWATCH_SCREENSHOT_WAIT=<seconds>` waits longer before the capture (default 1.5s).
`TIDEWATCH_DEV_WORKTREE=open|ask|delete` opens one worktree tab at startup; `ask` also shows its close
dialog, `delete` closes it with "Delete branch". Use a throwaway repo, and a temp `HOME` (with
`ZDOTDIR` set to it) so worktrees land in a temp `~/.tidewatch`, never in the real home.
`TIDEWATCH_DEV_SCOPE=whole` starts the graph on Whole repo, `TIDEWATCH_DEV_PICK=<row>` selects a graph row,
`TIDEWATCH_DEV_OPEN=commit|file|file:<n>` opens the selected row's files in the tree (and file n, 0 first, in the file view), `TIDEWATCH_DEV_PRESS=<command id>@<seconds>,...` presses those table commands (`file-view`, `back-to-terminal`, `next-file`, `prev-commit`...) through the real key path (`build-workflow` has no chord, so it runs directly), `TIDEWATCH_DEV_NOTE=<text>` writes a note on the first added line of the first diff shown (saved like a real one).
Notes live in `<home>/.tidewatch/.state/<repo-dir>/<tab>/notes.json`; the main tab's key is `main`.
`TIDEWATCH_HOME=<dir>` keeps `.tidewatch` and `.config/tidewatch` in `<dir>` while `HOME` stays real, so agent
CLIs keep their login. `TIDEWATCH_DEV_AGENT=<profile id>` opens that profile in a new tab at startup, ready for its first
message (level from `TIDEWATCH_DEV_LEVEL`; a level the group refuses, such as YOLO in Other, is reported on stderr and
nothing starts). `TIDEWATCH_DEV_TASK=<text>` types into that tab's composer (or a workflow tab's task box) and
`TIDEWATCH_DEV_START=1` also sends it (or presses Start run). `TIDEWATCH_DEV_DIALOG=choose|agent|workflow|<profile id>|workflow:<id>|1`
opens the new-tab dialog on its first screen, the agent list, the workflow list or on that pick, and
`TIDEWATCH_DEV_SETTINGS=1` the profile editor, for captures (use `TIDEWATCH_SCREENSHOT_WAIT=6`: profiles load and
versions are probed in the background). Check an agent tab with `ps -o args= -p $(pgrep -n claude)`.
`TIDEWATCH_DEV_GH_DIR=<dir>` puts `<dir>` first on the PATH `git` and `gh` run with, for a fake `gh` script (the real GitHub API must
never be reached from a check). `TIDEWATCH_DEV_AUTO=1` turns Deliver when idle on, `TIDEWATCH_DEV_SEND=one|all` sends the first or every
unsent note, `TIDEWATCH_DEV_DELIVER=open|approve` opens the delivery dialog (and presses Push and open PR), `TIDEWATCH_DEV_PR=pull|ask|post`
pulls the PR's comments or opens (and confirms) the post dialog for the first note, each once, as soon as the active tab is ready.
`TIDEWATCH_DEV_OVERVIEW=1` and `TIDEWATCH_DEV_FOCUS=1` start on the Overview or in focus mode; `TIDEWATCH_DEV_STATUS_LOG=1`
prints every hook and status change to stderr with a wall-clock time in ms; `TIDEWATCH_DEV_KEYS='\e[B\r'` (after
`TIDEWATCH_DEV_KEYS_AFTER` seconds, default 8) types into the newest agent tab once, for Claude's folder-trust prompt.
`TIDEWATCH_DEV_WORKFLOW=<id>` opens a workflow tab that waits for its task (add `TIDEWATCH_DEV_TASK` and `TIDEWATCH_DEV_START=1` to run it),
`TIDEWATCH_DEV_RESUME=<worktree name>` opens the tab of a saved run (`.state/<repo-dir>/<name>/stages.json`) and carries on, `TIDEWATCH_DEV_ASK=approve|retry|stop|reject:<reason>`
answers the next question a run asks, `TIDEWATCH_DEV_OPEN_STAGE=<n>` opens stage n under the stepper, `TIDEWATCH_DEV_SETTINGS=workflows` opens the workflow editor, `TIDEWATCH_DEV_PROPOSE=<toml file>` opens that workflow (a `workflows.toml` table or one bare workflow) as an unsaved draft marked as coming from a stand-in agent (use `TIDEWATCH_DEV_TRUST=no` so the first-launch dialog is not over it), `TIDEWATCH_DEV_SETTINGS=trust` the Folder trust page, `TIDEWATCH_DEV_SETTINGS=general` the General page, `TIDEWATCH_DEV_SETTINGS=shortcuts` the Shortcuts page, `TIDEWATCH_DEV_SETTINGS=house` the House style page, `TIDEWATCH_DEV_SETTINGS=dirty` the profiles with an unsaved edit (the save bar), `TIDEWATCH_DEV_PALETTE=<query>|1` opens the command palette filled with that query (use `TIDEWATCH_SCREENSHOT_WAIT=7`: profiles load in the background), `TIDEWATCH_DEV_STATUS_LOG=1` also prints each command a key ran, `TIDEWATCH_FAKE_UPDATE=<version>` shows an update (pill and Settings line; the run makes no check and installs nothing; with a cask install a click on the pill would still run `brew upgrade`, so never click it), `TIDEWATCH_DEV_TRUST=yes|no|undo` answers the first-launch folder trust question once (a fresh `TIDEWATCH_HOME` otherwise shows the dialog; nothing is written until yes).
`TIDEWATCH_DEV_READER=raw` starts agent tabs on the raw terminal, `TIDEWATCH_DEV_READER_KEY=<s>` presses Cmd+Shift+T after that many
seconds through the real key path, `TIDEWATCH_DEV_ANSWER=allow|deny` clicks a permission button once, `TIDEWATCH_DEV_REPLY=<text>`
(`_REPLY_AFTER=<s>`) sends a reply from the Reader's box. Run Claude checks with `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1` when the
shell that starts Tidewatch is itself a Claude session: the child otherwise writes no transcript and the Reader falls back to the terminal.
Session: every change is saved to `<home>/.tidewatch/.state/session.toml` and the next launch restores it (Settings, General, turns that off). To check it in the background: `TIDEWATCH_DEV_QUIT=<s>` closes the window through the close request (the quit-time save runs), `TIDEWATCH_DEV_SAVE_DEBOUNCE_MS=600000` leaves that quit-time write as the only one, `TIDEWATCH_DEV_SELECT=<n>` (`_SELECT_AFTER`) selects tab n, `TIDEWATCH_DEV_SHOT=<png>` (`_SHOT_AFTER`) saves a screenshot without quitting (the screenshot harness itself saves nothing), `TIDEWATCH_DEV_LEFTOVERS=open|clean` opens the leftover worktrees dialog or presses Clean up. The last run's end decides a resume: `.state/running` stays when a run is killed, so a relaunch types the resume command instead of running it (`resume_agents` in `settings.toml`: auto, prefill, off). A YOLO tab is always typed, whatever the mode and however the run ended (Off keeps it a plain shell): it resumes with the full YOLO launch of its profile and waits for Enter. A YOLO tab outside a worktree is still refused by `launch_command` and reopens as a shell with that reason. Relaunch with the same `TIDEWATCH_HOME` and repo; `TIDEWATCH_DEV_REPLY` asks a resumed agent a question from the Reader.
Keys (macOS; Linux uses Ctrl+Shift or Alt, the table is `commands.rs` and `docs/shortcuts.md`, and a change goes in the table first): Cmd+Shift+P the command palette (its Build workflow from this session has no chord: on an idle agent tab it pastes a prompt that asks the agent to turn the session into a reusable workflow, and the answer, a `propose_workflow` call or a `toml` block in its reply, opens as an unsaved draft in Settings, Workflows; to check it without a real agent, put a stand-in `claude` first on `PATH` that posts hooks and calls the tool from its `--settings` and `--mcp-config` files, and run with `TIDEWATCH_DEV_PRESS=build-workflow@<seconds>`), Cmd+T new tab (the dialog: Agent, Workflow or Plain shell with A, W or S; the agent list picks a profile and, with Left and Right, its permission level; Enter opens the tab, Esc goes back or closes; the task is typed in the tab, not the dialog), Cmd+O open a repo, Cmd+W
close the active tab (not the pinned main tab; a worktree tab asks keep/delete branch first), Cmd+1..9 select (in the repo on show), Cmd+Ctrl+] / [ next and previous repo,
Cmd+Shift+T flip the active agent tab between the Reader and the raw terminal, Cmd+, the profile editor, Cmd+0 the Overview (Esc or Cmd+0 closes it), Cmd+. focus mode (hides History and Diff). Panels (Settings, General, Window; `[window] panels` in settings.toml, `side` by default or `below`): side by side puts History on the right at full height, a click on a commit opens its files under its row and a click on a file shows its diff (notes, Unified/Split, Send, Post to PR) in the terminal's place under a Terminal chip, with the agent still running; Below keeps the old terminal over History and Diff. File view keys (only while the file's diff has the keyboard and no note is being typed, so the terminal and text fields keep Esc, arrows and brackets): Esc back to the terminal, Cmd+J (Linux Ctrl+Shift+J) flips between the terminal and the last opened file, Up and Down the previous or next file of the open commit, [ and ] the previous or next row of the tree.
The sibling crates are git dependencies pinned by commit (`rev`), not by tag (PLAN.md,
Deviations log). To pick up a sibling change, merge it there, then bump `rev`.

Check the real exit code of a build (`cargo build 2>build.log; echo $?`), never a piped
`tail`. Verify UI changes with the screenshot switch instead of asking for a manual check.

## Performance rules

- Idle CPU stays at 0%: repaint only on input, pty output or finished background work.
  Animations run only while something is happening. The mascot animates only while it is drawn in a focused, visible window and asks for a repaint every 33 ms only then (`mascot_motion::should_animate`); otherwise it shows its still frame and asks for nothing.
- Never block the UI thread on git, the network, disk-heavy work or a child process.
- The graph draws only visible rows; per-row work is O(visible lanes).
- Many agents at once is the normal case: a hidden tab costs no paint time.

## Colors

Colors come from `brand.rs`: accent fills and strokes are `brand::TEAL`, accent text on dark is `brand::TEAL_TEXT`, statuses are `brand::WORKING`, `NEEDS`, `DONE` and `FAILED` (a test keeps them apart in lightness, so color-blind readers tell them apart), errors `brand::RED`, warnings `brand::ORANGE`. Never use `colors::accent()`, `colors::yellow()` or Kelp's `theme::accent()` in Tidewatch code: stdusk's accent is the terminal's ANSI blue and Kelp's is green. Kelp's own widgets keep Kelp's colors (diff hunk header and notes, the focus ring, the Below layout's graph); do not copy them. The side layout's graph takes `brand::TEAL` as its accent through `GraphStyle::embedded`.

## Showcase page and releases

Releases follow `packaging/README.md` (bump `Cargo.toml`, tag `v<version>`). The tap job stays off until the
repository variable `PUBLISH_TAP` is `true`, which waits for the repo to go public.

The showcase page is `site/` (plain HTML and CSS, no build step), deployed by `pages.yml` to `tidewatch.hoboware.dev`
whenever `site/**` changes on `main`. Same upkeep as Kelp: a user-visible change updates `README.md` and the page in the
same branch, removed behavior comes off, and the page only describes merged features. The install section says
"not released yet, build from source" until the first release; then it switches to the Homebrew cask and the Linux
downloads. Screenshots come from the screenshot switch on a throwaway demo repo with neutral names (never a private
or work repo, no real home path or user name in any pixel), at 2560x1600, resized to 2200x1375 and compressed
(`pngquant --force --speed 1 --strip 64`, then `oxipng -o 6 --strip safe`, PSNR above 40 dB). A shell prompt shows
user and host: use a throwaway `HOME`, `ZDOTDIR` with a neutral `PROMPT`, and `GIT_PAGER=cat`. Agent tabs without a
real agent: a stand-in `claude` script first on `PATH` that posts the hook events to the listener from its
`--settings` JSON and a hand-written transcript under the throwaway `HOME`'s `.claude/projects`, restored through a
seeded `session.toml`. The og image is a 1200x630 page rendered in headless Chromium. Check the page headless only, at
1280 and 390 wide, with no horizontal scroll.

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
