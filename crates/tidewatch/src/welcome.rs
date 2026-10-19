//! The repos the welcome screen offers, from the last-workspace file and the saved session.

use std::path::{Path, PathBuf};

use tidewatch_core::last_workspace::{self, Remembered};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::session::{SavedTab, Session};

const MAX_RECENT: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recent {
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    pub(crate) tabs: usize,
}

impl Recent {
    /// `3 tabs`, `1 tab`, or nothing for a repo that had none open.
    pub(crate) fn tabs_text(&self) -> String {
        match self.tabs {
            0 => String::new(),
            1 => "1 tab".to_owned(),
            n => format!("{n} tabs"),
        }
    }
}

/// The folder as `~/code/web` when it is under `home`, so the list does not show a user name.
pub(crate) fn shown_path(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

/// The repos the last window had, then the ones the session file remembers, each once and only
/// while the folder is still there. A plain folder (the Other group) and `/` are not repos to offer.
pub(crate) fn recent(
    remembered: Option<&Remembered>,
    session: &Session,
    is_dir: impl Fn(&Path) -> bool,
) -> Vec<Recent> {
    let tabs_of = |dir: &Path| {
        session.window(dir).map_or(0, |window| {
            window.tabs.iter().filter(|tab| !matches!(tab, SavedTab::Main)).count()
        })
    };
    let from_last = remembered.into_iter().flat_map(|r| std::iter::once(&r.dir).chain(&r.also));
    let from_session = session.windows.iter().filter(|window| !window.other).map(|w| &w.dir);
    let mut found: Vec<Recent> = Vec::new();
    for dir in from_last.chain(from_session) {
        let known = found.iter().any(|recent| recent.dir == *dir);
        if known || dir == Path::new("/") || !is_dir(dir) {
            continue;
        }
        let name = dir
            .file_name()
            .map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        found.push(Recent { name, dir: dir.clone(), tabs: tabs_of(dir) });
    }
    found.truncate(MAX_RECENT);
    found
}

/// Every repo folder the last window and the saved session remember, each once and only while it
/// is still there: where a pasted GitHub link may already have a checkout.
pub(crate) fn known_dirs(
    remembered: Option<&Remembered>,
    session: &Session,
    is_dir: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let from_last = remembered.into_iter().flat_map(|r| std::iter::once(&r.dir).chain(&r.also));
    let from_session = session.windows.iter().filter(|window| !window.other).map(|w| &w.dir);
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in from_last.chain(from_session) {
        if dir != Path::new("/") && is_dir(dir) && !found.contains(dir) {
            found.push(dir.clone());
        }
    }
    found
}

pub(crate) fn known_repo_dirs(paths: Option<&TidewatchPaths>) -> Vec<PathBuf> {
    let Some(paths) = paths else { return Vec::new() };
    known_dirs(last_workspace::load(paths).as_ref(), &Session::load(paths), Path::is_dir)
}

/// Reads the two files the list comes from. They are small and read once, at launch.
pub(crate) fn recent_repos(paths: Option<&TidewatchPaths>) -> Vec<Recent> {
    let Some(paths) = paths else { return Vec::new() };
    let remembered = last_workspace::load(paths);
    recent(remembered.as_ref(), &Session::load(paths), Path::is_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::session::SavedWindow;

    fn window(dir: &str, other: bool, tabs: Vec<SavedTab>) -> SavedWindow {
        SavedWindow { dir: PathBuf::from(dir), other, active: 0, focus_mode: false, tabs }
    }

    fn shell(dir: &str) -> SavedTab {
        SavedTab::Shell { dir: PathBuf::from(dir), branch: None }
    }

    #[test]
    fn every_remembered_repo_is_known_once_while_its_folder_is_there() {
        let session = Session {
            windows: vec![
                window("/code/api", false, vec![SavedTab::Main]),
                window("/code/web", false, vec![]),
                window("/scratch", true, vec![]),
                window("/code/old", false, vec![]),
                window("/", false, vec![]),
            ],
        };
        let remembered = Remembered {
            dir: PathBuf::from("/code/web"),
            also: vec![PathBuf::from("/code/api"), PathBuf::from("/code/gone")],
        };
        let known = known_dirs(Some(&remembered), &session, |dir| dir != Path::new("/code/gone"));
        let want: Vec<PathBuf> = ["/code/web", "/code/api", "/code/old"].map(PathBuf::from).into();
        assert_eq!(known, want);
        assert_eq!(known_dirs(None, &Session { windows: vec![] }, |_| true), Vec::<PathBuf>::new());
    }

    #[test]
    fn the_last_window_comes_first_then_the_session_each_repo_once_with_its_open_tabs() {
        let session = Session {
            windows: vec![
                window("/code/api", false, vec![SavedTab::Main, shell("/w/a"), shell("/w/b")]),
                window("/code/web", false, vec![SavedTab::Main, shell("/w/c")]),
                window("/code/dotfiles", false, vec![SavedTab::Main]),
            ],
        };
        let last = Remembered { dir: "/code/web".into(), also: vec!["/code/gone".into()] };
        let got = recent(Some(&last), &session, |dir| dir != Path::new("/code/gone"));
        let names: Vec<_> = got.iter().map(|r| (r.name.as_str(), r.tabs)).collect();
        assert_eq!(names, [("web", 1), ("api", 2), ("dotfiles", 0)]);
    }

    #[test]
    fn plain_folders_the_root_and_missing_folders_are_not_offered_and_the_list_is_capped() {
        let mut windows = vec![window("/notes", true, vec![SavedTab::Main])];
        windows.extend((0..8).map(|n| window(&format!("/code/r{n}"), false, vec![])));
        let got = recent(
            Some(&Remembered { dir: "/".into(), also: Vec::new() }),
            &Session { windows },
            |_| true,
        );
        assert_eq!(got.len(), MAX_RECENT);
        assert!(got.iter().all(|r| r.name.starts_with('r')), "{got:?}");
    }

    #[test]
    fn nothing_remembered_is_an_empty_list() {
        assert_eq!(recent(None, &Session::default(), |_| true), Vec::<Recent>::new());
    }

    #[test]
    fn tab_counts_read_as_words_and_paths_hide_the_home_folder() {
        let r = |tabs| Recent { name: String::new(), dir: PathBuf::new(), tabs };
        assert_eq!(
            (r(0).tabs_text(), r(1).tabs_text(), r(3).tabs_text()),
            (String::new(), "1 tab".into(), "3 tabs".into())
        );
        let home = Path::new("/Users/me");
        assert_eq!(shown_path(Path::new("/Users/me/code/web"), Some(home)), "~/code/web");
        assert_eq!(shown_path(Path::new("/Users/me"), Some(home)), "~");
        assert_eq!(shown_path(Path::new("/srv/web"), Some(home)), "/srv/web");
        assert_eq!(shown_path(Path::new("/srv/web"), None), "/srv/web");
    }
}
