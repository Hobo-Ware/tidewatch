//! Take-over, the half that touches the machine: `ps` with `TZ=UTC` (Claude writes `procStart` in
//! UTC), `lsof` or `/proc` for a folder, session files, and ending a session with SIGTERM to one
//! re-checked pid, never SIGKILL (claude 2.1.301 exits in under a second). Call it off the UI thread.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::agents::AgentTable;
use crate::elsewhere::{
    Activity, Candidate, Cli, Presence, ProcessTable, RegistryEntry, Running, Scope, Session,
    claude_sessions_in, is_session_id, other_candidates_in, parse_ps, parse_registry_entry,
    registry_pid,
};
use crate::limited;
use crate::permission_mode::last_claude_level;
use crate::restore::codex_session_id;
use crate::tools::Tools;
use crate::transcript::{Parser, Role, Source, parse_all};
use crate::transcript_locate::{Locator, claude_project_dir_name};

const TAIL_BYTES: u64 = 128 * 1024;
const REGISTRY_FILE_MAX_BYTES: u64 = 64 * 1024;
const FRESH_WRITE: Duration = Duration::from_secs(3);
const START_SLACK_MS: i64 = 5000;
const RECENT_DAYS: i64 = 30;
const CODEX_RECENT_FILES: usize = 30;
const COPILOT_RECENT_DIRS: usize = 40;
const TITLE_CACHE_FILES: usize = 256;
const TITLE_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(100);

pub trait System {
    fn ps(&self) -> Option<String>;
    fn cwd_of(&self, pid: u32) -> Option<PathBuf>;
    fn git_root(&self, dir: &Path) -> Option<PathBuf>;
    /// Every process whose working directory is `dir` or inside it. `None` when it cannot look.
    fn cwd_holders(&self, dir: &Path) -> Option<Vec<u32>>;
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
        let output = limited::output(
            self.tools
                .command(program, Path::new("/"))
                .env("TZ", "UTC")
                .env("LC_ALL", "C")
                .args(args)
                .stderr(Stdio::null()),
            self.tools.limit(),
        )
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
        let out = limited::output(
            self.tools.git(dir).args(["rev-parse", "--show-toplevel"]),
            self.tools.limit(),
        )
        .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let root = text.trim();
        (out.status.success() && !root.is_empty()).then(|| PathBuf::from(root))
    }

    #[cfg(target_os = "macos")]
    fn cwd_holders(&self, dir: &Path) -> Option<Vec<u32>> {
        let output = limited::output(
            self.tools
                .command("lsof", Path::new("/"))
                .env("LC_ALL", "C")
                .args(["-w", "-d", "cwd", "-Fpn"])
                .stderr(Stdio::null()),
            self.tools.limit(),
        )
        .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        (output.status.success() || !text.trim().is_empty())
            .then(|| parse_lsof_holders(&text, &folder_forms(dir)))
    }

    #[cfg(not(target_os = "macos"))]
    fn cwd_holders(&self, dir: &Path) -> Option<Vec<u32>> {
        let forms = folder_forms(dir);
        let mut holders = Vec::new();
        for entry in fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if fs::read_link(entry.path().join("cwd")).is_ok_and(|cwd| inside(&forms, &cwd)) {
                holders.push(pid);
            }
        }
        Some(holders)
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

fn folder_forms(dir: &Path) -> Vec<PathBuf> {
    let mut forms = vec![dir.to_path_buf()];
    if let Ok(real) = fs::canonicalize(dir)
        && real != dir
    {
        forms.push(real);
    }
    forms
}

fn inside(forms: &[PathBuf], cwd: &Path) -> bool {
    forms.iter().any(|form| cwd.starts_with(form))
}

