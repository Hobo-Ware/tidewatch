//! The thread that finds a tab's transcript and reads what is appended to it (PLAN.md, D12). It
//! sleeps until it is poked (the pty printed something, a hook arrived), wakes on a backoff that
//! grows while the file stays quiet, and sends new entries to the UI, which it then wakes. A tab
//! that is idle costs one stat and one empty read every few seconds, and no repaint.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use tidewatch_core::token_usage::Tally;
use tidewatch_core::transcript::{Entry, Source, Tail};
use tidewatch_core::transcript_facts::{self, Timed};
use tidewatch_core::transcript_locate::Locator;

const LOCATE_FAST: Duration = Duration::from_millis(500);
const LOCATE_SLOW: Duration = Duration::from_secs(5);
const LOCATE_FAST_WINDOW: Duration = Duration::from_secs(30);
const MISSING_AFTER: Duration = Duration::from_secs(20);
const TAIL_FIRST: Duration = Duration::from_millis(400);
const TAIL_MAX: Duration = Duration::from_secs(15);
const TAIL_MAX_WORKING: Duration = Duration::from_secs(2);
const NEWER_COPY_LOOK: Duration = Duration::from_secs(5);

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
///
/// A working agent gets a lower ceiling: a turn that ends is seen within seconds even when no hook
/// says so.
pub(crate) fn tail_delay(empty_reads: u32, working: bool) -> Duration {
    let ceiling = if working { TAIL_MAX_WORKING } else { TAIL_MAX };
    TAIL_FIRST.saturating_mul(1_u32.checked_shl(empty_reads).unwrap_or(u32::MAX)).min(ceiling)
}

#[derive(Debug)]
pub(crate) enum Message {
    Found(PathBuf),
    Missing,
    Entries(Vec<Entry>),
    Facts(Vec<Timed>),
    Title(String),
    Tokens(Tally),
}

enum Command {
    Poke,
    Hint(PathBuf),
    Stop,
}

pub(crate) struct Feed {
    commands: Sender<Command>,
    messages: Receiver<Message>,
    working: std::sync::Arc<AtomicBool>,
}

impl Feed {
    /// Starts looking for the transcript of `source`. `wake` is called after every message, from the
    /// feed's thread.
    pub(crate) fn start(
        source: Source,
        locator: Locator,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        Self::start_looking(source, locator, NEWER_COPY_LOOK, wake)
    }

    fn start_looking(
        source: Source,
        locator: Locator,
        copy_look: Duration,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (message_tx, messages) = mpsc::channel();
        let working = std::sync::Arc::new(AtomicBool::new(false));
        let reading = std::sync::Arc::clone(&working);
        thread::spawn(move || {
            run(source, locator, copy_look, &reading, &command_rx, &message_tx, &wake);
        });
        Self { commands, messages, working }
    }

    /// A switch to working pokes the thread so the lower ceiling applies at once.
    pub(crate) fn set_working(&self, working: bool) {
        if self.working.swap(working, Ordering::Relaxed) != working && working {
            self.poke();
        }
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
    copy_look: Duration,
    working: &AtomicBool,
    commands: &Receiver<Command>,
    messages: &Sender<Message>,
    wake: &impl Fn(),
) {
    let started = Instant::now();
    let since_ms = locator.started_after_ms.unwrap_or(0);
    let mut tail: Option<Tail> = None;
    let mut empty_reads = 0_u32;
    let mut said_missing = false;
    let mut sent_title: Option<String> = None;
    let mut sent_stamp = (0_u32, 0_u32);
    let mut delay = Duration::ZERO;
    let mut copy_looked = Instant::now();
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
        if source == Source::Claude && copy_looked.elapsed() >= copy_look {
            copy_looked = Instant::now();
            let newer = tail.as_ref().and_then(|tail| locator.newer_claude(tail.path()));
            if let Some(path) = newer
                && tail.as_mut().is_some_and(|tail| tail.follow(&path))
            {
                empty_reads = 0;
                if !send(Message::Found(path)) {
                    return;
                }
            }
        }
        let (entries, marks) = tail.as_mut().map(Tail::poll_marked).unwrap_or_default();
        let title = tail
            .as_ref()
            .and_then(Tail::session_title)
            .filter(|title| sent_title.as_deref() != Some(*title))
            .map(str::to_owned);
        if let Some(title) = title {
            sent_title = Some(title.clone());
            if !send(Message::Title(title)) {
                return;
            }
        }
        let stamp = tail.as_ref().map_or((0, 0), Tail::token_stamp);
        if stamp != sent_stamp {
            sent_stamp = stamp;
            if let Some(tail) = &tail
                && !send(Message::Tokens(tail.tokens()))
            {
                return;
            }
        }
        if entries.is_empty() && marks.is_empty() {
            empty_reads = empty_reads.saturating_add(1);
        } else {
            empty_reads = 0;
            let facts = transcript_facts::facts_of(&entries, &marks, since_ms);
            if !facts.is_empty() && !send(Message::Facts(facts)) {
                return;
            }
            if !entries.is_empty() && !send(Message::Entries(entries)) {
                return;
            }
        }
        delay = tail_delay(empty_reads, working.load(Ordering::Relaxed));
        if source == Source::Claude {
            delay = delay.min(copy_look.saturating_sub(copy_looked.elapsed()));
        }
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
        let delays: Vec<_> = (0..8).map(|n| tail_delay(n, false).as_millis()).collect();
        assert_eq!(delays, [400, 800, 1600, 3200, 6400, 12800, 15000, 15000]);
        assert_eq!(tail_delay(u32::MAX, false), TAIL_MAX);
    }

