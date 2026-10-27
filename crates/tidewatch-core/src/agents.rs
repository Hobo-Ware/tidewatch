//! The agent table: which CLIs Tidewatch can start, the flags behind each permission level, and the
//! user overrides from `profiles.toml`. Pure data plus a version probe; the UI calls the probe off-thread.
//!
//! Flags were checked against the installed `--help` of claude 2.1.296, gemini 0.46.0, copilot 1.0.91 and
//! codex 0.162.1, and Claude's and Codex's also by running them (see `docs/research/phase-0-spikes.md`
//! and the PLAN.md Phase 0 findings). `--help` does not catch flags that conflict; `codex <flags> completion bash`
//! does, without a model call. Repeatable flags
//! that take a list (`--allow-tool`, `--mcp-config`, `--include-directories`) are written in the `--flag=value`
//! form and a trailing task prompt follows a literal `--`, otherwise the list swallows it (checked with claude).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How much freedom an agent gets. TOML names are the lowercase variant names (`plan`, `ask`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Plan,
    Ask,
    Edits,
    Auto,
    Yolo,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Plan, Level::Ask, Level::Edits, Level::Auto, Level::Yolo];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Level::Plan => "Read only",
            Level::Ask => "Ask",
            Level::Edits => "Auto edit",
            Level::Auto => "Auto",
            Level::Yolo => "YOLO",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Which per-CLI wiring (hooks, MCP, house style) `launch` applies. `Custom` gets none of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wiring {
    Claude,
    Codex,
    Gemini,
    Copilot,
    Custom,
}

/// The way a CLI reopens a session, so a restart can continue it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeForm {
    /// `<flag> <id>`
    Flag(&'static str),
    /// `<flag>=<id>`
    FlagEquals(&'static str),
    /// `<word> <id>` as the first argument
    Subcommand(&'static str),
    /// `<flag> latest`: the CLI cannot resume by id, so the newest session of the working directory is used
    FlagLatest(&'static str),
}

/// Whether the CLI takes a name for the remote session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteName {
    Takes,
    Ignores,
}

/// How a CLI is driven from a phone or a browser, checked with `--help` of claude 2.1.302 (`--remote-control
/// [name]`), copilot 1.0.96 (`--remote`), codex 0.162.1 (a daemon, `codex remote-control start`, not a
/// per-session flag) and gemini 0.46.0 (none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    /// A flag that turns it on for this session. `app` is where the session is driven from.
    Flag {
        flag: &'static str,
        name: RemoteName,
        app: &'static str,
    },
    /// The CLI has it, but not as a flag Tidewatch can pass yet.
    Later,
    None,
}

impl Remote {
    #[must_use]
    pub fn app(self) -> Option<&'static str> {
        match self {
            Self::Flag { app, .. } => Some(app),
            Self::Later | Self::None => None,
        }
    }
}

/// How a profile's effort reaches the CLI: `<flag> <value_prefix><effort>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffortFlag {
    pub flag: String,
    pub value_prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub bin: String,
    pub wiring: Wiring,
    levels: [Option<Vec<String>>; 5],
    pub model_flag: Option<String>,
    pub effort: Option<EffortFlag>,
    pub session_id_flag: Option<String>,
    pub resume: Option<ResumeForm>,
    pub remote: Remote,
    /// False for custom agents: only the built-in CLIs were run live.
    pub verified: bool,
    /// The CLI version the built-in flags were checked against.
    pub checked_version: Option<&'static str>,
}

impl Agent {
    /// `None` when the agent has no flags for this level (only a custom agent can lack one).
    #[must_use]
    pub fn level_args(&self, level: Level) -> Option<&[String]> {
        self.levels[level.index()].as_deref()
    }

    /// The most permissive level this agent has flags for, short of YOLO.
    #[must_use]
    pub fn highest_level_below_yolo(&self) -> Option<Level> {
        Level::ALL
            .into_iter()
            .rev()
            .filter(|level| *level != Level::Yolo)
            .find(|level| self.level_args(*level).is_some())
    }

