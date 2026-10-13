//! Startup, before any window exists: a second `tidewatch <path>` for a folder that already has a
//! window hands over its path and exits, and `tidewatch --toggle` reaches the running windows.
//!
//! One process serves one folder, so the claim is per folder. A different folder starts its own
//! process and window: that is today's model, and moving several workspaces into one window is a
//! larger change (PLAN.md, Deviations log).

use std::path::{Path, PathBuf};

use tidewatch_core::instance::{self, Claim, Command, Primary};
use tidewatch_core::paths::TidewatchPaths;

/// How this process takes part in single-instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    /// Hand the path to a running window and exit; otherwise serve the folder's socket.
    HandOver,
    /// Serve the socket when it is free; when a window is already there, say nothing to it and run
    /// on without one. Background runs use this, so they never make a real window come forward.
    Quiet,
}

/// What kind of run this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Run {
    Screenshot,
    Background { isolated: bool },
    Foreground,
}

/// Screenshot runs take no part. A background run does only inside a throwaway `TIDEWATCH_HOME`,
/// because in the real one it would shadow the owner's own window for that folder.
pub(crate) fn mode(run: Run, has_socket: bool) -> Mode {
    match run {
        _ if !has_socket => Mode::Off,
        Run::Screenshot | Run::Background { isolated: false } => Mode::Off,
        Run::Background { isolated: true } => Mode::Quiet,
        Run::Foreground => Mode::HandOver,
    }
}

pub(crate) enum Start {
    /// Carry on and open the window, serving the socket when there is one.
    Run(Option<Primary>),
    /// A running window has the folder now; exit without opening one.
    Exit,
}

pub(crate) fn begin(mode: Mode, socket: &Path, dir: &Path) -> Start {
    let hand_over = Command::Open(dir.to_path_buf());
    let claim = match mode {
        Mode::Off => return Start::Run(None),
        Mode::HandOver => instance::claim(socket, Some(&hand_over)),
        Mode::Quiet => instance::claim(socket, None),
    };
    match (claim, mode) {
        (Claim::Primary(primary), _) => Start::Run(Some(primary)),
        (Claim::Secondary, Mode::HandOver) => Start::Exit,
        (Claim::Secondary, _) => Start::Run(None),
        (Claim::Unavailable(reason), _) => {
            eprintln!("tidewatch: no single-instance socket ({reason}); opening a window anyway");
            Start::Run(None)
        }
    }
}

