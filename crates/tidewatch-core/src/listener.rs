//! The local listener (PLAN.md, D5 and D6): one HTTP server on `127.0.0.1:0` that every agent tab
//! reports to.
//!
//! - `POST /hook/<agent>` takes a hook payload and emits [`TabEvent::Hook`] when it carries a status.
//! - `POST /github/<repo token>` takes a repo's GitHub webhook deliveries ([`crate::github_hooks`]): no bearer, the path token and the signature are the check.
//! - `POST /mcp` is the Tidewatch MCP server ([`crate::mcp`]); `GET` and the other methods get 405.
//! - Every request needs `Authorization: Bearer <token>`. Tokens are per tab: [`Listener::register_tab`]
//!   makes one, [`Listener::unregister_tab`] revokes it. Anything else is 401, including unknown paths.
//!
//! The server is hand-rolled on `std::net` with `httparse` for the request head: HTTP/1.1 keep-alive,
//! `Content-Length` and chunked bodies, `Expect: 100-continue`, head and body size caps, one thread per
//! connection. That is about the size of the glue a crate would need, and it keeps the unmaintained
//! `tiny_http` out. Events leave through an `mpsc` channel plus a repaint callback, so the UI never
//! waits on a socket.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::avatar_ask::{self, Face};
use crate::avatar_store;
use crate::background;
use crate::events::{Repaint, RepoMessage, TabEvent, TabMessage};
use crate::github_hooks::{self, Answer, GithubHooks, RepoHook};
use crate::launch::HookListener;
use crate::mcp::{AskWait, McpReply, TabRole, ToolCtx, handle_post};
use crate::paths::{
    TidewatchPaths, avatar_asked_file_beside, avatar_file_beside, progress_file_beside,
    sent_file_beside,
};
use crate::sent::{self, Reason, Record, because};
use crate::status::parse_hook;
use crate::user_markup;
use crate::{progress_store, prompt_answer};

/// Where the listener binds: loopback only, a random free port (PLAN.md, Constants).
pub const HOOK_BIND: &str = "127.0.0.1:0";
const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024;
const MAX_CONNECTIONS: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const TOKEN_BYTES: usize = 16;
const PORT_RETRY: Duration = Duration::from_millis(50);
/// How long a held `ask_user` call waits for the user before the agent is told to take the reversible or recommended option and list it in Needs you.
pub const ASK_TIMEOUT: Duration = Duration::from_mins(30);
const ASK_POLL: Duration = Duration::from_millis(500);

/// Who a token belongs to.
#[derive(Clone, Debug)]
pub struct TabRegistration {
    /// The app's id for the tab; every [`TabMessage`] carries it.
    pub tab_id: u64,
    /// The agent id (`claude`, `gemini`, ...). Signs the agent's replies in the notes.
    pub agent: String,
    /// Which Tidewatch tools the tab is offered.
    pub role: TabRole,
    /// The repository the tab belongs to, and the tab's state name: together they pick `notes.json`.
    pub repo: PathBuf,
    pub tab: String,
    /// Stamped on every message the token's holder sends. A tab that starts a new agent in place of
    /// the old one registers again with a higher number, so the app can tell the old agent's late
    /// messages apart. Tabs that never swap their agent use 0.
    pub generation: u64,
}

/// What a running agent holds that another process must honour for the agent to keep reporting:
/// the token its launch baked in, and what the token stands for. A restart that keeps the agent
/// running carries it to the new process, which adopts it under the tab's new id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub token: String,
    pub generation: u64,
    pub agent: String,
    /// Missing in a credential an older version saved: such a tab is offered the plain agent's tools.
    #[serde(default)]
    pub role: TabRole,
    pub notes_file: PathBuf,
}

struct Entry {
    tab_id: u64,
    generation: u64,
    agent: String,
    role: TabRole,
    notes_file: PathBuf,
}

struct Shared {
    tabs: Mutex<HashMap<String, Arc<Entry>>>,
    events: Sender<TabMessage>,
    repaint: Repaint,
    notes_lock: Mutex<()>,
    ask_timeout_ms: AtomicU64,
    avatar_ask_lock: Mutex<()>,
    avatar_ask_exempt: Mutex<HashSet<u64>>,
    service_lines: Mutex<HashMap<u64, String>>,
    service_told: Mutex<HashMap<u64, String>>,
    github: GithubHooks,
}

impl Shared {
    fn entry(&self, token: &str) -> Option<Arc<Entry>> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner).get(token).cloned()
    }

    fn emit(&self, entry: &Entry, event: TabEvent) {
        let _ = self.events.send(TabMessage {
            tab_id: entry.tab_id,
            generation: entry.generation,
            event,
        });
        (self.repaint)();
    }
}

/// The running server. Dropping it stops accepting connections.
pub struct Listener {
    shared: Arc<Shared>,
    github: Mutex<Receiver<RepoMessage>>,
    paths: TidewatchPaths,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Listener {
    /// Binds `127.0.0.1:0` and starts serving on background threads. The returned receiver gets every
    /// event; `repaint` is called after each one is queued. An `Err` means the bind failed: the app
    /// falls back to the terminal and process layers of the status (PLAN.md, Failure model).
    pub fn start(
        paths: TidewatchPaths,
        repaint: Repaint,
    ) -> io::Result<(Self, Receiver<TabMessage>)> {
        Self::start_on(TcpListener::bind(HOOK_BIND)?, paths, repaint)
    }

    /// Binds the loopback port an earlier process served, which is the one a running agent's launch
    /// has in its hook and MCP addresses. The earlier process may still be shutting down, so a
    /// busy port is tried again until `patience` runs out.
    pub fn start_on_port(
        port: u16,
        paths: TidewatchPaths,
        repaint: Repaint,
        patience: Duration,
    ) -> io::Result<(Self, Receiver<TabMessage>)> {
        let deadline = Instant::now() + patience;
        let listener = loop {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => break listener,
                Err(e) if Instant::now() >= deadline => return Err(e),
                Err(_) => std::thread::sleep(PORT_RETRY),
            }
        };
        Self::start_on(listener, paths, repaint)
    }

    /// How long a held `ask_user` call waits from now on (30 minutes unless changed).
    pub fn set_ask_timeout(&self, timeout: Duration) {
        let millis = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        self.shared.ask_timeout_ms.store(millis, Ordering::SeqCst);
    }

    /// The tabs whose prompts never carry the avatar ask: a Meeseeks wears a fixed face, and a
    /// host that writes nothing must not leave a marker. Replaces the earlier list.
    pub fn exempt_from_avatar_ask(&self, tabs: &[u64]) {
        let mut exempt =
            self.shared.avatar_ask_exempt.lock().unwrap_or_else(PoisonError::into_inner);
        exempt.clear();
        exempt.extend(tabs.iter().copied());
    }

