//! Writes the saved window to `session.toml` off the UI thread. Saves are debounced: a burst of
//! changes (a tab opening, then its agent starting, then its title settling) is one write, but a
//! window that never stops changing still gets one every `max`. A flush writes at once and waits,
//! for the quit path, where the process is about to end.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::session::{SavedWindow, Session};

pub(crate) const QUIET: Duration = Duration::from_millis(400);
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(2);
const FLUSH_WAIT: Duration = Duration::from_secs(3);

/// When a pending save is due: `quiet` after the last change, but never later than `max` after the
/// first one. Pure: the caller passes the time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Debounce {
    quiet: Duration,
    max: Duration,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debounce {
    pub(crate) fn new(quiet: Duration, max: Duration) -> Self {
        Self { quiet, max, first: None, last: None }
    }

    pub(crate) fn touch(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// How long until the pending save is due (zero when it is), or `None` with nothing pending.
    pub(crate) fn wait(&self, now: Instant) -> Option<Duration> {
        let (first, last) = self.first.zip(self.last)?;
        let due = (last + self.quiet).min(first + self.max);
        Some(due.saturating_duration_since(now))
    }

    pub(crate) fn settle(&mut self) {
        self.first = None;
        self.last = None;
    }
}

enum Message {
    Save(Box<SavedWindow>),
    Flush(Box<SavedWindow>, Sender<()>),
}

pub(crate) struct Writer {
    sender: Sender<Message>,
}

impl Writer {
    pub(crate) fn start(paths: TidewatchPaths, debounce: Debounce) -> Self {
        let (sender, receiver) = channel();
        std::thread::spawn(move || run(&paths, debounce, &receiver));
        Self { sender }
    }

    /// Queues the window; the write follows once the changes go quiet.
    pub(crate) fn save(&self, window: SavedWindow) {
        let _ = self.sender.send(Message::Save(Box::new(window)));
    }

    /// Writes the window now and waits for the file, for a few seconds at most.
    pub(crate) fn flush(&self, window: SavedWindow) {
        let (ack, done) = channel();
        if self.sender.send(Message::Flush(Box::new(window), ack)).is_ok() {
            let _ = done.recv_timeout(FLUSH_WAIT);
        }
    }
}

fn run(paths: &TidewatchPaths, mut debounce: Debounce, receiver: &Receiver<Message>) {
    let mut pending: Option<SavedWindow> = None;
    loop {
        let next = match debounce.wait(Instant::now()) {
            Some(wait) => receiver.recv_timeout(wait),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Message::Save(window)) => {
                pending = Some(*window);
                debounce.touch(Instant::now());
            }
            Ok(Message::Flush(window, ack)) => {
                write(paths, *window);
                pending = None;
                debounce.settle();
                let _ = ack.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(window) = pending.take() {
                    write(paths, window);
                }
                debounce.settle();
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(window) = pending.take() {
                    write(paths, window);
                }
                return;
            }
        }
    }
}

