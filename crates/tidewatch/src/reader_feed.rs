//! The thread that finds a tab's transcript and reads what is appended to it (PLAN.md, D12). It
//! sleeps until it is poked (the pty printed something, a hook arrived), wakes on a backoff that
//! grows while the file stays quiet, and sends new entries to the UI, which it then wakes. A tab
//! that is idle costs one stat and one empty read every few seconds, and no repaint.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use tidewatch_core::transcript::{Entry, Source, Tail};
use tidewatch_core::transcript_locate::Locator;

const LOCATE_FAST: Duration = Duration::from_millis(500);
const LOCATE_SLOW: Duration = Duration::from_secs(5);
const LOCATE_FAST_WINDOW: Duration = Duration::from_secs(30);
const MISSING_AFTER: Duration = Duration::from_secs(20);
const TAIL_FIRST: Duration = Duration::from_millis(400);
const TAIL_MAX: Duration = Duration::from_secs(15);

/// How long to wait before the next look for the file: a fast retry right after launch (the file
/// appears late), a slow one after.
pub(crate) fn locate_delay(since_start: Duration) -> Duration {
    if since_start < LOCATE_FAST_WINDOW { LOCATE_FAST } else { LOCATE_SLOW }
}

/// Whether the search has gone on long enough to tell the user there is no transcript (it keeps
/// looking, slowly, and the Reader appears when the file does).
pub(crate) fn is_missing(since_start: Duration) -> bool {
    since_start >= MISSING_AFTER
}

/// How long to wait before the next read: short right after new lines, doubling for every read
/// that found nothing, up to a ceiling.
pub(crate) fn tail_delay(empty_reads: u32) -> Duration {
    TAIL_FIRST.saturating_mul(1_u32.checked_shl(empty_reads).unwrap_or(u32::MAX)).min(TAIL_MAX)
}

#[derive(Debug)]
pub(crate) enum Message {
    Found(PathBuf),
    Missing,
    Entries(Vec<Entry>),
}

enum Command {
    Poke,
    Hint(PathBuf),
    Stop,
}

pub(crate) struct Feed {
    commands: Sender<Command>,
    messages: Receiver<Message>,
}

impl Feed {
    /// Starts looking for the transcript of `source`. `wake` is called after every message, from the
    /// feed's thread.
    pub(crate) fn start(
        source: Source,
        locator: Locator,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (message_tx, messages) = mpsc::channel();
        thread::spawn(move || run(source, locator, &command_rx, &message_tx, &wake));
        Self { commands, messages }
    }

    pub(crate) fn poke(&self) {
        let _ = self.commands.send(Command::Poke);
    }

    /// The path a hook reported; used when the file is not found yet.
    pub(crate) fn hint(&self, path: PathBuf) {
        let _ = self.commands.send(Command::Hint(path));
    }

    pub(crate) fn drain(&self) -> Vec<Message> {
        self.messages.try_iter().collect()
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

fn run(
    source: Source,
    mut locator: Locator,
    commands: &Receiver<Command>,
    messages: &Sender<Message>,
    wake: &impl Fn(),
) {
    let started = Instant::now();
    let mut tail: Option<Tail> = None;
    let mut empty_reads = 0_u32;
    let mut said_missing = false;
    let mut delay = Duration::ZERO;
    let send = |message: Message| {
        let sent = messages.send(message).is_ok();
        wake();
        sent
    };
    loop {
        match commands.recv_timeout(delay) {
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(command) => {
                let mut next = Some(command);
                while let Some(command) = next.take() {
                    match command {
                        Command::Stop => return,
                        Command::Hint(path) => locator.hint = Some(path),
                        Command::Poke => empty_reads = 0,
                    }
                    next = commands.try_recv().ok();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if tail.is_none() {
            let Some(path) = locator.locate(source) else {
                if !said_missing && is_missing(started.elapsed()) {
                    said_missing = true;
                    if !send(Message::Missing) {
                        return;
                    }
                }
                delay = locate_delay(started.elapsed());
                continue;
            };
            if !send(Message::Found(path.clone())) {
                return;
            }
            tail = Some(Tail::new(source, path));
            empty_reads = 0;
        }
        let entries = tail.as_mut().map(Tail::poll).unwrap_or_default();
        if entries.is_empty() {
            empty_reads = empty_reads.saturating_add(1);
        } else {
            empty_reads = 0;
            if !send(Message::Entries(entries)) {
                return;
            }
        }
        delay = tail_delay(empty_reads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_file_is_looked_for_fast_at_first_then_slowly() {
        assert_eq!(locate_delay(Duration::ZERO), LOCATE_FAST);
        assert_eq!(locate_delay(Duration::from_secs(29)), LOCATE_FAST);
        assert_eq!(locate_delay(Duration::from_secs(31)), LOCATE_SLOW);
    }

    #[test]
    fn a_missing_transcript_is_reported_after_the_retry_window() {
        assert!(!is_missing(Duration::from_secs(19)));
        assert!(is_missing(Duration::from_secs(20)));
    }

    #[test]
    fn reads_back_off_while_the_file_is_quiet_and_reset_after_lines() {
        let delays: Vec<_> = (0..8).map(tail_delay).map(|d| d.as_millis()).collect();
        assert_eq!(delays, [400, 800, 1600, 3200, 6400, 12800, 15000, 15000]);
        assert_eq!(tail_delay(u32::MAX), TAIL_MAX);
    }

    fn claude_line(text: &str) -> String {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-10-13T10:00:00Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
        )
    }

    fn wait_for(feed: &Feed, want: impl Fn(&Message) -> bool) -> Vec<Message> {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            seen.extend(feed.drain());
            if seen.iter().any(&want) {
                return seen;
            }
            thread::sleep(Duration::from_millis(20));
        }
        seen
    }

    #[test]
    fn a_file_that_appears_late_is_found_and_read_and_the_ui_is_woken() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some("11111111-aaaa-bbbb-cccc-000000000001".into());
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let feed = Feed::start(Source::Claude, locator, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        thread::sleep(Duration::from_millis(300));
        assert_eq!(feed.drain().len(), 0);
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("11111111-aaaa-bbbb-cccc-000000000001.jsonl"), claude_line("hello"))
            .unwrap();
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        assert!(seen.iter().any(|m| matches!(m, Message::Found(_))));
        let texts: Vec<_> = seen
            .iter()
            .filter_map(|m| match m {
                Message::Entries(entries) => Some(entries.iter().map(|e| e.text.clone())),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(texts, ["hello"]);
        assert_ne!(woken.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_poke_reads_appended_lines_without_waiting_for_the_backoff() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("22222222-aaaa-bbbb-cccc-000000000002.jsonl");
        fs::write(&file, claude_line("one")).unwrap();
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some("22222222-aaaa-bbbb-cccc-000000000002".into());
        let feed = Feed::start(Source::Claude, locator, || {});
        wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        thread::sleep(Duration::from_millis(1500));
        let mut text = fs::read_to_string(&file).unwrap();
        text.push_str(&claude_line("two"));
        fs::write(&file, text).unwrap();
        feed.poke();
        let started = Instant::now();
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        assert!(seen.iter().any(|m| matches!(m, Message::Entries(e) if e[0].text == "two")));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
