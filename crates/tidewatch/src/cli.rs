//! The command line: `tidewatch [--no-restore] [path]` and `tidewatch --toggle`. The path is the
//! first argument that is not a flag; with none, the current folder. `--handoff <socket>` is how a
//! window that is restarting launches its replacement: the replacement takes the window's terminals
//! over that socket. `--from-stdusk <socket>` is how stdusk launches Tidewatch to take its live
//! panes as shell tabs.

use std::ffi::OsString;
use std::path::PathBuf;

const NO_RESTORE: &str = "--no-restore";
const TOGGLE: &str = "--toggle";
const HANDOFF: &str = "--handoff";
const FROM_STDUSK: &str = "--from-stdusk";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) path: Option<PathBuf>,
    /// Start with just the main tab, whatever the last session had.
    pub(crate) no_restore: bool,
    /// Show or hide the running windows and exit, without opening one. For desktops with no global
    /// hotkey (Wayland) and for scripts.
    pub(crate) toggle: bool,
    /// The socket of the window this one replaces.
    pub(crate) handoff: Option<PathBuf>,
    /// The socket of the stdusk window that hands its panes over.
    pub(crate) from_stdusk: Option<PathBuf>,
}

/// Everything after the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Args {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == HANDOFF {
            parsed.handoff = args.next().map(PathBuf::from);
        } else if arg == FROM_STDUSK {
            parsed.from_stdusk = args.next().map(PathBuf::from);
        } else if arg == NO_RESTORE {
            parsed.no_restore = true;
        } else if arg == TOGGLE {
            parsed.toggle = true;
        } else if parsed.path.is_none() {
            parsed.path = Some(PathBuf::from(arg));
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        parse(list.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_means_the_current_folder_and_restoring() {
        assert_eq!(args(&[]), Args::default());
    }

    #[test]
    fn the_path_is_the_first_argument_that_is_not_the_flag_wherever_the_flag_is() {
        let want =
            Args { path: Some(PathBuf::from("/work/web")), no_restore: true, ..Args::default() };
        assert_eq!(args(&["--no-restore", "/work/web"]), want);
        assert_eq!(args(&["/work/web", "--no-restore"]), want);
    }

    #[test]
    fn the_flag_alone_leaves_the_path_unset_and_a_second_path_is_ignored() {
        assert_eq!(args(&["--no-restore"]), Args { no_restore: true, ..Args::default() });
        assert_eq!(args(&["/a", "/b"]).path, Some(PathBuf::from("/a")));
    }

    #[test]
    fn toggle_is_a_flag_wherever_it_stands_and_is_never_taken_for_the_path() {
        let want = Args { toggle: true, ..Args::default() };
        assert_eq!(args(&["--toggle"]), want);
        let with_path = args(&["--toggle", "/work/web"]);
        assert!(with_path.toggle);
        assert_eq!(with_path.path, Some(PathBuf::from("/work/web")));
        assert!(args(&["/work/web", "--toggle"]).toggle);
    }

    #[test]
    fn handoff_takes_the_next_argument_as_the_socket_and_never_as_the_path() {
        let got = args(&["--handoff", "/state/handoff-7.sock"]);
        assert_eq!(got.handoff, Some(PathBuf::from("/state/handoff-7.sock")));
        assert_eq!(got.path, None);
        let with_path = args(&["/work/web", "--handoff", "/s.sock", "--no-restore"]);
        assert_eq!(with_path.path, Some(PathBuf::from("/work/web")));
        assert_eq!(with_path.handoff, Some(PathBuf::from("/s.sock")));
        assert!(with_path.no_restore);
        assert_eq!(args(&["--handoff"]).handoff, None);
    }

    #[test]
    fn from_stdusk_takes_the_next_argument_as_the_socket_and_never_as_the_path() {
        let got = args(&["--from-stdusk", "/tmp/stdusk-4.sock"]);
        assert_eq!(got.from_stdusk, Some(PathBuf::from("/tmp/stdusk-4.sock")));
        assert_eq!((got.path, got.handoff), (None, None));
        let with_path = args(&["/work/web", "--from-stdusk", "/s.sock", "--no-restore"]);
        assert_eq!(with_path.path, Some(PathBuf::from("/work/web")));
        assert_eq!(with_path.from_stdusk, Some(PathBuf::from("/s.sock")));
        assert!(with_path.no_restore);
        assert_eq!(args(&["--from-stdusk"]).from_stdusk, None);
    }
}
