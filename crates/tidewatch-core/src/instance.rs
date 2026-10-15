//! One running Tidewatch window, found through a Unix socket under the state dir.
//!
//! The window holds every repo (or folder in the Other group) opened in it. A second
//! `tidewatch <path>` connects to the socket, hands over the path and exits, and the window opens
//! that folder as one more repo, or shows it when it is already open. `tidewatch --toggle` reaches
//! every live window. A socket file nobody answers on (a crash) is removed and taken over. Lines
//! on the wire: `open <path>`, `adopt-stdusk <socket>` and `toggle`, each answered with one line,
//! `ok` or `closing`.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SOCKET_EXT: &str = "sock";
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSING_POLL: Duration = Duration::from_millis(50);
const CLOSING_PATIENCE: Duration = Duration::from_secs(10);
const MAX_LINE: u64 = 8192;
const OPEN: &[u8] = b"open";
const ADOPT_STDUSK: &[u8] = b"adopt-stdusk";

/// What a launch can ask of a running window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open this folder in the window, as a repo of its own unless it is open already, and bring
    /// the window forward.
    Open(PathBuf),
    /// Take the live terminals stdusk is handing over on this socket, as shell tabs.
    AdoptStdusk(PathBuf),
    /// Show the window if it is hidden, hide it if it is shown.
    Toggle,
}

impl Command {
    /// The line sent over the socket, newline included. `None` for a path that has a newline in it.
    #[must_use]
    pub fn encode(&self) -> Option<Vec<u8>> {
        let (verb, path) = match self {
            Self::Open(path) => (OPEN, path),
            Self::AdoptStdusk(socket) => (ADOPT_STDUSK, socket),
            Self::Toggle => return Some(b"toggle\n".to_vec()),
        };
        let bytes = path.as_os_str().as_bytes();
        if bytes.contains(&b'\n') {
            return None;
        }
        let mut line = verb.to_vec();
        line.push(b' ');
        line.extend_from_slice(bytes);
        line.push(b'\n');
        Some(line)
    }

    /// One line off the socket. A blank or unknown line is `None`, so the protocol can grow.
    #[must_use]
    pub fn parse(line: &[u8]) -> Option<Self> {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.trim_ascii() == b"toggle" {
            return Some(Self::Toggle);
        }
        let path_after = |verb: &[u8]| {
            let rest = line.strip_prefix(verb)?.strip_prefix(b" ")?;
            (!rest.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(rest)))
        };
        path_after(OPEN).map(Self::Open).or_else(|| path_after(ADOPT_STDUSK).map(Self::AdoptStdusk))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Accepted,
    Closing,
}

impl Reply {
    fn line(self) -> &'static [u8] {
        match self {
            Self::Accepted => b"ok\n",
            Self::Closing => b"closing\n",
        }
    }

    /// Anything but `closing` counts as accepted: a window of an older build says nothing at all.
    fn parse(line: &[u8]) -> Self {
        if line.trim_ascii() == b"closing" { Self::Closing } else { Self::Accepted }
    }
}

/// What to do at startup, from whether the socket file exists and whether anything answered on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Startup {
    Primary,
    Secondary,
    TakeOverStale,
}

fn decide_startup(file_exists: bool, answered: bool) -> Startup {
    match (file_exists, answered) {
        (_, true) => Startup::Secondary,
        (false, false) => Startup::Primary,
        (true, false) => Startup::TakeOverStale,
    }
}

/// Removes the socket file when the window shuts down.
pub struct Guard {
    path: PathBuf,
    closing: Arc<AtomicBool>,
}

impl Guard {
    /// From now on the window answers `closing` to launches and passes nothing on. Set it when the
    /// window is about to close for good, and keep the guard until the session is saved: the
    /// launches that hear it wait for the socket file to go.
    pub fn set_closing(&self, closing: bool) {
        self.closing.store(closing, Ordering::SeqCst);
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The window that owns the socket.
pub struct Primary {
    listener: UnixListener,
    guard: Guard,
}

impl Primary {
    /// Starts the thread that accepts launches and hands each command to `on`. Keep the returned
    /// guard for as long as the window lives.
    pub fn serve(self, mut on: impl FnMut(Command) + Send + 'static) -> Guard {
        let Self { listener, guard } = self;
        let closing = Arc::clone(&guard.closing);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let Some(command) = read_command(&stream) else { continue };
                let reply =
                    if closing.load(Ordering::SeqCst) { Reply::Closing } else { Reply::Accepted };
                if reply == Reply::Accepted {
                    on(command);
                }
                let _ = (&stream).write_all(reply.line());
            }
        });
        guard
    }
}

