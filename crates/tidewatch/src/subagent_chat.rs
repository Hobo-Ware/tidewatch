//! The chat of one sub-agent, open in the place of a tab's Reader: its rows, the view state that
//! keeps its scroll place, and a thread that tails the agent's own file while the chat is open.
//! Dropping the chat stops the thread, so a chat that is not shown reads nothing.
//! `TIDEWATCH_DEV_SUBAGENT_OPEN=<index>` opens the row at that index of the front tab's list once.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;

use tidewatch_core::subagents::{Agent, Phase};
use tidewatch_core::transcript::{Entry, Tail, parse_subagent};

use crate::reader_feed::tail_delay;
use crate::reader_rows::Rows;
use crate::reader_view::ViewState;

const RUNNING_FIXTURE: &str =
    include_str!("../../tidewatch-core/tests/fixtures/subagent_running.jsonl");
const FINISHED_FIXTURE: &str =
    include_str!("../../tidewatch-core/tests/fixtures/subagent_finished.jsonl");

/// Where a chat reads its lines from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    File(PathBuf),
    /// The stand-in of `TIDEWATCH_FAKE_SUBAGENTS`: a redacted real transcript, read once.
    Fake {
        finished: bool,
    },
}

struct Feed {
    entries: Receiver<Vec<Entry>>,
    _stop: Sender<()>,
}

impl Feed {
    fn start(path: PathBuf, ctx: &egui::Context) -> Self {
        let (sender, entries) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut tail = Tail::subagent(path);
            let mut empty_reads = 0_u32;
            loop {
                let fresh = tail.poll();
                if fresh.is_empty() {
                    empty_reads = empty_reads.saturating_add(1);
                } else {
                    empty_reads = 0;
                    if sender.send(fresh).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
                match stopped.recv_timeout(tail_delay(empty_reads, true)) {
                    Err(RecvTimeoutError::Timeout) => {}
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Self { entries, _stop: stop }
    }
}

pub(crate) struct Open {
    pub(crate) tab: u64,
    pub(crate) agent: Agent,
    pub(crate) rows: Rows,
    pub(crate) view: ViewState,
    feed: Option<Feed>,
    drawn: bool,
    fresh: bool,
}

impl Open {
    pub(crate) fn new(tab: u64, agent: Agent, source: Source, ctx: &egui::Context) -> Self {
        let mut rows = Rows::default();
        let feed = match source {
            Source::File(path) => Some(Feed::start(path, ctx)),
            Source::Fake { finished } => {
                let text = if finished { FINISHED_FIXTURE } else { RUNNING_FIXTURE };
                rows.push_all(parse_subagent(text));
                None
            }
        };
        Self { tab, agent, rows, view: ViewState::default(), feed, drawn: false, fresh: true }
    }

    /// A number that is this chat's alone, to keep its scroll place apart from every Reader's.
    pub(crate) fn key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.agent.id.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(test)]
    pub(crate) fn reading(&self) -> bool {
        self.feed.is_some()
    }

    /// Takes the lines the thread read since the last call into the rows.
    pub(crate) fn drain(&mut self) {
        let Some(feed) = &self.feed else { return };
        for entries in feed.entries.try_iter() {
            self.rows.push_all(entries);
        }
    }

    /// Follows the agent's row in the list: its state moves on, and an agent that left the list
    /// has ended by `now_ms`.
    pub(crate) fn refresh(&mut self, found: Option<&Agent>, now_ms: i64) {
        match found {
            Some(agent) => self.agent = agent.clone(),
            None if self.agent.is_running() => self.agent.phase = Phase::Finished(now_ms),
            None => {}
        }
    }

    pub(crate) fn mark_drawn(&mut self) {
        self.drawn = true;
    }
}

#[derive(Default)]
pub(crate) struct Host {
    open: Option<Open>,
    dev_open: Option<usize>,
}

impl Host {
    pub(crate) fn from_env() -> Self {
        let dev_open = std::env::var("TIDEWATCH_DEV_SUBAGENT_OPEN")
            .ok()
            .and_then(|value| value.trim().parse().ok());
        Self { open: None, dev_open }
    }

    pub(crate) fn open(&mut self, chat: Open) {
        self.open = Some(chat);
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The id of the agent whose chat is open over `tab`.
    pub(crate) fn id_over(&self, tab: u64) -> Option<&str> {
        self.open.as_ref().filter(|chat| chat.tab == tab).map(|chat| chat.agent.id.as_str())
    }

    pub(crate) fn chat_mut(&mut self, tab: u64) -> Option<&mut Open> {
        self.open.as_mut().filter(|chat| chat.tab == tab)
    }

    pub(crate) fn dev_open(&self) -> Option<usize> {
        self.dev_open
    }

    pub(crate) fn done_dev_open(&mut self) {
        self.dev_open = None;
    }

    /// Ends a chat that was not drawn this frame, a frame after it opened: a hidden chat is gone,
    /// so it reads nothing, and it opens afresh when its row is clicked again.
    pub(crate) fn end_frame(&mut self) {
        let Some(chat) = self.open.as_mut() else { return };
        if std::mem::take(&mut chat.fresh) {
            return;
        }
        if !std::mem::take(&mut chat.drawn) {
            self.open = None;
        }
    }
}

#[cfg(test)]
mod tests;
