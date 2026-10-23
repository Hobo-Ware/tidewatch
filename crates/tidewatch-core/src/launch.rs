//! Turns an agent, a profile and a tab's context into the program, arguments and environment to start.
//!
//! Pure: no process is started and nothing is written, except `ensure_no_push_helper` and the Codex
//! house-style read. Each piece of per-CLI wiring (hooks, MCP, house style) is applied only when its
//! `LaunchCtx` field is `Some`. The never-push guard (D7) and the YOLO rule (D11) live here.
//!
//! Wiring was checked against claude 2.1.296, gemini 0.46.0, copilot 1.0.91 and codex 0.162.1 (`--help`
//! and `docs/research/phase-0-spikes.md`). Codex's hooks, MCP server and never-push guard were run
//! live: `-c` overrides carry the hooks and the MCP server, and the MCP server needs
//! `default_tools_approval_mode="approve"`, otherwise Codex rejects its tools under approval `never`.
//! `Launch::experimental` is left for custom agents.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::agents::{Agent, Level, Wiring};
use crate::guard_shim::GuardCtx;
use crate::listener::ASK_TIMEOUT;
use crate::profiles::Profile;

/// Name of the MCP server Tidewatch attaches to every agent.
pub const MCP_SERVER_NAME: &str = "tidewatch";
pub const NO_PUSH_HELPER_NAME: &str = "git-remote-tidewatch-no-push";
pub const NO_PUSH_MESSAGE: &str = "Tidewatch: an agent tab may push only its own agent/ branch; Tidewatch pushes the rest after approval.";
pub const PUSH_BRANCH_ENV: &str = "TIDEWATCH_PUSH_BRANCH";
const NO_PUSH_URL_PREFIX: &str = "url.tidewatch-no-push://blocked/.pushInsteadOf";
const CODEX_MCP_TOKEN_ENV: &str = "TIDEWATCH_MCP_TOKEN";
/// How long a CLI waits for one Tidewatch tool call: the longest `ask_user` hold and five minutes
/// more, so the CLI never gives up first. Codex defaults to 60 s and Gemini to 600 s.
const MCP_TOOL_TIMEOUT_SECS: u64 = ASK_TIMEOUT.as_secs() + 5 * 60;
const CLAUDE_HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "Notification",
    "PermissionRequest",
    "Stop",
    "StopFailure",
];
const GEMINI_HOOK_EVENTS: [&str; 5] =
    ["SessionStart", "BeforeAgent", "Notification", "AfterAgent", "SessionEnd"];
const CODEX_HOOK_EVENTS: [&str; 4] =
    ["SessionStart", "UserPromptSubmit", "PermissionRequest", "Stop"];
/// Where the generated Gemini settings go, relative to the worktree.
pub const GEMINI_SETTINGS_PATH: &str = ".gemini/settings.json";

/// The local Tidewatch listener an agent reports to: hooks at `/hook/<agent>`, MCP at `/mcp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookListener {
    /// For example `http://127.0.0.1:51234`, no trailing slash.
    pub base_url: String,
    /// Per-tab secret, sent as a bearer token.
    pub token: String,
}

impl HookListener {
    #[must_use]
    pub fn hook_url(&self, agent_id: &str) -> String {
        format!("{}/hook/{agent_id}", self.base_url)
    }

    #[must_use]
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

/// Everything about the tab a launch depends on.
#[derive(Clone, Debug, Default)]
pub struct LaunchCtx {
    pub worktree: PathBuf,
    /// True for an agent worktree tab. False for the main checkout, Other and split panes.
    pub worktree_tab: bool,
    pub session_id: Option<String>,
    /// The name of the remote session. Setting it turns remote control on for a CLI that has a flag
    /// for it (`Agent::remote_args`); the others ignore it.
    pub remote: Option<String>,
    pub hooks: Option<HookListener>,
    /// The MCP config file the caller wrote (Claude, Copilot). Setting it turns MCP on for every CLI;
    /// Gemini and Codex also need `hooks` for the listener address.
    pub mcp_config: Option<PathBuf>,
    pub house_style: Option<PathBuf>,
    /// Directory holding `tidewatch.instructions.md` (Copilot), see `house_style::sync_copilot_instructions`.
    pub copilot_instructions_dir: Option<PathBuf>,
    /// Directory holding the never-push helper, see `ensure_no_push_helper`.
    pub guard_bin_dir: Option<PathBuf>,
    /// The tab's own `agent/<name>` branch, the only ref the push guard lets through.
    pub push_branch: Option<String>,
    /// Wakes the `git` and `gh` shims in `guard_bin_dir`, see `guard_shim`.
    pub guards: Option<GuardCtx>,
    /// The environment the agent would otherwise get. Read for `GIT_CONFIG_COUNT` and `PATH`.
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    /// Variables to set on top of `LaunchCtx::env`.
    pub env: Vec<(String, String)>,
    /// The agent's flags were not verified with a live run (Codex, custom agents).
    pub experimental: bool,
}

impl Launch {
    /// The command as one shell line, for the preview and for typing it into a shell.
    #[must_use]
    pub fn shell_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// YOLO only runs inside an agent worktree, never in the main checkout or Other.
    YoloOutsideWorktree,
    /// A custom agent has no flags for this level.
    LevelUnavailable { agent: String, level: Level },
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::YoloOutsideWorktree => {
                write!(
                    f,
                    "YOLO only runs in an agent worktree tab, not in the main checkout or Other"
                )
            }
            Self::LevelUnavailable { agent, level } => {
                write!(f, "{agent} has no flags for the {} level", level.label())
            }
        }
    }
}

impl std::error::Error for LaunchError {}