    #[test]
    fn a_working_agent_is_looked_at_every_two_seconds_at_most() {
        let delays: Vec<_> = (0..8).map(|n| tail_delay(n, true).as_millis()).collect();
        assert_eq!(delays, [400, 800, 1600, 2000, 2000, 2000, 2000, 2000]);
        assert_eq!(tail_delay(u32::MAX, true), TAIL_MAX_WORKING);
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

    #[test]
    fn a_session_title_is_sent_once_and_again_when_it_changes() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        fs::create_dir_all(&dir).unwrap();
        let id = "33333333-aaaa-bbbb-cccc-000000000003";
        let file = dir.join(format!("{id}.jsonl"));
        let title = |text: &str| format!("{{\"type\":\"ai-title\",\"aiTitle\":\"{text}\"}}\n");
        fs::write(&file, title("Fix login redirect")).unwrap();
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some(id.into());
        let feed = Feed::start(Source::Claude, locator, || {});
        let titles = |seen: &[Message]| -> Vec<String> {
            seen.iter()
                .filter_map(|m| match m {
                    Message::Title(title) => Some(title.clone()),
                    _ => None,
                })
                .collect()
        };
        let seen = wait_for(&feed, |m| matches!(m, Message::Title(_)));
        assert_eq!(titles(&seen), ["Fix login redirect"]);
        let mut text = fs::read_to_string(&file).unwrap();
        text.push_str(&title("Fix login redirect"));
        text.push_str(&title("Send people back after login"));
        fs::write(&file, text).unwrap();
        feed.poke();
        let seen = wait_for(&feed, |m| matches!(m, Message::Title(_)));
        assert_eq!(titles(&seen), ["Send people back after login"]);
    }

