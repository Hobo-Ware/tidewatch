# Phase 0 spikes: agent CLI integration checks

Run 2026-10-12 against locally installed CLIs, each in a throwaway `mktemp -d` git repo, headless modes, `timeout 120`, tiny prompts.

| CLI | Version | Live runs |
|---|---|---|
| claude | 2.1.296 | yes |
| gemini | 0.46.0 | yes |
| copilot | 1.0.91 | yes |
| codex | 0.162.1 | blocked: not logged in |

Already verified elsewhere (not redone): Claude `--settings` http hooks, Gemini project `.gemini/settings.json` hooks with `GEMINI_CLI_TRUST_WORKSPACE=true`, Claude `--append-system-prompt-file`.

## Summary

| # | Spike | Claude | Gemini | Copilot | Codex |
|---|---|---|---|---|---|
| 1 | Terminal signals | n/a | n/a | PASS (negative: no OSC 9;4, title is not a status) | blocked |
| 2 | MCP per session (stdio) | PASS | PASS (needs `trust:true`) | PASS (needs `--allow-tool`) | blocked |
| 2 | MCP per session (http + bearer) | PASS | PASS | PASS | blocked |
| 3 | Instruction injection | already verified | PASS (relative name only) | PASS (`*.instructions.md` only) | blocked |
| 4 | Transcript location and format | n/a | PASS | PASS | blocked |
| 5 | `egui_commonmark` for egui 0.35 | 0.24.0 | | | |

## Spike 1: Copilot terminal signals

Goal: does Copilot emit OSC 9;4 progress or useful title sequences that Tidewatch can read from the pty?

Commands (temp repo, driven through a small Python `pty.fork` harness at 120x40 that logs every OSC and sends Enter to accept the folder-trust prompt):

```
script -q out.log copilot --no-auto-update -p "reply with the word hi" </dev/null
python3 ptyrun.py out.log 70 copilot --no-auto-update -i "Run ls, then reply with one sentence." --allow-tool=read
```

Result: PASS as a finding. Copilot never emits OSC 9;4. Its only title sequence is a plain OSC 0 that does not encode status.

Evidence (timeline of OSC sequences, interactive run):

```
0.7s  OSC 10;?   OSC 11;?   OSC 4;0..15;?     (colour queries, expects a reply)
0.8s  OSC 0   'GitHub Copilot'
4.6s  OSC 0   'Run ls, then reply with one sentence. - GitHub Copilot'
5.4s  OSC 0   'Execute LS Command - GitHub Copilot'
```

- `]9;4` count: 0 in both interactive and `-p` runs. No OSC 2, 9, 133, 777 or 1337 either.
- The 3 BEL bytes seen were OSC 0 terminators, not standalone bells. No bell on permission prompt or on completion.
- The title is only "app name", then "first prompt", then an LLM-written session title. It does not change between working, waiting for approval and idle, so it is not a status signal.
- `-p` under `script` produced 188 bytes with no OSC at all.
- On startup Copilot sends terminal queries (OSC 10/11/4 colours, `CSI ?2026$p`, `?996n`, `?u`). A pty host that never answers still works, but answering is cleaner.
- First interactive launch in a new directory shows a "Confirm folder trust" TUI that blocks until Enter. Worktrees will hit this unless trusted (`trustedFolders` in `~/.copilot/config.json`, or `--add-dir`). Not tested whether `--add-dir` suppresses the prompt.
- Screen text shows a spinner with "Working" and a footer with "Manual Approval", usable only via screen scraping.

Implication: for Copilot status use hooks (events.jsonl records `hook.start` / `hook.end`, see spike 4) or tail `events.jsonl` (`assistant.turn_start` / `assistant.turn_end` / `session.shutdown`), not terminal sequences.

## Spike 2: MCP server attached per session

Test server: dependency-free Python stdio JSON-RPC server, one tool `ping` returning `pong-<random>` (random is per server process). A second copy speaks plain JSON POST over http on `127.0.0.1` and checks `Authorization: Bearer <token>`, logging each request. Pass = the random string appears in the CLI output and the server-side log confirms the call.

Prompt for all: `Call the mcp tool ping on server tw and print exactly the string it returns, nothing else.`

### Claude (2.1.296)