/// Builds the command for `profile` running `agent` in the tab described by `ctx`.
///
/// Argument order: permission level, model, effort, session id, remote control, wiring, then the profile's extra
/// arguments. Append a task prompt after a literal `--`: list-valued flags would swallow it otherwise.
pub fn launch_command(
    agent: &Agent,
    profile: &Profile,
    ctx: &LaunchCtx,
) -> Result<Launch, LaunchError> {
    if profile.level == Level::Yolo && !ctx.worktree_tab {
        return Err(LaunchError::YoloOutsideWorktree);
    }
    let level_args = agent.level_args(profile.level).ok_or_else(|| {
        LaunchError::LevelUnavailable { agent: agent.id.clone(), level: profile.level }
    })?;

    let mut args = level_args.to_vec();
    if let (Some(flag), Some(model)) = (&agent.model_flag, non_empty(profile.model.as_deref())) {
        args.extend([flag.clone(), model.to_owned()]);
    }
    if let (Some(effort), Some(value)) = (&agent.effort, non_empty(profile.effort.as_deref())) {
        args.extend([effort.flag.clone(), format!("{}{value}", effort.value_prefix)]);
    }
    if let (Some(flag), Some(id)) = (&agent.session_id_flag, ctx.session_id.as_deref()) {
        args.extend([flag.clone(), id.to_owned()]);
    }
    if let Some(name) = &ctx.remote {
        args.extend(agent.remote_args(name));
    }

    let mut env = Vec::new();
    let house_style = profile.house_style.then_some(ctx.house_style.as_deref()).flatten();
    match agent.wiring {
        Wiring::Claude => claude_wiring(ctx, house_style, &mut args),
        Wiring::Codex => codex_wiring(agent, ctx, house_style, &mut args, &mut env),
        Wiring::Gemini => gemini_wiring(house_style, &mut args, &mut env),
        Wiring::Copilot => copilot_wiring(ctx, house_style, &mut args, &mut env),
        Wiring::Custom => {}
    }
    if profile.never_push && ctx.worktree_tab {
        env.extend(guard_env(&ctx.env, ctx.guard_bin_dir.as_deref(), ctx.push_branch.as_deref()));
    }
    if ctx.worktree_tab
        && let (Some(guards), Some(dir)) = (&ctx.guards, ctx.guard_bin_dir.as_deref())
    {
        env.extend(guards.env(dir));
        if !env.iter().any(|(k, _)| k == "PATH") {
            let inherited = std::env::var("PATH").map(|path| vec![("PATH".to_owned(), path)]);
            let base = if ctx.env.iter().any(|(k, _)| k == "PATH") {
                ctx.env.clone()
            } else {
                inherited.unwrap_or_default()
            };
            env.extend(path_with_first(&base, dir));
        }
    }
    args.extend(profile.extra_args.iter().cloned());

    Ok(Launch { program: agent.bin.clone(), args, env, experimental: !agent.verified })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

fn claude_wiring(ctx: &LaunchCtx, house_style: Option<&Path>, args: &mut Vec<String>) {
    let comment_guard =
        ctx.guards.as_ref().filter(|_| ctx.worktree_tab).map(|guards| guards.bin.as_path());
    if let Some(settings) = claude_settings(ctx.hooks.as_ref(), comment_guard) {
        args.extend(["--settings".to_owned(), settings]);
    }
    if let Some(config) = &ctx.mcp_config {
        args.push(format!("--mcp-config={}", config.display()));
        args.push(format!("--allowedTools=mcp__{MCP_SERVER_NAME}__*"));
    }
    if let Some(file) = house_style {
        args.extend(["--append-system-prompt-file".to_owned(), file.display().to_string()]);
    }
}

fn gemini_wiring(
    house_style: Option<&Path>,
    args: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
) {
    env.push(("GEMINI_CLI_TRUST_WORKSPACE".into(), "true".into()));
    if let Some(dir) = house_style.and_then(Path::parent) {
        args.push(format!("--include-directories={}", dir.display()));
    }
}

fn copilot_wiring(
    ctx: &LaunchCtx,
    house_style: Option<&Path>,
    args: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
) {
    if let Some(config) = &ctx.mcp_config {
        args.extend(["--additional-mcp-config".to_owned(), format!("@{}", config.display())]);
        args.push(format!("--allow-tool={MCP_SERVER_NAME}"));
    }
    if let (Some(_), Some(dir)) = (house_style, &ctx.copilot_instructions_dir) {
        env.push(("COPILOT_CUSTOM_INSTRUCTIONS_DIRS".into(), dir.display().to_string()));
    }
}

fn codex_wiring(
    agent: &Agent,
    ctx: &LaunchCtx,
    house_style: Option<&Path>,
    args: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
) {
    if let Some(listener) = &ctx.hooks {
        let command = curl_hook_command(&listener.hook_url(&agent.id), &listener.bearer());
        for event in CODEX_HOOK_EVENTS {
            let value = format!(
                "hooks.{event}=[{{hooks=[{{type=\"command\",command={}}}]}}]",
                toml_string(&command)
            );
            args.extend(["-c".to_owned(), value]);
        }
        args.push("--dangerously-bypass-hook-trust".to_owned());
        if ctx.mcp_config.is_some() {
            args.extend([
                "-c".to_owned(),
                format!("mcp_servers.{MCP_SERVER_NAME}.url={}", toml_string(&listener.mcp_url())),
                "-c".to_owned(),
                format!(
                    "mcp_servers.{MCP_SERVER_NAME}.bearer_token_env_var={}",
                    toml_string(CODEX_MCP_TOKEN_ENV)
                ),
                "-c".to_owned(),
                format!("mcp_servers.{MCP_SERVER_NAME}.default_tools_approval_mode=\"approve\""),
                "-c".to_owned(),
                format!("mcp_servers.{MCP_SERVER_NAME}.tool_timeout_sec={MCP_TOOL_TIMEOUT_SECS}"),
            ]);
            env.push((CODEX_MCP_TOKEN_ENV.into(), listener.token.clone()));
        }
    }
    let text = house_style.and_then(|file| std::fs::read_to_string(file).ok());
    if let Some(text) = text {
        args.extend(["-c".to_owned(), format!("developer_instructions={}", toml_string(&text))]);
    }
}

fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

pub(crate) fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain { word.to_owned() } else { format!("'{}'", word.replace('\'', "'\\''")) }
}

/// Posts the hook payload from stdin to the listener and never fails or prints, so a CLI that waits
/// on its hook is never held up. Used for the CLIs whose hooks only run commands.
fn curl_hook_command(url: &str, bearer: &str) -> String {
    format!(
        "curl -s -m 2 -X POST -H {} -H 'Content-Type: application/json' --data-binary @- {} >/dev/null 2>&1 || true",
        shell_quote(&format!("Authorization: {bearer}")),
        shell_quote(url)
    )
}

/// The `--settings` JSON for Claude: http hooks for the events the status layer reads (D5).
#[must_use]
pub fn claude_hook_settings(listener: &HookListener) -> String {
    claude_settings(Some(listener), None).unwrap_or_default()
}

