//! The command line: `tidewatch [--no-restore] [path]`. The path is the first argument that is not
//! the flag; with none, the current folder.

use std::ffi::OsString;
use std::path::PathBuf;

const NO_RESTORE: &str = "--no-restore";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) path: Option<PathBuf>,
    /// Start with just the main tab, whatever the last session had.
    pub(crate) no_restore: bool,
}

/// Everything after the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Args {
    let mut parsed = Args::default();
    for arg in args {
        if arg == NO_RESTORE {
            parsed.no_restore = true;
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
        assert_eq!(args(&[]), Args { path: None, no_restore: false });
    }

    #[test]
    fn the_path_is_the_first_argument_that_is_not_the_flag_wherever_the_flag_is() {
        let want = Args { path: Some(PathBuf::from("/work/web")), no_restore: true };
        assert_eq!(args(&["--no-restore", "/work/web"]), want);
        assert_eq!(args(&["/work/web", "--no-restore"]), want);
    }

    #[test]
    fn the_flag_alone_leaves_the_path_unset_and_a_second_path_is_ignored() {
        assert_eq!(args(&["--no-restore"]), Args { path: None, no_restore: true });
        assert_eq!(args(&["/a", "/b"]).path, Some(PathBuf::from("/a")));
    }
}