/// The timeout is best effort: macOS refuses to set it on a socket whose client already hung up,
/// and such a client has nothing left to wait for.
fn read_command(stream: &UnixStream) -> Option<Command> {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_LINE)).read_until(b'\n', &mut line).ok()?;
    Command::parse(&line)
}

/// How the claim on the socket went.
pub enum Claim {
    /// No other window is running: serve the socket.
    Primary(Primary),
    /// A window is already running (and got the command, when one was given).
    Secondary,
    /// The socket could not be used. Run without single-instance rather than not at all.
    Unavailable(std::io::Error),
}

/// Claims `path`. A live window there means `Secondary`, after sending it `hand_over` when given.
/// With none it is only probed, which the window ignores: a run that must not disturb anything
/// passes `None`. A socket file nobody answers on is removed first. A window that answers
/// `closing` is waited for (up to ten seconds) and then replaced.
#[must_use]
pub fn claim(path: &Path, hand_over: Option<&Command>) -> Claim {
    claim_within(path, hand_over, CLOSING_PATIENCE)
}

fn claim_within(path: &Path, hand_over: Option<&Command>, patience: Duration) -> Claim {
    let deadline = Instant::now() + patience;
    loop {
        if let Some(claim) = attempt(path, hand_over) {
            return claim;
        }
        if Instant::now() >= deadline {
            let gone =
                std::io::Error::new(ErrorKind::TimedOut, "the window did not finish closing");
            return Claim::Unavailable(gone);
        }
        std::thread::sleep(CLOSING_POLL);
    }
}

/// Claims `path` for a window that takes over from the one serving it, which is leaving once it has
/// handed everything over. Nothing is sent to that window, so it never receives a folder it is about
/// to lose; this waits (up to ten seconds) until it no longer answers, then serves the socket.
#[must_use]
pub fn claim_replacing(path: &Path) -> Claim {
    claim_replacing_within(path, CLOSING_PATIENCE)
}

fn claim_replacing_within(path: &Path, patience: Duration) -> Claim {
    let deadline = Instant::now() + patience;
    loop {
        let answered = UnixStream::connect(path).is_ok();
        match decide_startup(path.exists(), answered) {
            Startup::Secondary => {}
            Startup::Primary => {
                if let Some(claim) = bind(path, None) {
                    return claim;
                }
            }
            Startup::TakeOverStale => {
                let _ = std::fs::remove_file(path);
                if let Some(claim) = bind(path, None) {
                    return claim;
                }
            }
        }
        if Instant::now() >= deadline {
            let stayed =
                std::io::Error::new(ErrorKind::TimedOut, "the window did not stop answering");
            return Claim::Unavailable(stayed);
        }
        std::thread::sleep(CLOSING_POLL);
    }
}

/// `None` when the window there is closing.
fn attempt(path: &Path, hand_over: Option<&Command>) -> Option<Claim> {
    let stream = UnixStream::connect(path);
    match decide_startup(path.exists(), stream.is_ok()) {
        Startup::Secondary => {
            stream.ok().map_or(Some(Claim::Secondary), |stream| secondary(&stream, hand_over))
        }
        Startup::Primary => bind(path, hand_over),
        Startup::TakeOverStale => {
            let _ = std::fs::remove_file(path);
            bind(path, hand_over)
        }
    }
}

fn secondary(stream: &UnixStream, hand_over: Option<&Command>) -> Option<Claim> {
    let Some(command) = hand_over else { return Some(Claim::Secondary) };
    match write_command(stream, command).map(|()| read_reply(stream)) {
        Ok(Reply::Closing) => None,
        _ => Some(Claim::Secondary),
    }
}

fn bind(path: &Path, hand_over: Option<&Command>) -> Option<Claim> {
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Some(Claim::Unavailable(e));
    }
    match UnixListener::bind(path) {
        Ok(listener) => Some(Claim::Primary(Primary {
            listener,
            guard: Guard { path: path.to_owned(), closing: Arc::new(AtomicBool::new(false)) },
        })),
        Err(first) if first.kind() == ErrorKind::AddrInUse => match UnixStream::connect(path) {
            Ok(stream) => secondary(&stream, hand_over),
            Err(e) => Some(Claim::Unavailable(e)),
        },
        Err(e) => Some(Claim::Unavailable(e)),
    }
}

fn write_command(mut stream: &UnixStream, command: &Command) -> std::io::Result<()> {
    let line = command
        .encode()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "path has a newline"))?;
    stream.write_all(&line)?;
    stream.flush()
}