    #[must_use]
    pub fn remote_args(&self, name: &str) -> Vec<String> {
        match self.remote {
            Remote::Flag { flag, name: RemoteName::Takes, .. } => {
                vec![flag.to_owned(), name.to_owned()]
            }
            Remote::Flag { flag, name: RemoteName::Ignores, .. } => vec![flag.to_owned()],
            Remote::Later | Remote::None => Vec::new(),
        }
    }

    /// Arguments that reopen `session_id`, for the CLIs that can.
    #[must_use]
    pub fn resume_args(&self, session_id: &str) -> Option<Vec<String>> {
        let args = match self.resume? {
            ResumeForm::Flag(flag) => vec![flag.to_owned(), session_id.to_owned()],
            ResumeForm::FlagEquals(flag) => vec![format!("{flag}={session_id}")],
            ResumeForm::Subcommand(word) => vec![word.to_owned(), session_id.to_owned()],
            ResumeForm::FlagLatest(flag) => vec![flag.to_owned(), "latest".to_owned()],
        };
        Some(args)
    }
}

fn split_line(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn levels_from(lines: [&str; 5]) -> [Option<Vec<String>>; 5] {
    lines.map(|line| Some(split_line(line)))
}

fn builtin_agents() -> Vec<Agent> {
    vec![
        Agent {
            id: "claude".into(),
            name: "Claude Code".into(),
            bin: "claude".into(),
            wiring: Wiring::Claude,
            levels: levels_from([
                "--permission-mode dontAsk --allowedTools=Read,Grep,Glob",
                "--permission-mode manual",
                "--permission-mode acceptEdits",
                "--permission-mode auto",
                "--dangerously-skip-permissions",
            ]),
            model_flag: Some("--model".into()),
            effort: Some(EffortFlag { flag: "--effort".into(), value_prefix: String::new() }),
            session_id_flag: Some("--session-id".into()),
            resume: Some(ResumeForm::Flag("--resume")),
            remote: Remote::Flag {
                flag: "--remote-control",
                name: RemoteName::Takes,
                app: "the Claude app",
            },
            verified: true,
            checked_version: Some("2.1.296"),
        },
        Agent {
            id: "codex".into(),
            name: "Codex".into(),
            bin: "codex".into(),
            wiring: Wiring::Codex,
            levels: levels_from([
                "--sandbox read-only --ask-for-approval never",
                "--sandbox read-only --ask-for-approval on-request",
                "--sandbox workspace-write --ask-for-approval on-request",
                "--approve-for-me",
                "--dangerously-bypass-approvals-and-sandbox",
            ]),
            model_flag: Some("--model".into()),
            effort: Some(EffortFlag {
                flag: "-c".into(),
                value_prefix: "model_reasoning_effort=".into(),
            }),
            session_id_flag: None,
            resume: Some(ResumeForm::Subcommand("resume")),
            remote: Remote::Later,
            verified: true,
            checked_version: Some("0.162.1"),
        },
        Agent {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            bin: "gemini".into(),
            wiring: Wiring::Gemini,
            levels: levels_from([
                "--approval-mode plan",
                "--approval-mode default",
                "--approval-mode auto_edit",
                "--approval-mode yolo --sandbox",
                "--yolo",
            ]),
            model_flag: Some("--model".into()),
            effort: None,
            session_id_flag: Some("--session-id".into()),
            resume: Some(ResumeForm::FlagLatest("--resume")),
            remote: Remote::None,
            verified: true,
            checked_version: Some("0.46.0"),
        },
        Agent {
            id: "copilot".into(),
            name: "Copilot CLI".into(),
            bin: "copilot".into(),
            wiring: Wiring::Copilot,
            levels: levels_from([
                "--mode plan",
                "",
                "--allow-tool=write",
                "--allow-all-tools",
                "--yolo",
            ]),
            model_flag: Some("--model".into()),
            effort: Some(EffortFlag {
                flag: "--reasoning-effort".into(),
                value_prefix: String::new(),
            }),
            session_id_flag: Some("--session-id".into()),
            resume: Some(ResumeForm::FlagEquals("--resume")),
            remote: Remote::Flag {
                flag: "--remote",
                name: RemoteName::Ignores,
                app: "GitHub mobile",
            },
            verified: true,
            checked_version: Some("1.0.91"),
        },
    ]
}

/// A list of arguments in TOML: either one string (split on whitespace) or an array of strings.
/// Use the array form for an argument that contains a space.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArgList(pub Vec<String>);

impl std::ops::Deref for ArgList {
    type Target = [String];

    fn deref(&self) -> &[String] {
        &self.0
    }
}

impl From<Vec<String>> for ArgList {
    fn from(args: Vec<String>) -> Self {
        Self(args)
    }
}

impl Serialize for ArgList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ArgList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Line(String),
            List(Vec<String>),
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::Line(line) => Self(split_line(&line)),
            Shape::List(list) => Self(list),
        })
    }
}

