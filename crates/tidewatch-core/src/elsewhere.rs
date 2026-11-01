//! Agent sessions running outside Tidewatch, the pure half: the `ps` table, the hosting app and
//! Claude Code's registry, `~/.claude/sessions/<pid>.json` (2.1.301; `procStart` is UTC; a pid
//! alive with another start time is a reused pid). Only `<digits>.json` is read: the `.key` files
//! beside them are secrets.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::agents::AgentTable;
use crate::civil_date::days_from_civil;
use crate::elsewhere_flags::{Flags, basename, parse_command_line};
use crate::text::squash;
use crate::transcript::Source;

const ANCESTOR_LIMIT: usize = 64;
const NAME_MAX_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cli {
    Claude,
    Codex,
    Gemini,
    Copilot,
}

impl Cli {
    pub const ALL: [Cli; 4] = [Cli::Claude, Cli::Codex, Cli::Gemini, Cli::Copilot];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Cli::Claude => "claude",
            Cli::Codex => "codex",
            Cli::Gemini => "gemini",
            Cli::Copilot => "copilot",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cli| cli.id() == id)
    }

    #[must_use]
    pub fn source(self) -> Source {
        match self {
            Cli::Claude => Source::Claude,
            Cli::Codex => Source::Codex,
            Cli::Gemini => Source::Gemini,
            Cli::Copilot => Source::Copilot,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRow {
    pub pid: u32,
    pub ppid: u32,
    pub started: String,
    pub argv: Vec<String>,
}

impl ProcessRow {
    #[must_use]
    pub fn command(&self) -> String {
        self.argv.join(" ")
    }
}

/// `Thu Oct  8 21:57:15 2026` and `Thu Oct 8 21:57:15 2026` are the same time.
#[must_use]
pub fn normalize_start(text: &str) -> String {
    squash(text)
}

#[must_use]
pub fn parse_ps(output: &str) -> Vec<ProcessRow> {
    output.lines().filter_map(parse_ps_line).collect()
}

fn parse_ps_line(line: &str) -> Option<ProcessRow> {
    let mut words = line.split_whitespace();
    let pid = words.next()?.parse().ok()?;
    let ppid = words.next()?.parse().ok()?;
    let start: Vec<&str> = words.by_ref().take(5).collect();
    if start.len() < 5 || !start[3].contains(':') {
        return None;
    }
    Some(ProcessRow {
        pid,
        ppid,
        started: start.join(" "),
        argv: words.map(str::to_owned).collect(),
    })
}

#[derive(Clone, Debug, Default)]
pub struct ProcessTable {
    rows: Vec<ProcessRow>,
    index: HashMap<u32, usize>,
}

impl ProcessTable {
    #[must_use]
    pub fn new(rows: Vec<ProcessRow>) -> Self {
        let index = rows.iter().enumerate().map(|(at, row)| (row.pid, at)).collect();
        Self { rows, index }
    }

    #[must_use]
    pub fn get(&self, pid: u32) -> Option<&ProcessRow> {
        self.index.get(&pid).map(|at| &self.rows[*at])
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProcessRow> {
        self.rows.iter()
    }

    #[must_use]
    pub fn ancestors(&self, pid: u32) -> Vec<&ProcessRow> {
        let mut chain = Vec::new();
        let mut at = self.get(pid).map(|row| row.ppid);
        while let Some(parent) = at.and_then(|ppid| self.get(ppid)) {
            let seen =
                parent.pid == pid || chain.iter().any(|seen: &&ProcessRow| seen.pid == parent.pid);
            if chain.len() >= ANCESTOR_LIMIT || seen {
                break;
            }
            chain.push(parent);
            at = Some(parent.ppid);
        }
        chain
    }

    #[must_use]
    pub fn descends_from(&self, pid: u32, ancestor: u32) -> bool {
        self.ancestors(pid).iter().any(|row| row.pid == ancestor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Stdusk,
    Terminal,
    ITerm2,
    VsCode,
    Ghostty,
    WezTerm,
    Warp,
    Tmux,
    Other,
}

impl Host {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Host::Stdusk => "stdusk",
            Host::Terminal => "Terminal",
            Host::ITerm2 => "iTerm2",
            Host::VsCode => "VS Code",
            Host::Ghostty => "Ghostty",
            Host::WezTerm => "WezTerm",
            Host::Warp => "Warp",
            Host::Tmux => "tmux",
            Host::Other => "another terminal",
        }
    }
}

fn host_of_row(row: &ProcessRow) -> Option<Host> {
    let program = basename(row.argv.first()?).trim_end_matches(':').to_lowercase();
    let all = row.command().to_lowercase();
    let has = |part: &str| all.contains(part);
    if program == "stdusk" || has("/stdusk.app/") {
        Some(Host::Stdusk)
    } else if program == "tmux" {
        Some(Host::Tmux)
    } else if has("/terminal.app/") {
        Some(Host::Terminal)
    } else if has("/iterm.app/") || program.starts_with("iterm") {
        Some(Host::ITerm2)
    } else if has("visual studio code") || has("/code helper") || has("vscode") {
        Some(Host::VsCode)
    } else if program == "ghostty" || has("/ghostty.app/") {
        Some(Host::Ghostty)
    } else if program.starts_with("wezterm") || has("/wezterm.app/") {
        Some(Host::WezTerm)
    } else if program == "warp" || program == "warp-terminal" || has("/warp.app/") {
        Some(Host::Warp)
    } else {
        None
    }
}

#[must_use]
pub fn host_of(table: &ProcessTable, pid: u32) -> Host {
    table.ancestors(pid).into_iter().find_map(host_of_row).unwrap_or(Host::Other)
}

/// A process Tidewatch started, or one that carries its wiring: the pty children of this window
/// (also after a restart handed them over, when their parent is launchd) are not "elsewhere".
#[must_use]
pub fn is_ours(table: &ProcessTable, row: &ProcessRow, own_pid: u32) -> bool {
    table.descends_from(row.pid, own_pid)
        || row
            .argv
            .iter()
            .any(|word| word.contains("/.tidewatch/") || word.contains("mcp__tidewatch__"))
}

/// Which processes a scan looks at: the ones outside Tidewatch, or the ones inside given shells
/// (a shell, a process it started at any depth, or the shell's own process after an `exec`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope<'a> {
    Elsewhere { own_pid: u32 },
    Inside(&'a [u32]),
}

impl Scope<'_> {
    #[must_use]
    pub fn admits(self, table: &ProcessTable, row: &ProcessRow) -> bool {
        match self {
            Scope::Elsewhere { own_pid } => !is_ours(table, row, own_pid),
            Scope::Inside(shells) => {
                shells.iter().any(|shell| row.pid == *shell || table.descends_from(row.pid, *shell))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Working,
    Quiet,
}

impl Activity {
    #[must_use]
    pub fn of_registry_status(status: Option<&str>) -> Self {
        match status {
            Some("idle") => Self::Idle,
            Some("busy" | "shell") => Self::Working,
            _ => Self::Quiet,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryEntry {
    pub pid: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub started_at_ms: Option<i64>,
    pub proc_start: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
}

/// The pid a registry file name stands for: `<digits>.json` and nothing else, so the `.key` files
/// beside them never match.
#[must_use]
pub fn registry_pid(file_name: &str) -> Option<u32> {
    let stem = file_name.strip_suffix(".json")?;
    (!stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())).then_some(stem)?.parse().ok()
}

/// A session id is typed into a shell line: only the characters of a UUID or a short id.
#[must_use]
pub fn is_session_id(text: &str) -> bool {
    (8..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[must_use]
pub fn parse_registry_entry(text: &str) -> Option<RegistryEntry> {
    let value: Value = serde_json::from_str(text).ok()?;
    let text_of = |key: &str| value[key].as_str().map(str::to_owned);
    let session_id = text_of("sessionId").filter(|id| is_session_id(id))?;
    let cwd = PathBuf::from(text_of("cwd")?);
    let proc_start = normalize_start(&text_of("procStart")?);
    if !cwd.is_absolute() || proc_start.is_empty() {
        return None;
    }
    let name = text_of("name")
        .map(|name| {
            name.chars().filter(|c| !c.is_control()).take(NAME_MAX_CHARS).collect::<String>()
        })
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Some(RegistryEntry {
        pid: u32::try_from(value["pid"].as_u64()?).ok()?,
        session_id,
        cwd,
        started_at_ms: value["startedAt"].as_i64(),
        proc_start,
        kind: text_of("kind"),
        name,
        status: text_of("status"),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    /// The process start as `ps` prints it in UTC, checked again right before a signal.
    pub proc_start: String,
    pub host: Host,
    pub activity: Activity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    Running(Running),
    NotRunning,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Process { pid: u32, proc_start: String },
    Stored { cli: Cli, id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub cli: Cli,
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    pub name: Option<String>,
    pub flags: Flags,
    pub presence: Presence,
    /// Unix ms: when it started (running) or was last written (not running).
    pub since_ms: Option<i64>,
}

impl Session {
    #[must_use]
    pub fn running(&self) -> Option<&Running> {
        match &self.presence {
            Presence::Running(running) => Some(running),
            Presence::NotRunning => None,
        }
    }

    #[must_use]
    pub fn key(&self) -> Option<Key> {
        match (&self.presence, &self.session_id) {
            (Presence::Running(r), _) => {
                Some(Key::Process { pid: r.pid, proc_start: r.proc_start.clone() })
            }
            (Presence::NotRunning, Some(id)) => Some(Key::Stored { cli: self.cli, id: id.clone() }),
            (Presence::NotRunning, None) => None,
        }
    }

    #[must_use]
    pub fn display_name(&self, now_ms: i64) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let cli = self.cli.id();
        let age = self.since_ms.map(|at| age_text(now_ms - at));
        match (&self.presence, age) {
            (Presence::Running(_), Some(age)) => format!("{cli} · {age}"),
            (Presence::NotRunning, Some(age)) => format!("{cli} · ended {age}"),
            (_, None) => cli.to_owned(),
        }
    }
}

#[must_use]
pub fn age_text(elapsed_ms: i64) -> String {
    let minutes = elapsed_ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=2879 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

#[must_use]
pub fn parse_lstart_ms(text: &str) -> Option<i64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let [_, month, day, clock, year] = words[..] else { return None };
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|name| *name == month)? as i64
        + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let mut parts = clock.split(':').map(str::parse::<i64>);
    let (h, m, s) = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?);
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + h) * 60 + m) * 60_000 + s * 1000)
}

/// The Claude Code sessions the registry and the process table agree on: a live process, the start
/// time the file recorded, an interactive session, not one of Tidewatch's own tabs. One session per
/// pid, in pid order.
#[must_use]
pub fn claude_sessions(
    entries: &[RegistryEntry],
    table: &ProcessTable,
    own_pid: u32,
    agents: &AgentTable,
) -> Vec<Session> {
    claude_sessions_in(entries, table, Scope::Elsewhere { own_pid }, agents)
}

/// [`claude_sessions`] for any [`Scope`]. A `claude -p` run is never a session to take.
#[must_use]
pub fn claude_sessions_in(
    entries: &[RegistryEntry],
    table: &ProcessTable,
    scope: Scope,
    agents: &AgentTable,
) -> Vec<Session> {
    let Some(agent) = agents.get(Cli::Claude.id()) else { return Vec::new() };
    let mut sessions: Vec<Session> = entries
        .iter()
        .filter(|entry| entry.kind.as_deref().is_none_or(|kind| kind == "interactive"))
        .filter_map(|entry| {
            let row = table.get(entry.pid).filter(|row| row.started == entry.proc_start)?;
            if !scope.admits(table, row) {
                return None;
            }
            let parsed = parse_command_line(Cli::Claude, agent, &row.argv);
            if parsed.non_interactive {
                return None;
            }
            Some(Session {
                cli: Cli::Claude,
                session_id: Some(entry.session_id.clone()),
                cwd: entry.cwd.clone(),
                name: entry.name.clone(),
                flags: parsed.flags,
                presence: Presence::Running(Running {
                    pid: entry.pid,
                    proc_start: row.started.clone(),
                    host: host_of(table, entry.pid),
                    activity: Activity::of_registry_status(entry.status.as_deref()),
                }),
                since_ms: entry.started_at_ms.or_else(|| parse_lstart_ms(&row.started)),
            })
        })
        .collect();
    sessions.sort_by_key(|session| session.running().map(|r| r.pid));
    sessions.dedup_by_key(|session| session.running().map(|r| r.pid));
    sessions
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub cli: Cli,
    pub pid: u32,
    pub proc_start: String,
    pub host: Host,
    pub flags: Flags,
    pub resume_id: Option<String>,
    pub started_ms: Option<i64>,
}

const RUNTIMES: [&str; 4] = ["node", "nodejs", "bun", "deno"];

#[must_use]
pub fn program_cli(argv: &[String]) -> Option<Cli> {
    let first = basename(argv.first()?);
    let program = if RUNTIMES.contains(&first) { basename(argv.get(1)?) } else { first };
    [Cli::Codex, Cli::Gemini, Cli::Copilot].into_iter().find(|cli| cli.id() == program)
}

/// The processes of the table that run one of those CLIs interactively and are not Tidewatch's. A
/// wrapper and the binary it starts count once, the outer one.
#[must_use]
pub fn other_candidates(table: &ProcessTable, own_pid: u32, agents: &AgentTable) -> Vec<Candidate> {
    other_candidates_in(table, Scope::Elsewhere { own_pid }, agents)
}

/// [`other_candidates`] for any [`Scope`].
#[must_use]
pub fn other_candidates_in(
    table: &ProcessTable,
    scope: Scope,
    agents: &AgentTable,
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = table
        .iter()
        .filter_map(|row| {
            let cli = program_cli(&row.argv)?;
            let agent = agents.get(cli.id())?;
            if !scope.admits(table, row) {
                return None;
            }
            let parsed = parse_command_line(cli, agent, &row.argv);
            (!parsed.non_interactive).then(|| Candidate {
                cli,
                pid: row.pid,
                proc_start: row.started.clone(),
                host: host_of(table, row.pid),
                resume_id: parsed.resume_id.filter(|id| is_session_id(id)),
                flags: parsed.flags,
                started_ms: parse_lstart_ms(&row.started),
            })
        })
        .collect();
    let pids: Vec<(u32, Cli)> = found.iter().map(|c| (c.pid, c.cli)).collect();
    found.retain(|c| table.get(c.pid).is_none_or(|row| !pids.contains(&(row.ppid, c.cli))));
    found.sort_by_key(|c| c.pid);
    found
}

#[cfg(test)]
mod tests;