#[cfg(any(target_os = "macos", test))]
fn parse_lsof_holders(output: &str, forms: &[PathBuf]) -> Vec<u32> {
    let mut holders = Vec::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(number) = line.strip_prefix('p') {
            pid = number.parse::<u32>().ok();
        } else if let (Some(name), Some(pid)) = (line.strip_prefix('n'), pid)
            && inside(forms, Path::new(name))
            && !holders.contains(&pid)
        {
            holders.push(pid);
        }
    }
    holders
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

    /// Where an app that keeps its files under `paths` finds the CLIs' session files: the home
    /// folder above `.tidewatch` and the Claude folder `paths` names.
    #[must_use]
    pub fn for_paths(paths: &crate::paths::TidewatchPaths) -> Self {
        let from_env = Self::from_env();
        let home = paths.root().parent().map_or(from_env.home, Path::to_path_buf);
        Self { home, claude_dir: paths.claude_dir().to_path_buf(), ..from_env }
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

    fn claude_file(&self, cwd: &Path, id: &str) -> PathBuf {
        self.claude_dir
            .join("projects")
            .join(claude_project_dir_name(cwd))
            .join(format!("{id}.jsonl"))
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
    scan_in(system, env, agents, Scope::Elsewhere { own_pid: env.own_pid }).map(|(_, scan)| scan)
}

/// The sessions running inside the given shells, each with the shell it runs in. A session found
/// under two shells goes to the nearest one, the one with the later pid.
#[must_use]
pub fn scan_in_shells(
    system: &dyn System,
    env: &ScanEnv,
    agents: &AgentTable,
    shells: &[u32],
) -> Option<Vec<(u32, Session)>> {
    let (table, scan) = scan_in(system, env, agents, Scope::Inside(shells))?;
    let mut found: Vec<(u32, Session)> = scan
        .sessions
        .into_iter()
        .filter_map(|session| {
            let pid = session.running()?.pid;
            let shell = shells
                .iter()
                .copied()
                .filter(|shell| pid == *shell || table.descends_from(pid, *shell))
                .max()?;
            Some((shell, session))
        })
        .collect();
    found.sort_by_key(|(shell, session)| (*shell, session.running().map(|r| r.pid)));
    Some(found)
}

fn scan_in(
    system: &dyn System,
    env: &ScanEnv,
    agents: &AgentTable,
    scope: Scope,
) -> Option<(ProcessTable, Scan)> {
    let table = ProcessTable::new(parse_ps(&system.ps()?));
    let entries = registry_entries(&env.claude_dir);
    let held_ids = entries
        .iter()
        .filter(|entry| table.get(entry.pid).is_some_and(|row| row.started == entry.proc_start))
        .map(|entry| entry.session_id.clone())
        .collect();
    let mut sessions = claude_sessions_in(&entries, &table, scope, agents);
    for session in &mut sessions {
        let file = session.session_id.as_deref().map(|id| env.claude_file(&session.cwd, id));
        session.flags.level =
            file.and_then(|file| last_claude_level(&file)).or(session.flags.level);
    }
    for candidate in other_candidates_in(&table, scope, agents) {
        if let Some(cwd) = system.cwd_of(candidate.pid) {
            sessions.push(session_of(candidate, cwd, env));
        }
    }
    Some((table, Scan { sessions, held_ids }))
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

/// What keeps a session out of the recent list: an id, or the transcript file, that a running
/// process or an open Tidewatch tab holds, or that Tidewatch started or resumed itself. Either one
/// is enough, because a tab can write to another file than the id it was launched with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub ids: HashSet<String>,
    pub transcripts: HashSet<PathBuf>,
}

impl Held {
    pub fn add_id(&mut self, id: &str) {
        if !self.ids.contains(id) {
            self.ids.insert(id.to_owned());
        }
    }

    pub fn add_transcript(&mut self, file: &Path) {
        if !self.transcripts.contains(file) {
            self.transcripts.insert(file.to_path_buf());
        }
    }

    fn resolved(&self) -> HashSet<PathBuf> {
        self.transcripts.iter().map(|file| resolve(file)).collect()
    }
}

fn resolve(file: &Path) -> PathBuf {
    fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf())
}

/// The newest session of each of `folders` whose process is gone, at most `per_folder` for each
/// folder across every CLI, left out when `held` names its id or its transcript.
#[must_use]
pub fn recent_sessions(
    env: &ScanEnv,
    folders: &[PathBuf],
    held: &Held,
    titles: &TitleCache,
    per_folder: usize,
) -> Vec<Session> {
    let now = unix_ms(SystemTime::now());
    let oldest = now - RECENT_DAYS * 86_400_000;
    let files = held.resolved();
    let mut found = Vec::new();
    for folder in folders {
        let all = [
            claude_recent(env, folder),
            codex_recent(env, folder),
            gemini_recent(env, folder),
            copilot_recent(env, folder),
        ]
        .concat();
        for recent in newest_unheld(all, &held.ids, &files, oldest, per_folder) {
            let title = recent.file.as_deref().filter(|_| recent.cli == Cli::Claude);
            let name = title.and_then(|file| titles.title_of(file));
            let level = title.and_then(last_claude_level);
            let mut session = Session { name, ..stored(recent.cli, recent.id, folder, recent.at) };
            session.flags.level = level;
            found.push(session);
        }
    }
    found
}

