//! Keeps `session.toml` current: once per frame it asks whether anything worth saving changed
//! (a fingerprint, no allocation) and, when it did, hands the window to the debounced writer. The
//! quit path flushes synchronously. Nothing is saved under the screenshot harness, and not while a
//! restore is still putting the saved tabs back (a save then would overwrite them with a partial
//! window).

use tidewatch_core::paths::TidewatchPaths;

use crate::pane::Pane;
use crate::session_save::{WindowView, saved_window, signature};
use crate::session_writer::{Debounce, Writer};
use crate::tabs::Tabs;

pub(crate) struct Saver {
    writer: Option<Writer>,
    last: Option<u64>,
}

impl Saver {
    /// Without `paths` or with `enabled` false the saver does nothing.
    pub(crate) fn new(paths: Option<&TidewatchPaths>, enabled: bool, debounce: Debounce) -> Self {
        let writer = paths.filter(|_| enabled).map(|paths| Writer::start(paths.clone(), debounce));
        Self { writer, last: None }
    }

    /// Queues a save when the window changed since the last one. `settled` is false while a restore
    /// is under way. Returns whether a save was queued.
    pub(crate) fn tick<T>(
        &mut self,
        settled: bool,
        view: &WindowView,
        tabs: &Tabs<Pane<T>>,
    ) -> bool {
        let Some(writer) = self.writer.as_ref().filter(|_| settled) else { return false };
        let sig = signature(view, tabs);
        if self.last == Some(sig) {
            return false;
        }
        self.last = Some(sig);
        writer.save(saved_window(view, tabs));
        true
    }

    /// Writes the window now and waits, for the quit path. Skipped like `tick`.
    pub(crate) fn flush<T>(&mut self, settled: bool, view: &WindowView, tabs: &Tabs<Pane<T>>) {
        if let Some(writer) = self.writer.as_ref().filter(|_| settled) {
            writer.flush(saved_window(view, tabs));
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
        WindowView { dir: Path::new(DIR), other: false, focus }
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
        assert!(saver.tick(true, &view(false), &tabs), "the first look always saves");
        assert!(!saver.tick(true, &view(false), &tabs));
        with_worktree_tab(&mut tabs);
        assert!(saver.tick(true, &view(false), &tabs), "a new tab");
        assert!(!saver.tick(true, &view(false), &tabs));
        assert!(saver.tick(true, &view(true), &tabs), "focus mode");
        tabs.apply(Action::Select(0), || unreachable!());
        assert!(saver.tick(true, &view(true), &tabs), "the active tab");
    }

    #[test]
    fn nothing_is_queued_while_a_restore_is_under_way() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), true, slow());
        let tabs = Tabs::new(Pane::shell(0));
        assert!(!saver.tick(false, &view(false), &tabs));
        saver.flush(false, &view(false), &tabs);
        assert_eq!(Session::load(&paths), Session::default());
        assert!(saver.tick(true, &view(false), &tabs), "the first settled look saves");
    }

    #[test]
    fn a_disabled_saver_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut saver = Saver::new(Some(&paths), false, slow());
        let tabs = Tabs::new(Pane::shell(0));
        assert!(!saver.tick(true, &view(false), &tabs));
        saver.flush(true, &view(false), &tabs);
        let mut without_paths = Saver::new(None, true, slow());
        assert!(!without_paths.tick(true, &view(false), &tabs));
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
        saver.flush(true, &view(true), &tabs);
        assert!(started.elapsed() < Duration::from_secs(5));
        let session = Session::load(&paths);
        let window = session.window(Path::new(DIR)).expect("the window was written");
        assert!(window.focus_mode);
        assert_eq!(window.active, 1);
        assert!(matches!(window.tabs[..], [SavedTab::Main, SavedTab::Shell { .. }]));
    }
}