/// Replaces this window's entry and leaves the other windows' alone.
fn write(paths: &TidewatchPaths, window: SavedWindow) {
    let mut session = Session::load(paths);
    session.replace_window(window);
    if let Err(e) = session.save(paths) {
        eprintln!("tidewatch: cannot save the session: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::session::SavedTab;

    fn window(dir: &str, active: usize) -> SavedWindow {
        SavedWindow {
            dir: PathBuf::from(dir),
            other: false,
            active,
            focus_mode: false,
            tabs: vec![SavedTab::Main],
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn nothing_pending_means_nothing_is_due() {
        let debounce = Debounce::new(ms(400), ms(2000));
        assert_eq!(debounce.wait(Instant::now()), None);
    }

    #[test]
    fn a_change_is_due_after_the_quiet_time() {
        let t0 = Instant::now();
        let mut debounce = Debounce::new(ms(400), ms(2000));
        debounce.touch(t0);
        assert_eq!(debounce.wait(t0), Some(ms(400)));
        assert_eq!(debounce.wait(t0 + ms(150)), Some(ms(250)));
        assert_eq!(debounce.wait(t0 + ms(400)), Some(ms(0)));
        assert_eq!(debounce.wait(t0 + ms(900)), Some(ms(0)), "overdue is still just due");
    }

    #[test]
    fn each_new_change_pushes_the_save_back() {
        let t0 = Instant::now();
        let mut debounce = Debounce::new(ms(400), ms(2000));
        debounce.touch(t0);
        debounce.touch(t0 + ms(300));
        assert_eq!(debounce.wait(t0 + ms(300)), Some(ms(400)));
        assert_eq!(debounce.wait(t0 + ms(700)), Some(ms(0)));
    }

    #[test]
    fn a_window_that_never_settles_is_still_saved_at_the_max_wait() {
        let t0 = Instant::now();
        let mut debounce = Debounce::new(ms(400), ms(2000));
        for step in 0..8 {
            debounce.touch(t0 + ms(300 * step));
        }
        debounce.touch(t0 + ms(2000));
        assert_eq!(debounce.wait(t0 + ms(2100)), Some(ms(0)), "2 s after the first change");
        assert_eq!(debounce.wait(t0 + ms(1900)), Some(ms(100)));
    }

    #[test]
    fn settling_clears_what_was_pending() {
        let t0 = Instant::now();
        let mut debounce = Debounce::new(ms(400), ms(2000));
        debounce.touch(t0);
        debounce.settle();
        assert_eq!(debounce.wait(t0), None);
        debounce.touch(t0 + ms(5000));
        assert_eq!(
            debounce.wait(t0 + ms(5000)),
            Some(ms(400)),
            "the max counts from the new burst"
        );
    }

    fn wait_for(paths: &TidewatchPaths, want: &Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Session::load(paths) != *want {
            assert!(Instant::now() < deadline, "the session file never became {want:?}");
            std::thread::sleep(ms(5));
        }
    }

    #[test]
    fn a_burst_of_saves_ends_as_the_last_one_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let writer = Writer::start(paths.clone(), Debounce::new(ms(30), ms(500)));
        for active in 0..5 {
            writer.save(window("/work/web", active));
        }
        wait_for(&paths, &Session { windows: vec![window("/work/web", 4)] });
    }

    #[test]
    fn a_save_waits_for_the_quiet_time() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let writer = Writer::start(paths.clone(), Debounce::new(ms(10_000), ms(20_000)));
        writer.save(window("/work/web", 1));
        std::thread::sleep(ms(100));
        assert_eq!(Session::load(&paths), Session::default());
    }

    #[test]
    fn a_flush_writes_at_once_even_with_a_long_quiet_time() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let writer = Writer::start(paths.clone(), Debounce::new(ms(10_000), ms(20_000)));
        writer.save(window("/work/web", 1));
        writer.flush(window("/work/web", 2));
        assert_eq!(Session::load(&paths), Session { windows: vec![window("/work/web", 2)] });
    }

    #[test]
    fn a_save_leaves_other_windows_alone() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        Session { windows: vec![window("/work/api", 0)] }.save(&paths).unwrap();
        let writer = Writer::start(paths.clone(), Debounce::new(ms(10_000), ms(20_000)));
        writer.flush(window("/work/web", 3));
        let session = Session::load(&paths);
        assert_eq!(session.windows.len(), 2);
        assert_eq!(
            session.window(std::path::Path::new("/work/api")),
            Some(&window("/work/api", 0))
        );
        assert_eq!(
            session.window(std::path::Path::new("/work/web")),
            Some(&window("/work/web", 3))
        );
    }

    #[test]
    fn dropping_the_writer_writes_what_was_pending() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let writer = Writer::start(paths.clone(), Debounce::new(ms(10_000), ms(20_000)));
        writer.save(window("/work/web", 1));
        drop(writer);
        wait_for(&paths, &Session { windows: vec![window("/work/web", 1)] });
    }
}
