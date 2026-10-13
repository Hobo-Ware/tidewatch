//! Keeps `session.toml` current: once per frame it asks whether anything worth saving changed
//! (a fingerprint over every repo of the window, no allocation) and, when it did, hands the
//! window to the debounced writer. The
//! quit path flushes synchronously. Nothing is saved under the screenshot harness, and not while a
//! restore is still putting the saved tabs back (a save then would overwrite them with a partial
//! window).

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::session::SavedWindow;

use crate::pane::Pane;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::session_save::{WindowView, saved_window, signature};
use crate::session_writer::{Debounce, Writer};
use crate::tabs::Tabs;

pub(crate) struct Saver {
    writer: Option<Writer>,
    last: Option<u64>,
    retired: Vec<SavedWindow>,
}

impl Saver {
    /// Without `paths` or with `enabled` false the saver does nothing.
    pub(crate) fn new(paths: Option<&TidewatchPaths>, enabled: bool, debounce: Debounce) -> Self {
        let writer = paths.filter(|_| enabled).map(|paths| Writer::start(paths.clone(), debounce));
        Self { writer, last: None, retired: Vec::new() }
    }

    /// Writes a closed repo's last window with the next save, so what the repo had open is not
    /// brought back when it is opened again.
    pub(crate) fn retire(&mut self, window: SavedWindow) {
        if self.writer.is_some() {
            self.retired.push(window);
            self.last = None;
        }
    }

