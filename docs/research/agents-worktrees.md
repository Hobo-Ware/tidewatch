# Tidewatch research: agent CLIs and git worktrees

Confidence markers: [doc] official doc, [help] local --help output, [src] source code, [forum] blog/forum, [test] run locally (git 2.55.0), [infer] my inference, not verified.
Versions checked locally: claude 2.1.289, gemini 0.46.0, copilot 1.0.91. Codex not installed (docs only).

Doc URLs
- Claude hooks: https://code.claude.com/docs/en/hooks
- Codex hooks: https://learn.chatgpt.com/docs/hooks (redirect from https://developers.openai.com/codex/hooks)
- Codex CLI reference: https://learn.chatgpt.com/docs/developer-commands?surface=cli (redirect from https://developers.openai.com/codex/cli/reference)
- Codex advanced config: https://developers.openai.com/codex/config-advanced
- Gemini hooks: https://geminicli.com/docs/hooks/reference/
- Copilot hooks: https://docs.github.com/en/copilot/reference/hooks-configuration
- Copilot local help: `copilot help config|permissions|environment` [help]
- git worktree: https://git-scm.com/docs/git-worktree (man page run locally, git 2.55.0)

======================================================================
# TOPIC 1: AGENT CLIs
======================================================================

## 1. Claude Code

### 1.1 Hook / notification events
- Events: SessionStart, Setup, SessionEnd, UserPromptSubmit, UserPromptExpansion, Stop, StopFailure, PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest, PermissionDenied, PostToolBatch, PreCompact, PostCompact, Notification, MessageDisplay, SubagentStart, SubagentStop, TeammateIdle, TaskCreated, TaskCompleted, CwdChanged, DirectoryAdded, FileChanged, ConfigChange, InstructionsLoaded, WorktreeCreate, WorktreeRemove, PreModelSwitch, PostModelSwitch, Elicitation, ElicitationResult. [doc] https://code.claude.com/docs/en/hooks
- Notification `notification_type` values: permission_prompt, idle_prompt, auth_success, elicitation_*, agent_needs_input, agent_completed, quota_auto_resume_*. [doc] same URL
- Stop and SubagentStop add `last_assistant_message`. StopFailure adds `error_type` (rate_limit, overloaded, authentication_failed, billing_error, server_error, max_output_tokens, unknown, ...). [doc] same URL
- Handler types: command, http (POSTs payload to a URL), mcp_tool, prompt, agent. [doc] same URL
- Suggested status map [infer]: UserPromptSubmit/PreToolUse = working; PermissionRequest or Notification(permission_prompt|idle_prompt|agent_needs_input|elicitation_dialog) = waiting; Stop = finished (turn done); StopFailure = failed; SessionEnd = gone.

### 1.2 Configuration and per-session injection
- Files: ~/.claude/settings.json (user), .claude/settings.json (project, shared), .claude/settings.local.json (project, gitignored), managed policy, plugin hooks/hooks.json, skill/subagent frontmatter. [doc] same URL
- YES, per-session injection without touching user files: `--settings <file-or-json>` loads extra settings (hooks included). [help] `claude --help`. Docs show `--settings '{"disableAllHooks": true}'` as an example. [doc]
- Other relevant flags: `--setting-sources user,project,local`, `--bare` (skips hooks), `--plugin-dir`. [help]
- Caveat [infer]: `--settings` hooks should MERGE with user hooks (not replace). Not verified; test it.
- Env var available to hooks: ${CLAUDE_PROJECT_DIR}. [doc]

### 1.3 Hook payload (stdin JSON)
- session_id, prompt_id, transcript_path, cwd, scratchpad_dir, permission_mode, hook_event_name, effort, agent_id, agent_type. [doc] https://code.claude.com/docs/en/hooks
- permission_mode values: default (UI "Manual"), plan, acceptEdits, auto, dontAsk, bypassPermissions. [doc]
- Hook JSON output supports `terminalSequence` (e.g. an OSC 777 string) so a hook can make the terminal emit a sequence. [doc]