/// The `--settings` JSON for Claude: the http hooks of the status layer when there is a `listener`,
/// and, when `comment_guard` names the Tidewatch binary, the comment guard's command hooks beside
/// them: PostToolUse after Write, Edit and MultiEdit, and a Stop sweep of the branch diff. `None`
/// when neither applies.
#[must_use]
pub fn claude_settings(
    listener: Option<&HookListener>,
    comment_guard: Option<&Path>,
) -> Option<String> {
    let mut hooks = serde_json::Map::new();
    if let Some(listener) = listener {
        let hook = json!({
            "type": "http",
            "url": listener.hook_url("claude"),
            "headers": { "Authorization": listener.bearer() },
        });
        for event in CLAUDE_HOOK_EVENTS {
            hooks.insert(event.to_owned(), json!([{ "hooks": [hook.clone()] }]));
        }
    }
    if let Some(bin) = comment_guard {
        let command = |tail: &str| json!({ "type": "command", "command": format!("{} --guard comments{tail}", shell_quote(&bin.display().to_string())) });
        let mut push = |event: &str, group: Value| {
            let slot = hooks.entry(event.to_owned()).or_insert_with(|| json!([]));
            if let Some(groups) = slot.as_array_mut() {
                groups.push(group);
            }
        };
        push("PostToolUse", json!({ "matcher": "Write|Edit|MultiEdit", "hooks": [command("")] }));
        push("Stop", json!({ "hooks": [command(" --stop")] }));
    }
    (!hooks.is_empty()).then(|| json!({ "hooks": hooks }).to_string())
}

/// The MCP config file content for the CLIs that read one (Claude, Copilot). `None` for the others.
#[must_use]
pub fn mcp_config_json(wiring: Wiring, listener: &HookListener) -> Option<String> {
    let mut server = json!({
        "type": "http",
        "url": listener.mcp_url(),
        "headers": { "Authorization": listener.bearer() },
    });
    match wiring {
        Wiring::Claude => {}
        Wiring::Copilot => server["tools"] = json!(["*"]),
        Wiring::Codex | Wiring::Gemini | Wiring::Custom => return None,
    }
    Some(json!({ "mcpServers": { MCP_SERVER_NAME: server } }).to_string())
}

/// The project `.gemini/settings.json` for a Gemini tab: command hooks, the Tidewatch MCP server with
/// `trust`, and a `context.fileName` list that keeps `GEMINI.md` and `AGENTS.md` next to the house
/// style. `None` when the context asks for none of them.
///
/// The caller writes it into the worktree and adds `gemini_exclude_lines()` to the worktree's
/// `info/exclude`. A repo that tracks its own `.gemini/settings.json` needs a merge instead.
#[must_use]
pub fn gemini_project_settings(profile: &Profile, ctx: &LaunchCtx) -> Option<String> {
    let mut settings = serde_json::Map::new();
    if let Some(listener) = &ctx.hooks {
        let command = curl_hook_command(&listener.hook_url("gemini"), &listener.bearer());
        let hook = json!({ "type": "command", "command": command });
        let hooks: serde_json::Map<String, Value> = GEMINI_HOOK_EVENTS
            .iter()
            .map(|event| ((*event).to_owned(), json!([{ "hooks": [hook.clone()] }])))
            .collect();
        settings.insert("hooks".into(), Value::Object(hooks));
        if ctx.mcp_config.is_some() {
            settings.insert(
                "mcpServers".into(),
                json!({ MCP_SERVER_NAME: {
                    "httpUrl": listener.mcp_url(),
                    "headers": { "Authorization": listener.bearer() },
                    "trust": true,
                    "timeout": MCP_TOOL_TIMEOUT_SECS * 1000,
                } }),
            );
        }
    }
    let house_name = ctx
        .house_style
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|_| profile.house_style);
    if let Some(name) = house_name {
        settings.insert(
            "context".into(),
            json!({
                "fileName": ["GEMINI.md", "AGENTS.md", name],
                "loadMemoryFromIncludeDirectories": true,
            }),
        );
    }
    (!settings.is_empty()).then(|| Value::Object(settings).to_string())
}

/// Lines for the worktree's `info/exclude` so the generated Gemini settings are never committed.
#[must_use]
pub fn gemini_exclude_lines() -> [&'static str; 1] {
    ["/.gemini/settings.json"]
}

/// Environment that routes every `git push` in the tab through the guard helper (D7).
///
/// Appends one `url.tidewatch-no-push://blocked/.pushInsteadOf` entry (empty value, so it matches every
/// remote) after any `GIT_CONFIG_COUNT` entries already in `base_env`, names the tab's own branch in
/// `TIDEWATCH_PUSH_BRANCH`, and puts `guard_bin_dir` first on `PATH` so git finds the helper. The helper
/// forwards a push of that branch alone and refuses everything else. Without a helper every push
/// fails, with git's own "remote helper not found" error.
#[must_use]
pub fn guard_env(
    base_env: &[(String, String)],
    guard_bin_dir: Option<&Path>,
    push_branch: Option<&str>,
) -> Vec<(String, String)> {
    let var = |key: &str| base_env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let count = var("GIT_CONFIG_COUNT").and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(0);
    let mut env = vec![
        ("GIT_CONFIG_COUNT".to_owned(), (count + 1).to_string()),
        (format!("GIT_CONFIG_KEY_{count}"), NO_PUSH_URL_PREFIX.to_owned()),
        (format!("GIT_CONFIG_VALUE_{count}"), String::new()),
        (PUSH_BRANCH_ENV.to_owned(), push_branch.unwrap_or_default().to_owned()),
    ];
    env.extend(guard_bin_dir.and_then(|dir| path_with_first(base_env, dir)));
    env
}

/// `PATH` from `base_env` with `dir` in front.
pub(crate) fn path_with_first(
    base_env: &[(String, String)],
    dir: &Path,
) -> Option<(String, String)> {
    let path = base_env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.as_str());
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(path.iter().flat_map(|path| std::env::split_paths(path)));
    let joined = std::env::join_paths(dirs).ok()?;
    Some(("PATH".to_owned(), joined.to_string_lossy().into_owned()))
}

/// Content of the `git-remote-tidewatch-no-push` helper, a git remote helper (`gitremote-helpers`):
/// it lists the real remote's refs, and forwards a push batch to the real url only when every ref
/// in it is `refs/heads/$TIDEWATCH_PUSH_BRANCH`. Deletes, tags and any other ref refuse the batch.
#[must_use]
pub fn no_push_helper_script() -> String {
    GUARD_HELPER_TEMPLATE
        .replace("@MESSAGE@", NO_PUSH_MESSAGE)
        .replace("@GUARD_KEY@", NO_PUSH_URL_PREFIX)
}