/// `[agent.<id>.permissions]`: the flags for each level. A missing level keeps the built-in flags, or,
/// for a custom agent, leaves that level unavailable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ArgList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<ArgList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<ArgList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<ArgList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yolo: Option<ArgList>,
}

impl LevelOverrides {
    fn get(&self, level: Level) -> Option<&ArgList> {
        match level {
            Level::Plan => self.plan.as_ref(),
            Level::Ask => self.ask.as_ref(),
            Level::Edits => self.edits.as_ref(),
            Level::Auto => self.auto.as_ref(),
            Level::Yolo => self.yolo.as_ref(),
        }
    }
}

/// `[agent.<id>]` in `profiles.toml`: changes a built-in agent, or defines a custom one (needs `bin`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id_flag: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub permissions: LevelOverrides,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTable {
    agents: Vec<Agent>,
}

impl AgentTable {
    #[must_use]
    pub fn builtin() -> Self {
        Self { agents: builtin_agents() }
    }

    /// Built-ins first, with the user's changes applied, then the custom agents in id order. A custom
    /// entry without `bin` is skipped with a stderr note.
    #[must_use]
    pub fn with_overrides(overrides: &BTreeMap<String, AgentOverride>) -> Self {
        let mut agents = builtin_agents();
        for agent in &mut agents {
            if let Some(over) = overrides.get(&agent.id) {
                apply_override(agent, over);
            }
        }
        for (id, over) in overrides {
            if agents.iter().any(|agent| &agent.id == id) {
                continue;
            }
            match custom_agent(id, over) {
                Some(agent) => agents.push(agent),
                None => eprintln!("tidewatch: custom agent `{id}` has no `bin`; ignoring it"),
            }
        }
        Self { agents }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Agent> {
        self.agents.iter()
    }

    /// Ids of the agents whose binary is on `path_var`, in table order. Looks at the disk, runs nothing.
    #[must_use]
    pub fn found_on_path(&self, path_var: Option<&OsStr>) -> Vec<String> {
        self.agents
            .iter()
            .filter(|agent| find_on_path(&agent.bin, path_var).is_some())
            .map(|agent| agent.id.clone())
            .collect()
    }
}

fn apply_override(agent: &mut Agent, over: &AgentOverride) {
    if let Some(name) = &over.name {
        agent.name.clone_from(name);
    }
    if let Some(bin) = &over.bin {
        agent.bin.clone_from(bin);
    }
    if let Some(flag) = &over.model_flag {
        agent.model_flag = Some(flag.clone());
    }
    if let Some(flag) = &over.session_id_flag {
        agent.session_id_flag = Some(flag.clone());
    }
    for level in Level::ALL {
        if let Some(args) = over.permissions.get(level) {
            agent.levels[level.index()] = Some(args.0.clone());
        }
    }
}

fn custom_agent(id: &str, over: &AgentOverride) -> Option<Agent> {
    let bin = over.bin.clone()?;
    Some(Agent {
        id: id.to_owned(),
        name: over.name.clone().unwrap_or_else(|| id.to_owned()),
        bin,
        wiring: Wiring::Custom,
        levels: Level::ALL.map(|level| over.permissions.get(level).map(|args| args.0.clone())),
        model_flag: over.model_flag.clone(),
        effort: None,
        session_id_flag: over.session_id_flag.clone(),
        resume: None,
        remote: Remote::None,
        verified: false,
        checked_version: None,
    })
}

/// First version-looking token of `--version` output: digits and dots, optional `v` prefix and
/// `-pre`/`+build` suffix. `GitHub Copilot CLI 1.0.91.` gives `1.0.91`.
#[must_use]
pub fn parse_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';'));
        let token = token.strip_prefix('v').unwrap_or(token);
        let core_end = token.find(['-', '+']).unwrap_or(token.len());
        let (core, suffix) = token.split_at(core_end);
        let core = if suffix.is_empty() { core.trim_end_matches('.') } else { core };
        let mut parts = core.split('.');
        let looks_like_version = core.contains('.')
            && parts.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
        looks_like_version.then(|| format!("{core}{suffix}"))
    })
}

