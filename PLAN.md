# Tidewatch Plan

**Status: REVIEWED, decisions closed 2026-10-12** (the owner delegated the calls; every recommendation was accepted). Drafted 2026-10-05. Branch `main`. Concept mockup: `docs/concept.html`.
Research: `docs/research/stdusk-recon.md`, `docs/research/kelp-recon.md`,
`docs/research/agents-worktrees.md`. Work protocol: `.agents/rules/workflow.md`.

## 1. Summary

Tidewatch is a native app for macOS and Linux for running AI coding agents. It joins three things that
today live in two apps:

| From | What Tidewatch takes |
| --- | --- |
| stdusk | Terminal engine, tab strip, splits, quake window, agent detection and resume, restart handoff, settings |
| Kelp | Commit graph, diff view (unified/split, word highlight), review threads, worktree ops, self-update |
| New | Worktree per tab, profiles with permission levels, workflows (work, review, refactor), agent status, Overview, focus mode |

Closed product decisions (from the concept round, not revisited here):

| Topic | Decision |
| --- | --- |
| Name | Tidewatch |
| Repo | `Hobo-Ware/tidewatch`, its own repo |
| Worktrees | under `~/.tidewatch/` (exact layout in D9) |
| Layout | B: terminal full width on top, graph + diff below, plus focus mode |
| Segregation | one repo per view, never a combined tree; "Other" group for folders that are not git |
| Tabs | look exactly like stdusk |
| Git actions in the UI | none at first, except double-click to check out |
| Code sharing | library crates inside the sibling repos, used as git dependencies |

Scale: one user, one Mac, typically 1 to 10 agents across 1 to 5 repos. No server side.

## 2. Facts that shape the design

### stdusk (from `docs/research/stdusk-recon.md`)

- **It's a single binary crate** with no `[workspace]` and no `[lib]`. Package `stdusk` 1.9.0, edition 2024. eframe 0.35 is used with `features = ["glow","__screenshot"]` hard-enabled.
- **`PtyTerm::spawn(cols, rows, ctx: egui::Context, &SpawnOpts)`** (`terminal.rs:702`) ties the terminal core to egui only to request a repaint. Its `SpawnOpts.profile` is the app's `config::Profile`.
- **`terminal.rs` (3541 lines) uses the agent cluster.** The cluster is `agents`, `agent_track`, `agent_pane`, `agent_restore`, `agent_codex*` and `procwatch`. Its only link back to the app is `session::SavedAgent`.
- **The pure modules have no egui:** `osc`, `progress`, `modes`, `screen`, `search`, `links`, `shell`, the `agents*` modules, and half of `repo`.
- **The egui-light modules are reusable as they are:** `pane` (a generic split tree), `mouse`, `fonts`, `widgets`.
- **`render_grid`, `collect_input`, `draw_tab` and the toast and overlay painters live in `ui.rs` (2565 lines).** They sit next to app-only code (quake geometry, quit text).
- **The theme is a process-wide global** (`colors.rs:19`, about 30 free accessors), and `widgets`/`ui` read it implicitly. `colors.rs` imports `crate::settings`, which is a possible dependency cycle.
- **Restart handoff passes pty master fds over a Unix socket (SCM_RIGHTS, via `rustix`).** It is tied to `impl Stdusk`, `SavedSession`, `update::bundle_path`, and the static `GAVE_SOCKET`.
- **Several paths are hardcoded to `stdusk`:** the config, the session file (`~/.config/stdusk/session.toml`, with a global `WRITER` OnceLock), shell integration and the instance socket.
- **Agent restore watches processes; it uses no hooks.** It reads a sysinfo snapshot plus Claude's registry and Codex's rollout files, then types `claude --resume <id>` or `codex resume <id>` at the prompt.
- **All tests are inline.** There is no `tests/` folder. `test_support.rs` spawns real shells.

### Kelp (from `docs/research/kelp-recon.md`)

- **`kelp-core` has no egui and is not published.** The workspace is 0.8.17 with resolver 3, gix 0.88.0 and eframe 0.35.0. Release tags are `v*`.
- **Everything in `kelp-core` blocks.** The caller runs it on a thread. It shells out to `git` and `gh`.
- **Two pure functions are reusable directly:**
  - `graph::layout(rows, parents_fn, colors) -> Layout`, plus `Layout::edges(row)`.
  - `diff::build(path, old, new) -> FileDiff`, with word-highlight spans returned in `Line.emphasis`.
- **The UI is not reusable as it is:**
  - `GraphView::show` (`graph_view.rs`, 2024 lines) returns the app's `Command` and reads gix during paint.
  - `DiffView::ui` (`diff_view.rs`, 1949 lines) returns staging and discard `Op`s.
  - Review threads are private `DiffView` methods.
  - `theme.rs` (643 lines) is a global.