const GUARD_HELPER_TEMPLATE: &str = r#"#!/bin/sh
url="${2#tidewatch-no-push://blocked/}"
own="${TIDEWATCH_PUSH_BRANCH:-}"
i=0
while [ "$i" -lt "${GIT_CONFIG_COUNT:-0}" ]; do
  eval "key=\${GIT_CONFIG_KEY_$i:-}"
  if [ "$key" = "@GUARD_KEY@" ]; then export "GIT_CONFIG_KEY_$i=tidewatch.pushguard"; fi
  i=$((i + 1))
done
nl='
'
dry=false
force=
leases=
batch=
finish() {
  out=
  specs=
  refused=
  old_ifs=$IFS
  IFS=$nl
  for spec in $batch; do
    src="${spec%%:*}"
    dst="${spec#*:}"
    plus=$force
    case "$src" in +*) plus=+; src="${src#+}" ;; esac
    case "$leases" in *"=$dst:"*) plus= ;; esac
    if [ -z "$src" ]; then
      refused=yes
      out="${out}error $dst deleting a branch is blocked in agent tabs$nl"
    elif [ -z "$own" ] || [ "$dst" != "refs/heads/$own" ]; then
      refused=yes
      out="${out}error $dst only refs/heads/${own:-agent/<name>} can be pushed from this tab$nl"
    else
      out="${out}ok $dst$nl"
      specs="$specs$plus$src:$dst$nl"
    fi
  done
  IFS=$old_ifs
  if [ -n "$refused" ]; then
    echo "@MESSAGE@" >&2
    printf '%s' "$out" | sed 's/^ok \(.*\)$/error \1 refused with the rest of the push/'
  elif [ "$dry" = true ]; then
    printf '%s' "$out"
  else
    IFS=$nl
    if git push $leases "$url" $specs </dev/null 1>&2; then
      IFS=$old_ifs
      printf '%s' "$out"
    else
      IFS=$old_ifs
      printf '%s' "$out" | sed 's/^ok \(.*\)$/error \1 the push to the remote failed/'
    fi
  fi
  echo
  batch=
}
while IFS= read -r line; do
  case "$line" in
    capabilities) printf 'option\npush\n\n' ;;
    "option dry-run "*) dry="${line#option dry-run }"; echo ok ;;
    "option force true") force=+; echo ok ;;
    "option cas "*) leases="$leases$nl--force-with-lease=${line#option cas }"; echo ok ;;
    "option verbosity "* | "option progress "* | "option force "*) echo ok ;;
    "option "*) echo unsupported ;;
    list | "list for-push")
      git ls-remote "$url" </dev/null 2>/dev/null | while read -r sha ref; do
        case "$ref" in HEAD | *"^{}") ;; *) echo "$sha $ref" ;; esac
      done
      echo
      ;;
    "push "*) batch="$batch${line#push }$nl" ;;
    "")
      if [ -z "$batch" ]; then exit 0; fi
      finish
      ;;
    *) echo "unsupported command: $line" >&2; exit 1 ;;
  esac
done
"#;