fn read_reply(stream: &UnixStream) -> Reply {
    let _ = stream.set_read_timeout(Some(REPLY_TIMEOUT));
    let mut line = Vec::new();
    let _ = BufReader::new(stream.take(MAX_LINE)).read_until(b'\n', &mut line);
    Reply::parse(&line)
}

/// Sends `command` to the window listening on `path`.
pub fn send(path: &Path, command: &Command) -> std::io::Result<()> {
    write_command(&UnixStream::connect(path)?, command)
}

/// Sends a toggle to every live window under `dir`, except the one on `skip`. A socket file
/// nobody answers on is removed on the way. Returns how many windows were reached.
#[must_use]
pub fn toggle_all(dir: &Path, skip: Option<&Path>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut reached = 0;
    for path in entries.filter_map(|entry| Some(entry.ok()?.path())) {
        if path.extension() != Some(OsStr::new(SOCKET_EXT)) || skip == Some(path.as_path()) {
            continue;
        }
        match send(&path, &Command::Toggle) {
            Ok(()) => reached += 1,
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                let _ = std::fs::remove_file(&path);
            }
            Err(_) => {}
        }
    }
    reached
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    fn serve(primary: Primary) -> (Guard, Receiver<Command>) {
        let (tx, rx) = channel();
        let guard = primary.serve(move |command| {
            let _ = tx.send(command);
        });
        (guard, rx)
    }

    fn primary(claimed: Claim) -> Primary {
        match claimed {
            Claim::Primary(primary) => primary,
            Claim::Secondary => panic!("expected to be the primary, a window answered"),
            Claim::Unavailable(e) => panic!("expected to be the primary: {e}"),
        }
    }

    fn next(rx: &Receiver<Command>) -> Option<Command> {
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }

    fn silent(rx: &Receiver<Command>) -> bool {
        rx.recv_timeout(Duration::from_millis(200)).is_err()
    }

    #[test]
    fn commands_survive_the_wire_including_spaces_and_unicode() {
        let paths = ["/work/web", "/Users/me/My Projects/web app", "/work/prøjekt-ü", "rel/ative"];
        for path in paths {
            let command = Command::Open(PathBuf::from(path));
            let line = command.encode().unwrap();
            assert_eq!(Command::parse(&line), Some(command), "{path}");
        }
        assert_eq!(Command::parse(&Command::Toggle.encode().unwrap()), Some(Command::Toggle));
        let adopt = Command::AdoptStdusk(PathBuf::from("/tmp/My Dir/stdusk-7.sock"));
        assert_eq!(adopt.encode().unwrap(), b"adopt-stdusk /tmp/My Dir/stdusk-7.sock\n");
        assert_eq!(Command::parse(&adopt.encode().unwrap()), Some(adopt));
    }

    #[test]
    fn a_path_with_a_newline_cannot_be_sent() {
        assert_eq!(Command::Open(PathBuf::from("/work/we\nb")).encode(), None);
        assert_eq!(Command::AdoptStdusk(PathBuf::from("/s\n.sock")).encode(), None);
    }

    #[test]
    fn blank_unknown_and_pathless_lines_are_ignored() {
        let lines = [
            "",
            "\n",
            "quit\n",
            "open\n",
            "open \n",
            "opens /x\n",
            "new-tab\n",
            "adopt-stdusk\n",
            "adopt-stdusk \n",
            "adopt-stdusks /x\n",
        ];
        for line in lines {
            assert_eq!(Command::parse(line.as_bytes()), None, "{line:?}");
        }
    }

    #[test]
    fn line_endings_and_padding_around_a_toggle_do_not_matter_and_a_path_keeps_its_spaces() {
        assert_eq!(Command::parse(b"toggle\r\n"), Some(Command::Toggle));
        assert_eq!(Command::parse(b"  toggle  \n"), Some(Command::Toggle));
        assert_eq!(
            Command::parse(b"open /a b/ \r\n"),
            Some(Command::Open(PathBuf::from("/a b/ ")))
        );
    }

    #[test]
    fn startup_decision_table() {
        assert_eq!(decide_startup(true, true), Startup::Secondary);
        assert_eq!(decide_startup(false, true), Startup::Secondary);
        assert_eq!(decide_startup(false, false), Startup::Primary);
        assert_eq!(decide_startup(true, false), Startup::TakeOverStale);
    }

    #[test]
    fn a_second_launch_hands_its_path_to_the_first_and_a_toggle_reaches_it() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (_guard, rx) = serve(primary(claim(&socket, None)));

        let handed = Command::Open(PathBuf::from("/work/My Repo"));
        assert!(matches!(claim(&socket, Some(&handed)), Claim::Secondary));
        assert_eq!(next(&rx), Some(handed));

        send(&socket, &Command::Toggle).unwrap();
        assert_eq!(next(&rx), Some(Command::Toggle));
        assert!(silent(&rx), "each command arrives once");
    }

    #[test]
    fn probing_without_a_command_finds_the_window_and_tells_it_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (_guard, rx) = serve(primary(claim(&socket, None)));
        assert!(matches!(claim(&socket, None), Claim::Secondary));
        assert!(silent(&rx));
        send(&socket, &Command::Toggle).unwrap();
        assert_eq!(next(&rx), Some(Command::Toggle), "the probe did not wedge the accept loop");
    }

    /// A test thread that forks before the listener is dropped holds a copy of it until its exec.
    fn leave_stale_socket(socket: &Path) {
        drop(UnixListener::bind(socket).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while send(socket, &Command::Toggle).is_ok() {
            assert!(std::time::Instant::now() < deadline, "the dropped listener still answers");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stale_socket_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("stale.sock");
        leave_stale_socket(&socket);
        assert!(socket.exists(), "the crash left the file behind");

        let (_guard, rx) = serve(primary(claim(&socket, Some(&Command::Toggle))));
        assert!(silent(&rx), "a stale socket gets no command");
        send(&socket, &Command::Toggle).unwrap();
        assert_eq!(next(&rx), Some(Command::Toggle));
    }

    #[test]
    fn the_guard_removes_the_socket_when_the_window_goes() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("nested").join("a.sock");
        let guard = primary(claim(&socket, None)).serve(|_| {});
        assert!(socket.exists(), "the folder was created too");
        drop(guard);
        assert!(!socket.exists());
    }

    #[test]
    fn an_unusable_path_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("x".repeat(200)).join("a.sock");
        assert!(matches!(claim(&socket, None), Claim::Unavailable(_)));
    }

    #[test]
    fn toggle_all_reaches_every_live_window_but_the_skipped_one_and_clears_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, own, stale) = (
            dir.path().join("a.sock"),
            dir.path().join("b.sock"),
            dir.path().join("own.sock"),
            dir.path().join("stale.sock"),
        );
        let (_ga, rx_a) = serve(primary(claim(&a, None)));
        let (_gb, rx_b) = serve(primary(claim(&b, None)));
        let (_go, rx_own) = serve(primary(claim(&own, None)));
        leave_stale_socket(&stale);
        std::fs::write(dir.path().join("notes.txt"), "not a socket").unwrap();

        assert_eq!(toggle_all(dir.path(), Some(&own)), 2);
        assert_eq!(next(&rx_a), Some(Command::Toggle));
        assert_eq!(next(&rx_b), Some(Command::Toggle));
        assert!(silent(&rx_own));
        assert!(!stale.exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn toggling_with_nothing_running_reaches_nobody() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(toggle_all(dir.path(), None), 0);
        assert_eq!(toggle_all(&dir.path().join("missing"), None), 0);
    }

    #[test]
    fn a_throwaway_home_never_talks_to_the_real_one() {
        let real = tempfile::tempdir().unwrap();
        let throwaway = tempfile::tempdir().unwrap();
        let (_guard, rx) = serve(primary(claim(&real.path().join("a.sock"), None)));

        assert_eq!(toggle_all(throwaway.path(), None), 0);
        assert!(silent(&rx));
        let own = primary(claim(&throwaway.path().join("a.sock"), Some(&Command::Toggle)));
        let (_own_guard, own_rx) = serve(own);
        assert!(silent(&rx) && silent(&own_rx));
    }

    #[test]
    fn a_silent_client_does_not_hold_up_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (_guard, rx) = serve(primary(claim(&socket, None)));
        let _holding = UnixStream::connect(&socket).unwrap();
        let started = Instant::now();
        send(&socket, &Command::Toggle).unwrap();
        assert_eq!(next(&rx), Some(Command::Toggle));
        assert!(started.elapsed() < Duration::from_secs(2) + READ_TIMEOUT);
    }

    fn ask(socket: &Path, command: &Command) -> Vec<u8> {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream.write_all(&command.encode().unwrap()).unwrap();
        let mut reply = Vec::new();
        BufReader::new(&mut stream).read_until(b'\n', &mut reply).unwrap();
        reply
    }

    #[test]
    fn replies_are_one_line_and_only_closing_means_closing() {
        assert_eq!(Reply::parse(Reply::Accepted.line()), Reply::Accepted);
        assert_eq!(Reply::parse(Reply::Closing.line()), Reply::Closing);
        assert_eq!(Reply::parse(b"closing\r\n"), Reply::Closing);
        for line in ["", "\n", "ok\n", "open\n", "closing soon\n"] {
            assert_eq!(Reply::parse(line.as_bytes()), Reply::Accepted, "{line:?}");
        }
    }

    #[test]
    fn a_window_answers_ok_while_open_and_closing_once_it_is_closing_and_passes_nothing_on() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, rx) = serve(primary(claim(&socket, None)));
        let open = Command::Open(PathBuf::from("/work/web"));

        assert_eq!(ask(&socket, &open), b"ok\n");
        assert_eq!(next(&rx), Some(open.clone()));

        guard.set_closing(true);
        assert_eq!(ask(&socket, &open), b"closing\n");
        assert_eq!(ask(&socket, &Command::Toggle), b"closing\n");
        assert!(silent(&rx), "a closing window acts on nothing");

        guard.set_closing(false);
        assert_eq!(ask(&socket, &open), b"ok\n");
        assert_eq!(next(&rx), Some(open));
    }

    #[test]
    fn the_stdusk_handover_is_answered_like_the_other_commands() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, rx) = serve(primary(claim(&socket, None)));
        let adopt = Command::AdoptStdusk(PathBuf::from("/tmp/stdusk-1.sock"));

        assert_eq!(ask(&socket, &adopt), b"ok\n");
        assert_eq!(next(&rx), Some(adopt.clone()));

        guard.set_closing(true);
        assert_eq!(ask(&socket, &adopt), b"closing\n");
        assert!(silent(&rx), "a closing window takes nothing from stdusk");
    }

    #[test]
    fn a_launch_beside_a_closing_window_waits_for_the_socket_and_takes_it_over() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, rx) = serve(primary(claim(&socket, None)));
        guard.set_closing(true);
        let leaving = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(guard);
        });

        let started = Instant::now();
        let handed = Command::Open(PathBuf::from("/work/web"));
        let (_taken, taken_rx) =
            serve(primary(claim_within(&socket, Some(&handed), Duration::from_secs(10))));
        leaving.join().unwrap();

        assert!(started.elapsed() >= Duration::from_millis(200), "it waited for the old window");
        assert!(silent(&rx), "the closing window was told nothing it would act on");
        assert!(silent(&taken_rx), "the new window opens its own folder, nothing was sent to it");
    }

    #[test]
    fn a_launch_gives_up_on_a_window_that_never_finishes_closing() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, _rx) = serve(primary(claim(&socket, None)));
        guard.set_closing(true);

        let claimed = claim_within(&socket, Some(&Command::Toggle), Duration::from_millis(200));
        assert!(matches!(claimed, Claim::Unavailable(e) if e.kind() == ErrorKind::TimedOut));
    }

    #[test]
    fn a_successor_waits_for_the_old_window_to_go_and_sends_it_nothing_even_when_it_is_not_closing()
    {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, rx) = serve(primary(claim(&socket, None)));
        let leaving = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(guard);
        });

        let started = Instant::now();
        let (_taken, taken_rx) =
            serve(primary(claim_replacing_within(&socket, Duration::from_secs(10))));
        leaving.join().unwrap();

        assert!(started.elapsed() >= Duration::from_millis(200), "it waited for the old window");
        assert!(silent(&rx), "the old window was sent nothing");
        assert!(silent(&taken_rx));
    }

    #[test]
    fn a_successor_gives_up_on_a_window_that_never_goes() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (_guard, rx) = serve(primary(claim(&socket, None)));
        let claimed = claim_replacing_within(&socket, Duration::from_millis(200));
        assert!(matches!(claimed, Claim::Unavailable(e) if e.kind() == ErrorKind::TimedOut));
        assert!(silent(&rx));
    }

    #[test]
    fn a_successor_takes_a_free_path_and_a_stale_socket_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let free = dir.path().join("free.sock");
        assert!(matches!(claim_replacing_within(&free, Duration::ZERO), Claim::Primary(_)));

        let stale = dir.path().join("stale.sock");
        leave_stale_socket(&stale);
        assert!(stale.exists());
        assert!(matches!(claim_replacing_within(&stale, Duration::ZERO), Claim::Primary(_)));
    }

    #[test]
    fn a_probe_beside_a_closing_window_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (guard, _rx) = serve(primary(claim(&socket, None)));
        guard.set_closing(true);
        let started = Instant::now();
        assert!(matches!(claim(&socket, None), Claim::Secondary));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_window_of_an_older_build_that_never_replies_counts_as_having_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let old = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_command(&stream)
        });
        let handed = Command::Open(PathBuf::from("/work/web"));
        assert!(matches!(claim(&socket, Some(&handed)), Claim::Secondary));
        assert_eq!(old.join().unwrap(), Some(handed));
    }
}