```
claude -p "<prompt>" --mcp-config cfg.json --strict-mcp-config --allowedTools mcp__tw__ping </dev/null
```

- stdio cfg: `{"mcpServers":{"tw":{"command":"python3","args":["srv/mcp_stdio.py"]}}}`. Output `pong-507209`. PASS, no permission prompt.
- http cfg: `{"mcpServers":{"tw":{"type":"http","url":"http://127.0.0.1:18765/mcp","headers":{"Authorization":"Bearer <token>"}}}}`. Output `pong-H924099`. PASS.
- Server log for http: `auth_ok=True` on `server/discover`, `initialize`, `notifications/initialized`, `tools/list`, `tools/call`. Bearer header is sent on every request.
- Redirect stdin (`</dev/null`) or Claude waits 3s with a "no stdin data" warning.

### Copilot (1.0.91)

```
copilot -p "<prompt>" --additional-mcp-config @cfg.json --allow-tool='tw(ping)' --no-auto-update -s </dev/null
```

- `--additional-mcp-config` takes inline JSON or `@file`, repeatable, and augments `~/.copilot/mcp-config.json` for that session only.
- stdio cfg: `{"mcpServers":{"tw":{"type":"local","command":"python3","args":["srv/mcp_stdio.py"],"tools":["*"]}}}`. Output `pong-394553`. PASS.
- http cfg: `{"mcpServers":{"tw":{"type":"http","url":"http://127.0.0.1:18765/mcp","headers":{"Authorization":"Bearer <token>"},"tools":["*"]}}}`. Output `pong-H924099`, server log `auth_ok=True` through `tools/call`. PASS.
- Pre-allow syntax is `--allow-tool='<server>(<tool>)'` (or `<server>` for all its tools).
- Negative control, same command without `--allow-tool`: `Permission denied - approval was required but unavailable.` So the allow flag is required in `-p`; interactive would show a prompt.
- `--mcp-github-auth <server=origin>` exists for sending the GitHub credential to a loopback http server. Not needed for a bearer header.

### Gemini (0.46.0)

Project `.gemini/settings.json` in the temp repo, run with `GEMINI_CLI_TRUST_WORKSPACE=true`:

```
GEMINI_CLI_TRUST_WORKSPACE=true gemini -p "<prompt>" </dev/null
```

- stdio: `{"mcpServers":{"tw":{"command":"python3","args":["srv/mcp_stdio.py"],"trust":true}}}`. Output `pong-495782`, server log shows `call pong-495782`. PASS.
- http: `{"mcpServers":{"tw":{"httpUrl":"http://127.0.0.1:18765/mcp","headers":{"Authorization":"Bearer <token>"},"trust":true}}}`. Output `pong-H924099`, server log `auth_ok=True` through `tools/call`. PASS. The key is `httpUrl` (streamable http); plain `url` is the SSE transport.
- **Warning, false positive:** without `"trust":true` the tool is NOT executed in headless mode (server log empty), yet the model still printed a `pong-...` string, a stale value from an earlier run in the same project directory. Always confirm via a server-side log, never from model output alone.
- With a project settings file the model also tried `read_file` on the server script path and was blocked by workspace bounds; harmless.
- Not tested: project `mcpServers` without `GEMINI_CLI_TRUST_WORKSPACE`, and the `--policy` route instead of `trust:true`.

Implication: Tidewatch should write the per-session MCP entry into the worktree's `.gemini/settings.json` (merged with its hooks), with `trust:true` for its own server only.

## Spike 3: Instruction injection

Marker file text: `End every answer with the word TIDEWATCH-OK.` Repo-level instruction: `AGENTS.md` (Copilot) / `GEMINI.md` (Gemini) containing `Always say BANANA once in every answer.` Pass = marker present AND BANANA still present.

### Copilot (1.0.91)

Discovery: `COPILOT_CUSTOM_INSTRUCTIONS_DIRS=<dir> copilot instruction list`.

| File in the dir | `instruction list` shows it | Followed live |
|---|---|---|
| `AGENTS.md` | yes (as "Nested AGENTS.md") | NO (3 runs, marker never appeared) |
| `.github/copilot-instructions.md` | no | not tested |
| `copilot-instructions.md` (dir root) | no | not tested |
| `tw.instructions.md` with `applyTo: "**"` front matter | yes, tagged `[external]` | YES |

