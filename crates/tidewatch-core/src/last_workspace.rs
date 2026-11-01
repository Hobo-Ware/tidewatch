//! The folders of the window used last, in `.state/last-workspace.toml`, and the choice of which
//! folders a launch opens. A launch with no path and no repo around it (the Finder, or `open -n`
//! after an update, both start in `/`) reopens them, so session restore finds their tabs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::save_toml;
use crate::paths::TidewatchPaths;

/// The repo that was in front, and the others that had tabs open next to it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    pub dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<PathBuf>,
}

/// The folders a launch opens: the one in front, then the rest in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub dir: PathBuf,
    pub also: Vec<PathBuf>,
}

impl Opening {
    fn only(dir: PathBuf) -> Self {
        Self { dir, also: Vec::new() }
    }
}

/// A path on the command line wins, then a working folder inside a git repo, then the remembered
/// folders while the first still exists (the others are kept only while they exist too), then the
/// working folder as it is.
pub fn pick(
    arg: Option<PathBuf>,
    cwd: Option<PathBuf>,
    remembered: Option<Remembered>,
    in_repo: impl FnOnce(&Path) -> bool,
    is_dir: impl Fn(&Path) -> bool,
) -> Opening {
    if let Some(arg) = arg {
        return Opening::only(arg);
    }
    if let Some(cwd) = cwd.as_deref().filter(|cwd| in_repo(cwd)) {
        return Opening::only(cwd.to_path_buf());
    }
    if let Some(remembered) = remembered.filter(|remembered| is_dir(&remembered.dir)) {
        let also = remembered.also.into_iter().filter(|dir| is_dir(dir)).collect();
        return Opening { dir: remembered.dir, also };
    }
    Opening::only(cwd.unwrap_or_else(|| PathBuf::from(".")))
}

/// Whether a launch has nothing to open: no path on the command line, nothing remembered to bring
/// back, and the working folder is the root, which is where the Finder starts an app. The window then
/// asks for a repo instead of opening a shell in `/`.
#[must_use]
pub fn is_blank_launch(has_path_arg: bool, opening: &Opening) -> bool {
    !has_path_arg && opening.also.is_empty() && opening.dir == Path::new("/")
}

/// A missing or unreadable file is no folder: a bad state file must never stop the app.
#[must_use]
pub fn load(paths: &TidewatchPaths) -> Option<Remembered> {
    let text = std::fs::read_to_string(paths.last_workspace_file()).ok()?;
    match toml::from_str::<Remembered>(&text) {
        Ok(remembered) => Some(remembered),
        Err(e) => {
            eprintln!("tidewatch: last-workspace.toml is unreadable ({e}); ignoring it");
            None
        }
    }
}

pub fn save(paths: &TidewatchPaths, remembered: &Remembered) -> std::io::Result<()> {
    save_toml(&paths.last_workspace_file(), remembered)
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
            remembered.map(|dir| Remembered { dir: PathBuf::from(dir), also: Vec::new() }),
            |_| cwd_in_repo,
            |_| remembered_exists,
        )
        .dir
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
    fn only_a_launch_from_the_root_with_nothing_to_bring_back_is_blank() {
        let opening = |dir: &str| Opening::only(PathBuf::from(dir));
        assert!(is_blank_launch(false, &opening("/")));
        assert!(!is_blank_launch(true, &opening("/")), "a path was asked for");
        assert!(!is_blank_launch(false, &opening("/work/notes")), "a folder is a place to work");
        let also = Opening { dir: PathBuf::from("/"), also: vec![PathBuf::from("/work/web")] };
        assert!(!is_blank_launch(false, &also));
    }

    #[test]
    fn a_working_folder_in_a_repo_never_looks_at_the_remembered_one() {
        let got = pick(
            None,
            Some(PathBuf::from("/work/web")),
            Some(Remembered { dir: PathBuf::from("/work/old"), also: Vec::new() }),
            |_| true,
            |_| unreachable!("the repo decides"),
        );
        assert_eq!(got.dir, PathBuf::from("/work/web"));
    }

    #[test]
    fn the_folder_round_trips_through_the_file_with_spaces_and_unicode() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(load(&paths), None);
        let dir = PathBuf::from("/Users/me/My Projects/prøjekt \"web\"");
        let first = Remembered { dir, also: vec![PathBuf::from("/work/api")] };
        save(&paths, &first).unwrap();
        assert_eq!(load(&paths), Some(first));
        let second = Remembered { dir: PathBuf::from("/work/api"), also: Vec::new() };
        save(&paths, &second).unwrap();
        assert_eq!(load(&paths), Some(second));
    }

    #[test]
    fn a_file_from_before_several_repos_loads_with_none_beside_the_first() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::write(paths.last_workspace_file(), "dir = \"/work/web\"\n").unwrap();
        assert_eq!(
            load(&paths),
            Some(Remembered { dir: PathBuf::from("/work/web"), also: Vec::new() })
        );
    }

    #[test]
    fn a_reopened_launch_brings_back_the_other_repos_that_still_exist() {
        let remembered = Remembered {
            dir: PathBuf::from("/work/web"),
            also: vec![PathBuf::from("/work/api"), PathBuf::from("/work/gone")],
        };
        let got = pick(
            None,
            Some(PathBuf::from("/")),
            Some(remembered.clone()),
            |_| false,
            |dir| dir != Path::new("/work/gone"),
        );
        assert_eq!(
            got,
            Opening { dir: PathBuf::from("/work/web"), also: vec![PathBuf::from("/work/api")] }
        );
        let explicit =
            pick(Some(PathBuf::from("/work/web")), None, Some(remembered), |_| false, |_| true);
        assert_eq!(explicit.also, Vec::<PathBuf>::new(), "a path asked for opens only that");
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