    /// What each tab's next prompt tells it about the repo's services, replacing the earlier
    /// lines. A tab hears a line once, and again only when its text changes.
    pub fn set_service_lines(&self, lines: HashMap<u64, String>) {
        replace_service_lines(&self.shared, lines);
    }

    /// The port the listener serves.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    fn start_on(
        listener: TcpListener,
        paths: TidewatchPaths,
        repaint: Repaint,
    ) -> io::Result<(Self, Receiver<TabMessage>)> {
        let addr = listener.local_addr()?;
        let (events, receiver) = channel();
        let (github, github_inbox) = GithubHooks::new();
        let shared = Arc::new(Shared {
            tabs: Mutex::new(HashMap::new()),
            github,
            events,
            repaint,
            notes_lock: Mutex::new(()),
            avatar_ask_lock: Mutex::new(()),
            avatar_ask_exempt: Mutex::new(HashSet::new()),
            service_lines: Mutex::new(HashMap::new()),
            service_told: Mutex::new(HashMap::new()),
            ask_timeout_ms: AtomicU64::new(
                u64::try_from(ASK_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
            ),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let accept = {
            let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("tidewatch-listener".into())
                .spawn(move || accept_loop(&listener, &shared, &stop))?
        };
        Ok((
            Self {
                shared,
                github: Mutex::new(github_inbox),
                paths,
                addr,
                stop,
                accept: Some(accept),
            },
            receiver,
        ))
    }

    /// For example `http://127.0.0.1:51234`, no trailing slash.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Makes a fresh token for the tab and returns what `launch_command` needs. Registering a tab
    /// that already has a token revokes the old one.
    pub fn register_tab(&self, registration: TabRegistration) -> HookListener {
        let token = new_token();
        let entry = Arc::new(Entry {
            tab_id: registration.tab_id,
            generation: registration.generation,
            agent: registration.agent,
            role: registration.role,
            notes_file: self.paths.notes_file(&registration.repo, &registration.tab),
        });
        let mut tabs = self.shared.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        tabs.retain(|_, existing| existing.tab_id != entry.tab_id);
        tabs.insert(token.clone(), entry);
        HookListener { base_url: self.base_url(), token }
    }

    /// The credential the tab's agent reports with, for a process that takes the agent over.
    #[must_use]
    pub fn credential_of(&self, tab_id: u64) -> Option<Credential> {
        let tabs = self.shared.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        tabs.iter().find(|(_, entry)| entry.tab_id == tab_id).map(|(token, entry)| Credential {
            token: token.clone(),
            generation: entry.generation,
            agent: entry.agent.clone(),
            role: entry.role,
            notes_file: entry.notes_file.clone(),
        })
    }

    /// Honours another process's credential for `tab_id`, so the agent that holds the token keeps
    /// reporting to this tab. Like [`Self::register_tab`], it revokes the tab's earlier tokens.
    pub fn adopt_tab(&self, tab_id: u64, credential: &Credential) {
        let entry = Arc::new(Entry {
            tab_id,
            generation: credential.generation,
            agent: credential.agent.clone(),
            role: credential.role,
            notes_file: credential.notes_file.clone(),
        });
        let mut tabs = self.shared.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        tabs.retain(|_, existing| existing.tab_id != tab_id);
        tabs.insert(credential.token.clone(), entry);
    }

    /// Makes the address and secret `gh webhook forward` is started with for `repo`, replacing the
    /// repo's earlier ones.
    #[must_use]
    pub fn register_repo(&self, repo: &str) -> RepoHook {
        let (token, secret) = (new_token(), new_token());
        let url = format!("{}{}{token}", self.base_url(), github_hooks::PATH_PREFIX);
        self.shared.github.register(repo, token, secret.clone());
        RepoHook { url, secret }
    }

    pub fn unregister_repo(&self, repo: &str) {
        self.shared.github.unregister(repo);
    }

    /// The GitHub deliveries that arrived since the last call, oldest first.
    #[must_use]
    pub fn drain_github(&self) -> Vec<RepoMessage> {
        self.github.lock().unwrap_or_else(PoisonError::into_inner).try_iter().collect()
    }

    /// Revokes every token of the tab. Later requests with them get 401.
    pub fn unregister_tab(&self, tab_id: u64) {
        let mut tabs = self.shared.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        tabs.retain(|_, entry| entry.tab_id != tab_id);
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0u8; TOKEN_BYTES];
    let from_os = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok();
    if !from_os {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let digest = Sha256::digest(format!(
            "{nanos}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        bytes.copy_from_slice(&digest[..TOKEN_BYTES]);
    }
    bytes.iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

struct ConnectionSlot(Arc<AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, stop: &AtomicBool) {
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let slot = ConnectionSlot(Arc::clone(&active));
        let shared = Arc::clone(shared);
        let spawned = std::thread::Builder::new().name("tidewatch-conn".into()).spawn(move || {
            serve(&shared, stream);
            drop(slot);
        });
        drop(spawned);
    }
}

fn authorised(shared: &Shared, head: &Head) -> bool {
    head.bearer.as_deref().is_some_and(|token| shared.entry(token).is_some())
        || github_token(&head.path).is_some_and(|token| shared.github.known(token))
}

fn serve(shared: &Shared, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let mut conn = Conn { stream, buf: Vec::new() };
    loop {
        match read_request(&mut conn, &|head| authorised(shared, head)) {
            Ok(request) => {
                let response = route(shared, &request, &conn.stream);
                if write_response(&mut conn.stream, &response, request.keep_alive).is_err()
                    || !request.keep_alive
                {
                    break;
                }
            }
            Err(Fail::Reply(response)) => {
                let _ = write_response(&mut conn.stream, &response, false);
                break;
            }
            Err(Fail::Closed) => break,
        }
    }
    let _ = conn.stream.shutdown(Shutdown::Both);
}

struct Request {
    method: String,
    path: String,
    bearer: Option<String>,
    github_event: Option<String>,
    signature: Option<String>,
    body: Vec<u8>,
    keep_alive: bool,
}

struct Response {
    status: u16,
    content_type: &'static str,
    extra: Vec<(&'static str, &'static str)>,
    body: String,
}

impl Response {
    fn json(status: u16, body: String) -> Self {
        Self { status, content_type: "application/json", extra: vec![], body }
    }

    fn empty(status: u16) -> Self {
        Self { status, content_type: "text/plain", extra: vec![], body: String::new() }
    }

    fn error(status: u16, message: &str) -> Self {
        Self::json(status, serde_json::json!({ "error": message }).to_string())
    }

    fn unauthorized() -> Self {
        Self {
            extra: vec![("WWW-Authenticate", "Bearer")],
            ..Self::error(401, "missing or unknown bearer token")
        }
    }

    fn method_not_allowed() -> Self {
        Self { extra: vec![("Allow", "POST")], ..Self::error(405, "use POST") }
    }
}

fn github_token(path: &str) -> Option<&str> {
    path.split('?').next()?.strip_prefix(github_hooks::PATH_PREFIX).filter(|t| !t.contains('/'))
}

fn github_route(shared: &Shared, token: &str, request: &Request) -> Response {
    if request.method != "POST" {
        return Response::method_not_allowed();
    }
    let answer = shared.github.deliver(
        token,
        request.github_event.as_deref(),
        request.signature.as_deref(),
        &request.body,
    );
    match answer {
        Answer::Taken => {
            (shared.repaint)();
            Response::json(200, "{}".into())
        }
        Answer::BadSignature => Response::error(401, "bad signature"),
        Answer::BadRequest(message) => Response::error(400, message),
    }
}

fn route(shared: &Shared, request: &Request, stream: &TcpStream) -> Response {
    if let Some(token) = github_token(&request.path) {
        return github_route(shared, token, request);
    }
    let Some(entry) = request.bearer.as_deref().and_then(|token| shared.entry(token)) else {
        return Response::unauthorized();
    };
    let path = request.path.split('?').next().unwrap_or_default();
    if path == "/mcp" {
        return mcp_route(shared, &entry, request, stream);
    }
    match path.strip_prefix("/hook/").filter(|agent| valid_agent_id(agent)) {
        Some(agent) => hook_route(shared, &entry, agent, request),
        None => Response::error(404, "not found"),
    }
}

fn valid_agent_id(agent: &str) -> bool {
    (1..=32).contains(&agent.len())
        && agent.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn hook_route(shared: &Shared, entry: &Entry, agent: &str, request: &Request) -> Response {
    if request.method != "POST" {
        return Response::method_not_allowed();
    }
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
        return Response::error(400, "the body is not JSON");
    };
    if let Some(signal) = background::from_hook(agent, &payload) {
        shared.emit(entry, TabEvent::Background(signal));
    }
    if let Some(event) = parse_hook(agent, &payload) {
        shared.emit(entry, TabEvent::Hook(event));
    }
    Response::json(200, prompt_reply(shared, entry, agent, &payload).unwrap_or_else(|| "{}".into()))
}

/// The answer to a prompt: extra context that asks a tab with no avatar, never asked, to draw
/// one, and one that asks a tab with no ledger to write it or a tab whose ledger holds open items
/// to refresh it. Both ride the one answer when both apply. The avatar marker is written first, so a tab whose state folder cannot
/// be written is not asked on every prompt.
fn prompt_reply(
    shared: &Shared,
    entry: &Entry,
    agent: &str,
    payload: &serde_json::Value,
) -> Option<String> {
    let on_prompt = payload.get("hook_event_name").and_then(serde_json::Value::as_str)
        == Some("UserPromptSubmit");
    if !on_prompt || !avatar_ask::answers_prompts(agent) {
        return None;
    }
    let mut contexts = Vec::new();
    if avatar_due(shared, entry, on_prompt) {
        contexts.push(avatar_ask::CONTEXT.to_owned());
        tell(entry, Reason::Avatar, because::AVATAR_ASK, avatar_ask::CONTEXT);
    }
    let typed_by_user = prompt_is_users(payload);
    let ledger_line = match progress_store::load(&progress_file_beside(&entry.notes_file)) {
        Some(ledger) => prompt_answer::ledger_context(&ledger),
        None => Some(prompt_answer::MISSING_LEDGER_CONTEXT.to_owned()),
    };
    if let Some(line) = ledger_line {
        let answer = line == prompt_answer::LEDGER_CONTEXT;
        let missing = line == prompt_answer::MISSING_LEDGER_CONTEXT;
        let told = told_since_last_save(entry, &line);
        let one_off = answer || missing;
        let silent = one_off && (!typed_by_user || (answer && told));
        if !silent {
            if one_off && !told {
                let why = if answer { because::LEDGER_NUDGE } else { because::LEDGER_MISSING };
                tell(entry, Reason::LedgerGuard, why, &line);
            }
            contexts.push(line);
        }
    }
    if let Some(line) = service_line_due(shared, entry.tab_id) {
        contexts.push(line);
    }
    prompt_answer::answer(&contexts.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Whether the prompt is the user's own words. A background task's notice and the other texts
/// Claude Code files as a user line are not; a payload with no prompt is taken as the user's.
fn prompt_is_users(payload: &serde_json::Value) -> bool {
    payload.get("prompt").and_then(serde_json::Value::as_str).is_none_or(|prompt| {
        !prompt.trim_start().starts_with(background::NOTICE_OPENING)
            && matches!(user_markup::classify(prompt), user_markup::Record::Text(_))
    })
}

fn replace_service_lines(shared: &Shared, lines: HashMap<u64, String>) {
    let mut told = shared.service_told.lock().unwrap_or_else(PoisonError::into_inner);
    told.retain(|tab, _| lines.contains_key(tab));
    *shared.service_lines.lock().unwrap_or_else(PoisonError::into_inner) = lines;
}

fn service_line_due(shared: &Shared, tab: u64) -> Option<String> {
    let line =
        shared.service_lines.lock().unwrap_or_else(PoisonError::into_inner).get(&tab).cloned()?;
    let mut told = shared.service_told.lock().unwrap_or_else(PoisonError::into_inner);
    if told.get(&tab) == Some(&line) {
        return None;
    }
    told.insert(tab, line.clone());
    Some(line)
}

fn told_since_last_save(entry: &Entry, text: &str) -> bool {
    let saved = std::fs::metadata(progress_file_beside(&entry.notes_file))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
    sent::load(&sent_file_beside(&entry.notes_file))
        .iter()
        .rev()
        .find(|record| record.reason == Reason::LedgerGuard && record.text == text)
        .is_some_and(|last| last.at_ms >= saved)
}

fn tell(entry: &Entry, reason: Reason, because: &str, text: &str) {
    let record = Record::new(reason, because, text, None);
    let _ = sent::append(&sent_file_beside(&entry.notes_file), &record);
}

fn avatar_due(shared: &Shared, entry: &Entry, on_prompt: bool) -> bool {
    let exempt = shared
        .avatar_ask_exempt
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&entry.tab_id);
    let _one_at_a_time = shared.avatar_ask_lock.lock().unwrap_or_else(PoisonError::into_inner);
    let marker = avatar_asked_file_beside(&entry.notes_file);
    let face = if exempt {
        Face::Fixed
    } else if avatar_store::load(&avatar_file_beside(&entry.notes_file)).is_some() {
        Face::Picture
    } else {
        Face::Letter
    };
    let due = avatar_ask::due(&avatar_ask::Facts {
        answers_prompts: true,
        face,
        asked: avatar_store::was_asked(&marker),
        on_prompt,
    });
    due && avatar_store::mark_asked(&marker).is_ok()
}

/// Whether the client of `stream` still waits for its reply: false once it has closed the
/// connection. A held `ask_user` call asks while it waits, because nothing reads the socket then.
fn client_waits(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut byte = [0u8; 1];
    let waits = match stream.peek(&mut byte) {
        Ok(read) => read > 0,
        Err(e) => matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted),
    };
    let _ = stream.set_nonblocking(false);
    waits
}

fn mcp_route(shared: &Shared, entry: &Entry, request: &Request, stream: &TcpStream) -> Response {
    if request.method != "POST" {
        return Response::method_not_allowed();
    }
    let alive = || client_waits(stream);
    let progress_file = progress_file_beside(&entry.notes_file);
    let ctx = ToolCtx {
        agent: &entry.agent,
        role: entry.role,
        notes_file: &entry.notes_file,
        notes_lock: &shared.notes_lock,
        progress_file: &progress_file,
        ask: AskWait {
            timeout: Duration::from_millis(shared.ask_timeout_ms.load(Ordering::SeqCst)),
            poll: ASK_POLL,
            alive: &alive,
        },
    };
    match handle_post(&request.body, &ctx, &|event| shared.emit(entry, event)) {
        McpReply::Json(body) => Response::json(200, body),
        McpReply::Accepted => Response::empty(202),
    }
}

enum Fail {
    Closed,
    Reply(Response),
}

struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Conn {
    fn fill(&mut self) -> Result<(), Fail> {
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(Fail::Closed),
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    return Ok(());
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(Fail::Closed),
            }
        }
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>, Fail> {
        while self.buf.len() < n {
            self.fill()?;
        }
        Ok(self.buf.drain(..n).collect())
    }

    fn take_line(&mut self) -> Result<String, Fail> {
        loop {
            if let Some(end) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line: Vec<u8> = self.buf.drain(..end + 2).take(end).collect();
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            if self.buf.len() > MAX_LINE_BYTES {
                return Err(bad_request("chunk line too long"));
            }
            self.fill()?;
        }
    }
}

fn bad_request(message: &str) -> Fail {
    Fail::Reply(Response::error(400, message))
}

struct Head {
    method: String,
    path: String,
    content_length: Option<usize>,
    chunked: bool,
    expect_continue: bool,
    bearer: Option<String>,
    github_event: Option<String>,
    signature: Option<String>,
    keep_alive: bool,
}

fn parse_head(buf: &[u8]) -> Result<Option<(Head, usize)>, Fail> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut headers);
    let head_len = match parsed.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(httparse::Error::TooManyHeaders) => {
            return Err(Fail::Reply(Response::error(431, "too many headers")));
        }
        Err(_) => return Err(bad_request("malformed request")),
    };
    let (Some(method), Some(path)) = (parsed.method, parsed.path) else {
        return Err(bad_request("malformed request"));
    };
    let mut head = Head {
        method: method.to_owned(),
        path: path.to_owned(),
        content_length: None,
        chunked: false,
        expect_continue: false,
        bearer: None,
        github_event: None,
        signature: None,
        keep_alive: parsed.version == Some(1),
    };
    for header in parsed.headers.iter() {
        let value = String::from_utf8_lossy(header.value);
        let value = value.trim();
        match header.name.to_ascii_lowercase().as_str() {
            "content-length" => {
                let length: usize = value.parse().map_err(|_| bad_request("bad Content-Length"))?;
                if head.content_length.is_some_and(|seen| seen != length) {
                    return Err(bad_request("conflicting Content-Length"));
                }
                head.content_length = Some(length);
            }
            "transfer-encoding" => {
                if !value.eq_ignore_ascii_case("chunked") {
                    return Err(bad_request("unsupported Transfer-Encoding"));
                }
                head.chunked = true;
            }
            "expect" => head.expect_continue = value.eq_ignore_ascii_case("100-continue"),
            "authorization" => head.bearer = bearer_token(value).map(str::to_owned),
            "x-github-event" => head.github_event = Some(value.to_owned()),
            "x-hub-signature-256" => head.signature = Some(value.to_owned()),
            "connection" => {
                for token in value.split(',').map(str::trim) {
                    if token.eq_ignore_ascii_case("close") {
                        head.keep_alive = false;
                    } else if token.eq_ignore_ascii_case("keep-alive") && parsed.version == Some(0)
                    {
                        head.keep_alive = true;
                    }
                }
            }
            _ => {}
        }
    }
    if head.chunked && head.content_length.is_some() {
        return Err(bad_request("both Content-Length and chunked"));
    }
    Ok(Some((head, head_len)))
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token.trim()).filter(|t| !t.is_empty())
}