Command:

```
COPILOT_CUSTOM_INSTRUCTIONS_DIRS=<dir> copilot -p "In one sentence, what is a git worktree?" --no-auto-update -s </dev/null
```

Result: PASS using `*.instructions.md`. Every run with the file in place contained the marker, and BANANA from the repo AGENTS.md was also present in the 3 runs checked with full output.

```
A git worktree (BANANA) is a linked working directory that lets you ... without needing multiple clones. TIDEWATCH-OK
```

Notes:
- The `AGENTS.md` case is surprising: it is listed by `instruction list` yet had no effect on the model. Do not use it for injection.
- Front matter: `---\napplyTo: "**"\n---` then the text.
- Tidewatch can write one `tidewatch.instructions.md` outside the worktree and point the env var at its directory. Nothing is added to the repo.

### Gemini (0.46.0)

The bundled settings schema has `context.fileName` (string or array of strings; default `GEMINI.md`) and `context.loadMemoryFromIncludeDirectories` (bool). Setting `fileName` replaces the default, so list `GEMINI.md` too if the repo uses it.

| Setting | Result |
|---|---|
| `{"context":{"fileName":["GEMINI.md","tw-instructions.md"]}}` with the file inside the repo | PASS: `2 + 2 equals 4 BANANA. TIDEWATCH-OK` |
| `fileName` containing an absolute path outside the repo | FAIL: marker absent, only BANANA |
| `fileName:["GEMINI.md","tw-abs.md"]` + `loadMemoryFromIncludeDirectories:true` + `--include-directories <dir>` (file lives in `<dir>`) | PASS: `2+2 is 4 BANANA. TIDEWATCH-OK` |

Commands:

```
GEMINI_CLI_TRUST_WORKSPACE=true gemini -p "What is 2+2? Answer in one short sentence." --include-directories <dir>
```

Implication: for Gemini put a uniquely named file in an out-of-repo directory, pass it via `--include-directories` and set `loadMemoryFromIncludeDirectories:true` plus `fileName` in the same per-worktree settings file used for hooks and MCP. Alternative is a git-excluded file inside the worktree.

## Spike 4: Transcripts

### Gemini (0.46.0)

- Location: `~/.gemini/tmp/<project-slug>/chats/session-<YYYY-MM-DDTHH-MM>-<first 8 chars of sessionId>.jsonl`. `<project-slug>` is the repo directory basename (listed in `~/.gemini/projects.json`), so same-named worktrees may collide or get suffixed.
- `--session-id <uuid>` is honoured: file ended in `-8d36b033` for a uuid starting `8d36b033`.
- The SessionStart hook payload carries the path. Payload keys: `session_id, transcript_path, cwd, hook_event_name, timestamp, source`. Easiest source of truth for Tidewatch, since it already receives the hook.
- Format: JSONL. Line 1 is a header; later `{"$set":{...}}` lines patch session fields (for example `lastUpdated`); message lines are objects keyed by `id`, and the same `id` can appear again with more fields (tool calls attached), so the last record per `id` wins.
- Live or at end: LIVE, per completed message (not per token). File was 1154 B at the start, 1440 B after the user prompt, 3648 B once the final answer landed; the user line (21:22:06), tool-call message (21:22:08) and final answer (21:22:16) were each appended at their own timestamps.

Redacted sample:

```
{"sessionId":"<uuid>","projectHash":"<sha>","startTime":"...","lastUpdated":"...","kind":"main"}
{"id":"..","timestamp":"..","type":"user","content":[{"text":"First run the shell command 'cat notes.txt', then ..."}]}
{"id":"..","type":"gemini","content":"","thoughts":[{"subject":"..","description":".."}],"tokens":{"input":13238,"output":32,"cached":0,"thoughts":227},"model":"gemini-3.5-flash","toolCalls":[{"id":"run_shell_command__call_N","name":"run_shell_command","args":{"command":"cat notes.txt"},"status":"success","resultDisplay":"hello world","displayName":"Shell"}]}
{"id":"..","type":"user","content":[{"functionResponse":{"name":"run_shell_command","response":{"output":"hello world ..."}}}]}
{"id":"..","type":"gemini","content":"1. The silver moon pulls gently at the deep,\n2. ...","model":"gemini-3.5-flash"}
```