- **Review threads are stored at `<git_common_dir>/kelp/comments.json`.** Saves are atomic, but there is no lock. Each thread records `Thread{id, path, commit, anchor, resolved, comments}`. `Anchor::locate` re-finds the line at render time.
- **Kelp already lists worktrees** (`git worktree list --porcelain`) and finds agent processes in them with `ps` and `lsof`.
- **The job runner is small:** `jobs.rs` (60 lines) runs one thread per job, sends results over a channel, then requests a repaint.
- **Self-update has Kelp's repo and cask hardcoded** (`update.rs:7-8`).
- **`KELP_SCREENSHOT` renders 3 frames, saves a PNG and quits.**

### Agent CLIs and git (from `docs/research/agents-worktrees.md`)

| | Claude Code 2.1.289 | Codex (docs only) | Gemini CLI 0.46.0 | Copilot CLI 1.0.91 |
| --- | --- | --- | --- | --- |
| Per-session hooks | `--settings <json>` (http hooks possible; merge vs replace untested) | no, config files only, hooks need trust | no flag; `settings.json` | no flag; `COPILOT_HOME` moves all config |
| Waiting signal | `Notification`, `PermissionRequest` | `PermissionRequest` | `Notification` (ToolPermission) | `notification`, `permissionRequest` |
| Done / failed | `Stop` / `StopFailure` | `Stop` / none | `AfterAgent` / none | `agentStop` / `errorOccurred` |
| Terminal signals | `--name` in title | `osc9` notifications (unconfirmed) | unknown | OSC 9;4 progress, intent in title |
| Per-session MCP | `--mcp-config`, `--strict-mcp-config` | unverified (`-c`) | no flag; project `.gemini/settings.json` | `--additional-mcp-config` |
| Choose session id | `--session-id <uuid>` | no (read from hook) | `--session-id` | `--session-id` |
| Resume | `--resume <id>` | `codex resume <id>` | `--resume latest` or `--resume N` | `--resume <id>` |
| Headless | `-p --output-format stream-json` | `codex exec --json` | `-p -o stream-json` | `-p --output-format json` |

Other findings from the same report:

- **Codex:** `--full-auto` is deprecated. `--yolo` is an alias of `--dangerously-bypass-approvals-and-sandbox`. The `--ask-for-approval` values are unverified; the docs showed only `on-request|never`.
- **Copilot:** `--allow-tool write` is valid. `--deny-tool` beats every allow rule.
- **A branch can't be checked out in two worktrees.** git refuses with `already used by worktree at <path>`.
- **Moving the main repo breaks linked worktrees.** `git worktree repair <path>` fixes them, but only when the paths are passed explicitly.
- **`git worktree remove` refuses dirty or locked worktrees.** `lock` protects against prune and remove.
- **A pre-push hook can be limited to one worktree** with `extensions.worktreeConfig` plus `git config --worktree core.hooksPath`. It is repo-wide, though, and it disables husky/lefthook in that worktree.

## 3. Decisions for review

**D1. How is stdusk split?**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner.

Recommendation: **two library crates inside the stdusk repo**, which becomes a workspace with the app still at the root:

- `stdusk-core` has no egui. It holds `PtyTerm`, the parsers, `shell`, `pane`, the agent cluster, the session types and the handoff wire protocol.
- `stdusk-egui` holds `render_grid`, `collect_input`, `draw_tab`, `widgets`, `fonts`, toasts, `colors`, and the quake and macOS window helpers.

`PtyTerm::spawn` takes a repaint callback instead of `egui::Context`. Paths become an `AppPaths` value the app passes in. This mirrors Kelp's core/app split, keeps the release binary path unchanged, and lets Tidewatch take the window behavior too.

Alternatives:

- One crate with an `egui` feature: smaller diff, blurrier boundary.
- Move the app into `crates/stdusk`: cleaner tree, but the release workflow paths change.

**D2. What happens to stdusk's global theme?**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner.

Recommendation: **keep it a process-wide global inside `stdusk-egui`, set by the app at startup.** Each app is its own process, so two apps never share it. Tidewatch has one theme anyway. Fix the `colors -> settings` import by moving what `colors` needs from `settings` into `colors`.

Alternative: pass a `&Theme` into every widget. That is cleaner but touches about 30 accessors and every widget call.

**D3. How is Kelp's UI shared?**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner.

Recommendation: **extract a `kelp-ui` crate in the Kelp repo**, with plain-data inputs and its own small `Action` enums:

- the graph painter, built on `graph::layout`,
- the diff body with split view, word highlights and jump between changes,
- the review thread card and composer,
- the avatar store, the theme and the job runner.

Kelp's app maps those actions to its `Command`/`Op`. Staging actions stay in the app. This is the same choice as option 1 for stdusk.

Alternative: Tidewatch reimplements the painters on top of `kelp-core`. That is faster to start, but it is the duplication you ruled out.

**D4. What is a review comment?**

DECIDED 2026-10-12 by the owner (revised from "share Kelp's comments file"): comments are first of all notes to the tab's agent, and second PR comments.