/// When an agent session of any CLI last wrote for `folder`, whether or not its process is still
/// there. `None` when none left a file.
#[must_use]
pub fn newest_session_write(env: &ScanEnv, folder: &Path) -> Option<SystemTime> {
    let newest = folder_forms(folder)
        .iter()
        .flat_map(|form| {
            [
                claude_recent(env, form),
                codex_recent(env, form),
                gemini_recent(env, form),
                copilot_recent(env, form),
            ]
            .concat()
        })
        .map(|recent| recent.at)
        .max()?;
    u64::try_from(newest).ok().map(|ms| SystemTime::UNIX_EPOCH + Duration::from_millis(ms))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recent {
    cli: Cli,
    id: Option<String>,
    at: i64,
    file: Option<PathBuf>,
}

fn newest_unheld(
    mut list: Vec<Recent>,
    ids: &HashSet<String>,
    files: &HashSet<PathBuf>,
    oldest: i64,
    take: usize,
) -> Vec<Recent> {
    list.retain(|recent| {
        let id_held = recent.id.as_ref().is_some_and(|id| ids.contains(id));
        let file_held = recent.file.as_deref().is_some_and(|file| files.contains(&resolve(file)));
        recent.at >= oldest && !id_held && !file_held
    });
    list.sort_by_key(|recent| std::cmp::Reverse(recent.at));
    list.truncate(take);
    list
}

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

type ReadTitle = (u64, i64, Option<String>);

/// The titles read from transcripts, kept per file with the size and modification time they were
/// read at, so a scan reads a transcript again only after it changed. Clones share one table.
#[derive(Clone, Debug, Default)]
pub struct TitleCache {
    read: Arc<Mutex<HashMap<PathBuf, ReadTitle>>>,
}

impl TitleCache {
    fn title_of(&self, file: &Path) -> Option<String> {
        let meta = fs::metadata(file).ok()?;
        let stamp = (meta.len(), meta.modified().ok().map_or(0, unix_ms));
        let hit = self.read.lock().ok()?.get(file).cloned();
        if let Some((len, at, title)) = hit
            && (len, at) == stamp
        {
            return title;
        }
        let title = claude_title(file, stamp.0);
        let mut read = self.read.lock().ok()?;
        if read.len() >= TITLE_CACHE_FILES {
            read.clear();
        }
        read.insert(file.to_path_buf(), (stamp.0, stamp.1, title.clone()));
        title
    }
}

fn claude_title(file: &Path, len: u64) -> Option<String> {
    use std::io::BufRead as _;
    if len > TITLE_FILE_MAX_BYTES {
        return None;
    }
    let mut parser = Parser::new(Source::Claude);
    for line in std::io::BufReader::new(fs::File::open(file).ok()?).lines().map_while(Result::ok) {
        if line.contains("-title\"") || line.contains("\"type\":\"summary\"") {
            parser.feed_line(&line);
        }
    }
    parser.session_title().map(str::to_owned)
}

fn claude_recent(env: &ScanEnv, folder: &Path) -> Vec<Recent> {
    let dir = env.claude_dir.join("projects").join(claude_project_dir_name(folder));
    let is_session = |path: &Path| path.extension().is_some_and(|ext| ext == "jsonl");
    newest_first(&dir, is_session)
        .into_iter()
        .filter_map(|(path, at)| {
            let id = path.file_stem()?.to_str()?.to_owned();
            is_session_id(&id).then_some(Recent {
                cli: Cli::Claude,
                id: Some(id),
                at,
                file: Some(path),
            })
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
            Some(Recent {
                cli: Cli::Codex,
                id: Some(id),
                at: modified_ms(&path)?,
                file: Some(path),
            })
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
            Some(Recent { cli: Cli::Gemini, id: Some(id.to_owned()), at, file: Some(path) })
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
            (Path::new(cwd.trim()) == folder && is_session_id(id)).then(|| Recent {
                cli: Cli::Copilot,
                id: Some(id.to_owned()),
                at,
                file: Some(dir.join("events.jsonl")),
            })
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
    /// End it even while it works on a turn.
    pub allow_working: bool,
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
    end_session_in(Scope::Elsewhere { own_pid: env.own_pid }, system, env, agents, request, wait)
}

/// [`end_session`] for a session found in any [`Scope`]: the scan that checks it is still the
/// same one looks where it was found.
#[must_use]
pub fn end_session_in(
    scope: Scope,
    system: &dyn System,
    env: &ScanEnv,
    agents: &AgentTable,
    request: &EndRequest,
    wait: Duration,
) -> Ended {
    let Some((_, scan)) = scan_in(system, env, agents, scope) else {
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
        (crate::elsewhere::Host::Stdusk, _) if matches!(scope, Scope::Elsewhere { .. }) => {
            return Ended::Refused(Refusal::Stdusk);
        }
        (_, Activity::Working) if !request.allow_working => {
            return Ended::Refused(Refusal::Working);
        }
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