### 1.4 Terminal signals
- `--name` is shown in prompt box, /resume picker AND terminal title. [help] `claude --help`
- OSC 9;4 / OSC 9 / 777 built-in emission: NOT confirmed. Gap. Use hooks (http type or command) instead of parsing the terminal.

### 1.5 MCP per session
- `--mcp-config <files-or-JSON strings...>` loads servers for that run; `--strict-mcp-config` ignores all other MCP config. [help]
- Transports: stdio, http, sse (`claude mcp add --transport http ...`). [help] `claude mcp add --help`
- Good for the "reply to comment" tool: inject a per-agent config with a localhost http server URL (include a per-agent token in --header or URL).

### 1.6 Session id and resume
- `--session-id <uuid>` (you choose the id), `--resume <id>` / `-r`, `--continue`/`-c`, `--fork-session`, `--name`. [help]
- `--bg/--background` starts a detached session; `claude attach|logs|stop|rm|agents` exist. [help]
- Built-in `-w/--worktree [name]` creates a worktree itself. [help] You do not need it; but note WorktreeCreate/WorktreeRemove hook events exist.

### 1.7 Headless
- `-p/--print`, `--output-format text|json|stream-json`, `--input-format stream-json`, `--include-hook-events` (hook lifecycle events in stream), `--include-partial-messages`, `--json-schema`-style structured output flag, `--no-session-persistence`, `--permission-prompts host|none`, `--permission-prompt-tool`. [help]

### 1.8 Permission flags (confirm)
- `--permission-mode acceptEdits|auto|bypassPermissions|manual|dontAsk|plan` CONFIRMED (6 choices; "default" is the old name, hook payload still says "default"). [help]
- `--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions` (enables bypass as an option without starting in it), `--allowedTools`, `--disallowedTools`, `--tools`. [help]

## 2. Codex CLI (docs only, not installed)

### 2.1 Hook / notification events
- Events: PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, UserPromptSubmit, SubagentStop, Stop, Interrupt, SessionStart (source: startup|resume|clear|compact), SessionEnd, SubagentStart. [doc] https://learn.chatgpt.com/docs/hooks
- Older, simpler `notify` setting: runs a program with a JSON payload; ONLY event is `agent-turn-complete`. [forum] https://codex.danielvaughan.com/2026/04/10/codex-cli-agent-notifications-desktop-alerts-monitoring/ (search snippet; payload field list not verified)
- PermissionRequest payload adds tool_name, tool_input (incl. command, description); can answer allow/deny. [doc]

### 2.2 Configuration and per-session injection
- Files: ~/.codex/hooks.json, ~/.codex/config.toml ([hooks] table), <repo>/.codex/hooks.json, <repo>/.codex/config.toml. [doc]
- Per-session injection of hooks via flag/env: NOT supported. Hooks must be discovered from config layers and "reviewed/trusted before running". [doc] same URL
- Workaround [infer]: `-c key=value` overrides config values (repeatable, TOML). [doc] https://learn.chatgpt.com/docs/developer-commands?surface=cli. Might work for `notify=[...]`; NOT verified for hooks. Alternative: a project-level .codex/hooks.json inside the worktree (needs trust review) or `--profile`.
- Only command hooks documented (no http type). [doc]
- Managed hooks via requirements.toml (`allow_managed_hooks_only`) can disable user/project/session hooks. [doc] https://raw.githubusercontent.com/openai/codex/main/docs/config.md

### 2.3 Hook payload
- session_id, transcript_path (optional), cwd, hook_event_name, model, permission_mode (default|acceptEdits|plan|dontAsk|bypassPermissions); turn-scoped hooks add turn_id. [doc] https://learn.chatgpt.com/docs/hooks
- No Notification-style "idle/waiting" event; use PermissionRequest = waiting, Stop = turn finished. No documented failure event. [infer from event list]

### 2.4 Terminal signals
- `tui.notification_method`: auto | osc9 | bel; `tui.notifications` enable/restrict types; `tui.terminal_title` array of items (empty = untouched), /title command. [forum] https://developers.openai.com/codex/config-advanced (from search snippet, not opened)
- OSC 9;4 progress: not confirmed. Gap.