    #[test]
    fn the_tokens_of_a_transcript_are_sent_when_they_change_and_not_otherwise() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        fs::create_dir_all(&dir).unwrap();
        let id = "44444444-aaaa-bbbb-cccc-000000000004";
        let file = dir.join(format!("{id}.jsonl"));
        let usage = include_str!("../../tidewatch-core/tests/fixtures/claude_usage.jsonl");
        let lines: Vec<&str> = usage.lines().collect();
        fs::write(&file, format!("{}\n", lines[..5].join("\n"))).unwrap();
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some(id.into());
        let feed = Feed::start(Source::Claude, locator, || {});
        let tokens = |seen: &[Message]| -> Vec<(u32, u32)> {
            seen.iter()
                .filter_map(|m| match m {
                    Message::Tokens(tally) => Some((tally.calls, tally.turn_count)),
                    _ => None,
                })
                .collect()
        };
        let seen = wait_for(&feed, |m| matches!(m, Message::Tokens(_)));
        assert_eq!(tokens(&seen), [(2, 1)]);
        feed.poke();
        thread::sleep(Duration::from_millis(900));
        assert_eq!(tokens(&feed.drain()), Vec::<(u32, u32)>::new());
        fs::write(&file, usage).unwrap();
        feed.poke();
        let seen = wait_for(&feed, |m| matches!(m, Message::Tokens(_)));
        assert_eq!(tokens(&seen), [(3, 2)]);
    }

    fn user_line(text: &str) -> String {
        format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{text}\"}}}}\n")
    }

    fn entries_of(message: &Message) -> Vec<Entry> {
        match message {
            Message::Entries(entries) => entries.clone(),
            _ => Vec::new(),
        }
    }

    fn texts(seen: &[Message]) -> Vec<String> {
        seen.iter()
            .filter_map(|m| match m {
                Message::Entries(entries) => Some(entries.iter().map(|e| e.text.clone())),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn copy_into(dir: &std::path::Path, name: &str, text: &str, secs_ago: u64) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let file = dir.join(name);
        fs::write(&file, text).unwrap();
        let at = std::time::SystemTime::now() - Duration::from_secs(secs_ago);
        fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(at).unwrap();
        file
    }

    #[test]
    fn a_newer_copy_of_the_session_takes_over_and_clears_the_queued_message() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        let projects = home.path().join(".claude").join("projects");
        let own = projects.join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        let id = "44444444-aaaa-bbbb-cccc-000000000004";
        let name = format!("{id}.jsonl");
        let stale = user_line("start") + &claude_line("working");
        copy_into(&own, &name, &stale, 600);
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some(id.into());
        let feed = Feed::start_looking(Source::Claude, locator, Duration::from_millis(100), || {});
        let mut rows = crate::reader_rows::Rows::default();
        rows.queue_user("fix it", 0);
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        rows.push_all(seen.iter().flat_map(entries_of).collect());
        assert_eq!(
            (texts(&seen), rows.queued_len()),
            (["start", "working"].map(Into::into).to_vec(), 1)
        );

        let live = stale + &user_line("fix it") + &claude_line("on it");
        let live_file = copy_into(&projects.join("-live-copy"), &name, &live, 1);
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        assert!(seen.iter().any(|m| matches!(m, Message::Found(path) if *path == live_file)));
        assert_eq!(texts(&seen), ["fix it", "on it"]);
        rows.push_all(seen.iter().flat_map(entries_of).collect());
        assert_eq!(rows.queued_len(), 0);
    }

    #[test]
    fn a_quiet_session_with_no_newer_copy_stays_on_its_file() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        let own = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        let id = "55555555-aaaa-bbbb-cccc-000000000005";
        copy_into(&own, &format!("{id}.jsonl"), &claude_line("one"), 10);
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some(id.into());
        let feed = Feed::start_looking(Source::Claude, locator, Duration::from_millis(50), || {});
        wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        thread::sleep(Duration::from_millis(500));
        let seen = feed.drain();
        assert!(seen.iter().all(|m| !matches!(m, Message::Found(_) | Message::Entries(_))));
    }

    const SESSION: &str = "33333333-aaaa-bbbb-cccc-000000000003";

    fn turn_file(since_ms: Option<i64>) -> (tempfile::TempDir, Feed) {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(tidewatch_core::transcript_locate::claude_project_dir_name(&cwd));
        fs::create_dir_all(&dir).unwrap();
        let prompt = "{\"type\":\"user\",\"timestamp\":\"2026-10-13T10:00:00Z\",\"message\":{\"content\":\"go\"}}\n";
        let end = "{\"type\":\"assistant\",\"timestamp\":\"2026-10-13T10:00:05Z\",\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n";
        fs::write(dir.join(format!("{SESSION}.jsonl")), format!("{prompt}{end}")).unwrap();
        let mut locator = Locator::new(home.path(), &cwd);
        locator.session_id = Some(SESSION.into());
        locator.started_after_ms = since_ms;
        (home, Feed::start(Source::Claude, locator, || {}))
    }

    #[test]
    fn the_turn_a_transcript_shows_arrives_as_panel_facts_beside_its_entries() {
        let (_home, feed) = turn_file(None);
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        let facts: Vec<_> = seen
            .iter()
            .filter_map(|m| match m {
                Message::Facts(facts) => Some(facts.iter().map(|timed| timed.fact.name())),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(facts, ["turn_started", "said", "turn_ended"]);
    }

    #[test]
    fn a_turn_written_before_the_tab_started_is_history_and_sends_no_facts() {
        let (_home, feed) = turn_file(Some(1_791_936_000_000));
        let seen = wait_for(&feed, |m| matches!(m, Message::Entries(_)));
        assert!(seen.iter().all(|m| !matches!(m, Message::Facts(_))));
    }
}
