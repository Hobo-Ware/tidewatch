//! Take-over, the half that touches the machine: `ps` with `TZ=UTC` (Claude writes `procStart` in
//! UTC), `lsof` or `/proc` for a folder, session files, and ending a session with SIGTERM to one
//! re-checked pid, never SIGKILL (claude 2.1.301 exits in under a second). Call it off the UI thread.

use std::collections::HashSet;
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use crate::agents::AgentTable;
use crate::elsewhere::{
    Activity, Candidate, Cli, Presence, ProcessTable, RegistryEntry, Running, Session,
    claude_sessions, is_session_id, other_candidates, parse_ps, parse_registry_entry, registry_pid,
};
use crate::restore::codex_session_id;
use crate::tools::Tools;
use crate::transcript::{Role, parse_all};
use crate::transcript_locate::{Locator, claude_project_dir_name};

const TAIL_BYTES: u64 = 128 * 1024;
const REGISTRY_FILE_MAX_BYTES: u64 = 64 * 1024;
const FRESH_WRITE: Duration = Duration::from_secs(3);
const START_SLACK_MS: i64 = 5000;
const RECENT_DAYS: i64 = 30;
const CODEX_RECENT_FILES: usize = 30;
const COPILOT_RECENT_DIRS: usize = 40;
const POLL: Duration = Duration::from_millis(100);

pub trait System {
    fn ps(&self) -> Option<String>;
    fn cwd_of(&self, pid: u32) -> Option<PathBuf>;
    fn git_root(&self, dir: &Path) -> Option<PathBuf>;
    fn terminate(&self, pid: u32) -> bool;
    fn alive(&self, pid: u32) -> Option<String>;
}

pub struct RealSystem {
    tools: Tools,
}