### 2.5 MCP per session
- Reference page says per-session MCP is configured via ~/.codex/config.toml and `codex mcp add/remove`; no dedicated flag. [doc] developer-commands page
- Workaround [infer]: `-c 'mcp_servers.tidewatch.command="..."'` style override (TOML dotted keys). Unverified. Config supports stdio and streamable http servers [infer from general Codex knowledge, not verified here].
- `codex app-server` (experimental, `--listen`, WebSocket) and `codex mcp-server` exist. [doc] developer-commands page. app-server is a protocol with turn/approval events: strongest option for rich status, but heavier.

### 2.6 Session id and resume
- `codex resume [SESSION_ID]`, `--last`; `codex exec resume [SESSION_ID]` with `--last`/`--all`. [doc] developer-commands page
- Can you choose the session id up front? Not documented. Get it from the hook payload (SessionStart). [infer]

### 2.7 Headless
- `codex exec`, `--json` (JSONL events), `--output-schema <file>`, `-o/--output-last-message <file>`. [doc] developer-commands page

### 2.8 Permission flags (corrections)
- `--sandbox/-s read-only|workspace-write|danger-full-access` CONFIRMED. [doc]
- `--ask-for-approval`: the fetched reference lists `on-request | never` only. Your list (untrusted, on-failure, on-request, never) may be partly outdated or the summary may be truncated. UNVERIFIED, re-check on the live page or `codex --help`. [doc, partial]
- `--full-auto` is DEPRECATED; docs say use `--sandbox workspace-write` (with on-request). [doc]
- `--dangerously-bypass-approvals-and-sandbox` has alias `--yolo`. [doc]
- Also `--profile/-p`, `--cd/-C`, `-c key=value`, `--oss`. [doc]

## 3. Gemini CLI

### 3.1 Hook / notification events
- Events: BeforeTool, AfterTool, BeforeAgent, AfterAgent, BeforeModel, BeforeToolSelection, AfterModel, SessionStart, SessionEnd, Notification, PreCompress. [doc] https://geminicli.com/docs/hooks/reference/
- Notification: `notification_type` is only "ToolPermission", plus `message` and `details`; observability only. [doc] same URL
- Status map [infer]: BeforeAgent = working; Notification(ToolPermission) = waiting; AfterAgent = finished; no failure event documented.
- `gemini hooks migrate` converts Claude Code hooks to Gemini format. [help] `gemini hooks --help`

### 3.2 Configuration and per-session injection
- Hooks live in a `hooks` object in settings.json (user ~/.gemini/settings.json and project .gemini/settings.json [infer: standard Gemini layout; the doc page did not distinguish]). [doc]
- Per-session flag/env for hooks: none in `gemini --help`; the doc is silent. [help]
- Possible [infer, unverified]: GEMINI_CLI_SYSTEM_SETTINGS_PATH env var and extensions (`-e`) can carry hooks.
- Handler type: command only documented. [doc]

### 3.3 Hook payload
- session_id, transcript_path, cwd, hook_event_name, timestamp (ISO 8601). [doc] https://geminicli.com/docs/hooks/reference/

### 3.4 Terminal signals
- Not checked. Gap (OSC 9/777/title for Gemini).

### 3.5 MCP per session
- No flag to ADD a server for one run. `gemini mcp add` writes config; `--scope user|project` (default project); `--transport stdio|sse|http`. `--allowed-mcp-server-names` only filters. `-e/--extensions` selects extensions. [help] `gemini --help`, `gemini mcp add --help`
- So per-session attach = write a project-scope .gemini/settings.json inside the worktree [infer] (worktree-local, so does not touch user files).

### 3.6 Session id and resume
- `--session-id <uuid>` (choose your own), `-r/--resume latest|<index>`, `--list-sessions`, `--delete-session`, `--session-file <json>`. [help] Resume takes an INDEX, not id; use --list-sessions to map. [help]
- Built-in `-w/--worktree [name]`. [help]

