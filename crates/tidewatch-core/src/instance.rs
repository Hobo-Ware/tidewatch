//! One running Tidewatch window per folder, found through a Unix socket under the state dir.
//!
//! A window serves one workspace (a repo, or a folder in the Other group), so each folder gets its
//! own socket. A second `tidewatch <path>` for the same folder connects to it, hands over the
//! path and exits. `tidewatch --toggle` reaches every live window. A socket file nobody answers on
//! (a crash) is removed and taken over. Lines on the wire: `open <path>` and `toggle`.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SOCKET_EXT: &str = "sock";
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_LINE: u64 = 8192;

/// What a launch can ask of a running window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Bring the window for this folder forward.
    Open(PathBuf),
    /// Show the window if it is hidden, hide it if it is shown.
    Toggle,
}

impl Command {
    /// The line sent over the socket, newline included. `None` for a path that has a newline in it.
    #[must_use]
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut line = match self {
            Self::Open(path) => {
                let bytes = path.as_os_str().as_bytes();
                if bytes.contains(&b'\n') {
                    return None;
                }
                let mut line = b"open ".to_vec();
                line.extend_from_slice(bytes);
                line
            }
            Self::Toggle => b"toggle".to_vec(),
        };
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
        let path = line.strip_prefix(b"open ")?;
        (!path.is_empty()).then(|| Self::Open(PathBuf::from(OsStr::from_bytes(path))))
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
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The window that owns a folder's socket.
pub struct Primary {
    listener: UnixListener,
    guard: Guard,
}

impl Primary {
    /// Starts the thread that accepts launches and hands each command to `on`. Keep the returned
    /// guard for as long as the window lives.
    pub fn serve(self, mut on: impl FnMut(Command) + Send + 'static) -> Guard {
        let Self { listener, guard } = self;
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if let Some(command) = read_command(stream) {
                    on(command);
                }
            }
        });
        guard
    }
}

/// The timeout is best effort: macOS refuses to set it on a socket whose client already hung up,
/// and such a client has nothing left to wait for.
fn read_command(stream: UnixStream) -> Option<Command> {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_LINE)).read_until(b'\n', &mut line).ok()?;
    Command::parse(&line)
}

/// How the claim on a folder's socket went.
pub enum Claim {
    /// Nobody else has this folder: serve its socket.
    Primary(Primary),
    /// A window for this folder is already running (and got the command, when one was given).
    Secondary,
    /// The socket could not be used. Run without single-instance rather than not at all.
    Unavailable(std::io::Error),
}

/// Claims `path`. A live window there means `Secondary`, after sending it `hand_over` when given.
/// With none it is only probed, which the window ignores: a run that must not disturb anything
/// passes `None`. A socket file nobody answers on is removed first.
#[must_use]
pub fn claim(path: &Path, hand_over: Option<&Command>) -> Claim {
    let stream = UnixStream::connect(path);
    match decide_startup(path.exists(), stream.is_ok()) {
        Startup::Secondary => {
            if let (Ok(stream), Some(command)) = (stream, hand_over) {
                let _ = write_command(stream, command);
            }
            Claim::Secondary
        }
        Startup::Primary => bind(path, hand_over),
        Startup::TakeOverStale => {
            let _ = std::fs::remove_file(path);
            bind(path, hand_over)
        }
    }
}

fn bind(path: &Path, hand_over: Option<&Command>) -> Claim {
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Claim::Unavailable(e);
    }
    match UnixListener::bind(path) {
        Ok(listener) => {
            Claim::Primary(Primary { listener, guard: Guard { path: path.to_owned() } })
        }
        Err(first) if first.kind() == ErrorKind::AddrInUse => match UnixStream::connect(path) {
            Ok(stream) => {
                if let Some(command) = hand_over {
                    let _ = write_command(stream, command);
                }
                Claim::Secondary
            }
            Err(e) => Claim::Unavailable(e),
        },
        Err(e) => Claim::Unavailable(e),
    }
}

fn write_command(mut stream: UnixStream, command: &Command) -> std::io::Result<()> {
    let line = command
        .encode()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "path has a newline"))?;
    stream.write_all(&line)?;
    stream.flush()
}

/// Sends `command` to the window listening on `path`.
pub fn send(path: &Path, command: &Command) -> std::io::Result<()> {
    write_command(UnixStream::connect(path)?, command)
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
    }

    #[test]
    fn a_path_with_a_newline_cannot_be_sent() {
        assert_eq!(Command::Open(PathBuf::from("/work/we\nb")).encode(), None);
    }

    #[test]
    fn blank_unknown_and_pathless_lines_are_ignored() {
        for line in ["", "\n", "quit\n", "open\n", "open \n", "opens /x\n", "new-tab\n"] {
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

    #[test]
    fn a_stale_socket_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("stale.sock");
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists(), "the crash left the file behind");
        assert!(send(&socket, &Command::Toggle).is_err());

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
        drop(UnixListener::bind(&stale).unwrap());
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
}
