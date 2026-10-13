//! One agent tab's Reader (PLAN.md, D12): which view the tab shows, the feed that tails its
//! transcript, and the rows read so far. The decision of what is shown is a pure function of the
//! user's choice and what the search found, so it is tested without a window.

use std::path::{Path, PathBuf};

use tidewatch_core::transcript::{Entry, Source};
use tidewatch_core::transcript_locate::Locator;

use crate::commands::Command;
use crate::reader_feed::{Feed, Message};
use crate::reader_rows::Rows;
use crate::reader_view::ViewState;

const POKE_GAP_S: f64 = 0.3;

/// Why a tab can never show the Reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// A custom agent: there is no parser for its files.
    NoReader,
    /// The parser exists but was not checked against a real run (Codex).
    Unverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    /// Not started: waiting for the tab's agent to run.
    Idle,
    Searching,
    Found,
    /// Not found within the retry window. The search goes on, slowly.
    Missing,
    Unavailable(Unavailable),
}

/// The small note shown over the terminal when it is shown instead of the Reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    Searching,
    Missing,
    NoReader,
    Unverified,
    ChosenRaw,
}

impl Note {
    pub(crate) fn text(self) -> String {
        match self {
            Self::Searching => "Looking for the transcript... showing the terminal".to_owned(),
            Self::Missing => "No transcript found, showing the terminal".to_owned(),
            Self::NoReader => {
                "No transcript reader for this agent, showing the terminal".to_owned()
            }
            Self::Unverified => {
                "This agent's transcript is not verified yet, showing the terminal".to_owned()
            }
            Self::ChosenRaw => {
                format!("Raw terminal. {} for the Reader", Command::Reader.keys())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Reader,
    Terminal(Option<Note>),
}

/// The view an agent tab shows: the Reader when the user wants it and the transcript was found,
/// else the terminal, with a note saying why when that is not the user's own choice.
pub(crate) fn shown(wants_reader: bool, link: Link) -> Shown {
    match (link, wants_reader) {
        (Link::Unavailable(Unavailable::NoReader), _) => Shown::Terminal(Some(Note::NoReader)),
        (Link::Unavailable(Unavailable::Unverified), _) => Shown::Terminal(Some(Note::Unverified)),
        (Link::Found, true) => Shown::Reader,
        (Link::Found, false) => Shown::Terminal(Some(Note::ChosenRaw)),
        (_, false) => Shown::Terminal(None),
        (Link::Missing, true) => Shown::Terminal(Some(Note::Missing)),
        (Link::Idle | Link::Searching, true) => Shown::Terminal(Some(Note::Searching)),
    }
}

pub(crate) struct Reader {
    source: Option<Source>,
    session_id: Option<String>,
    cwd: PathBuf,
    chosen: bool,
    link: Link,
    feed: Option<Feed>,
    last_poke: f64,
    answered: bool,
    pub(crate) rows: Rows,
    pub(crate) view: ViewState,
}

impl Reader {
    fn first_link(source: Option<Source>) -> Link {
        match source {
            None => Link::Unavailable(Unavailable::NoReader),
            Some(source) if !source.verified() => Link::Unavailable(Unavailable::Unverified),
            Some(_) => Link::Idle,
        }
    }

    pub(crate) fn for_agent(agent_id: &str, session_id: Option<String>, cwd: PathBuf) -> Self {
        let source = Source::from_agent_id(agent_id);
        let link = Self::first_link(source);
        Self {
            source,
            session_id,
            cwd,
            chosen: true,
            link,
            feed: None,
            last_poke: f64::NEG_INFINITY,
            answered: false,
            rows: Rows::default(),
            view: ViewState::default(),
        }
    }

    /// A workflow stage's agent takes over the tab: the old feed stops, the rows stay with a divider
    /// where the new stage begins, and the feed looks for the new agent's own transcript.
    pub(crate) fn relaunch(&mut self, agent_id: &str, session_id: Option<String>, divider: String) {
        self.feed = None;
        self.source = Source::from_agent_id(agent_id);
        self.link = Self::first_link(self.source);
        self.session_id = session_id;
        self.answered = false;
        self.rows.divider(divider);
    }

    pub(crate) fn source(&self) -> Option<Source> {
        self.source
    }

    #[cfg(test)]
    pub(crate) fn link(&self) -> Link {
        self.link
    }

    pub(crate) fn shown(&self) -> Shown {
        shown(self.chosen, self.link)
    }

    #[cfg(test)]
    pub(crate) fn wants_reader(&self) -> bool {
        self.chosen
    }

    pub(crate) fn set_wants_reader(&mut self, wants: bool) {
        self.chosen = wants;
    }

    /// The Reader chord. Returns what the tab shows afterwards.
    pub(crate) fn toggle(&mut self) -> Shown {
        self.chosen = !self.chosen;
        self.shown()
    }

    pub(crate) fn needs_start(&self) -> bool {
        self.link == Link::Idle && self.feed.is_none()
    }

    /// Starts tailing. `wake` repaints the window from the feed's thread.
    pub(crate) fn start(&mut self, home: &Path, wake: impl Fn() + Send + 'static) {
        let (Some(source), true) = (self.source, self.needs_start()) else { return };
        let mut locator = Locator::new(home, &self.cwd);
        locator.session_id.clone_from(&self.session_id);
        locator.copilot_home = std::env::var_os("COPILOT_HOME").map(PathBuf::from);
        self.feed = Some(Feed::start(source, locator, wake));
        self.link = Link::Searching;
    }

    /// The pty printed something or a hook arrived: read the file soon. At most one poke per gap.
    pub(crate) fn poke(&mut self, now: f64) {
        if now - self.last_poke < POKE_GAP_S {
            return;
        }
        if let Some(feed) = &self.feed {
            self.last_poke = now;
            feed.poke();
        }
    }

    /// The user answered the agent's prompt from the buttons. The status keeps saying "needs you"
    /// until a hook or new output says otherwise (a denied prompt sends no hook), so the buttons
    /// stay off until the next hook, or a second tap would type into the idle input.
    pub(crate) fn mark_answered(&mut self) {
        self.answered = true;
    }

    pub(crate) fn prompt_answered(&self) -> bool {
        self.answered
    }

    /// A hook always pokes, and may tell where the file is.
    pub(crate) fn hook(&mut self, transcript_path: Option<PathBuf>) {
        self.answered = false;
        if let Some(feed) = &self.feed {
            if let Some(path) = transcript_path {
                feed.hint(path);
            }
            feed.poke();
        }
    }

    /// Takes what the feed found since the last frame. Returns whether anything changed.
    pub(crate) fn drain(&mut self) -> bool {
        let Some(feed) = &self.feed else { return false };
        let messages = feed.drain();
        let changed = !messages.is_empty();
        for message in messages {
            match message {
                Message::Found => self.link = Link::Found,
                Message::Missing if self.link == Link::Searching => self.link = Link::Missing,
                Message::Missing => {}
                Message::Entries(entries) => self.add(entries),
            }
        }
        changed
    }

    fn add(&mut self, entries: Vec<Entry>) {
        self.rows.push_all(entries);
        if let Some(first) = self.rows.iter().next() {
            self.view.heights.keep_from(first.seq);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_link(&mut self, link: Link) {
        self.link = link;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_shows_only_when_wanted_and_found() {
        assert_eq!(shown(true, Link::Found), Shown::Reader);
        assert_eq!(shown(false, Link::Found), Shown::Terminal(Some(Note::ChosenRaw)));
    }

    #[test]
    fn while_the_transcript_is_not_there_the_terminal_shows_with_a_note() {
        assert_eq!(shown(true, Link::Idle), Shown::Terminal(Some(Note::Searching)));
        assert_eq!(shown(true, Link::Searching), Shown::Terminal(Some(Note::Searching)));
        assert_eq!(shown(true, Link::Missing), Shown::Terminal(Some(Note::Missing)));
        assert_eq!(shown(false, Link::Searching), Shown::Terminal(None));
    }

    #[test]
    fn an_unavailable_reader_always_shows_the_terminal_with_its_reason() {
        for wants in [true, false] {
            assert_eq!(
                shown(wants, Link::Unavailable(Unavailable::Unverified)),
                Shown::Terminal(Some(Note::Unverified))
            );
            assert_eq!(
                shown(wants, Link::Unavailable(Unavailable::NoReader)),
                Shown::Terminal(Some(Note::NoReader))
            );
        }
    }

    #[test]
    fn agents_start_in_the_reader_and_codex_and_custom_agents_cannot() {
        let claude = Reader::for_agent("claude", Some("id".into()), PathBuf::from("/w"));
        assert_eq!(claude.link(), Link::Idle);
        assert!(claude.wants_reader());
        let codex = Reader::for_agent("codex", None, PathBuf::from("/w"));
        assert_eq!(codex.link(), Link::Unavailable(Unavailable::Unverified));
        let custom = Reader::for_agent("aider", None, PathBuf::from("/w"));
        assert_eq!(custom.link(), Link::Unavailable(Unavailable::NoReader));
        assert!(!codex.needs_start() && !custom.needs_start());
    }

    #[test]
    fn the_toggle_is_per_tab() {
        let mut a = Reader::for_agent("claude", None, PathBuf::from("/a"));
        let b = Reader::for_agent("claude", None, PathBuf::from("/b"));
        a.set_link(Link::Found);
        assert_eq!(a.toggle(), Shown::Terminal(Some(Note::ChosenRaw)));
        assert!(!a.wants_reader());
        assert!(b.wants_reader());
        assert_eq!(a.toggle(), Shown::Reader);
    }

    #[test]
    fn toggling_a_tab_that_cannot_read_still_shows_the_terminal() {
        let mut codex = Reader::for_agent("codex", None, PathBuf::from("/w"));
        assert_eq!(codex.toggle(), Shown::Terminal(Some(Note::Unverified)));
        assert_eq!(codex.toggle(), Shown::Terminal(Some(Note::Unverified)));
    }

    #[test]
    fn an_answered_prompt_stays_answered_until_the_next_hook() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        assert!(!reader.prompt_answered());
        reader.mark_answered();
        assert!(reader.prompt_answered());
        reader.hook(None);
        assert!(!reader.prompt_answered());
    }

    #[test]
    fn a_reader_that_never_started_ignores_pokes_and_hooks() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        reader.poke(1.0);
        reader.hook(Some(PathBuf::from("/t.jsonl")));
        assert!(!reader.drain());
    }

    #[test]
    fn a_relaunch_keeps_the_rows_marks_the_new_stage_and_looks_for_the_new_transcript() {
        let mut reader = Reader::for_agent("claude", Some("first".into()), PathBuf::from("/w"));
        reader.set_link(Link::Found);
        reader.mark_answered();
        reader.add(vec![Entry {
            role: tidewatch_core::transcript::Role::Assistant,
            text: "built it".into(),
            tool: None,
            ts: None,
        }]);
        reader.relaunch("gemini", Some("second".into()), "--- Stage 2: Review ---".into());
        assert_eq!(reader.link(), Link::Idle);
        assert!(reader.needs_start());
        assert_eq!(reader.source(), Some(Source::Gemini));
        assert!(!reader.prompt_answered());
        let texts: Vec<_> = reader.rows.iter().map(|row| row.text.clone()).collect();
        assert_eq!(texts, ["built it", "--- Stage 2: Review ---"]);
    }

    #[test]
    fn a_relaunch_into_an_agent_without_a_reader_shows_the_terminal() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        reader.relaunch("codex", None, "--- Stage 3 ---".into());
        assert_eq!(reader.link(), Link::Unavailable(Unavailable::Unverified));
        assert!(!reader.needs_start());
        reader.relaunch("claude", None, "--- Stage 4 ---".into());
        assert_eq!(reader.link(), Link::Idle);
    }
}