### 3.7 Headless
- `-p/--prompt`, `-o/--output-format text|json|stream-json`; stdin is appended. `--acp` (Agent Client Protocol server). [help]

### 3.8 Permission flags (confirm)
- `--approval-mode default|auto_edit|yolo|plan`, `-y/--yolo`, `-s/--sandbox`: CONFIRMED. [help]
- `--allowed-tools` is DEPRECATED in favor of `--policy` / `--admin-policy` files (policy engine). [help]
- `--skip-trust` trusts the workspace for the session (needed to run hooks/MCP in untrusted dirs [infer]). [help]

## 4. Copilot CLI

### 4.1 Hook / notification events
- camelCase or PascalCase names: sessionStart, sessionEnd, userPromptSubmitted, preToolUse, postToolUse, postToolUseFailure, agentStop (Stop), subagentStart, subagentStop, errorOccurred, preCompact, notification (CLI only), permissionRequest (CLI only). [doc] https://docs.github.com/en/copilot/reference/hooks-configuration
- Best event coverage for failures: errorOccurred. [doc]
- Handler types: command, http, prompt. [doc]

### 4.2 Configuration and per-session injection
- Files: .github/hooks/*.json (repo), ~/.copilot/hooks/ or $COPILOT_HOME/hooks/ (user), .github/copilot/settings(.local).json, ~/.copilot/settings.json, plugins, policy /etc/github-copilot/policy.d/*.json. Schema `{ "version": 1, "hooks": { event: [...] } }`. [doc] same URL
- Inline `hooks` key in config.json/settings.json also works. [help] `copilot help config`
- Per-session flag for hooks: none. BUT `COPILOT_HOME` env var relocates the whole config dir [help] `copilot help environment`: set it per agent to inject hooks without touching ~/.copilot (cost: also moves auth/sessions/config; copy or symlink needed) [infer].
- Repo-level hooks load only for a trusted folder (`COPILOT_ALLOW_ALL=true` exactly trusts cwd, which loads hooks/MCP). [help] `copilot help environment`
- Hooks in the worktree's .github/hooks/ would be untracked files in the worktree [infer].

### 4.3 Hook payload
- sessionId (or session_id), timestamp (ms, or ISO in PascalCase mode), cwd, transcriptPath/transcript_path (some events). [doc] same URL

### 4.4 Terminal signals
- `terminalProgress` (default TRUE): emits OSC 9;4 indeterminate progress while the agent works (Windows Terminal, iTerm2, Ghostty, ConEmu). [help] `copilot help config`
- `updateTerminalTitle` (default true): sets title to current intent; disable via COPILOT_DISABLE_TERMINAL_TITLE. [help]
- `notifications` (default false) desktop toasts; `terminalNotifications` (default false) prefers OSC 777 for Ghostty and WezTerm only. [help]
- This is the only CLI where OSC 9;4 is confirmed. If Tidewatch's terminal view parses OSC 9;4 it gets working/idle for free.

### 4.5 MCP per session
- `--additional-mcp-config <json|@file>` (repeatable) augments ~/.copilot/mcp-config.json for this session only. `--disable-mcp-server`, `--enable-mcp-server` (this run only). [help] `copilot --help`
- Transports: stdio and http [infer from `--mcp-github-auth` mention of HTTPS/loopback HTTP servers; not stated explicitly]. Server names matter for permissions: `--allow-tool 'MyMCP(tool)'`. [help]

### 4.6 Session id and resume
- `--session-id <id>` (resume existing OR set UUID for a new one), `-r/--resume [value]`, `--continue`, `-n/--name`. [help]

### 4.7 Headless
- `-p/--prompt`, `-s/--silent`, `--output-format text|json` (JSONL), `--allow-all-tools` REQUIRED for non-interactive, `--share`, `--acp`. [help]
- Remote control: `--remote`, `--remote-export`. [help]

### 4.8 Permission flags (confirm)
- `--mode interactive|plan|autopilot`; also `--plan`, `--autopilot`, `--max-autopilot-continues`. `--mode plan` CONFIRMED. [help]
- `--allow-all-tools`, `--allow-all-paths`, `--allow-all-urls`, `--allow-all` = `--yolo`. CONFIRMED. [help]
- `--allow-tool 'write'` IS VALID. Kinds: `shell(cmd:*)`, `write(path?)`, `<mcp-server-name>(tool?)`, `url(domain?)`. Bare `write` matches all file writes; `write` does not cover shell redirections. `--deny-tool` beats everything, even --allow-all-tools. [help] `copilot help permissions`
- Also `--available-tools`, `--excluded-tools`, `--assisted-approval` (experimental). [help]

======================================================================
# Side-by-side summary (Topic 1)
======================================================================
| Item | Claude | Codex | Gemini | Copilot |
|---|---|---|---|---|
| "Waiting for user" signal | Notification, PermissionRequest | PermissionRequest | Notification (ToolPermission) | notification, permissionRequest |
| "Finished" | Stop | Stop | AfterAgent | agentStop |
| "Failed" | StopFailure | none documented | none documented | errorOccurred |
| Per-session hook injection | `--settings` YES | no (maybe `-c`) | no | no (COPILOT_HOME hack) |
| http hook (no script) | yes | no | no | yes |
| Per-session MCP flag | `--mcp-config` | no (maybe `-c`) | no | `--additional-mcp-config` |
| Choose session id | `--session-id` | no | `--session-id` | `--session-id` |
| Resume | `--resume <id>` | `codex resume <id>` | `--resume <index>` | `--resume <id>` |
| Headless JSON | stream-json | exec --json | -o stream-json | --output-format json |
| OSC 9;4 | unconfirmed | unconfirmed (osc9 notif yes) | unchecked | YES default on |

======================================================================
# TOPIC 2: GIT WORKTREES (git 2.55.0 man page + local tests)
======================================================================
Source for all: https://git-scm.com/docs/git-worktree [doc] and `git help worktree` [doc, local]. Items marked [test] were run in a scratch repo.

## add
- `git worktree add [-f] [--detach] [--lock [--reason]] [-b <new-branch>] <path> [<commit-ish>]`. [doc]
- Same branch in two worktrees is REFUSED: `fatal: 'feat' is already used by worktree at '<path>'`. Same error from `git checkout feat` in the main repo. [test]
- `--force` overrides the check (the man page: when the branch is already checked out by another worktree, or the path is missing but registered). Do not use it for double-click UX. [doc]
- Design implication: double-click on a branch already in a worktree should focus that worktree's tab. Look up via `git worktree list --porcelain` (`branch refs/heads/<name>`). To check out in a second place use `--detach` or `-b <new>`. [infer]
- Creating at ~/.tidewatch/<repo>/<name> (outside the repo) is fine; worktree metadata lives in <main>/.git/worktrees/<id>/ and absolute paths are stored. [doc]

## list --porcelain
- One attribute per line, records separated by blank line; first line is `worktree <path>`. Fields: `HEAD <sha>`, `branch refs/heads/<name>`, `bare`, `detached`, `locked [reason]`, `prunable <reason>`. Boolean fields appear only if true. Use `-z` for NUL separators (lock reasons with odd characters are quoted otherwise). [doc]
- Deleted worktree directory shows as `prunable gitdir file points to non-existent location`. [doc, test]
- Bare entries have no HEAD/branch lines. Detached has `detached` and no `branch`. [doc]

## remove
- Refuses if there are modified or untracked files: `fatal: '<path>' contains modified or untracked files, use --force to delete it`. [test]
- Locked: `fatal: cannot remove a locked working tree, lock reason: app` (needs `remove -f -f` or unlock). [test]
- Main worktree cannot be removed. [doc]
- Safe pattern: `git worktree remove <path>` without --force; show the user the error. Branch is NOT deleted; delete separately with `git branch -d`. [doc/infer]

## prune
- Removes admin data in .git/worktrees for missing directories; runs automatically via gc after `gc.worktreePruneExpire` (default 3 months). `-n` dry-run, `-v`, `--expire`. [doc] [test: `git worktree prune -v` removed the entry for a deleted dir]
- `git worktree list` can annotate missing as prunable only if older than expire. [doc]

## lock
- Locking prevents prune, move and remove. Use for worktrees on removable/unmounted volumes. `--lock --reason` on add. [doc]
- Useful for Tidewatch: lock a worktree while an agent is running to avoid another tool removing it [infer].

## repair and "what if the main repo moves"
- If the main repo moves, linked worktrees cannot find it: `git status` in the linked worktree gives `fatal: not a git repository: (null)`. [test]
- Fix: run `git worktree repair <worktree-path>...` from the moved main repo, passing each worktree path. After repair, status works again. [test; doc: "running repair within the main worktree ... multiple linked worktrees"]
- Caveat from test: bare `git worktree repair` with no paths printed "repair: .git file broken" for every worktree, so pass explicit paths (and, for a moved linked worktree, run repair from inside it or the main repo). [test]
- If a LINKED worktree directory is moved manually, run repair from the main repo with the new path (or `git worktree move` next time). [doc]
- Tidewatch should store the repo path and worktree paths in its own DB and run repair on startup if `list` shows prunable or a worktree reports "not a git repository". [infer]

## Hooks and config sharing
- Hooks dir is shared (lives in the common git dir) so a hook installed in the main repo's .git/hooks runs for all worktrees. [doc: "By default, the repository config file is shared across all worktrees"; hooks follow `core.hooksPath` from config]
- ONE-worktree-only pre-push: 
  1. `git config extensions.worktreeConfig true` (run once in the repo).
  2. In that worktree: `git config --worktree core.hooksPath /abs/path/to/hooks-dir`.
  Stored in `.git/worktrees/<id>/config.worktree`, read after .git/config. [doc] [test: `rev-parse --git-path hooks` in the worktree returned the custom dir; `git config core.hooksPath` in main stayed empty]
- Caveats:
  - Setting `core.hooksPath` REPLACES the default hooks dir, so other hooks (husky, lefthook, user's own) stop running in that worktree. Your dir must delegate or re-implement them. [doc/infer]
  - `extensions.worktreeConfig` is a repo-wide extension: "Older Git versions will refuse to access repositories with this extension." [doc]
  - Before enabling, move `core.bare=true` and `core.worktree` out of the shared config into the main worktree's config.worktree. [doc]
  - `git config --worktree` without the extension enabled writes to the shared config (git may fall back); enable the extension first. [doc]
  - Absolute paths only for hooksPath outside the worktree; relative paths resolve from the worktree root [infer].
  - Modifies the user's repo-level config (a visible side effect); remove the key and optionally the extension on worktree teardown. [infer]
  - Alternative without extension: a per-worktree env var `GIT_CONFIG_COUNT/KEY_0/VALUE_0` (core.hooksPath) set only in the agent's terminal. Only affects git commands run from that terminal [infer, unverified]; no repo changes.
- Test note: `git worktree remove` also deletes .git/worktrees/<id>/config.worktree. [infer]

======================================================================
# GAPS
======================================================================
- Codex: exact `--ask-for-approval` value set (untrusted/on-failure still valid?); whether `-c` can set `notify`/`hooks`/`mcp_servers`; `notify` payload fields; OSC 9;4; session id selection. Codex not installed, only 3 pages fetched.
- Gemini: OSC 9/777/title behavior; settings file precedence and a per-session settings override; hooks in extensions; whether `--worktree` conflicts with Tidewatch-created worktrees.
- Claude: does `--settings` merge or replace hook sets; any built-in OSC 9;4 or notification channel (preferredNotifChannel); `--bare` implications; exact behaviors of agent_needs_input vs idle_prompt.
- Copilot: MCP transport list stated explicitly; payload fields for notification/permissionRequest events; COPILOT_HOME side effects on auth.
- Git: behavior of `git worktree repair` with no args after main moves (test was inconsistent); `GIT_CONFIG_COUNT` approach untested; Windows-specific notes not covered (macOS only).
- Budget: used ~22 tool calls, 7 WebFetch, 2 WebSearch.