/// The folder a launch is for, as the socket names it.
pub(crate) fn workspace_key(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// `tidewatch --toggle`: the exit code. 1 when no window answered, so a script can tell.
pub(crate) fn toggle_running(paths: Option<&TidewatchPaths>) -> i32 {
    let reached = paths.map_or(0, |paths| instance::toggle_all(&paths.instance_dir(), None));
    if reached == 0 {
        eprintln!("tidewatch: no running window to toggle");
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    fn short_home() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap()
    }

    fn serve(primary: Primary) -> (instance::Guard, Receiver<Command>) {
        let (tx, rx) = channel();
        let guard = primary.serve(move |command| {
            let _ = tx.send(command);
        });
        (guard, rx)
    }

    fn primary(start: Start) -> Primary {
        match start {
            Start::Run(Some(primary)) => primary,
            Start::Run(None) => panic!("expected to serve the socket, got none"),
            Start::Exit => panic!("expected to serve the socket, a window answered"),
        }
    }

    #[test]
    fn the_mode_table() {
        let isolated = Run::Background { isolated: true };
        let real_home = Run::Background { isolated: false };
        let cases = [
            ((Run::Foreground, true), Mode::HandOver),
            ((isolated, true), Mode::Quiet),
            ((real_home, true), Mode::Off),
            ((Run::Screenshot, true), Mode::Off),
            ((Run::Foreground, false), Mode::Off),
            ((isolated, false), Mode::Off),
        ];
        for ((run, has_socket), want) in cases {
            assert_eq!(mode(run, has_socket), want, "{run:?} socket={has_socket}");
        }
    }

    #[test]
    fn the_second_launch_for_a_folder_hands_over_its_path_and_exits_without_a_window() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let repo = Path::new("/work/My Repo");
        let (_guard, rx) = serve(primary(begin(Mode::HandOver, &socket, repo)));

        assert!(matches!(begin(Mode::HandOver, &socket, repo), Start::Exit));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Command::Open(repo.to_path_buf())));
    }

    #[test]
    fn the_restart_launch_beside_a_window_that_is_still_closing_ends_up_with_a_window() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let repo = Path::new("/work/web");
        let (old, old_rx) = serve(primary(begin(Mode::HandOver, &socket, repo)));
        old.set_closing(true);
        let closed = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(old);
        });

        let new = begin(Mode::HandOver, &socket, repo);
        closed.join().unwrap();

        assert!(matches!(new, Start::Run(Some(_))), "the new copy opens its own window");
        assert!(
            old_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "the old one acted on nothing"
        );
    }

    #[test]
    fn a_background_run_beside_a_live_window_neither_exits_nor_disturbs_it() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        let (_guard, rx) = serve(primary(begin(Mode::HandOver, &socket, Path::new("/work/web"))));

        assert!(matches!(begin(Mode::Quiet, &socket, Path::new("/work/web")), Start::Run(None)));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "nothing was sent");
    }

    #[test]
    fn a_stale_socket_from_a_crash_is_taken_over_by_the_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("a.sock");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());
        let (_guard, rx) = serve(primary(begin(Mode::HandOver, &socket, Path::new("/work/web"))));
        assert!(matches!(begin(Mode::HandOver, &socket, Path::new("/work/web")), Start::Exit));
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn off_never_touches_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("sub").join("a.sock");
        assert!(matches!(begin(Mode::Off, &socket, Path::new("/work/web")), Start::Run(None)));
        assert!(!socket.parent().unwrap().exists());
    }

    #[test]
    fn a_socket_that_cannot_be_made_runs_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("x".repeat(200)).join("a.sock");
        assert!(matches!(begin(Mode::HandOver, &socket, Path::new("/w")), Start::Run(None)));
    }

    #[test]
    fn two_homes_never_share_a_window() {
        let real = short_home();
        let throwaway = short_home();
        let repo = Path::new("/work/web");
        let (real_paths, other_paths) =
            (TidewatchPaths::new(real.path()), TidewatchPaths::new(throwaway.path()));
        let (_guard, rx) =
            serve(primary(begin(Mode::HandOver, &real_paths.instance_socket(repo), repo)));

        let other = begin(Mode::HandOver, &other_paths.instance_socket(repo), repo);
        let (_other_guard, other_rx) = serve(primary(other));
        assert_eq!(toggle_running(Some(&other_paths)), 0);
        assert_eq!(other_rx.recv_timeout(Duration::from_secs(2)), Ok(Command::Toggle));
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "the real window heard nothing"
        );
    }

    #[test]
    fn toggle_with_no_window_running_exits_nonzero_and_with_one_exits_zero() {
        let home = short_home();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(toggle_running(Some(&paths)), 1);
        assert_eq!(toggle_running(None), 1);
        let repo = Path::new("/work/web");
        let (_guard, rx) =
            serve(primary(begin(Mode::HandOver, &paths.instance_socket(repo), repo)));
        assert_eq!(toggle_running(Some(&paths)), 0);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Command::Toggle));
    }

    #[test]
    fn the_workspace_key_resolves_links_and_keeps_a_missing_folder_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(workspace_key(&link), workspace_key(&real));
        assert_eq!(workspace_key(Path::new("/no/such/folder")), PathBuf::from("/no/such/folder"));
    }

    #[test]
    fn a_usual_home_leaves_room_in_a_unix_socket_path_but_a_deep_one_does_not() {
        let socket =
            |home: &str| TidewatchPaths::new(Path::new(home)).instance_socket(Path::new("/w"));
        assert!(socket("/Users/a-rather-long-user-name").as_os_str().len() < 104);
        assert!(socket("/home/someone").as_os_str().len() < 104);
        let deep = socket(&format!("/var/folders/{}/T/.tmpAbCdEf", "x".repeat(40)));
        assert!(deep.as_os_str().len() >= 104, "a throwaway home under $TMPDIR is too long");
    }
}