/// Runs `<bin> --version` and parses it. Blocks for up to five seconds: call it off the UI thread.
#[must_use]
pub fn probe_version(bin: &str) -> Option<String> {
    probe_version_in(bin, None)
}

/// `probe_version` with `path_var` as the child's `PATH`, so a CLI that starts through `env node`
/// finds its interpreter even when the app itself was started with a bare `PATH`. The name is looked
/// up in `path_var` too.
#[must_use]
pub fn probe_version_in(bin: &str, path_var: Option<&OsStr>) -> Option<String> {
    let program = path_var.map_or_else(
        || PathBuf::from(bin),
        |path| find_on_path(bin, Some(path)).unwrap_or_else(|| PathBuf::from(bin)),
    );
    let mut command = Command::new(program);
    if let Some(path) = path_var {
        command.env("PATH", path);
    }
    let mut child = command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < VERSION_PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    parse_version(&String::from_utf8_lossy(&output.stdout))
        .or_else(|| parse_version(&String::from_utf8_lossy(&output.stderr)))
}

/// Looks `bin` up in `path_var` like a shell would. A name with a slash is checked as given.
#[must_use]
pub fn find_on_path(bin: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    if bin.contains('/') {
        return is_executable(Path::new(bin)).then(|| PathBuf::from(bin));
    }
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(bin))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(agent: &Agent, level: Level) -> String {
        agent.level_args(level).map_or_else(|| "<none>".into(), |a| a.join(" "))
    }

    #[test]
    fn builtin_ladder_matches_the_installed_clis_claude_2_1_296_codex_0_162_1_gemini_0_46_0_copilot_1_0_91()
     {
        let table = AgentTable::builtin();
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
        for (id, want) in cases {
            let agent = table.get(id).unwrap();
            for (level, want) in Level::ALL.into_iter().zip(want) {
                assert_eq!(args(agent, level), want, "{id} {level:?}");
            }
        }
    }

    #[test]
    fn every_built_in_agent_is_verified() {
        let table = AgentTable::builtin();
        let verified: Vec<_> = table.iter().map(|a| (a.id.as_str(), a.verified)).collect();
        assert_eq!(
            verified,
            [("claude", true), ("codex", true), ("gemini", true), ("copilot", true)]
        );
    }

    #[test]
    fn model_effort_and_session_flags_per_cli() {
        let table = AgentTable::builtin();
        let rows: Vec<_> = table
            .iter()
            .map(|a| {
                (
                    a.id.as_str(),
                    a.model_flag.as_deref(),
                    a.effort.as_ref().map(|e| format!("{} {}", e.flag, e.value_prefix)),
                    a.session_id_flag.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("claude", Some("--model"), Some("--effort ".into()), Some("--session-id")),
                ("codex", Some("--model"), Some("-c model_reasoning_effort=".into()), None),
                ("gemini", Some("--model"), None, Some("--session-id")),
                (
                    "copilot",
                    Some("--model"),
                    Some("--reasoning-effort ".into()),
                    Some("--session-id")
                ),
            ]
        );
    }

    #[test]
    fn remote_control_per_cli_claude_2_1_302_copilot_1_0_96_codex_0_162_1_gemini_0_46_0() {
        let table = AgentTable::builtin();
        let rows: Vec<_> = table.iter().map(|a| (a.id.as_str(), a.remote)).collect();
        assert_eq!(
            rows,
            [
                (
                    "claude",
                    Remote::Flag {
                        flag: "--remote-control",
                        name: RemoteName::Takes,
                        app: "the Claude app"
                    }
                ),
                ("codex", Remote::Later),
                ("gemini", Remote::None),
                (
                    "copilot",
                    Remote::Flag {
                        flag: "--remote",
                        name: RemoteName::Ignores,
                        app: "GitHub mobile"
                    }
                ),
            ]
        );
        let cases: [(&str, &[&str]); 4] = [
            ("claude", &["--remote-control", "web \u{b7} otter"]),
            ("codex", &[]),
            ("gemini", &[]),
            ("copilot", &["--remote"]),
        ];
        for (id, want) in cases {
            assert_eq!(table.get(id).unwrap().remote_args("web \u{b7} otter"), want, "{id}");
        }
    }

    #[test]
    fn a_custom_agent_has_no_remote_control() {
        let table = AgentTable::with_overrides(&overrides("[agent.aider]\nbin = \"aider\"\n"));
        let aider = table.get("aider").unwrap();
        assert_eq!(aider.remote, Remote::None);
        assert_eq!(aider.remote_args("x"), Vec::<String>::new());
    }

    #[test]
    fn resume_forms_per_cli() {
        let table = AgentTable::builtin();
        let id = "11111111-2222-3333-4444-555555555555";
        let resume_equals = format!("--resume={id}");
        let cases = [
            ("claude", vec!["--resume", id]),
            ("codex", vec!["resume", id]),
            ("gemini", vec!["--resume", "latest"]),
            ("copilot", vec![resume_equals.as_str()]),
        ];
        for (agent, want) in cases {
            assert_eq!(
                table.get(agent).unwrap().resume_args(id),
                Some(want.into_iter().map(String::from).collect()),
                "{agent}"
            );
        }
    }

    #[test]
    fn version_parsing_takes_the_first_version_looking_token() {
        let cases = [
            ("2.1.296 (Claude Code)\n", Some("2.1.296")),
            ("codex-cli 0.162.1\n", Some("0.162.1")),
            ("0.46.0\n", Some("0.46.0")),
            (
                "GitHub Copilot CLI 1.0.91.\nRun 'copilot update' to check for updates.\n",
                Some("1.0.91"),
            ),
            ("tool v3.4.5-beta.2 build 9.9.9", Some("3.4.5-beta.2")),
            ("version 12 of nothing", None),
            ("", None),
            ("error: unknown option", None),
        ];
        for (output, want) in cases {
            assert_eq!(parse_version(output).as_deref(), want, "output {output:?}");
        }
    }

    #[test]
    fn probe_runs_the_binary_with_version() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-agent");
        write_executable_from_a_child(
            &script,
            "#!/bin/sh\n[ \"$1\" = --version ] && echo 'fake 9.8.7'\n",
        );
        assert_eq!(probe_version(script.to_str().unwrap()).as_deref(), Some("9.8.7"));
        assert_eq!(probe_version("/nonexistent/tidewatch-agent"), None);
    }

    #[test]
    fn probe_in_a_given_path_finds_the_binary_and_hands_the_path_to_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-agent");
        let body = "#!/bin/sh\ncase \"$PATH\" in \"$FAKE_DIR\":*) echo \"fake 1.2.3\";; esac\n";
        write_executable_from_a_child(
            &script,
            &body.replace("$FAKE_DIR", dir.path().to_str().unwrap()),
        );
        let path =
            std::env::join_paths([dir.path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        assert_eq!(probe_version_in("fake-agent", Some(&path)).as_deref(), Some("1.2.3"));
        assert_eq!(probe_version_in("fake-agent", Some(OsStr::new("/usr/bin"))), None);
    }

    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Linux refuses to exec a file that any process holds open for writing (ETXTBSY), and a test
    /// thread that forks while this process writes the file inherits that handle, so `cp` writes it.
    fn write_executable_from_a_child(path: &Path, text: &str) {
        let source = path.with_extension("src");
        std::fs::write(&source, text).unwrap();
        let copied = Command::new("cp").arg(&source).arg(path).status().unwrap();
        assert!(copied.success(), "cp {} failed", path.display());
        make_executable(path);
    }

    #[test]
    fn path_lookup_finds_only_executables() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        make_executable(&claude);
        std::fs::write(dir.path().join("codex"), "not executable").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find_on_path("claude", Some(&path)), Some(claude));
        assert_eq!(find_on_path("codex", Some(&path)), None);
        assert_eq!(find_on_path("gemini", Some(&path)), None);
        assert_eq!(find_on_path("claude", None), None);
        assert_eq!(AgentTable::builtin().found_on_path(Some(&path)), ["claude"]);
    }

    fn overrides(toml_text: &str) -> BTreeMap<String, AgentOverride> {
        #[derive(Deserialize)]
        struct File {
            agent: BTreeMap<String, AgentOverride>,
        }
        toml::from_str::<File>(toml_text).unwrap().agent
    }

    #[test]
    fn override_replaces_one_level_and_keeps_the_rest() {
        let table = AgentTable::with_overrides(&overrides(
            "[agent.claude.permissions]\nedits = \"--permission-mode acceptEdits --verbose\"\n",
        ));
        let claude = table.get("claude").unwrap();
        assert_eq!(args(claude, Level::Edits), "--permission-mode acceptEdits --verbose");
        assert_eq!(
            args(claude, Level::Plan),
            "--permission-mode dontAsk --allowedTools=Read,Grep,Glob"
        );
    }

    #[test]
    fn override_accepts_an_array_for_args_with_spaces() {
        let table = AgentTable::with_overrides(&overrides(
            "[agent.gemini.permissions]\nplan = [\"--approval-mode\", \"plan\", \"--note\", \"two words\"]\n",
        ));
        let plan = table.get("gemini").unwrap().level_args(Level::Plan).unwrap();
        assert_eq!(plan, ["--approval-mode", "plan", "--note", "two words"]);
    }

    #[test]
    fn custom_agent_needs_bin_and_lacks_unset_levels() {
        let table = AgentTable::with_overrides(&overrides(
            "[agent.aider]\nname = \"Aider\"\nbin = \"aider\"\nmodel_flag = \"--model\"\n\
             [agent.aider.permissions]\nplan = \"--chat-mode ask\"\nask = \"\"\nyolo = \"--yes-always\"\n\
             [agent.broken.permissions]\nplan = \"--x\"\n",
        ));
        let aider = table.get("aider").unwrap();
        assert_eq!(
            (aider.name.as_str(), aider.bin.as_str(), aider.wiring),
            ("Aider", "aider", Wiring::Custom)
        );
        assert_eq!(aider.model_flag.as_deref(), Some("--model"));
        assert_eq!(aider.level_args(Level::Plan).unwrap(), ["--chat-mode", "ask"]);
        assert_eq!(aider.level_args(Level::Ask).unwrap(), Vec::<String>::new());
        assert_eq!(aider.level_args(Level::Edits), None);
        assert_eq!(aider.level_args(Level::Yolo).unwrap(), ["--yes-always"]);
        assert!(!aider.verified);
        assert!(table.get("broken").is_none());
        assert_eq!(table.iter().count(), 5);
    }

    #[test]
    fn override_can_repoint_the_binary_of_a_builtin() {
        let table = AgentTable::with_overrides(&overrides(
            "[agent.claude]\nbin = \"/opt/claude/bin/claude\"\n",
        ));
        assert_eq!(table.get("claude").unwrap().bin, "/opt/claude/bin/claude");
        assert!(table.get("claude").unwrap().verified);
    }
}