1. **Note to the agent (default).** A thread belongs to the agent tab (worktree and branch) and is stored in Tidewatch's state for that tab (`notes.json`), not in Kelp's file. You press Send (one thread, or all open ones), or turn on the per-tab "deliver when idle" switch, which sends queued notes in one message when the agent stops and waits. The agent's reply lands in the thread. Closing the tab keeps the notes as history.
2. **PR comment.** When the branch has an open pull request (found through `gh`, as Kelp's `pulls` module does), its review comments are pulled into the diff as threads and can be sent to the agent the same way. Any note can be posted to the PR as a GitHub review comment, as you, after a confirm.

Reused from Kelp: the thread card, the composer, and `Anchor::locate`, which keeps a thread on its line. Kelp's own `comments.json` is not read or written, so Kelp needs no lock and no new field.

Alternative (the original recommendation): share Kelp's `comments.json` with a file lock. It was dropped because local notes nobody else sees are not what the comments are for.

**D5. How does Tidewatch learn an agent's status?**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner, with the hook spikes (Phase 0 steps 1 to 3) to run first.

Recommendation: **use three layers, each a fallback for the one above it:**

1. Hooks that post to a local Tidewatch listener on 127.0.0.1 with a per-tab token.
   - Claude Code: `--settings` JSON with http hooks.
   - Gemini and Codex: a project-scope config file written into the worktree and listed in the worktree's `info/exclude`, so it never gets committed.
   - Copilot: not wired in v0.
2. Terminal signals from the stdusk engine: OSC 9;4 progress, title changes and the bell (Copilot already sends these).
3. Process facts: is the agent process alive, its exit code, and how long since it last printed anything (`STATUS_QUIET_SECS`).

Every layer feeds one `AgentStatus` enum. Phase 0 checks the uncertain parts.

Alternative: run agents headless and parse JSON. That is exact, but you lose the live TUI you watch and answer.

**D6. How do workflow stages and notes talk to Tidewatch?**

DECIDED 2026-10-12 by the owner (revised from "a file contract, MCP later"): **a Tidewatch MCP server, MCP only, in v1.**

- The server is built into the app, on the same local listener as the hooks (D5), with a per-tab token so an agent reaches only its own tab.
- Tools: `stage_done(summary)`, `report_finding(file, line, blocking, text)` (each finding becomes a note in the diff, D4), `list_notes()`, `reply_to_note(id, text)`.
- Attached per session: Claude `--mcp-config`, Copilot `--additional-mcp-config`, Gemini `mcpServers` in the worktree's `.gemini/settings.json` (the file the hooks already use), Codex the worktree's `.codex/config.toml` (unverified, experimental per D8).
- Tidewatch pre-allows only its own tools (Claude `--allowedTools mcp__tidewatch__*`, Copilot `--allow-tool 'tidewatch'`), so they never prompt.
- Gates stay Tidewatch's job: the test command, no blocking findings, your approval.
- Safety net, nothing extra to build: if a stage stops without `stage_done`, the tab shows the agent's last message (from the Stop hook payload, verified in Phase 0) and asks you to pass, retry or stop.

Alternatives: a file contract (works everywhere, but one-way, prompt-dependent, and a second channel to maintain); MCP plus a file fallback (two channels to build and test).

**D7. How is "never push" enforced, and how does work get delivered?**

DECIDED 2026-10-12 by the owner, mechanism changed after the Phase 0 check (step 4):

- **Guard:** the agent's terminal gets one git setting through the environment: `GIT_CONFIG_COUNT=1`, `GIT_CONFIG_KEY_0=url.tidewatch-no-push://blocked/.pushInsteadOf`, `GIT_CONFIG_VALUE_0=` (empty, so it matches every remote). Every push from that terminal is rewritten to a remote helper that does not exist, so it fails even with `--no-verify` or an explicit URL. Fetch and pull keep working. A tiny `git-remote-tidewatch-no-push` script on the agent's `PATH` turns the failure into a clear message ("this agent tab can't push; Tidewatch hands off when you approve"). Nothing is written to the repo, the repo's own hooks are untouched, and closing the tab undoes it. It is a guard rail: an agent that unsets the variable or passes `-c` can still push.
- **Delivery:** Tidewatch pushes, never the agent. When work is ready (the Hand off stage, or a single-agent tab that stops with commits not on the remote), the tab shows the commits plus the PR title and text, and asks. On approval Tidewatch runs `git push -u` and `gh pr create` outside the agent's environment.
- **By hand:** the pinned main tab and split panes are your terminals and have no guard.
- **Default:** "Never push" is on for every profile except Builder YOLO.

Alternatives: a forwarding `core.hooksPath` with a refusing `pre-push` (tested: works, but `--no-verify` skips it, and it reroutes every hook); a per-worktree git setting (repo-wide extension, breaks husky); always pushing by hand.

**D8. How much Codex support is in v0?**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner. Everything stays user-editable: profiles (settings editor or `profiles.toml`, including extra arguments), the flags behind each level per CLI (`[agent.<id>.permissions]`), per-repo overrides (`<repo>/.tidewatch.toml`), and **custom agents** (any CLI with a binary and five level flags; without hooks it gets status layers 2 and 3 only).

Recommendation: **ship Claude Code, Gemini CLI and Copilot CLI as verified agents, and Codex as "experimental"** until Phase 0 runs a real `codex --help` and a hook test.

- Correct the concept's mapping now (done after the Codex check, see Phase 0 findings).
- `untrusted` no longer exists in 0.162.1.

Alternative: hold Codex out of v0 entirely.

**D9. Exact worktree path.**

DECIDED 2026-10-12: recommendation accepted, confirmed by the owner.

Recommendation: **`~/.tidewatch/<repo-name>-<6 hex of the repo path hash>/<tab>`**, with Tidewatch state in `~/.tidewatch/.state/`. Two repos named `web` would otherwise collide in `~/.tidewatch/web/`. Store each worktree's repo path, and run `git worktree repair <paths>` on startup when the repo has moved.

Alternative: plain `~/.tidewatch/<repo-name>/`, with a numeric suffix added on collision.

**D10. Distribution and visibility.**

DECIDED 2026-10-12 by the owner, revised to add Linux: private until Phase 7, then public on **macOS and Linux**.

- Linux-ready from now: an Ubuntu CI job on every PR, macOS-only code behind small platform modules (shared crates included).
- Linux release with the macOS one (Phase 10): a tarball and an AppImage.
- Quake on Wayland: no global hotkey there, so users bind a desktop shortcut to `tidewatch --toggle`. X11 uses the global hotkey.
- Linux needs its own update path (Kelp's updater assumes an app bundle), font fallbacks and an "open in editor" fallback.

Recommendation: **keep the repo private until the first build you use daily (end of Phase 7). Then make it public** with the MIT license, a Homebrew cask `hobo-ware/tap/tidewatch`, and the same release workflow as Kelp (universal binary, ad-hoc signed until there is an Apple Developer account).

Alternative: public from day one, like the siblings.

**D11. The four product questions left open in the concept round.** Recommendation, one line each:

| Question | Recommendation |
| --- | --- |
| Pinned plain-shell "main" tab per repo | keep it |
| A gate hits its loop limit | stop and ask |
| Overview at launch | stay a button; launch restores last session |
| Agents in Other | allowed, YOLO refused, same-folder warning |

DECIDED 2026-10-12: all four recommendations accepted, confirmed by the owner.

**D12. How do agent tabs show the agent's output?** (added 2026-10-12, open)

Recommendation: **a Reader view over the live terminal.** The agent's own terminal UI keeps running in the pty. Tidewatch tails the CLI's session file (Claude Code `~/.claude/projects/.../<session>.jsonl`, Copilot `~/.copilot/session-state/<id>/events.jsonl`, Codex rollout files) and draws it as markdown with `egui_commonmark` 0.24 (the egui 0.35 line). Replies and permission buttons are typed into the pty. `Cmd+Shift+T` flips to the raw terminal. Verified: Claude's transcript is written live while the session runs; Copilot keeps a per-session `events.jsonl`.

Alternatives:

- Run agents through a structured protocol (ACP or stream-json): exact events, but no real terminal, and three of four CLIs need third-party adapters.
- Prettify the terminal grid only: no parsing, but still monospace and no real markdown.

## 4. Architecture

### Topology

```
                       Tidewatch.app (one process)
 +--------------------------------------------------------------------+
 |  crates/tidewatch (egui app)                                       |
 |   window/quake  tab strip  terminal panes  graph pane  diff pane   |
 |        |            |            |             |          |        |
 |   stdusk-egui   stdusk-egui  stdusk-egui    kelp-ui    kelp-ui     |
 |                                  |             |          |        |
 |  crates/tidewatch-core           |          kelp-core  kelp-core   |
 |   worktrees  profiles  workflows  status bus  hook listener        |
 |        |          |         |          ^            ^              |
 +--------|----------|---------|----------|------------|--------------+
          v          v         v          |            |
   git worktree   agent CLI in a pty (stdusk-core PtyTerm)
   ~/.tidewatch      |  hooks (http, token) ------------+
                     |  OSC / title / exit -> status bus
                     +- calls Tidewatch MCP tools (stage_done, report_finding, reply_to_note)
```

### File layout

```
tidewatch/
  Cargo.toml                  workspace, [workspace.lints] copied from stdusk
  crates/tidewatch-core/src/
    lib.rs
    paths.rs                  ~/.tidewatch layout (D9), state dirs
    worktree.rs               create/lock/remove/repair, leftovers
    repo_group.rs             repo vs Other grouping
    agents.rs                 AGENTS table: bin, version probe, level -> flags (from concept)
    profiles.rs               Profile, TOML load/save, first-launch seeding
    workflows.rs              Workflow, Stage, Gate, runner state machine
    status.rs                 AgentStatus enum + the three input layers (D5)
    hooks.rs                  127.0.0.1 listener, per-tab token, payload parsing
    guard.rs                  never-push env (D7), YOLO-only-in-worktree rule
    notes.rs                  per-tab review threads, delivery queue (D4)
    pr.rs                     open PR lookup, pull and post review comments through gh (D4)
    mcp.rs                    Tidewatch MCP server: stage_done, report_finding, list_notes, reply_to_note (D6)
  crates/tidewatch/src/
    main.rs                   eframe app loop (thin)
    app/                      state: groups, tabs, panes, focus mode
    tabs.rs                   stdusk tab strip + stepper
    panes/terminal.rs         stdusk-egui grid + splits
    panes/graph.rs            kelp-ui graph, scoped to the active worktree
    panes/diff.rs             kelp-ui diff + review threads + send-to-agent
    dialogs/new_tab.rs        workflow / single agent, permission ladder
    overview.rs               all agents view
    settings.rs               profiles + workflows editors, general settings
    dev_screenshot.rs         TIDEWATCH_SCREENSHOT harness
  docs/  .agents/rules/  PLAN.md  LEDGER.md
```

### Data model (TOML in `~/.config/tidewatch/`, state in `~/.tidewatch/.state/`)

| File | Contents |
| --- | --- |
| `config.toml` | quake hotkey, theme, focus mode default, test command per repo override |
| `profiles.toml` | `[profile.<id>]` agent, model, effort, permissions, setup, never_push, extra_args |
| `workflows.toml` | `[workflow.<id>]` name, `[[stage]]` action, agent, model, effort, permissions, gate, on_fail, max_loops, approve, prompt |
| `.state/session.toml` | groups, tabs, worktree paths, agent session ids, workflow position (stdusk-core session types) |
| `.state/<repo>/<tab>/notes.json` | the tab's review threads: anchor, comments, author (person or agent), sent, resolved, PR comment id when linked (D4) |
| `.state/<repo>/<tab>/stages.json` | per stage: state, summary and findings received through MCP (D6) |

A finding (from `report_finding`): `{ "file": str, "line": int, "blocking": bool, "text": str }`.

### Core types (pseudo-signatures)

```rust
enum Level { Plan, Ask, Edits, Auto, Yolo }
struct AgentKind { bin: &'static str, levels: [Option<&'static str>; 5], verified: bool }
struct Profile { agent: AgentId, model: Option<String>, effort: Option<String>, level: Level, setup: String, never_push: bool, extra: Vec<String> }
enum Gate { Tests, TestFails, TestPasses, NoBlocking, SameBehavior, Approve }
struct Stage { action: Action, run: Profile, gate: Gate, on_fail: Option<usize>, max_loops: u8, prompt: String }
enum StageState { Todo, Running, NeedsYou, Done, Failed }
enum AgentStatus { SettingUp(u8), Working, NeedsYou(Reason), Done, Failed(String), Shell }
fn launch_command(agent: &AgentKind, p: &Profile, task: &str) -> Vec<String>
```

### Tab lifecycle

```
New tab -> worktree add + lock -> setup cmd -> agent spawn (env: hooks token, stage dir, never-push)
 -> status events -> [workflow: gate check -> next stage | loop back | NeedsYou]
 -> close: confirm keep/delete branch -> kill pty -> unlock + worktree remove -> state cleanup
```

### Failure model

| Kind | Trigger | Effect |
| --- | --- | --- |
| Worktree add fails | branch exists, disk, git error | tab shows the git error, nothing half-made is left |
| Dirty worktree on close | uncommitted changes | ask: commit via agent, keep worktree, or discard (explicit) |
| Repo moved | path missing on startup | `worktree repair`, else mark the tab "repo missing" |
| Hook listener down | port bind fails | fall back to layers 2 and 3 (D5), log once |
| Agent exits non-zero | process exit | status Failed with the exit code; a workflow stage fails its gate |
| Stage stops without `stage_done` | agent ends without calling the tool | the tab shows its last message (Stop hook) and asks: pass, retry or stop |
| YOLO outside a worktree | main tab or Other | refused before spawn |

### Constants (tunable, named once)

| Name | Value |
| --- | --- |
| `ROOT_DIR` | `~/.tidewatch` |
| `STATUS_QUIET_SECS` | 8 |
| `DEFAULT_MAX_LOOPS` | 2 |
| `QUAKE_HOTKEY_DEFAULT` | `Ctrl+Alt+Grave` (stdusk keeps `Ctrl+Grave`) |
| `HOOK_BIND` | `127.0.0.1:0` (random port) |
| `GRAPH_PAGE_ROWS` | as Kelp |

## 5. Implementation plan

Each phase is one PR. Phases 1 and 2 are PRs in the sibling repos. Every runnable check is run
exactly as written and its trimmed output pasted in the checkpoint report.

### Phase 0: verification spikes (no product code)

Goal: settle every uncertain fact D5, D7 and D8 rely on.

Steps:

1. **Claude `--settings` hooks:** run `claude --settings '<json with http Notification/Stop hooks to 127.0.0.1:PORT>'` with `nc -l PORT` or a 10-line Python server listening. Check that the payloads arrive and that the user's own hooks still run (merge or replace).
2. **Gemini and Codex project hooks:** install Codex (`brew install codex` or npm) and record `codex --help` (approval values). In a scratch worktree, write a project `.gemini/settings.json` and a `.codex/config.toml` with hooks. Add both to `info/exclude`, and confirm `git status` stays clean and the hooks fire.
3. **Copilot signals:** record a session with `script -q out.log copilot ...` and grep for `]9;4` and title sequences. Try `--additional-mcp-config` with a dummy stdio server.
4. **Never-push env:** `GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/tmp/tw-hooks git push` with a refusing `pre-push`. Confirm the push is refused, and that the forwarding wrapper still runs a husky hook.
5. **Worktree flows on git 2.55:** add, lock, a dirty remove (refused), unlock and remove, move the main repo then `repair <path>`, and checking out a branch already used by another worktree.
6. **Dependency fit:** in a scratch crate, depend on `kelp-core` (path) and stdusk's current dependencies (alacritty_terminal 0.26, portable-pty 0.9, eframe 0.35). Run `cargo tree -d` to list duplicate or conflicting versions.
7. **MCP per session:** attach a dummy Tidewatch MCP server (one `ping` tool, http transport) to each CLI the D6 way and confirm the agent can call it without a permission prompt. Run before Phase 6.

Runnable: each step's command above. Results go into "Phase 0 findings" below.
Checkpoint 0 exit: every step has a recorded result. D5, D7 and D8 are confirmed or reopened with you.

### Phase 1: stdusk workspace and library crates (PR in `../stdusk`)

Goal: stdusk builds as a workspace with `stdusk-core` and `stdusk-egui`, with no behavior change.

Steps:

1. Add `[workspace]` with members `"."`, `crates/stdusk-core` and `crates/stdusk-egui`, and move `[lints]` to `[workspace.lints]`.
2. Move the pure modules (D1 list) to `stdusk-core`. Replace `egui::Context` in `PtyTerm::spawn` with `Arc<dyn Fn() + Send + Sync>`. Add `ShellProfile` and `AppPaths`. Move the session types and `SavedAgent` with the agent cluster. Move `test_support` behind a `test-support` feature.
3. Move `render_grid`, `collect_input`, `draw_tab`, `widgets`, `fonts`, `colors` and the toast/overlay painters to `stdusk-egui` (D2: global theme stays; break the `colors -> settings` import).
4. Move the quake window helpers and `macos.rs` glue that is not stdusk-specific to `stdusk-egui::platform`.
5. Widen only the items Tidewatch needs to `pub`, and keep a short `lib.rs` API list.
6. Tag a release (`stdusk-v1.10.0`) so Tidewatch can pin it.

Runnable: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`. Then `cargo run -- --screenshot /tmp/after.png`, compared against a `main` baseline with `magick compare -metric PSNR /tmp/before.png /tmp/after.png null:`.
Checkpoint 1 exit: all green, PSNR shows the image is identical, the release workflow builds the app, and the test count matches the baseline.

### Phase 2: `kelp-ui` crate and shared review (PR in `../kelp`)

Goal: Kelp's graph, diff and review UI are reusable widgets, with no behavior change in Kelp.

Steps:

1. Add `crates/kelp-ui`. Move `theme`, the avatar store, the job runner and small helpers (`paint_avatar`, `commit_tooltip`).
2. Graph painter: plain-data input (rows, `Layout`, labels, selection, hover) and its own `GraphAction` enum. Summaries are passed in, not read from gix during paint. Kelp maps `GraphAction` to `Command`.
3. Diff body and review threads: `DiffBody::ui(&FileDiff, &mut Review, mode, author) -> DiffAction`, without staging. Kelp keeps staging in its own wrapper.
5. Parametrize `update.rs` with the repo and cask, so Tidewatch passes its own.
6. Tag a release.

Runnable: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check && cargo bench -p kelp-core --bench layout`. Then `KELP_SCREENSHOT` before and after, compared by PSNR, and `KELP_BENCH_SCROLL=1` on trakt-web against the numbers in Kelp's ledger.
Checkpoint 2 exit: all green, the screenshots match, and scroll timings are within 5% of the ledger.

### Phase 3: Tidewatch skeleton

Goal: a Tidewatch window with stdusk's tab strip and one shell tab in a repo.

Steps:

1. Set up the workspace `Cargo.toml` with git dependencies pinned to the Phase 1 and 2 tags, the crates `tidewatch-core` and `tidewatch`, and lints from stdusk.
2. App loop using the stdusk-egui tab strip and terminal pane. Open with `tidewatch <repo>` or the current folder.
3. `TIDEWATCH_SCREENSHOT` harness (Kelp's pattern).
4. CI workflow (fmt, clippy, test on macos-14 and ubuntu-latest) and the Commands section in `project.md`.

Runnable: `cargo run -p tidewatch -- ~/Git/Hoboware/kelp`, then `TIDEWATCH_SCREENSHOT=/tmp/tw.png cargo run -p tidewatch -- ~/Git/Hoboware/kelp`.
Checkpoint 3 exit: the window opens with a working shell; the screenshot shows stdusk-identical tabs; CI is green.

### Phase 4: repo groups and the worktree manager

Goal: `Cmd+T` makes a worktree tab; closing it removes the worktree safely.

Steps:

1. `paths.rs` and `worktree.rs` (D9): add with `-b agent/<name>`, lock, remove with the keep/delete choice, repair on startup, and find leftovers.
2. Repo chip and group switching (stdusk behavior), the pinned main tab, and the Other group (D11).
3. Round-trip tests against temp repos (Kelp's `*_roundtrip.rs` pattern).

Runnable: open a tab, run `git -C ~/Git/Hoboware/kelp worktree list`, close the tab with "delete", and run the list again.
Checkpoint 4 exit: the worktree appears and disappears, no leftover directory, round-trip tests pass.

### Phase 5: graph, diff and review panes (layout B)

Goal: the bottom half of the concept works.

Steps:

1. Graph pane scoped to the active worktree, with the Whole repo switch and ahead/behind against main.
2. Diff pane: unified/split, word highlight, jump between changes, live working tree.
3. Review threads as notes for the tab's agent, stored in the tab state (D4).
4. Double-click checkout rules (concept): blocked while an agent works, jump to the owner tab.

Runnable: a screenshot of a repo with two agent worktrees, plus making a note, quitting and reopening Tidewatch, and seeing the note on the same line.
Checkpoint 5 exit: the screenshot matches the concept layout and notes survive a restart.

### Phase 6: profiles, permission levels and launching agents

Goal: start a real agent with the right flags from the new-tab dialog.

Steps:

1. `agents.rs` table (D8 corrections), with a version probe through `--version` on PATH. Built-in agents load first, then `[agent.<id>]` entries from `profiles.toml` override their flags or add custom agents.
2. `profiles.rs` with first-launch seeding, and the settings editor.
3. New-tab dialog: single agent with the permission ladder, YOLO refused outside worktrees, and a command preview.
4. Never-push guard (D7: the `pushInsteadOf` env setting plus the `git-remote-tidewatch-no-push` message script) and the setup command.

Runnable: open a Builder tab, then `ps -o args= -p $(pgrep -n claude)`. It must show `--permission-mode acceptEdits`. In that tab, `git push` and `git push --no-verify` are both refused with the Tidewatch message; in a split pane, `git push` works.
Checkpoint 6 exit: the flags match the table for every verified agent, YOLO is refused on the main tab, and pushing is blocked.

### Phase 7: agent status, Overview, focus mode, notifications

Goal: you can tell at a glance which agent needs you.

Steps:

1. `status.rs` and `hooks.rs` (D5): listener, token, the three layers, `AgentStatus`.
2. Tab top bar (stdusk style), the CLI logo slot, macOS notifications when hidden.
3. Overview (`Cmd+0`) and focus mode (`Cmd+.`).

Runnable: start Claude with a task that needs a permission. The tab turns amber within 1 second of the prompt, and a notification fires while the window is hidden.
Checkpoint 7 exit: working, needs you, done and failed show correctly for Claude, Gemini and Copilot. Make the repo public here if D10 is accepted.

### Phase 8: comments to the agent

Goal: review threads reach the agent and its answer comes back.

Steps:

1. Delivery (D7): the "push and open a PR?" card with commits, PR title and text; on approval Tidewatch runs `git push -u` and `gh pr create`.
2. "Send to agent" for one thread or all open ones (bracketed paste into the agent pane), and the per-tab "deliver when idle" switch (D4).
3. PR link (D4): find the open PR with `gh`, pull its review comments in as threads, post a note to the PR after a confirm.
4. Agent replies through the MCP server (D6): `list_notes`, `reply_to_note`; replies land in the thread.
5. Reader view (D12): session-file tailers per CLI, markdown rendering, reply box and permission buttons that type into the pty, `Cmd+Shift+T` toggle. Phase 0 adds a spike for Gemini's session storage.

Runnable: comment on a line, press Send, and see the paste in the agent pane and the reply in the thread.
Checkpoint 8 exit: the round trip works for Claude; the file path works for all agents.

### Phase 9: workflows

Goal: Ship a feature runs from Work to Hand off, with gates and loops.

Steps:

1. `workflows.rs` runner state machine with unit tests for each gate and loop path.
2. Stage launch in the same agent pane with a divider; `stage_done` and `report_finding` drive the gate and the notes (D6).
3. Stepper in the terminal header, the workflow editor in settings, and the default workflows from the concept.

Runnable: run Ship a feature on a toy repo with a planted bug the reviewer must catch. It loops once, then reaches Hand off and waits.
Checkpoint 9 exit: every gate is exercised by a test; the toy run completes.

### Phase 10: inherited app shell and release

Goal: Tidewatch behaves like its siblings day to day.

Steps:

1. Quake mode with its own default hotkey, one instance, `tidewatch <path>` opening a tab, and the menu-bar item.
2. Session restore with agent resume (stdusk-core), and restart handoff on upgrade.
3. Settings GUI, the command palette, the full shortcut table.
4. Self-update through the parametrized Kelp updater, the release workflow, the Homebrew cask, and the README/site.
5. Linux: tarball and AppImage in the release workflow, a Linux update path, `tidewatch --toggle` for Wayland quake.

Runnable: `brew install hobo-ware/tap/tidewatch`, then upgrade to a newer tag while two agents run. Both are still there after the restart.
Checkpoint 10 exit: tagged release installed from the tap, and the AppImage starts on Ubuntu; restart keeps agents; shortcuts match the table.

## 6. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| stdusk extraction breaks stdusk | Phase 1 is behavior-neutral, gated by screenshot PSNR and an unchanged test count |
| Two dependency trees conflict (egui, winit, gix) | Phase 0 step 6; pin the same egui 0.35 everywhere |
| Agent CLI flags change between versions | `agents.rs` probes `--version` and logs it; flags have a test table; Codex stays experimental (D8) |
| Hooks via `--settings` replace the user's hooks | Phase 0 step 1; if they replace, merge the user's hooks into the injected JSON |
| Hook config files get committed by an agent | `info/exclude` entries plus a check before every gate |
| Posting a note to a team PR by mistake | posting always asks first; notes stay local by default (D4) |
| YOLO agent damages files outside its worktree | only allowed in worktrees; documented as a guard rail, not a sandbox |
| Shared crates drift back to macOS-only | Ubuntu CI job on every PR, in Tidewatch and in the shared crates' repos |
| Many agents slow the UI | hidden tabs do not paint; the status bus is event-driven; performance rules apply |

## 7. Out of scope for v0

- Pull request status chips on agent labels.
- A free port per worktree for dev servers.
- Time and cost per agent.
- Windows.
- Signed and notarized builds (until there is an Apple Developer account).
- Manual git actions (fetch, push, merge, stage) in the UI.

## Phase 0 findings

- **2026-10-12, step 1 (Claude Code 2.1.289 hooks), PASS.**
  - Hooks passed with `--settings '<json>'` add to the project's `.claude/settings.json` hooks rather than replacing them: both Stop hooks ran.
  - `type: "http"` hooks POST to a local listener. The Stop payload carries `session_id`, `cwd`, `transcript_path`, `last_assistant_message` and `permission_mode`.
  - An http `SessionStart` hook did not fire in `-p` mode, so it needs a recheck in an interactive run, along with `Notification`.
  - Test: `claude -p ... --model haiku --settings '<json>'` in a temp git repo with a 15-line Python listener.
- **2026-10-12, step 2, Gemini part (Gemini CLI 0.46.0), PASS with a condition.**
  - Hooks in a project `.gemini/settings.json` fire (`SessionStart`, `AfterAgent`). The AfterAgent payload carries `session_id`, `cwd`, `transcript_path`, `prompt` and `prompt_response`.
  - **Condition:** Gemini refuses to run in an untrusted folder, and every worktree under `~/.tidewatch` is new. Tidewatch must set `GEMINI_CLI_TRUST_WORKSPACE=true` in a Gemini tab's environment.
  - The file shows as untracked in `git status`, which confirms the `info/exclude` step in D5 is needed.
- **2026-10-12, step 4 (never push, git 2.55.0), PASS, mechanism changed.**
  - A refusing `pre-push` through env-injected `core.hooksPath` works and the forwarding wrapper still ran the repo's own `post-commit` hook. But `git push --no-verify` skipped it.
  - An env-injected `url.tidewatch-no-push://blocked/.pushInsteadOf` with an empty value blocked `git push`, `git push --no-verify` and a push to an explicit URL. Fetch kept working, and a push from a normal shell worked. D7 now uses this.
- **2026-10-12, step 2, Codex part (codex-cli 0.162.1 from `brew install --cask codex`), flags verified, live test blocked.**
  - `--ask-for-approval` now takes only `on-request` and `never`. `--full-auto` and `--yolo` are gone from `--help`. A new `--approve-for-me` routes approvals to an automatic reviewer.
  - New mapping: Read only `--sandbox read-only --ask-for-approval never`; Ask `--sandbox read-only --ask-for-approval on-request`; Auto edit `--sandbox workspace-write --ask-for-approval on-request`; Auto `--sandbox workspace-write --approve-for-me`; YOLO `--dangerously-bypass-approvals-and-sandbox`.
  - `-c key=value` overrides any config value for one run, and `--dangerously-bypass-hook-trust` runs hooks without persisted trust. This is the per-session path for Codex hooks and MCP (D5, D6). No worktree file is needed.
  - The live hook and MCP test needs `codex login`, which only the owner can do.
- Step 3 (Copilot signals): not run yet.

## Deviations log

- **2026-10-12, order:** the goal is a first running build, so the order is Phase 1 (stdusk crates), then Phase 3 (skeleton), then Phase 2 (kelp-ui). Phase 3 needs only the stdusk crates.
- **2026-10-12, Phase 0:** only step 6 (dependency fit) runs before the first build. Steps 1 to 5 run before the phase that depends on them (5 before Phase 4, 4 before Phase 6, 1 to 3 before Phase 7).
- **2026-10-12, dependencies:** until stdusk tags a release with the library crates, Tidewatch uses path dependencies to the Phase 1 worktree (`../stdusk/.claude/worktrees/phase-1-lib-crates/crates/...`). Switch to git tags when Phase 1 merges.

## Appendices

- `docs/research/stdusk-recon.md`: module map, coupling, handoff and agent restore details.
- `docs/research/kelp-recon.md`: kelp-core API, UI coupling, review store, updater.
- `docs/research/agents-worktrees.md`: per-CLI hooks, MCP, sessions, headless modes, git worktree tests.
- `docs/concept.html`: the agreed concept and mockup.