    /// Queues a save when the window changed since the last one. `settled` is false while a restore
    /// is under way. Returns whether a save was queued.
    pub(crate) fn tick<'a, T>(
        &mut self,
        settled: bool,
        views: impl Iterator<Item = WindowView<'a>> + Clone,
        tabs: &Tabs<Pane<T>>,
    ) -> bool {
        let Some(writer) = self.writer.as_ref().filter(|_| settled) else { return false };
        let mut hasher = DefaultHasher::new();
        for view in views.clone() {
            signature(&view, tabs).hash(&mut hasher);
        }
        let sig = hasher.finish();
        if self.last == Some(sig) {
            return false;
        }
        self.last = Some(sig);
        writer.save(
            views.map(|view| saved_window(&view, tabs)).chain(self.retired.drain(..)).collect(),
        );
        true
    }

    /// Writes the window now and waits, for the quit path. Skipped like `tick`.
    pub(crate) fn flush<'a, T>(
        &mut self,
        settled: bool,
        views: impl Iterator<Item = WindowView<'a>>,
        tabs: &Tabs<Pane<T>>,
    ) {
        if let Some(writer) = self.writer.as_ref().filter(|_| settled) {
            writer.flush(
                views.map(|view| saved_window(&view, tabs)).chain(self.retired.drain(..)).collect(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use tidewatch_core::session::{SavedTab, Session};
    use tidewatch_core::worktree::Worktree;

    use crate::pane::Body;
    use crate::tabs::Action;

    const DIR: &str = "/work/web";

    fn view(focus: bool) -> WindowView<'static> {
        WindowView { group: 0, dir: Path::new(DIR), other: false, focus }
    }

    fn views(focus: bool) -> std::option::IntoIter<WindowView<'static>> {
        Some(view(focus)).into_iter()
    }

    fn slow() -> Debounce {
        Debounce::new(Duration::from_secs(60), Duration::from_secs(120))
    }

    fn with_worktree_tab(tabs: &mut Tabs<Pane<u32>>) {
        let worktree = Worktree {
            repo: PathBuf::from(DIR),
            path: PathBuf::from("/home/.tidewatch/web-abc123/calm-comet"),
            branch: "agent/calm-comet".into(),
        };
        tabs.push(
            "calm-comet".into(),
            Pane { body: Body::Shell(1), worktree: Some(worktree), agent: None },
        );
    }

    #[test]
    fn a_change_queues_a_save_and_the_same_window_again_does_not() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let mut tabs = Tabs::new(Pane::shell(0));
        assert!(saver.tick(true, views(false), &tabs), "the first look always saves");
        assert!(!saver.tick(true, views(false), &tabs));
        with_worktree_tab(&mut tabs);
        assert!(saver.tick(true, views(false), &tabs), "a new tab");
        assert!(!saver.tick(true, views(false), &tabs));
        assert!(saver.tick(true, views(true), &tabs), "focus mode");
        tabs.apply(Action::Select(0), || unreachable!());
        assert!(saver.tick(true, views(true), &tabs), "the active tab");
    }

    #[test]
    fn nothing_is_queued_while_a_restore_is_under_way() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let tabs = Tabs::new(Pane::shell(0));
        assert!(!saver.tick(false, views(false), &tabs));
        saver.flush(false, views(false), &tabs);
        assert_eq!(Session::load(&paths), Session::default());
        assert!(saver.tick(true, views(false), &tabs), "the first settled look saves");
    }

    #[test]
    fn a_disabled_saver_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), false, slow());
        let tabs = Tabs::new(Pane::shell(0));
        assert!(!saver.tick(true, views(false), &tabs));
        saver.flush(true, views(false), &tabs);
        let mut without_paths = Saver::new(None, true, slow());
        assert!(!without_paths.tick(true, views(false), &tabs));
        assert!(!paths.session_file().exists());
    }

    #[test]
    fn a_flush_leaves_the_window_on_disk_before_it_returns() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let mut tabs = Tabs::new(Pane::shell(0));
        with_worktree_tab(&mut tabs);
        let started = Instant::now();
        saver.flush(true, views(true), &tabs);
        assert!(started.elapsed() < Duration::from_secs(5));
        let session = Session::load(&paths);
        let window = session.window(Path::new(DIR)).expect("the window was written");
        assert!(window.focus_mode);
        assert_eq!(window.active, 1);
        assert!(matches!(window.tabs[..], [SavedTab::Main, SavedTab::Shell { .. }]));
    }

    #[test]
    fn every_repo_of_the_window_is_watched_and_written() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let mut tabs = Tabs::new(Pane::shell(0));
        let api = tabs.add_group(Pane::shell(10));
        let both = || {
            [
                view(false),
                WindowView { group: api, dir: Path::new("/work/api"), other: false, focus: false },
            ]
            .into_iter()
        };
        assert!(saver.tick(true, both(), &tabs));
        assert!(!saver.tick(true, both(), &tabs));
        let worktree = Worktree {
            repo: PathBuf::from("/work/api"),
            path: PathBuf::from("/home/.tidewatch/api-abc123/calm-comet"),
            branch: "agent/calm-comet".into(),
        };
        tabs.push_in(
            api,
            "calm-comet".into(),
            Pane { body: Body::Shell(1), worktree: Some(worktree), agent: None },
        );
        assert!(saver.tick(true, both(), &tabs), "a tab in the repo out of sight");
        saver.flush(true, both(), &tabs);
        let session = Session::load(&paths);
        assert_eq!(session.windows.len(), 2);
        let saved = session.window(Path::new("/work/api")).expect("the second repo was written");
        assert!(matches!(saved.tabs[..], [SavedTab::Main, SavedTab::Shell { .. }]));
    }

    #[test]
    fn a_closed_repo_is_written_with_only_its_main_shell() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let mut tabs = Tabs::new(Pane::shell(0));
        let api = tabs.add_group(Pane::shell(10));
        let api_view =
            WindowView { group: api, dir: Path::new("/work/api"), other: false, focus: false };
        let worktree = Worktree {
            repo: PathBuf::from("/work/api"),
            path: PathBuf::from("/home/.tidewatch/api-abc123/calm-comet"),
            branch: "agent/calm-comet".into(),
        };
        let tab = tabs.push_in(
            api,
            "calm-comet".into(),
            Pane { body: Body::Shell(1), worktree: Some(worktree), agent: None },
        );
        saver.flush(true, [view(false), api_view].into_iter(), &tabs);
        let saved = Session::load(&paths);
        let before = saved.window(Path::new("/work/api")).expect("both repos were written");
        assert!(matches!(before.tabs[..], [SavedTab::Main, SavedTab::Shell { .. }]));
        tabs.remove_id(tab);
        saver.retire(saved_window(&api_view, &tabs));
        assert_eq!(tabs.remove_group(api).len(), 1);
        saver.flush(true, views(false), &tabs);
        let session = Session::load(&paths);
        let after =
            session.window(Path::new("/work/api")).expect("the closed repo is still listed");
        assert!(matches!(after.tabs[..], [SavedTab::Main]));
    }
}
