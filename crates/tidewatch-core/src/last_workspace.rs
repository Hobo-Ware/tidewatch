//! The folder of the window used last, in `.state/last-workspace.toml`, and the choice of which
//! folder a launch opens. A launch with no path and no repo around it (the Finder, or `open -n`
//! after an update, both start in `/`) reopens that folder, so session restore finds its tabs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

#[derive(Serialize, Deserialize)]
struct Stored {
    dir: PathBuf,
}

/// A path on the command line wins, then a working folder inside a git repo, then the remembered
/// folder while it still exists, then the working folder as it is.
pub fn pick(
    arg: Option<PathBuf>,
    cwd: Option<PathBuf>,
    remembered: Option<PathBuf>,
    in_repo: impl FnOnce(&Path) -> bool,
    is_dir: impl FnOnce(&Path) -> bool,
) -> PathBuf {
    if let Some(arg) = arg {
        return arg;
    }
    if let Some(cwd) = cwd.as_deref().filter(|cwd| in_repo(cwd)) {
        return cwd.to_path_buf();
    }
    if let Some(remembered) = remembered.filter(|dir| is_dir(dir)) {
        return remembered;
    }
    cwd.unwrap_or_else(|| PathBuf::from("."))
}

/// A missing or unreadable file is no folder: a bad state file must never stop the app.
#[must_use]
pub fn load(paths: &TidewatchPaths) -> Option<PathBuf> {
    let text = std::fs::read_to_string(paths.last_workspace_file()).ok()?;
    match toml::from_str::<Stored>(&text) {
        Ok(stored) => Some(stored.dir),
        Err(e) => {
            eprintln!("tidewatch: last-workspace.toml is unreadable ({e}); ignoring it");
            None
        }
    }
}

pub fn save(paths: &TidewatchPaths, dir: &Path) -> std::io::Result<()> {
    let text =
        toml::to_string(&Stored { dir: dir.to_path_buf() }).map_err(std::io::Error::other)?;
    write_atomic(&paths.last_workspace_file(), &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick_with(
        arg: Option<&str>,
        cwd: Option<&str>,
        remembered: Option<&str>,
        cwd_in_repo: bool,
        remembered_exists: bool,
    ) -> PathBuf {
        pick(
            arg.map(PathBuf::from),
            cwd.map(PathBuf::from),
            remembered.map(PathBuf::from),
            |_| cwd_in_repo,
            |_| remembered_exists,
        )
    }

    #[test]
    fn a_path_on_the_command_line_always_wins() {
        let got = pick_with(Some("/work/api"), Some("/work/web"), Some("/work/old"), true, true);
        assert_eq!(got, PathBuf::from("/work/api"));
        let got = pick_with(Some("/work/api"), Some("/"), Some("/work/old"), false, true);
        assert_eq!(got, PathBuf::from("/work/api"));
    }

    #[test]
    fn a_working_folder_inside_a_repo_beats_the_remembered_one() {
        let got = pick_with(None, Some("/work/web/src"), Some("/work/old"), true, true);
        assert_eq!(got, PathBuf::from("/work/web/src"));
    }

    #[test]
    fn a_launch_from_the_root_folder_reopens_the_remembered_folder() {
        let got = pick_with(None, Some("/"), Some("/work/old"), false, true);
        assert_eq!(got, PathBuf::from("/work/old"));
    }

    #[test]
    fn a_remembered_folder_that_is_gone_falls_back_to_the_working_folder() {
        let got = pick_with(None, Some("/"), Some("/work/gone"), false, false);
        assert_eq!(got, PathBuf::from("/"));
    }

    #[test]
    fn nothing_remembered_keeps_todays_behavior() {
        assert_eq!(pick_with(None, Some("/"), None, false, true), PathBuf::from("/"));
        assert_eq!(pick_with(None, None, None, false, true), PathBuf::from("."));
    }

    #[test]
    fn a_working_folder_in_a_repo_never_looks_at_the_remembered_one() {
        let got = pick(
            None,
            Some(PathBuf::from("/work/web")),
            Some(PathBuf::from("/work/old")),
            |_| true,
            |_| unreachable!("the repo decides"),
        );
        assert_eq!(got, PathBuf::from("/work/web"));
    }

    #[test]
    fn the_folder_round_trips_through_the_file_with_spaces_and_unicode() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(load(&paths), None);
        let dir = Path::new("/Users/me/My Projects/prøjekt \"web\"");
        save(&paths, dir).unwrap();
        assert_eq!(load(&paths), Some(dir.to_path_buf()));
        save(&paths, Path::new("/work/api")).unwrap();
        assert_eq!(load(&paths), Some(PathBuf::from("/work/api")));
    }

    #[test]
    fn a_corrupt_file_loads_as_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::write(paths.last_workspace_file(), "dir = [[[").unwrap();
        assert_eq!(load(&paths), None);
    }
}