/// Writes the helper into `dir` as an executable file. Rewrites it only when the content differs.
pub fn ensure_no_push_helper(dir: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(NO_PUSH_HELPER_NAME);
    let script = no_push_helper_script();
    let current = std::fs::read_to_string(&file).ok();
    let executable = file.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
    if current.as_deref() != Some(script.as_str()) || !executable {
        crate::fsutil::write_atomic(&file, &script)?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentTable;

    fn profile(agent: &str, level: Level) -> Profile {
        Profile {
            name: "p".into(),
            agent: agent.into(),
            model: None,
            effort: None,
            level,
            setup: String::new(),
            never_push: false,
            extra_args: crate::agents::ArgList::default(),
            house_style: true,
        }
    }

    fn worktree_ctx() -> LaunchCtx {
        LaunchCtx { worktree: "/wt".into(), worktree_tab: true, ..LaunchCtx::default() }
    }

    fn listener() -> HookListener {
        HookListener { base_url: "http://127.0.0.1:5555".into(), token: "tok123".into() }
    }

    fn full_ctx() -> LaunchCtx {
        LaunchCtx {
            session_id: Some("11111111-2222-3333-4444-555555555555".into()),
            hooks: Some(listener()),
            mcp_config: Some("/state/tab/mcp.json".into()),
            house_style: Some("/cfg/house-style.md".into()),
            copilot_instructions_dir: Some("/cfg/copilot".into()),
            guard_bin_dir: Some("/state/guard-bin".into()),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            ..worktree_ctx()
        }
    }

    fn table() -> AgentTable {
        AgentTable::builtin()
    }

    fn run(agent: &str, profile: &Profile, ctx: &LaunchCtx) -> Launch {
        launch_command(table().get(agent).unwrap(), profile, ctx).unwrap()
    }

    fn env_of<'a>(launch: &'a Launch, key: &str) -> Option<&'a str> {
        launch.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn remote_control_adds_the_clis_flag_after_the_session_id_and_only_where_there_is_one() {
        let name = "acme-web \u{b7} otter";
        let cases: [(&str, Vec<&str>); 4] = [
            ("claude", vec!["--remote-control", name]),
            ("copilot", vec!["--remote"]),
            ("codex", vec![]),
            ("gemini", vec![]),
        ];
        for (agent, flag) in cases {
            let base = LaunchCtx {
                session_id: Some("11111111-2222-3333-4444-555555555555".into()),
                ..worktree_ctx()
            };
            let off = run(agent, &profile(agent, Level::Edits), &base);
            let on_ctx = LaunchCtx { remote: Some(name.into()), ..base };
            let on = run(agent, &profile(agent, Level::Edits), &on_ctx);
            if flag.is_empty() {
                assert_eq!(on.args, off.args, "{agent} has no flag to add");
                continue;
            }
            let at = off.args.len();
            assert_eq!(&on.args[..at], &off.args[..], "{agent} keeps its own arguments");
            assert_eq!(&on.args[at..], flag.as_slice(), "{agent}");
        }
    }

    #[test]
    fn remote_control_sits_between_the_session_id_and_the_wiring() {
        let ctx = LaunchCtx { remote: Some("web \u{b7} otter".into()), ..full_ctx() };
        let launch = run("claude", &profile("claude", Level::Edits), &ctx);
        let at = |needle: &str| launch.args.iter().position(|a| a == needle).unwrap();
        assert_eq!(launch.args[at("--remote-control") + 1], "web \u{b7} otter");
        assert!(at("--session-id") < at("--remote-control"));
        assert!(at("--remote-control") < at("--settings"));
        let copilot = run("copilot", &profile("copilot", Level::Edits), &ctx);
        let at = |needle: &str| copilot.args.iter().position(|a| a == needle).unwrap();
        assert!(at("--session-id") < at("--remote"));
        assert!(at("--remote") < at("--additional-mcp-config"));
    }

    #[test]
    fn the_shell_line_quotes_the_session_name() {
        let ctx = LaunchCtx { remote: Some("acme-web \u{b7} otter".into()), ..worktree_ctx() };
        let line = run("claude", &profile("claude", Level::Edits), &ctx).shell_line();
        assert!(line.contains("--remote-control 'acme-web \u{b7} otter'"), "{line}");
    }

    #[test]
    fn every_agent_and_level_launches_with_exactly_the_table_flags_cli_versions_2_1_296_0_162_1_0_46_0_1_0_91()
     {
        let cases: [(&str, [&str; 5]); 4] = [
            (
                "claude",
                [
                    "--permission-mode dontAsk --allowedTools=Read,Grep,Glob",
                    "--permission-mode manual",
                    "--permission-mode acceptEdits",
                    "--permission-mode auto",
                    "--dangerously-skip-permissions",
                ],
            ),
            (
                "codex",
                [
                    "--sandbox read-only --ask-for-approval never",
                    "--sandbox read-only --ask-for-approval on-request",
                    "--sandbox workspace-write --ask-for-approval on-request",
                    "--approve-for-me",
                    "--dangerously-bypass-approvals-and-sandbox",
                ],
            ),
            (
                "gemini",
                [
                    "--approval-mode plan",
                    "--approval-mode default",
                    "--approval-mode auto_edit",
                    "--approval-mode yolo --sandbox",
                    "--yolo",
                ],
            ),
            ("copilot", ["--mode plan", "", "--allow-tool=write", "--allow-all-tools", "--yolo"]),
        ];
        for (agent, flags) in cases {
            for (level, flags) in Level::ALL.into_iter().zip(flags) {
                let launch = run(agent, &profile(agent, level), &worktree_ctx());
                let want: Vec<&str> = flags.split_whitespace().collect();
                assert_eq!(launch.program, agent, "{agent} {level:?}");
                assert_eq!(launch.args, want, "{agent} {level:?}");
            }
        }
    }

    #[test]
    fn yolo_is_refused_outside_a_worktree_tab_for_every_agent() {
        let ctx = LaunchCtx { worktree_tab: false, ..worktree_ctx() };
        for agent in ["claude", "codex", "gemini", "copilot"] {
            let result =
                launch_command(table().get(agent).unwrap(), &profile(agent, Level::Yolo), &ctx);
            assert_eq!(result, Err(LaunchError::YoloOutsideWorktree), "{agent}");
            let non_yolo =
                launch_command(table().get(agent).unwrap(), &profile(agent, Level::Auto), &ctx);
            assert!(non_yolo.is_ok(), "{agent} auto outside a worktree");
        }
    }

    #[test]
    fn yolo_runs_in_a_worktree_tab() {
        let launch = run("claude", &profile("claude", Level::Yolo), &worktree_ctx());
        assert_eq!(launch.args, ["--dangerously-skip-permissions"]);
    }

    #[test]
    fn a_custom_agent_without_a_level_cannot_launch_at_it() {
        let overrides = toml::from_str::<crate::profiles::Profiles>(
            "[agent.aider]\nbin = \"aider\"\n[agent.aider.permissions]\nplan = \"--chat-mode ask\"\n",
        )
        .unwrap();
        let table = overrides.agent_table();
        let aider = table.get("aider").unwrap();
        let ok = launch_command(aider, &profile("aider", Level::Plan), &worktree_ctx()).unwrap();
        assert_eq!(
            (ok.args, ok.experimental),
            (vec!["--chat-mode".to_owned(), "ask".to_owned()], true)
        );
        let err =
            launch_command(aider, &profile("aider", Level::Edits), &worktree_ctx()).unwrap_err();
        assert_eq!(
            err,
            LaunchError::LevelUnavailable { agent: "aider".into(), level: Level::Edits }
        );
        assert_eq!(err.to_string(), "aider has no flags for the Auto edit level");
    }

    #[test]
    fn model_effort_session_and_extra_args_follow_the_level_in_order() {
        let mut p = profile("claude", Level::Edits);
        p.model = Some("opus".into());
        p.effort = Some("high".into());
        p.extra_args = vec!["--verbose".to_owned()].into();
        let ctx = LaunchCtx { session_id: Some("sid".into()), ..worktree_ctx() };
        assert_eq!(
            run("claude", &p, &ctx).args,
            [
                "--permission-mode",
                "acceptEdits",
                "--model",
                "opus",
                "--effort",
                "high",
                "--session-id",
                "sid",
                "--verbose"
            ]
        );
        let mut codex = profile("codex", Level::Plan);
        codex.model = Some("gpt-x".into());
        codex.effort = Some("low".into());
        assert_eq!(
            run("codex", &codex, &ctx).args,
            [
                "--sandbox",
                "read-only",
                "--ask-for-approval",
                "never",
                "--model",
                "gpt-x",
                "-c",
                "model_reasoning_effort=low"
            ]
        );
        let mut gemini = profile("gemini", Level::Plan);
        gemini.effort = Some("high".into());
        gemini.model = Some(String::new());
        assert_eq!(
            run("gemini", &gemini, &ctx).args,
            ["--approval-mode", "plan", "--session-id", "sid"]
        );
    }

    #[test]
    fn no_context_fields_means_no_wiring_args() {
        let mut ctx = worktree_ctx();
        ctx.env.clear();
        for (agent, extra_env) in [("claude", 0), ("codex", 0), ("gemini", 1), ("copilot", 0)] {
            let launch = run(agent, &profile(agent, Level::Plan), &ctx);
            let base = table().get(agent).unwrap().level_args(Level::Plan).unwrap().to_vec();
            assert_eq!(launch.args, base, "{agent}");
            assert_eq!(launch.env.len(), extra_env, "{agent}");
        }
    }

    #[test]
    fn claude_gets_hooks_mcp_and_house_style() {
        let launch = run("claude", &profile("claude", Level::Edits), &full_ctx());
        let args = &launch.args;
        let settings_at = args.iter().position(|a| a == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&args[settings_at + 1]).unwrap();
        assert_eq!(settings["hooks"]["Stop"][0]["hooks"][0]["type"], "http");
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["url"],
            "http://127.0.0.1:5555/hook/claude"
        );
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["headers"]["Authorization"],
            "Bearer tok123"
        );
        for event in CLAUDE_HOOK_EVENTS {
            assert!(settings["hooks"][event].is_array(), "{event}");
        }
        for want in
            ["--mcp-config=/state/tab/mcp.json", "--allowedTools=mcp__tidewatch__*", "--session-id"]
        {
            assert!(args.iter().any(|a| a == want), "{want}");
        }
        let style_at = args.iter().position(|a| a == "--append-system-prompt-file").unwrap();
        assert_eq!(args[style_at + 1], "/cfg/house-style.md");
        assert_eq!(launch.env, Vec::<(String, String)>::new());
        assert!(!launch.experimental);
    }

    #[test]
    fn each_claude_piece_is_gated_on_its_own_field() {
        let ctx = LaunchCtx { hooks: None, mcp_config: None, ..full_ctx() };
        let args = run("claude", &profile("claude", Level::Plan), &ctx).args;
        assert!(args.iter().any(|a| a == "--append-system-prompt-file"));
        assert!(!args.iter().any(|a| a == "--settings" || a.starts_with("--mcp-config")));
        let ctx = LaunchCtx { house_style: None, ..full_ctx() };
        let args = run("claude", &profile("claude", Level::Plan), &ctx).args;
        assert!(!args.iter().any(|a| a == "--append-system-prompt-file"));
        assert!(args.iter().any(|a| a == "--settings"));
    }

    #[test]
    fn a_profile_can_switch_the_house_style_off() {
        let mut p = profile("claude", Level::Plan);
        p.house_style = false;
        let args = run("claude", &p, &full_ctx()).args;
        assert!(!args.iter().any(|a| a == "--append-system-prompt-file"));
    }

    #[test]
    fn copilot_gets_mcp_allow_and_instructions_dir() {
        let launch = run("copilot", &profile("copilot", Level::Edits), &full_ctx());
        let at = launch.args.iter().position(|a| a == "--additional-mcp-config").unwrap();
        assert_eq!(launch.args[at + 1], "@/state/tab/mcp.json");
        assert!(launch.args.contains(&"--allow-tool=tidewatch".to_owned()));
        assert!(launch.args.contains(&"--allow-tool=write".to_owned()));
        assert_eq!(env_of(&launch, "COPILOT_CUSTOM_INSTRUCTIONS_DIRS"), Some("/cfg/copilot"));
        let no_dir = LaunchCtx { copilot_instructions_dir: None, ..full_ctx() };
        let launch = run("copilot", &profile("copilot", Level::Plan), &no_dir);
        assert_eq!(env_of(&launch, "COPILOT_CUSTOM_INSTRUCTIONS_DIRS"), None);
    }

    #[test]
    fn gemini_trusts_the_workspace_and_includes_the_house_style_directory() {
        let launch = run("gemini", &profile("gemini", Level::Plan), &full_ctx());
        assert_eq!(env_of(&launch, "GEMINI_CLI_TRUST_WORKSPACE"), Some("true"));
        assert!(launch.args.contains(&"--include-directories=/cfg".to_owned()));
        let none = run("gemini", &profile("gemini", Level::Plan), &worktree_ctx());
        assert_eq!(env_of(&none, "GEMINI_CLI_TRUST_WORKSPACE"), Some("true"));
        assert!(!none.args.iter().any(|a| a.starts_with("--include-directories")));
    }

    #[test]
    fn gemini_settings_carry_hooks_trusted_mcp_and_the_context_file_list() {
        let json: Value = serde_json::from_str(
            &gemini_project_settings(&profile("gemini", Level::Plan), &full_ctx()).unwrap(),
        )
        .unwrap();
        for event in GEMINI_HOOK_EVENTS {
            let hook = &json["hooks"][event][0]["hooks"][0];
            assert_eq!(hook["type"], "command", "{event}");
            let command = hook["command"].as_str().unwrap();
            assert!(command.contains("http://127.0.0.1:5555/hook/gemini"), "{event}");
            assert!(command.contains("Authorization: Bearer tok123"), "{event}");
        }
        assert_eq!(json["mcpServers"]["tidewatch"]["httpUrl"], "http://127.0.0.1:5555/mcp");
        assert_eq!(json["mcpServers"]["tidewatch"]["trust"], true);
        assert_eq!(json["mcpServers"]["tidewatch"]["timeout"], 2_100_000);
        assert_eq!(json["mcpServers"]["tidewatch"]["headers"]["Authorization"], "Bearer tok123");
        assert_eq!(
            json["context"]["fileName"],
            json!(["GEMINI.md", "AGENTS.md", "house-style.md"])
        );
        assert_eq!(json["context"]["loadMemoryFromIncludeDirectories"], true);
    }

    #[test]
    fn gemini_settings_are_gated_and_empty_means_nothing_to_write() {
        let bare = worktree_ctx();
        assert_eq!(gemini_project_settings(&profile("gemini", Level::Plan), &bare), None);
        let hooks_only = LaunchCtx { hooks: Some(listener()), ..worktree_ctx() };
        let json: Value = serde_json::from_str(
            &gemini_project_settings(&profile("gemini", Level::Plan), &hooks_only).unwrap(),
        )
        .unwrap();
        assert!(json.get("mcpServers").is_none());
        assert!(json.get("context").is_none());
        let mut style_off_profile = profile("gemini", Level::Plan);
        style_off_profile.house_style = false;
        let style_off = gemini_project_settings(&style_off_profile, &full_ctx()).unwrap();
        assert!(!style_off.contains("fileName"));
        assert_eq!(gemini_exclude_lines(), ["/.gemini/settings.json"]);
        assert_eq!(GEMINI_SETTINGS_PATH, ".gemini/settings.json");
    }

    #[test]
    fn codex_gets_c_overrides_for_hooks_and_the_mcp_server() {
        let dir = tempfile::tempdir().unwrap();
        let style = dir.path().join("house-style.md");
        std::fs::write(&style, "Say \"hi\"\nand \\ more").unwrap();
        let ctx = LaunchCtx { house_style: Some(style), ..full_ctx() };
        let launch = run("codex", &profile("codex", Level::Edits), &ctx);
        assert!(!launch.experimental);
        assert!(launch.args.contains(&"--dangerously-bypass-hook-trust".to_owned()));
        let overrides: Vec<&str> =
            launch.args.windows(2).filter(|w| w[0] == "-c").map(|w| w[1].as_str()).collect();
        assert_eq!(overrides.len(), CODEX_HOOK_EVENTS.len() + 5);
        for event in CODEX_HOOK_EVENTS {
            let hook =
                overrides.iter().find(|o| o.starts_with(&format!("hooks.{event}="))).unwrap();
            let value = hook.split_once('=').unwrap().1;
            let parsed: toml::Value = toml::from_str(&format!("x = {value}")).unwrap();
            let command = parsed["x"][0]["hooks"][0]["command"].as_str().unwrap();
            assert!(command.contains("http://127.0.0.1:5555/hook/codex"), "{event}");
            assert_eq!(parsed["x"][0]["hooks"][0]["type"].as_str(), Some("command"));
        }
        assert!(overrides.contains(&"mcp_servers.tidewatch.url=\"http://127.0.0.1:5555/mcp\""));
        assert!(
            overrides
                .contains(&"mcp_servers.tidewatch.bearer_token_env_var=\"TIDEWATCH_MCP_TOKEN\"")
        );
        assert!(
            overrides.contains(&"mcp_servers.tidewatch.default_tools_approval_mode=\"approve\"")
        );
        assert!(overrides.contains(&"mcp_servers.tidewatch.tool_timeout_sec=2100"));
        assert_eq!(env_of(&launch, "TIDEWATCH_MCP_TOKEN"), Some("tok123"));
        let instructions =
            overrides.iter().find(|o| o.starts_with("developer_instructions=")).unwrap();
        let parsed: toml::Value = toml::from_str(instructions).unwrap();
        assert_eq!(parsed["developer_instructions"].as_str(), Some("Say \"hi\"\nand \\ more"));
    }

    #[test]
    fn codex_without_hooks_or_a_readable_house_style_gets_no_overrides() {
        let ctx = LaunchCtx {
            hooks: None,
            house_style: Some("/nonexistent/style.md".into()),
            ..full_ctx()
        };
        let launch = run("codex", &profile("codex", Level::Plan), &ctx);
        assert!(!launch.args.contains(&"-c".to_owned()));
        assert!(!launch.args.contains(&"--dangerously-bypass-hook-trust".to_owned()));
        assert_eq!(env_of(&launch, "TIDEWATCH_MCP_TOKEN"), None);
    }

    #[test]
    fn never_push_adds_the_git_config_env_and_helper_path_in_worktree_tabs_only() {
        let mut guarded = profile("claude", Level::Plan);
        guarded.never_push = true;
        let launch = run("claude", &guarded, &full_ctx());
        assert_eq!(env_of(&launch, "GIT_CONFIG_COUNT"), Some("1"));
        assert_eq!(
            env_of(&launch, "GIT_CONFIG_KEY_0"),
            Some("url.tidewatch-no-push://blocked/.pushInsteadOf")
        );
        assert_eq!(env_of(&launch, "GIT_CONFIG_VALUE_0"), Some(""));
        assert_eq!(env_of(&launch, "PATH"), Some("/state/guard-bin:/usr/bin:/bin"));

        let main_tab = LaunchCtx { worktree_tab: false, ..full_ctx() };
        assert_eq!(env_of(&run("claude", &guarded, &main_tab), "GIT_CONFIG_COUNT"), None);
        let mut open = guarded.clone();
        open.never_push = false;
        assert_eq!(env_of(&run("claude", &open, &full_ctx()), "GIT_CONFIG_COUNT"), None);
    }

    fn guard_ctx() -> GuardCtx {
        GuardCtx {
            bin: "/app/tidewatch".into(),
            config: "/cfg/settings.toml".into(),
            repo: "/work/web".into(),
            cache_dir: "/state/guard-cache".into(),
            tab_dir: "/state/web/brave-otter".into(),
        }
    }

    #[test]
    fn the_outbound_guards_name_the_settings_and_put_the_shims_first_in_worktree_tabs_only() {
        let open = profile("claude", Level::Plan);
        let ctx = LaunchCtx { guards: Some(guard_ctx()), ..full_ctx() };
        let launch = run("claude", &open, &ctx);
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_CONFIG"), Some("/cfg/settings.toml"));
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_BIN"), Some("/app/tidewatch"));
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_REPO"), Some("/work/web"));
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_DIR"), Some("/state/guard-bin"));
        assert_eq!(env_of(&launch, "PATH"), Some("/state/guard-bin:/usr/bin:/bin"));
        assert_eq!(env_of(&launch, "GIT_CONFIG_COUNT"), None);

        let main_tab = LaunchCtx { worktree_tab: false, ..ctx.clone() };
        assert_eq!(env_of(&run("claude", &open, &main_tab), "TIDEWATCH_GUARD_CONFIG"), None);
        let no_dir = LaunchCtx { guard_bin_dir: None, ..ctx.clone() };
        let launch = run("claude", &open, &no_dir);
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_CONFIG"), None);
        assert_eq!(env_of(&launch, "PATH"), None);
        assert_eq!(env_of(&run("claude", &open, &full_ctx()), "TIDEWATCH_GUARD_CONFIG"), None);
    }

    #[test]
    fn the_comment_guard_adds_command_hooks_beside_the_status_hooks_in_worktree_tabs_only() {
        let ctx = LaunchCtx { guards: Some(guard_ctx()), ..full_ctx() };
        let args = run("claude", &profile("claude", Level::Plan), &ctx).args;
        let at = args.iter().position(|a| a == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
        let post = &settings["hooks"]["PostToolUse"];
        assert_eq!(post.as_array().unwrap().len(), 1);
        assert_eq!(post[0]["matcher"], "Write|Edit|MultiEdit");
        assert_eq!(post[0]["hooks"][0]["type"], "command");
        assert_eq!(post[0]["hooks"][0]["command"], "/app/tidewatch --guard comments");
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["type"], "http");
        assert_eq!(stop[1]["hooks"][0]["command"], "/app/tidewatch --guard comments --stop");
        assert!(settings["hooks"]["SessionStart"][0]["hooks"][0]["url"].is_string());

        let main_tab = LaunchCtx { worktree_tab: false, ..ctx.clone() };
        let args = run("claude", &profile("claude", Level::Plan), &main_tab).args;
        let at = args.iter().position(|a| a == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert!(settings["hooks"]["PostToolUse"].is_null());
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_comment_guard_alone_still_gets_a_settings_file_and_a_path_with_spaces_is_quoted() {
        let guards =
            GuardCtx { bin: "/Applications/Tide watch.app/tidewatch".into(), ..guard_ctx() };
        let settings = claude_settings(None, Some(&guards.bin)).unwrap();
        let parsed: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/Applications/Tide watch.app/tidewatch' --guard comments --stop"
        );
        assert_eq!(parsed["hooks"].as_object().unwrap().len(), 2);
        assert_eq!(claude_settings(None, None), None);
        let ctx = LaunchCtx { hooks: None, guards: Some(guard_ctx()), ..full_ctx() };
        assert!(
            run("claude", &profile("claude", Level::Plan), &ctx)
                .args
                .contains(&"--settings".to_owned())
        );
    }

    #[test]
    fn the_push_guard_and_the_outbound_guards_share_one_path_entry() {
        let mut both = profile("claude", Level::Plan);
        both.never_push = true;
        let ctx = LaunchCtx { guards: Some(guard_ctx()), ..full_ctx() };
        let launch = run("claude", &both, &ctx);
        let paths: Vec<&str> =
            launch.env.iter().filter(|(k, _)| k == "PATH").map(|(_, v)| v.as_str()).collect();
        assert_eq!(paths, ["/state/guard-bin:/usr/bin:/bin"]);
        assert_eq!(env_of(&launch, "GIT_CONFIG_COUNT"), Some("1"));
        assert_eq!(env_of(&launch, "TIDEWATCH_GUARD_CONFIG"), Some("/cfg/settings.toml"));
    }

    #[test]
    fn guard_env_appends_after_existing_git_config_entries() {
        let base = vec![
            ("GIT_CONFIG_COUNT".to_owned(), "2".to_owned()),
            ("GIT_CONFIG_KEY_0".to_owned(), "user.name".to_owned()),
            ("GIT_CONFIG_VALUE_0".to_owned(), "a".to_owned()),
            ("GIT_CONFIG_KEY_1".to_owned(), "user.email".to_owned()),
            ("GIT_CONFIG_VALUE_1".to_owned(), "b".to_owned()),
        ];
        let env = guard_env(&base, None, Some("agent/brave-otter"));
        assert_eq!(
            env,
            [
                ("GIT_CONFIG_COUNT".to_owned(), "3".to_owned()),
                ("GIT_CONFIG_KEY_2".to_owned(), NO_PUSH_URL_PREFIX.to_owned()),
                ("GIT_CONFIG_VALUE_2".to_owned(), String::new()),
                (PUSH_BRANCH_ENV.to_owned(), "agent/brave-otter".to_owned()),
            ]
        );
    }

    #[test]
    fn guard_env_treats_a_missing_or_garbage_count_as_zero() {
        for base in [vec![], vec![("GIT_CONFIG_COUNT".to_owned(), "lots".to_owned())]] {
            let env = guard_env(&base, None, None);
            assert_eq!(env[0], ("GIT_CONFIG_COUNT".to_owned(), "1".to_owned()));
            assert_eq!(env[1].0, "GIT_CONFIG_KEY_0");
        }
    }

    #[test]
    fn guard_env_puts_the_helper_dir_first_even_without_a_base_path() {
        let env = guard_env(&[], Some(Path::new("/g")), None);
        assert_eq!(env.last().unwrap(), &("PATH".to_owned(), "/g".to_owned()));
    }

    #[test]
    fn helper_script_is_a_remote_helper_that_names_the_guard_key_and_the_message() {
        let script = no_push_helper_script();
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(NO_PUSH_MESSAGE));
        assert!(script.contains(NO_PUSH_URL_PREFIX));
        assert!(script.contains("printf 'option\\npush\\n\\n'"));
        assert!(!script.contains("@MESSAGE@"));
        assert!(!script.contains("@GUARD_KEY@"));
    }

    #[test]
    fn helper_is_written_executable_and_repaired_when_changed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = ensure_no_push_helper(dir.path()).unwrap();
        assert_eq!(file.file_name().unwrap(), NO_PUSH_HELPER_NAME);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), no_push_helper_script());
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o755);
        std::fs::write(&file, "tampered").unwrap();
        ensure_no_push_helper(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), no_push_helper_script());
    }

    #[test]
    fn mcp_config_files_per_cli() {
        let claude: Value =
            serde_json::from_str(&mcp_config_json(Wiring::Claude, &listener()).unwrap()).unwrap();
        assert_eq!(claude["mcpServers"]["tidewatch"]["type"], "http");
        assert_eq!(claude["mcpServers"]["tidewatch"]["url"], "http://127.0.0.1:5555/mcp");
        assert_eq!(claude["mcpServers"]["tidewatch"]["headers"]["Authorization"], "Bearer tok123");
        assert!(claude["mcpServers"]["tidewatch"].get("tools").is_none());
        let copilot: Value =
            serde_json::from_str(&mcp_config_json(Wiring::Copilot, &listener()).unwrap()).unwrap();
        assert_eq!(copilot["mcpServers"]["tidewatch"]["tools"], json!(["*"]));
        for wiring in [Wiring::Gemini, Wiring::Codex, Wiring::Custom] {
            assert_eq!(mcp_config_json(wiring, &listener()), None);
        }
    }

    #[test]
    fn shell_line_quotes_what_a_shell_would_mangle() {
        let launch = Launch {
            program: "claude".into(),
            args: [
                "--allowedTools=mcp__tidewatch__*",
                "--settings",
                "{\"a\":\"it's\"}",
                "--plain=/p/q",
            ]
            .map(String::from)
            .to_vec(),
            env: vec![],
            experimental: false,
        };
        assert_eq!(
            launch.shell_line(),
            "claude '--allowedTools=mcp__tidewatch__*' --settings '{\"a\":\"it'\\''s\"}' --plain=/p/q"
        );
    }
}