fn read_request(conn: &mut Conn, authorised: &dyn Fn(&Head) -> bool) -> Result<Request, Fail> {
    let (head, head_len) = loop {
        if let Some(parsed) = parse_head(&conn.buf)? {
            break parsed;
        }
        if conn.buf.len() > MAX_HEAD_BYTES {
            return Err(Fail::Reply(Response::error(431, "request head too large")));
        }
        conn.fill()?;
    };
    conn.buf.drain(..head_len);
    if !authorised(&head) {
        return Err(Fail::Reply(Response::unauthorized()));
    }
    if head.expect_continue && conn.buf.is_empty() {
        conn.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").map_err(|_| Fail::Closed)?;
    }
    let body = if head.chunked {
        read_chunked(conn)?
    } else {
        let length = head.content_length.unwrap_or(0);
        if length > MAX_BODY_BYTES {
            return Err(too_large());
        }
        conn.take(length)?
    };
    Ok(Request {
        method: head.method,
        path: head.path,
        bearer: head.bearer,
        github_event: head.github_event,
        signature: head.signature,
        body,
        keep_alive: head.keep_alive,
    })
}

fn too_large() -> Fail {
    Fail::Reply(Response::error(413, "request body too large"))
}

fn read_chunked(conn: &mut Conn) -> Result<Vec<u8>, Fail> {
    let mut body = Vec::new();
    loop {
        let line = conn.take_line()?;
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| bad_request("bad chunk size"))?;
        if size == 0 {
            while !conn.take_line()?.is_empty() {}
            return Ok(body);
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(too_large());
        }
        body.extend(conn.take(size)?);
        if conn.take(2)? != b"\r\n" {
            return Err(bad_request("chunk not followed by CRLF"));
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

fn write_response(stream: &mut TcpStream, response: &Response, keep_alive: bool) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
        if keep_alive { "keep-alive" } else { "close" },
    );
    for (name, value) in &response.extra {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    head.push_str(&response.body);
    stream.write_all(head.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Notes;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicUsize;

    struct Fixture {
        shared: Arc<Shared>,
        receiver: Receiver<TabMessage>,
        paths: TidewatchPaths,
        repaints: Arc<AtomicUsize>,
        peer: TcpStream,
        _server: TcpListener,
        _home: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let home = tempfile::tempdir().unwrap();
        let (events, receiver) = channel();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        let shared = Arc::new(Shared {
            tabs: Mutex::new(HashMap::new()),
            events,
            repaint: Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            notes_lock: Mutex::new(()),
            avatar_ask_lock: Mutex::new(()),
            avatar_ask_exempt: Mutex::new(HashSet::new()),
            service_lines: Mutex::new(HashMap::new()),
            service_told: Mutex::new(HashMap::new()),
            github: GithubHooks::new().0,
            ask_timeout_ms: AtomicU64::new(1000),
        });
        let server = TcpListener::bind(HOOK_BIND).unwrap();
        let peer = TcpStream::connect(server.local_addr().unwrap()).unwrap();
        Fixture {
            shared,
            receiver,
            paths: TidewatchPaths::new(home.path()),
            repaints,
            peer,
            _server: server,
            _home: home,
        }
    }

    impl Fixture {
        fn register(&self, tab_id: u64, tab: &str) -> String {
            let token = new_token();
            let entry = Arc::new(Entry {
                tab_id,
                generation: 0,
                agent: "claude".into(),
                role: TabRole::default(),
                notes_file: self.paths.notes_file(Path::new("/work/web"), tab),
            });
            self.shared.tabs.lock().unwrap().insert(token.clone(), entry);
            token
        }

        fn send(&self, method: &str, path: &str, token: Option<&str>, body: &str) -> Response {
            let request = Request {
                method: method.into(),
                path: path.into(),
                bearer: token.map(str::to_owned),
                github_event: None,
                signature: None,
                body: body.as_bytes().to_vec(),
                keep_alive: true,
            };
            route(&self.shared, &request, &self.peer)
        }

        fn events(&self) -> Vec<TabMessage> {
            self.receiver.try_iter().collect()
        }
    }

    fn body_json(response: &Response) -> Value {
        serde_json::from_str(&response.body).unwrap()
    }

    const STOP: &str =
        r#"{"hook_event_name":"Stop","session_id":"s1","last_assistant_message":"done"}"#;
    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

    const PROMPT: &str =
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1","prompt":"hi"}"#;

    fn asks_for_an_avatar(response: &Response) -> bool {
        response.status == 200
            && body_json(response)["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .is_some_and(|context| context.contains("set_avatar"))
    }

    fn notes_of(f: &Fixture, token: &str) -> PathBuf {
        f.shared.entry(token).unwrap().notes_file.clone()
    }

    const CALM: &str = "# Goal\nShip\n\n## Status\nstate: working\n";

    #[test]
    fn a_credential_saved_before_roles_existed_reads_as_a_plain_agent() {
        let old = r#"{"token":"t","generation":1,"agent":"claude","notes_file":"/n.json"}"#;
        let credential: Credential = serde_json::from_str(old).unwrap();
        assert_eq!(credential.role, TabRole::Agent);
        let kept: Credential = serde_json::from_str(
            &serde_json::to_string(&Credential { role: TabRole::Meeseeks, ..credential }).unwrap(),
        )
        .unwrap();
        assert_eq!(kept.role, TabRole::Meeseeks);
    }

    #[test]
    fn a_credential_saved_for_a_workflow_stage_reads_as_a_plain_agent() {
        let old = r#"{"token":"t","generation":1,"agent":"claude","role":"Workflow","notes_file":"/n.json"}"#;
        let credential: Credential = serde_json::from_str(old).unwrap();
        assert_eq!(credential.role, TabRole::Agent);
    }

    #[test]
    fn the_first_prompt_of_a_tab_with_no_avatar_is_answered_with_the_ask_and_only_the_first() {
        let f = fixture();
        let token = f.register(1, "a");
        save_ledger(&f, &token, CALM);
        let marker = avatar_asked_file_beside(&notes_of(&f, &token));
        let first = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert!(asks_for_an_avatar(&first), "{}", first.body);
        assert_eq!(body_json(&first)["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert!(marker.exists());
        let second = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(second.body, "{}");
        assert_eq!(f.events().len(), 2, "the prompt is still a status event both times");
    }

    #[test]
    fn a_tab_asked_before_a_tab_with_a_picture_and_a_tab_that_is_exempt_are_not_asked() {
        let f = fixture();
        let asked = f.register(1, "asked");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &asked))).unwrap();
        let drawn = f.register(2, "drawn");
        let svg = "<svg><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";
        avatar_store::save(
            &avatar_file_beside(&notes_of(&f, &drawn)),
            &crate::avatar::clean(svg, "a dot").unwrap(),
        )
        .unwrap();
        let fixed = f.register(3, "fixed");
        let fixed_marker = avatar_asked_file_beside(&notes_of(&f, &fixed));
        let listener_exempt = [3];
        f.shared.avatar_ask_exempt.lock().unwrap().extend(listener_exempt);
        for (what, token) in [("asked", &asked), ("drawn", &drawn), ("fixed", &fixed)] {
            save_ledger(&f, token, CALM);
            let response = f.send("POST", "/hook/claude", Some(token), PROMPT);
            assert_eq!(response.body, "{}", "{what}");
        }
        assert!(!fixed_marker.exists(), "an exempt tab leaves no marker");
    }

    #[test]
    fn only_a_claude_prompt_carries_the_ask() {
        let f = fixture();
        let token = f.register(1, "a");
        let marker = avatar_asked_file_beside(&notes_of(&f, &token));
        let others = [
            ("/hook/claude", STOP),
            ("/hook/claude", r#"{"hook_event_name":"SessionStart"}"#),
            ("/hook/copilot", PROMPT),
            ("/hook/codex", PROMPT),
            ("/hook/gemini", r#"{"hook_event_name":"BeforeAgent"}"#),
        ];
        for (path, body) in others {
            let response = f.send("POST", path, Some(&token), body);
            assert_eq!(response.body, "{}", "{path} {body}");
        }
        assert!(!marker.exists());
    }

    #[test]
    fn a_tab_whose_marker_cannot_be_written_is_not_asked() {
        let f = fixture();
        let token = f.register(1, "a");
        let marker = avatar_asked_file_beside(&notes_of(&f, &token));
        let state_dir = marker.parent().unwrap();
        std::fs::create_dir_all(state_dir.parent().unwrap()).unwrap();
        std::fs::write(state_dir, "not a folder").unwrap();
        let response = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert!(!asks_for_an_avatar(&response), "{}", response.body);
    }

    fn context_of(response: &Response) -> String {
        body_json(response)["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn save_ledger(f: &Fixture, token: &str, text: &str) {
        let file = progress_file_beside(&notes_of(f, token));
        progress_store::save(&file, &crate::progress::Ledger::parse(text)).unwrap();
    }

    const QUESTION: &str = "# Goal\nShip\n\n## Status\nstate: waiting\n\n## Needs you\n- Yes or changes on the board?\n";

    #[test]
    fn a_prompt_to_a_tab_whose_ledger_waits_on_the_owner_is_answered_with_the_ledger_nudge() {
        let cases = [
            ("a question", QUESTION, true),
            ("waiting, no question", "# Goal\nShip\n\n## Status\nstate: waiting\n", true),
            ("blocked", "# Goal\nShip\n\n## Status\nstate: blocked\n", true),
            ("working", "# Goal\nShip\n\n## Status\nstate: working\n", false),
            (
                "done with a hand-off",
                "# Goal\nShip\n\n## Status\nstate: done\n\n## Needs you\n- Merge it\n",
                false,
            ),
        ];
        for (what, text, nudged) in cases {
            let f = fixture();
            let token = f.register(1, "a");
            avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
            save_ledger(&f, &token, text);
            let response = f.send("POST", "/hook/claude", Some(&token), PROMPT);
            if nudged {
                assert_eq!(context_of(&response), prompt_answer::LEDGER_CONTEXT, "{what}");
            } else {
                assert_eq!(response.body, "{}", "{what}");
            }
        }
    }

    #[test]
    fn a_prompt_to_a_tab_that_never_saved_a_ledger_asks_for_the_whole_ledger_this_turn() {
        let f = fixture();
        let token = f.register(1, "a");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
        for _ in 0..2 {
            let response = f.send("POST", "/hook/claude", Some(&token), PROMPT);
            assert_eq!(context_of(&response), prompt_answer::MISSING_LEDGER_CONTEXT);
        }
        save_ledger(&f, &token, CALM);
        let saved = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(saved.body, "{}", "a saved ledger ends the asking");
    }

    #[test]
    fn the_missing_ledger_ask_shares_the_answer_with_the_avatar_ask_and_rides_only_a_claude_prompt()
    {
        let f = fixture();
        let token = f.register(1, "a");
        let first = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        let context = context_of(&first);
        assert!(context.contains(avatar_ask::CONTEXT), "{context}");
        assert!(context.contains(prompt_answer::MISSING_LEDGER_CONTEXT), "{context}");
        for (path, body) in
            [("/hook/claude", STOP), ("/hook/copilot", PROMPT), ("/hook/codex", PROMPT)]
        {
            assert_eq!(f.send("POST", path, Some(&token), body).body, "{}", "{path}");
        }
    }

    #[test]
    fn the_ledger_nudge_comes_once_per_waiting_ledger_and_keeps_the_avatar_ask() {
        let f = fixture();
        let token = f.register(1, "a");
        save_ledger(&f, &token, QUESTION);
        let first = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        let context = context_of(&first);
        assert!(context.contains(avatar_ask::CONTEXT), "{context}");
        assert!(context.contains(prompt_answer::LEDGER_CONTEXT), "{context}");
        let second = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(second.body, "{}", "once per question");
        save_ledger(&f, &token, "# Goal\nShip\n\n## Status\nstate: working\n");
        let third = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(third.body, "{}", "the agent refreshed its ledger, nothing more to say");
    }

    #[test]
    fn a_system_prompt_gets_neither_the_answered_line_nor_the_missing_report_ask_and_leaves_no_record()
     {
        let notice = r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1","prompt":"<task-notification>\n<tool-use-id>toolu_9</tool-use-id>\n<status>completed</status>\n</task-notification>"}"#;
        let compacted = r#"{"hook_event_name":"UserPromptSubmit","prompt":"<system-reminder>x</system-reminder>"}"#;
        let f = fixture();
        let token = f.register(1, "a");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
        let file = sent_file_beside(&notes_of(&f, &token));
        for system in [notice, compacted] {
            assert_eq!(
                f.send("POST", "/hook/claude", Some(&token), system).body,
                "{}",
                "no report"
            );
        }
        save_ledger(&f, &token, QUESTION);
        for system in [notice, compacted] {
            assert_eq!(
                f.send("POST", "/hook/claude", Some(&token), system).body,
                "{}",
                "a question"
            );
        }
        assert_eq!(sent::load(&file), []);
        let answer = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(context_of(&answer), prompt_answer::LEDGER_CONTEXT);
        assert_eq!(sent::load(&file).len(), 1, "the owner's own message still counts");
    }

    #[test]
    fn a_prompt_to_a_tab_whose_ledger_holds_open_items_is_answered_with_the_open_line() {
        let open =
            "# Goal\nShip\n\n## Status\nstate: working\n\n## Not verified\n- Verified live\n";
        let f = fixture();
        let token = f.register(1, "a");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
        save_ledger(&f, &token, open);
        for _ in 0..2 {
            let response = f.send("POST", "/hook/claude", Some(&token), PROMPT);
            let context = context_of(&response);
            assert!(context.contains("\"Verified live\""), "{context}");
            assert!(context.contains("update_progress"), "{context}");
            assert_eq!(context.lines().count(), 1, "{context}");
        }
        save_ledger(
            &f,
            &token,
            "# Goal\nShip\n\n## Status\nstate: working\n\n## Not verified\n- Nothing\n",
        );
        let closed = f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(closed.body, "{}", "nothing open, nothing said");
        let told = sent::load(&sent_file_beside(&notes_of(&f, &token)));
        assert_eq!(told, [], "a line that rides every prompt leaves no record");
    }

    #[test]
    fn the_service_line_rides_a_prompt_once_and_again_only_when_its_text_changes() {
        let f = fixture();
        let one = f.register(1, "a");
        let two = f.register(2, "b");
        for token in [&one, &two] {
            avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, token))).unwrap();
            save_ledger(&f, token, CALM);
        }
        let line = |text: &str| HashMap::from([(1, text.to_owned()), (2, text.to_owned())]);
        let ask = |token: &str| f.send("POST", "/hook/claude", Some(token), PROMPT);
        assert_eq!(ask(&one).body, "{}", "nothing set, nothing said");
        replace_service_lines(&f.shared, line("db is running"));
        assert_eq!(context_of(&ask(&one)), "db is running");
        assert_eq!(ask(&one).body, "{}", "already told");
        assert_eq!(context_of(&ask(&two)), "db is running", "each tab is told on its own");
        replace_service_lines(&f.shared, line("db is crashed"));
        assert_eq!(context_of(&ask(&one)), "db is crashed");
        replace_service_lines(&f.shared, HashMap::new());
        assert_eq!(ask(&one).body, "{}");
        replace_service_lines(&f.shared, line("db is crashed"));
        assert_eq!(context_of(&ask(&one)), "db is crashed", "a list that came back is news");
        let told = sent::load(&sent_file_beside(&notes_of(&f, &one)));
        assert_eq!(told, [], "the service line leaves no record");
    }

    #[test]
    fn only_the_one_off_prompt_context_leaves_a_record() {
        let f = fixture();
        let token = f.register(1, "a");
        let file = sent_file_beside(&notes_of(&f, &token));
        save_ledger(&f, &token, QUESTION);
        for _ in 0..3 {
            f.send("POST", "/hook/claude", Some(&token), PROMPT);
        }
        let told = sent::load(&file);
        let reasons: Vec<Reason> = told.iter().map(|record| record.reason).collect();
        assert_eq!(reasons, [Reason::Avatar, Reason::LedgerGuard]);
        assert_eq!(told[0].text, avatar_ask::CONTEXT);
        assert_eq!(told[0].because, because::AVATAR_ASK);
        assert_eq!(told[1].text, prompt_answer::LEDGER_CONTEXT);
        assert_eq!(told[1].because, because::LEDGER_NUDGE);
    }

    #[test]
    fn the_missing_ledger_ask_leaves_one_record_until_a_ledger_is_saved() {
        let f = fixture();
        let token = f.register(1, "a");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
        let file = sent_file_beside(&notes_of(&f, &token));
        for _ in 0..3 {
            let response = f.send("POST", "/hook/claude", Some(&token), PROMPT);
            assert_eq!(context_of(&response), prompt_answer::MISSING_LEDGER_CONTEXT);
        }
        let told = sent::load(&file);
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].reason, Reason::LedgerGuard);
        assert_eq!(told[0].text, prompt_answer::MISSING_LEDGER_CONTEXT);
        assert_eq!(told[0].because, because::LEDGER_MISSING);
    }

    #[test]
    fn a_new_stretch_of_waiting_after_a_ledger_save_leaves_one_more_nudge_record() {
        let f = fixture();
        let token = f.register(1, "a");
        avatar_store::mark_asked(&avatar_asked_file_beside(&notes_of(&f, &token))).unwrap();
        let file = sent_file_beside(&notes_of(&f, &token));
        save_ledger(&f, &token, QUESTION);
        f.send("POST", "/hook/claude", Some(&token), PROMPT);
        f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(sent::load(&file).len(), 1);
        std::thread::sleep(Duration::from_millis(30));
        save_ledger(&f, &token, "# Goal\nShip\n\n## Status\nstate: working\n");
        f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(sent::load(&file).len(), 1, "a working ledger is not nudged");
        std::thread::sleep(Duration::from_millis(30));
        save_ledger(&f, &token, QUESTION);
        f.send("POST", "/hook/claude", Some(&token), PROMPT);
        f.send("POST", "/hook/claude", Some(&token), PROMPT);
        assert_eq!(sent::load(&file).len(), 2);
    }

    #[test]
    fn the_ledger_nudge_rides_only_a_claude_prompt() {
        let f = fixture();
        let token = f.register(1, "a");
        save_ledger(&f, &token, QUESTION);
        let others = [
            ("/hook/claude", STOP),
            ("/hook/copilot", PROMPT),
            ("/hook/gemini", r#"{"hook_event_name":"BeforeAgent"}"#),
        ];
        for (path, body) in others {
            let response = f.send("POST", path, Some(&token), body);
            assert_eq!(response.body, "{}", "{path}");
        }
    }

    #[test]
    fn a_missing_or_unknown_token_is_401_on_every_route_and_method() {
        let f = fixture();
        let _valid = f.register(1, "a");
        for token in [None, Some(""), Some("nope"), Some("0000")] {
            for (method, path, body) in [
                ("POST", "/hook/claude", STOP),
                ("GET", "/hook/claude", ""),
                ("POST", "/mcp", INIT),
                ("GET", "/mcp", ""),
                ("DELETE", "/mcp", ""),
                ("POST", "/elsewhere", ""),
                ("GET", "/", ""),
            ] {
                let response = f.send(method, path, token, body);
                assert_eq!(response.status, 401, "{method} {path} token {token:?}");
                assert_eq!(response.extra, [("WWW-Authenticate", "Bearer")]);
            }
        }
        assert_eq!(f.events(), Vec::new());
        assert_eq!(f.repaints.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_revoked_token_stops_working() {
        let f = fixture();
        let token = f.register(1, "a");
        assert_eq!(f.send("POST", "/hook/claude", Some(&token), STOP).status, 200);
        f.shared.tabs.lock().unwrap().clear();
        assert_eq!(f.send("POST", "/hook/claude", Some(&token), STOP).status, 401);
    }

    #[test]
    fn bearer_headers_are_parsed_leniently_but_not_loosely() {
        for (value, want) in [
            ("Bearer abc", Some("abc")),
            ("bearer abc", Some("abc")),
            ("BEARER   abc  ", Some("abc")),
            ("Bearer", None),
            ("Bearer ", None),
            ("Basic abc", None),
            ("abc", None),
            ("", None),
        ] {
            assert_eq!(bearer_token(value), want, "{value:?}");
        }
    }

    #[test]
    fn a_hook_payload_becomes_an_event_for_the_tab_that_owns_the_token() {
        let f = fixture();
        let one = f.register(1, "a");
        let two = f.register(2, "b");
        let response = f.send("POST", "/hook/claude", Some(&two), STOP);
        assert_eq!((response.status, response.body.as_str()), (200, "{}"));
        assert_eq!(response.content_type, "application/json");
        let events = f.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tab_id, 2);
        let TabEvent::Hook(hook) = &events[0].event else {
            panic!("not a hook event: {:?}", events[0])
        };
        assert_eq!(hook.message.as_deref(), Some("done"));
        assert_eq!(hook.session_id.as_deref(), Some("s1"));
        assert_eq!(f.repaints.load(Ordering::SeqCst), 1);
        f.send("POST", "/hook/gemini", Some(&one), r#"{"hook_event_name":"BeforeAgent"}"#);
        assert_eq!(f.events()[0].tab_id, 1);
    }

    #[test]
    fn hook_payloads_without_a_status_are_acknowledged_and_dropped() {
        let f = fixture();
        let token = f.register(1, "a");
        for body in [r#"{"hook_event_name":"PreToolUse"}"#, "{}", "[1]", "7", "null"] {
            assert_eq!(f.send("POST", "/hook/claude", Some(&token), body).status, 200, "{body}");
        }
        assert_eq!(f.events(), Vec::new());
    }

    #[test]
    fn hook_routing_errors() {
        let f = fixture();
        let token = f.register(1, "a");
        let t = Some(token.as_str());
        let cases = [
            ("POST", "/hook/claude", "not json", 400),
            ("POST", "/hook/claude", "", 400),
            ("GET", "/hook/claude", "", 405),
            ("PUT", "/hook/claude", STOP, 405),
            ("POST", "/hook/", STOP, 404),
            ("POST", "/hook", STOP, 404),
            ("POST", "/hook/../mcp", STOP, 404),
            ("POST", "/hook/a/b", STOP, 404),
            ("POST", "/hook/has space", STOP, 404),
            ("POST", "/hook/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", STOP, 404),
            ("POST", "/nowhere", "", 404),
            ("POST", "/mcp/", INIT, 404),
        ];
        for (method, path, body, want) in cases {
            assert_eq!(f.send(method, path, t, body).status, want, "{method} {path}");
        }
        assert_eq!(f.send("POST", "/hook/claude?x=1", t, STOP).status, 200);
        assert_eq!(f.events().len(), 1);
    }

    #[test]
    fn mcp_get_is_405_with_allow_and_post_is_json() {
        let f = fixture();
        let token = f.register(1, "a");
        for method in ["GET", "DELETE", "PUT"] {
            let response = f.send(method, "/mcp", Some(&token), "");
            assert_eq!(response.status, 405, "{method}");
            assert_eq!(response.extra, [("Allow", "POST")]);
        }
        let init = f.send("POST", "/mcp", Some(&token), INIT);
        assert_eq!(init.status, 200);
        assert_eq!(body_json(&init)["result"]["serverInfo"]["name"], "tidewatch");
        let note = f.send(
            "POST",
            "/mcp",
            Some(&token),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert_eq!((note.status, note.body.as_str()), (202, ""));
        let garbage = f.send("POST", "/mcp", Some(&token), "}{");
        assert_eq!(garbage.status, 200);
        assert_eq!(body_json(&garbage)["error"]["code"], -32700);
    }

    fn call(tool: &str, args: &Value) -> String {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": tool, "arguments": args}})
        .to_string()
    }

    #[test]
    fn tool_calls_are_attributed_to_the_token_s_tab() {
        let f = fixture();
        let one = f.register(1, "a");
        let two = f.register(2, "b");
        f.send(
            "POST",
            "/mcp",
            Some(&one),
            &call(
                "report_finding",
                &json!({"file": "a.rs", "line": 1, "blocking": true, "text": "from one"}),
            ),
        );
        f.send(
            "POST",
            "/mcp",
            Some(&two),
            &call(
                "report_finding",
                &json!({"file": "x.rs", "line": 3, "blocking": false, "text": "t"}),
            ),
        );
        let events: Vec<_> = f.events().into_iter().map(|m| (m.tab_id, m.event)).collect();
        assert_eq!(
            events,
            [
                (
                    1,
                    TabEvent::Finding {
                        file: "a.rs".into(),
                        line: 1,
                        blocking: true,
                        text: "from one".into(),
                    },
                ),
                (
                    2,
                    TabEvent::Finding {
                        file: "x.rs".into(),
                        line: 3,
                        blocking: false,
                        text: "t".into()
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_tab_reads_and_answers_only_its_own_notes() {
        use kelp_core::review::{Anchor, Comment, Side};
        let f = fixture();
        let one = f.register(1, "a");
        let two = f.register(2, "b");
        let seed = |tab: &str, body: &str| {
            let file = f.paths.notes_file(Path::new("/work/web"), tab);
            let mut notes = Notes::load(&file);
            let anchor = Anchor {
                side: Side::New,
                line_hint: 1,
                text: String::new(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            };
            notes.add(
                "f.rs",
                None,
                anchor,
                Comment { author: "You".into(), time: 1, body: body.into() },
            );
            notes.save(&file).unwrap();
        };
        seed("a", "note for a");
        seed("b", "note for b");
        let listed = f.send("POST", "/mcp", Some(&one), &call("list_notes", &json!({})));
        let text = body_json(&listed)["result"]["content"][0]["text"].as_str().unwrap().to_owned();
        assert!(text.contains("note for a") && !text.contains("note for b"), "{text}");
        let reply = f.send(
            "POST",
            "/mcp",
            Some(&two),
            &call("reply_to_note", &json!({"id": 1, "text": "ok"})),
        );
        assert_eq!(body_json(&reply)["result"]["isError"], false);
        let a = Notes::load(&f.paths.notes_file(Path::new("/work/web"), "a"));
        let b = Notes::load(&f.paths.notes_file(Path::new("/work/web"), "b"));
        assert_eq!((a.threads[0].comments.len(), b.threads[0].comments.len()), (1, 2));
        assert_eq!(
            f.events(),
            [TabMessage {
                tab_id: 2,
                generation: 0,
                event: TabEvent::NoteReply { note_id: 1, text: "ok".into() }
            }]
        );
    }

    #[test]
    fn each_tab_keeps_its_own_ledger() {
        let f = fixture();
        let one = f.register(1, "a");
        let two = f.register(2, "b");
        let post = |token: &String, tool: &str, args: Value| {
            body_json(&f.send("POST", "/mcp", Some(token), &call(tool, &args)))
        };
        let saved = post(&one, "update_progress", json!({"ledger": "# Goal\nFor a\n"}));
        assert!(saved["result"]["content"][0]["text"].as_str().unwrap().starts_with("saved"));
        post(&two, "update_progress", json!({"ledger": "# Goal\nFor b\n"}));
        let file = f.paths.progress_file(Path::new("/work/web"), "a");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "# Goal\nFor a\n");
        let read = post(&one, "get_progress", json!({}));
        let report: Value =
            serde_json::from_str(read["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(report["goal"], "For a");
        let events = f.events();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0].event, TabEvent::Progress { ledger } if ledger.goal == "For a")
        );
        assert_eq!(events[1].tab_id, 2);
    }

    #[test]
    fn tokens_are_32_hex_characters_and_never_repeat() {
        let tokens: std::collections::HashSet<String> = (0..200).map(|_| new_token()).collect();
        assert_eq!(tokens.len(), 200);
        for token in tokens {
            assert_eq!(token.len(), TOKEN_BYTES * 2);
            assert!(token.bytes().all(|b| b.is_ascii_hexdigit()), "{token}");
        }
    }

    #[test]
    fn heads_parse_into_method_path_length_and_connection_mode() {
        let parse = |text: &str| parse_head(text.as_bytes()).ok().flatten();
        let (head, len) = parse("POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nAuthorization: Bearer t\r\n\r\nhello").unwrap();
        assert_eq!(
            (head.method.as_str(), head.path.as_str(), head.content_length),
            ("POST", "/mcp", Some(5))
        );
        assert_eq!((head.bearer.as_deref(), head.keep_alive, len), (Some("t"), true, 75));
        assert!(!parse("GET / HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap().0.keep_alive);
        assert!(!parse("GET / HTTP/1.0\r\n\r\n").unwrap().0.keep_alive);
        assert!(parse("GET / HTTP/1.0\r\nConnection: Keep-Alive\r\n\r\n").unwrap().0.keep_alive);
        assert!(parse("POST / HTTP/1.1\r\nTransfer-Encoding: Chunked\r\n\r\n").unwrap().0.chunked);
        assert!(
            parse("POST / HTTP/1.1\r\nExpect: 100-continue\r\n\r\n").unwrap().0.expect_continue
        );
        assert!(parse("GET / HTTP/1.1\r\nHost: x\r\n").is_none());
        for bad in [
            "GARBAGE\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: abc\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
            "POST / HTTP/1.1\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n",
        ] {
            assert!(matches!(parse_head(bad.as_bytes()), Err(Fail::Reply(_))), "{bad:?}");
        }
    }
}