Roles: `type` is `user` or `gemini`. User text is `content[].text` (string items for the model, plus injected `<session_context>` blocks and `functionResponse` entries). Model text is `content` (string). Tool calls live inside the `gemini` message as `toolCalls[]` with args, status and result. Subagent sessions go in `chats/<sessionId>/<id>.jsonl`.

### Copilot (1.0.91)

- Location: `~/.copilot/session-state/<sessionId>/events.jsonl`, plus `workspace.yaml` (id, cwd, git_root, branch, name, created_at, updated_at). `COPILOT_HOME` relocates the base directory.
- `--session-id <uuid>` is honoured: the directory was created with exactly that uuid. Tidewatch can choose the id and know the path before launch.
- Format: JSONL, one event per line with keys `id, parentId, timestamp, type, data`. Event types seen: `session.start`, `session.model_change`, `hook.start`, `hook.end`, `user.message`, `system.message`, `assistant.turn_start`, `assistant.message`, `tool.execution_start`, `tool.execution_complete`, `assistant.turn_end`, `session.usage_checkpoint`, `session.shutdown`.
- Live or at end: LIVE, per event. Polling once a second showed 75115 B at 4s, 77640 B at 5s, 80135 B at 10s for a 15s run. Note the file starts large (about 75 kB) because `session.start` and `system.message` carry the full context.
- `hook.start` / `hook.end` events also appear in the file, including the user's own global hooks.

Redacted sample:

```
{"type":"user.message","data":{"content":"First run the shell command 'cat notes.txt', then ...","messageId":"..","turnId":".."}}
{"type":"assistant.message","data":{"model":"<model>","content":"","toolRequests":[{"toolCallId":"..","name":"bash","arguments":{"command":"cat notes.txt"}}]}}
{"type":"tool.execution_start","data":{"toolName":"bash","arguments":{"command":"cat notes.txt","description":".."}}}
{"type":"tool.execution_complete","data":{"success":true,"result":{"content":"hello world\n<shellId: 0 completed with exit code 0>"}}}
{"type":"assistant.message","data":{"content":"Output of `notes.txt`: `hello world` ...","toolRequests":[]}}
```

Roles: derived from `type` (`user.message` / `assistant.message`). Text is `data.content`; tool calls are `data.toolRequests[]` on the assistant message plus the `tool.execution_*` pair. Turn boundaries are `assistant.turn_start` / `assistant.turn_end`; `session.shutdown` marks process exit.

## Spike 5: `egui_commonmark` and egui 0.35

Commands:

```
curl -s -A "tidewatch-spike" https://crates.io/api/v1/crates/egui_commonmark/<ver>/dependencies
cargo info egui_commonmark@0.24
```

Result: PASS. **`egui_commonmark` 0.24.0 (2026-06-26) is the version for egui 0.35.**

| egui_commonmark | egui / egui_extras | pulldown-cmark | published |
|---|---|---|---|
| 0.25.0 | ^0.36.0 | ^0.13 | 2026-08-05 |
| **0.24.0** | **^0.35.0** | ^0.13 | 2026-06-26 |
| 0.23.0 | ^0.34.0 | ^0.13 | 2026-03-26 |
| 0.22.0 | ^0.33 | ^0.13 | 2025-10-09 |

`egui_commonmark_backend` is versioned in lockstep (`^0.24.0`). 0.24.0 needs rust-version 1.92. Default features are `load-images` and `pulldown_cmark`; `better_syntax_highlighting` is optional. Pin `egui_commonmark = "0.24"`; moving to egui 0.36 means 0.25.

## Surprises and cautions

- Gemini can print a plausible but stale MCP result when the tool never ran. Verify with a server-side log.
- Copilot's `AGENTS.md` in `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` is listed but not obeyed; `*.instructions.md` works.
- Copilot emits no OSC 9;4 and its title is not a state signal; use `events.jsonl` or hooks.
- Gemini ignores absolute paths in `context.fileName`; use `--include-directories`.
- Copilot interactive start in an untrusted directory blocks on a folder-trust TUI.
- Codex: blocked: not logged in. Nothing live was run; flags and hooks for Codex remain unverified here.