impl Default for RealSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl RealSystem {
    #[must_use]
    pub fn new() -> Self {
        Self { tools: Tools::system() }
    }

    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let output = self
            .tools
            .command(program, Path::new("/"))
            .env("TZ", "UTC")
            .env("LC_ALL", "C")
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl System for RealSystem {
    fn ps(&self) -> Option<String> {
        self.run("ps", &["-axo", "pid,ppid,lstart,args"])
    }

    #[cfg(target_os = "macos")]
    fn cwd_of(&self, pid: u32) -> Option<PathBuf> {
        let out = self.run("lsof", &["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])?;
        parse_lsof_cwd(&out)
    }

    #[cfg(not(target_os = "macos"))]
    fn cwd_of(&self, pid: u32) -> Option<PathBuf> {
        fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    fn git_root(&self, dir: &Path) -> Option<PathBuf> {
        let out = self.tools.git(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let root = text.trim();
        (out.status.success() && !root.is_empty()).then(|| PathBuf::from(root))
    }

    fn terminate(&self, pid: u32) -> bool {
        self.run("kill", &["-TERM", &pid.to_string()]).is_some()
    }

    fn alive(&self, pid: u32) -> Option<String> {
        let out = self.run("ps", &["-o", "stat=,lstart=", "-p", &pid.to_string()])?;
        let (stat, start) = out.trim().split_once(char::is_whitespace)?;
        (!stat.starts_with('Z')).then(|| crate::elsewhere::normalize_start(start))
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_lsof_cwd(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| line.strip_prefix('n')).map(PathBuf::from)
}

#[derive(Clone, Debug)]
pub struct ScanEnv {
    pub home: PathBuf,
    pub claude_dir: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub copilot_home: Option<PathBuf>,
    pub own_pid: u32,
}

impl ScanEnv {
    #[must_use]
    pub fn from_env() -> Self {
        let var = |key: &str| {
            std::env::var_os(key).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
        };
        let home = var("HOME").unwrap_or_default();
        Self {
            claude_dir: var("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex_home: var("CODEX_HOME"),
            copilot_home: var("COPILOT_HOME"),
            home,
            own_pid: std::process::id(),
        }
    }

    fn codex_sessions(&self) -> PathBuf {
        self.codex_home.clone().unwrap_or_else(|| self.home.join(".codex")).join("sessions")
    }

    fn copilot_state(&self) -> PathBuf {
        self.copilot_home
            .clone()
            .unwrap_or_else(|| self.home.join(".copilot"))
            .join("session-state")
    }

    fn locator(&self, cwd: &Path) -> Locator {
        let mut locator = Locator::new(&self.home, cwd);
        locator.codex_home.clone_from(&self.codex_home);
        locator.copilot_home.clone_from(&self.copilot_home);
        locator
    }
}

pub struct Scan {
    pub sessions: Vec<Session>,
    /// Every session id a live registry entry names, Tidewatch's own tabs included: not "recent".
    pub held_ids: HashSet<String>,
}

fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn modified_ms(path: &Path) -> Option<i64> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok().map(unix_ms)
}

fn registry_entries(claude_dir: &Path) -> Vec<RegistryEntry> {
    let Ok(dir) = fs::read_dir(claude_dir.join("sessions")) else { return Vec::new() };
    dir.flatten()
        .filter_map(|file| {
            let pid = registry_pid(file.file_name().to_str()?)?;
            let path = file.path();
            let small = fs::metadata(&path)
                .is_ok_and(|m| m.is_file() && m.len() <= REGISTRY_FILE_MAX_BYTES);
            let entry = parse_registry_entry(&fs::read_to_string(path).ok().filter(|_| small)?)?;
            (entry.pid == pid).then_some(entry)
        })
        .collect()
}

#[must_use]
pub fn scan_running(system: &dyn System, env: &ScanEnv, agents: &AgentTable) -> Option<Scan> {
    let table = ProcessTable::new(parse_ps(&system.ps()?));
    let entries = registry_entries(&env.claude_dir);
    let held_ids = entries
        .iter()
        .filter(|entry| table.get(entry.pid).is_some_and(|row| row.started == entry.proc_start))
        .map(|entry| entry.session_id.clone())
        .collect();
    let mut sessions = claude_sessions(&entries, &table, env.own_pid, agents);
    for candidate in other_candidates(&table, env.own_pid, agents) {
        if let Some(cwd) = system.cwd_of(candidate.pid) {
            sessions.push(session_of(candidate, cwd, env));
        }
    }
    Some(Scan { sessions, held_ids })
}

fn session_of(candidate: Candidate, cwd: PathBuf, env: &ScanEnv) -> Session {
    let Candidate { cli, pid, proc_start, host, flags, resume_id, started_ms } = candidate;
    let (session_id, file) = locate(cli, pid, resume_id, &cwd, started_ms, env);
    let activity = if file.as_deref().is_some_and(|file| turn_finished(cli, file)) {
        Activity::Idle
    } else {
        Activity::Quiet
    };
    Session {
        cli,
        session_id,
        cwd,
        name: None,
        flags,
        presence: Presence::Running(Running { pid, proc_start, host, activity }),
        since_ms: started_ms,
    }
}

fn locate(
    cli: Cli,
    pid: u32,
    resume_id: Option<String>,
    cwd: &Path,
    started_ms: Option<i64>,
    env: &ScanEnv,
) -> (Option<String>, Option<PathBuf>) {
    match cli {
        Cli::Claude => (resume_id, None),
        Cli::Codex => {
            let mut locator = env.locator(cwd);
            locator.session_id.clone_from(&resume_id);
            locator.started_after_ms = started_ms.map(|at| at - START_SLACK_MS);
            let file = locator.locate(cli.source());
            let id = resume_id.or_else(|| file.as_deref().and_then(codex_session_id));
            (id, file)
        }
        Cli::Gemini => {
            let file = newest_gemini_chat(&env.home, cwd);
            (None, file)
        }
        Cli::Copilot => {
            let dir = copilot_dir_of(&env.copilot_state(), pid);
            let id = dir
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .map(str::to_owned);
            (id.filter(|id| is_session_id(id)).or(resume_id), dir.map(|d| d.join("events.jsonl")))
        }
    }
}

fn newest_first(dir: &Path, wanted: impl Fn(&Path) -> bool) -> Vec<(PathBuf, i64)> {
    let mut files: Vec<(PathBuf, i64)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| wanted(path))
        .filter_map(|path| Some((modified_ms(&path)?, path)))
        .map(|(at, path)| (path, at))
        .collect();
    files.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    files
}

fn is_gemini_chat(path: &Path) -> bool {
    let named =
        path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("session-"));
    named && path.extension().is_some_and(|ext| ext == "jsonl")
}

fn gemini_chats(home: &Path, cwd: &Path) -> Vec<(PathBuf, i64)> {
    let Some(slug) = cwd.file_name() else { return Vec::new() };
    newest_first(&home.join(".gemini").join("tmp").join(slug).join("chats"), is_gemini_chat)
}

fn newest_gemini_chat(home: &Path, cwd: &Path) -> Option<PathBuf> {
    gemini_chats(home, cwd).into_iter().next().map(|(path, _)| path)
}

fn copilot_dir_of(state: &Path, pid: u32) -> Option<PathBuf> {
    let lock = format!("inuse.{pid}.lock");
    newest_first(state, Path::is_dir)
        .into_iter()
        .take(COPILOT_RECENT_DIRS)
        .map(|(dir, _)| dir)
        .find(|dir| dir.join(&lock).exists())
}

fn first_line(path: &Path) -> Option<String> {
    use std::io::BufRead as _;
    let mut line = String::new();
    std::io::BufReader::new(fs::File::open(path).ok()?).read_line(&mut line).ok()?;
    Some(line)
}

fn tail_text(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(if from > 0 { text.split_once('\n')?.1.to_owned() } else { text })
}

/// The last thing in the session file is the agent's reply, written a moment ago or earlier: a
/// finished turn. A file written in the last seconds may be mid-turn.
fn turn_finished(cli: Cli, file: &Path) -> bool {
    let fresh = fs::metadata(file)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age < FRESH_WRITE);
    if fresh {
        return false;
    }
    let Some(text) = tail_text(file) else { return false };
    parse_all(cli.source(), &text)
        .last()
        .is_some_and(|entry| entry.role == Role::Assistant && entry.tool.is_none())
}

/// Recent sessions of `folders` whose process is gone, newest first, at most `per_folder` for each
/// folder and CLI. `held` are the ids to leave out (running, or already open in a tab).
#[must_use]
pub fn recent_sessions<S: std::hash::BuildHasher>(
    env: &ScanEnv,
    folders: &[PathBuf],
    held: &HashSet<String, S>,
    per_folder: usize,
) -> Vec<Session> {
    let now = unix_ms(SystemTime::now());
    let oldest = now - RECENT_DAYS * 86_400_000;
    let mut found = Vec::new();
    for folder in folders {
        let recent = [
            claude_recent(env, folder),
            codex_recent(env, folder),
            gemini_recent(env, folder),
            copilot_recent(env, folder),
        ];
        for list in recent {
            found.extend(
                list.into_iter()
                    .filter(|(_, id, at)| {
                        *at >= oldest && id.as_ref().is_none_or(|id| !held.contains(id))
                    })
                    .take(per_folder)
                    .map(|(cli, id, at)| stored(cli, id, folder, at)),
            );
        }
    }
    found
}

type Recent = (Cli, Option<String>, i64);

fn stored(cli: Cli, session_id: Option<String>, cwd: &Path, at: i64) -> Session {
    Session {
        cli,
        session_id,
        cwd: cwd.to_path_buf(),
        name: None,
        flags: crate::elsewhere_flags::Flags::default(),
        presence: Presence::NotRunning,
        since_ms: Some(at),
    }
}

fn claude_recent(env: &ScanEnv, folder: &Path) -> Vec<Recent> {
    let dir = env.claude_dir.join("projects").join(claude_project_dir_name(folder));
    let is_session = |path: &Path| path.extension().is_some_and(|ext| ext == "jsonl");
    newest_first(&dir, is_session)
        .into_iter()
        .filter_map(|(path, at)| {
            let id = path.file_stem()?.to_str()?.to_owned();
            is_session_id(&id).then_some((Cli::Claude, Some(id), at))
        })
        .collect()
}

fn codex_recent(env: &ScanEnv, folder: &Path) -> Vec<Recent> {
    let mut files: Vec<PathBuf> = Vec::new();
    for year in descending_numbers(&env.codex_sessions(), 4) {
        for month in descending_numbers(&year, 2) {
            for day in descending_numbers(&month, 2) {
                let mut names: Vec<PathBuf> =
                    fs::read_dir(day).into_iter().flatten().flatten().map(|e| e.path()).collect();
                names.sort();
                files.extend(names.into_iter().rev());
            }
            if files.len() >= CODEX_RECENT_FILES {
                break;
            }
        }
        if files.len() >= CODEX_RECENT_FILES {
            break;
        }
    }
    files
        .into_iter()
        .take(CODEX_RECENT_FILES)
        .filter(|path| codex_cwd_is(path, folder))
        .filter_map(|path| {
            let id = codex_session_id(&path)?;
            Some((Cli::Codex, Some(id), modified_ms(&path)?))
        })
        .collect()
}

fn descending_numbers(dir: &Path, digits: usize) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.len() == digits && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    found.sort();
    found.reverse();
    found
}

fn codex_cwd_is(rollout: &Path, folder: &Path) -> bool {
    let Some(meta) =
        first_line(rollout).and_then(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
    else {
        return false;
    };
    meta["type"].as_str() == Some("session_meta")
        && meta["payload"]["thread_source"].as_str() != Some("subagent")
        && meta["payload"]["cwd"].as_str().is_some_and(|cwd| Path::new(cwd) == folder)
}

fn gemini_recent(env: &ScanEnv, folder: &Path) -> Vec<Recent> {
    gemini_chats(&env.home, folder)
        .into_iter()
        .filter_map(|(path, at)| {
            let head: serde_json::Value = serde_json::from_str(&first_line(&path)?).ok()?;
            let id = head["sessionId"].as_str().filter(|id| is_session_id(id))?;
            Some((Cli::Gemini, Some(id.to_owned()), at))
        })
        .collect()
}

fn copilot_recent(env: &ScanEnv, folder: &Path) -> Vec<Recent> {
    newest_first(&env.copilot_state(), Path::is_dir)
        .into_iter()
        .take(COPILOT_RECENT_DIRS)
        .filter_map(|(dir, at)| {
            let yaml = fs::read_to_string(dir.join("workspace.yaml")).ok()?;
            let cwd = yaml.lines().find_map(|line| line.strip_prefix("cwd: "))?;
            let id = dir.file_name()?.to_str()?;
            (Path::new(cwd.trim()) == folder && is_session_id(id))
                .then(|| (Cli::Copilot, Some(id.to_owned()), at))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndRequest {
    pub cli: Cli,
    pub pid: u32,
    pub proc_start: String,
    pub session_id: Option<String>,
    pub allow_quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not found by a scan just now, or found with another start time: nothing was signalled.
    Gone,
    Working,
    NotConfirmed,
    Stdusk,
    SignalFailed,
    ScanFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Exited,
    StillRunning,
    Refused(Refusal),
}

/// Scans again, and only when the same session is still there, in a state the request allows, sends
/// it SIGTERM and waits up to `wait` for the process to be gone. Never kills it harder.
#[must_use]
pub fn end_session(
    system: &dyn System,
    env: &ScanEnv,
    agents: &AgentTable,
    request: &EndRequest,
    wait: Duration,
) -> Ended {
    let Some(scan) = scan_running(system, env, agents) else {
        return Ended::Refused(Refusal::ScanFailed);
    };
    let found = scan.sessions.iter().find_map(|session| {
        let running = session.running()?;
        let same = session.cli == request.cli
            && running.pid == request.pid
            && running.proc_start == request.proc_start
            && (request.session_id.is_none() || session.session_id == request.session_id);
        same.then_some(running)
    });
    let Some(running) = found else { return Ended::Refused(Refusal::Gone) };
    match (running.host, running.activity) {
        (crate::elsewhere::Host::Stdusk, _) => return Ended::Refused(Refusal::Stdusk),
        (_, Activity::Working) => return Ended::Refused(Refusal::Working),
        (_, Activity::Quiet) if !request.allow_quiet => {
            return Ended::Refused(Refusal::NotConfirmed);
        }
        _ => {}
    }
    if !system.terminate(request.pid) {
        return Ended::Refused(Refusal::SignalFailed);
    }
    let deadline = Instant::now() + wait;
    loop {
        if system.alive(request.pid).as_deref() != Some(request.proc_start.as_str()) {
            return Ended::Exited;
        }
        if Instant::now() >= deadline {
            return Ended::StillRunning;
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests;
