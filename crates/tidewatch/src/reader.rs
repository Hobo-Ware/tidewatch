//! One agent tab's Reader (PLAN.md, D12): which view the tab shows, the feed that tails its
//! transcript, and the rows read so far. The decision of what is shown is a pure function of the
//! user's choice and what the search found, so it is tested without a window.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tidewatch_core::folder_repos;
use tidewatch_core::link_board::Link as Shared;
use tidewatch_core::link_scan;
use tidewatch_core::question::{AskReply, Question};
use tidewatch_core::restore::codex_session_id;
use tidewatch_core::sent::{self, Record};
use tidewatch_core::transcript::{Entry, Role, Source};
use tidewatch_core::transcript_facts::Timed;
use tidewatch_core::transcript_locate::Locator;

use crate::commands::Command;
use crate::question_auto::Countdown;
use crate::reader_feed::{Feed, Message};
use crate::reader_rows::Rows;
use crate::reader_view::ViewState;
use crate::reply_store::ReplyStore;

const POKE_GAP_S: f64 = 0.3;
const SENT_LOOK_S: f64 = 1.0;
const ROLLOUT_START_SLACK: Duration = Duration::from_secs(5);

fn file_len(file: &Path) -> u64 {
    std::fs::metadata(file).map_or(0, |meta| meta.len())
}

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

/// Why a tab can never show the Reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// A custom agent: there is no parser for its files.
    NoReader,
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
        (Link::Found, true) => Shown::Reader,
        (Link::Found, false) => Shown::Terminal(Some(Note::ChosenRaw)),
        (_, false) => Shown::Terminal(None),
        (Link::Missing, true) => Shown::Terminal(Some(Note::Missing)),
        (Link::Idle | Link::Searching, true) => Shown::Terminal(Some(Note::Searching)),
    }
}

/// The view of an agent tab that has not had its first message yet: there is no transcript to find
/// until it does, so the Reader (its empty state and composer) shows whenever the user wants it,
/// unless the transcript was searched for in vain or cannot be read at all.
pub(crate) fn shown_before_first_message(wants_reader: bool, link: Link) -> Shown {
    match link {
        Link::Idle | Link::Searching | Link::Found if wants_reader => Shown::Reader,
        _ => shown(wants_reader, link),
    }
}

/// One thing typed into the agent's terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Raw key bytes, one press.
    Keys(Vec<u8>),
    /// A message pasted into the input and submitted: the Enter follows through the Enter queue.
    Paste(String),
}

/// What a tab types for the user over time. Each step waits its `gap` after the one before, because
/// an agent's input takes presses sent in one write as a single paste.
#[derive(Debug, Default)]
struct Typing {
    queue: VecDeque<(f64, Step)>,
    due: f64,
}

impl Typing {
    fn queue(&mut self, steps: Vec<(f64, Step)>, now: f64) {
        if self.queue.is_empty() {
            self.due = now + steps.first().map_or(0.0, |(gap, _)| *gap);
        }
        self.queue.extend(steps);
    }

    fn take_due(&mut self, now: f64) -> Option<Step> {
        if self.queue.is_empty() || now < self.due {
            return None;
        }
        let (_, step) = self.queue.pop_front()?;
        self.due = now + self.queue.front().map_or(0.0, |(gap, _)| *gap);
        Some(step)
    }

    fn next_due(&self) -> Option<f64> {
        (!self.queue.is_empty()).then_some(self.due)
    }
}

pub(crate) struct Reader {
    source: Option<Source>,
    session_id: Option<String>,
    cwd: PathBuf,
    started_ms: i64,
    file: Option<PathBuf>,
    chosen: bool,
    opened: Option<String>,
    link: Link,
    feed: Option<Feed>,
    last_poke: f64,
    answered: bool,
    typing: Typing,
    pub(crate) auto: Countdown,
    session_title: Option<String>,
    first_message: Option<String>,
    pub(crate) rows: Rows,
    pub(crate) view: ViewState,
    pub(crate) store: ReplyStore,
    pub(crate) menu_gone: Option<(u64, f64)>,
    sent_file: Option<PathBuf>,
    sent_len: u64,
    sent_looked: f64,
    found_links: Vec<Shared>,
    edits: Vec<PathBuf>,
    panel_facts: Vec<Timed>,
}

impl Reader {
    fn first_link(source: Option<Source>) -> Link {
        match source {
            None => Link::Unavailable(Unavailable::NoReader),
            Some(_) => Link::Idle,
        }
    }

    pub(crate) fn for_agent(agent_id: &str, session_id: Option<String>, cwd: PathBuf) -> Self {
        let source = Source::from_agent_id(agent_id);
        let link = Self::first_link(source);
        let started_ms = unix_ms_now();
        Self {
            source,
            session_id,
            cwd,
            started_ms,
            file: None,
            chosen: true,
            opened: None,
            link,
            feed: None,
            last_poke: f64::NEG_INFINITY,
            answered: false,
            typing: Typing::default(),
            auto: Countdown::default(),
            session_title: None,
            first_message: None,
            rows: Rows::default(),
            view: ViewState::started_at(started_ms),
            store: ReplyStore::default(),
            menu_gone: None,
            sent_file: None,
            sent_len: 0,
            sent_looked: f64::NEG_INFINITY,
            found_links: Vec::new(),
            edits: Vec::new(),
            panel_facts: Vec::new(),
        }
    }

    pub(crate) fn transcript(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub(crate) fn session_title(&self) -> Option<&str> {
        self.session_title.as_deref()
    }

    pub(crate) fn fake_session_title(&mut self, title: &str) {
        self.session_title = Some(title.to_owned());
    }

    pub(crate) fn first_message(&self) -> Option<&str> {
        self.first_message.as_deref()
    }

    pub(crate) fn source(&self) -> Option<Source> {
        self.source
    }

    pub(crate) fn codex_session_id(&self) -> Option<String> {
        if self.source != Some(Source::Codex) {
            return None;
        }
        self.file.as_deref().and_then(codex_session_id)
    }

    #[cfg(test)]
    pub(crate) fn link(&self) -> Link {
        self.link
    }

    pub(crate) fn shown(&self) -> Shown {
        shown(self.chosen, self.link)
    }

    pub(crate) fn shown_before_first_message(&self) -> Shown {
        shown_before_first_message(self.chosen, self.link)
    }

    pub(crate) fn available(&self) -> bool {
        !matches!(self.link, Link::Unavailable(_))
    }

    #[cfg(test)]
    pub(crate) fn wants_reader(&self) -> bool {
        self.chosen
    }

    pub(crate) fn set_wants_reader(&mut self, wants: bool) {
        self.chosen = wants;
        self.opened = None;
    }

    /// Shows the terminal because the slash command `command` opened a dialog in the CLI there.
    pub(crate) fn open_in_terminal(&mut self, command: &str) {
        self.chosen = false;
        self.opened = Some(command.to_owned());
    }

    pub(crate) fn opened(&self) -> Option<&str> {
        self.opened.as_deref()
    }

    /// The Reader chord. Returns what the tab shows afterwards.
    pub(crate) fn toggle(&mut self) -> Shown {
        self.chosen = !self.chosen;
        self.opened = None;
        self.shown()
    }

    /// `TIDEWATCH_DEV_SEE`: the Reader shows its seeded rows as if a transcript was found.
    pub(crate) fn dev_found(&mut self) {
        self.link = Link::Found;
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
        locator.codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
        locator.started_after_ms = Some(
            self.started_ms
                .saturating_sub(i64::try_from(ROLLOUT_START_SLACK.as_millis()).unwrap_or_default()),
        );
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

    /// The agent called `ask_user`: its questions wait in the Reader for an answer.
    pub(crate) fn ask_arrived(&mut self, questions: Vec<Question>, reply: AskReply) {
        self.rows.push_ask(questions, reply);
    }

    pub(crate) fn attach_sent(&mut self, file: PathBuf) {
        if self.sent_file.is_none() {
            self.sent_file = Some(file);
            self.refresh_sent();
        }
    }

    pub(crate) fn poll_sent(&mut self, now: f64) {
        if now - self.sent_looked >= SENT_LOOK_S {
            self.sent_looked = now;
            self.refresh_sent();
        }
    }

    pub(crate) fn has_sent_file(&self) -> bool {
        self.sent_file.is_some()
    }

    pub(crate) fn tidewatch_said(&mut self, record: Record) {
        self.rows.tidewatch_said(vec![record]);
        self.save_claims();
    }

    fn refresh_sent(&mut self) {
        let Some(file) = &self.sent_file else { return };
        let len = file_len(file);
        if len == self.sent_len {
            return;
        }
        self.sent_len = len;
        self.rows.tidewatch_said(sent::load(file));
        self.save_claims();
    }

    /// Writes the claims the rows made to the record file. The file counts as read only when
    /// nothing but these lines was added to it since it was loaded.
    fn save_claims(&mut self) {
        let Some(file) = &self.sent_file else { return };
        let claims = self.rows.take_new_claims();
        if claims.is_empty() {
            return;
        }
        let unread = file_len(file) != self.sent_len;
        for (id, seq) in claims {
            let _ = sent::append_claim(file, id, seq);
        }
        if !unread {
            self.sent_len = file_len(file);
        }
    }

    /// The user sent `text` while the agent was busy: it shows as queued until the transcript has it.
    pub(crate) fn queue_user(&mut self, text: &str) {
        self.rows.queue_user(text, unix_ms_now());
    }

    /// Types `steps` into the agent one after the other, starting at `now`.
    pub(crate) fn type_steps(&mut self, steps: Vec<(f64, Step)>, now: f64) {
        self.typing.queue(steps, now);
    }

    /// The next step to type, when its time has come.
    pub(crate) fn take_due_step(&mut self, now: f64) -> Option<Step> {
        self.typing.take_due(now)
    }

    /// When the next queued step is due, if any.
    pub(crate) fn next_step_due(&self) -> Option<f64> {
        self.typing.next_due()
    }

    /// A hook always pokes, and may tell where the file is.
    pub(crate) fn hook(&mut self, transcript_path: Option<PathBuf>) {
        self.answered = false;
        self.refresh_sent();
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
                Message::Found(path) => {
                    self.file = Some(path);
                    self.link = Link::Found;
                }
                Message::Missing if self.link == Link::Searching => self.link = Link::Missing,
                Message::Missing => {}
                Message::Entries(entries) => {
                    self.add(entries);
                    self.refresh_sent();
                }
                Message::Facts(facts) => self.panel_facts.extend(facts),
                Message::Title(title) => self.session_title = Some(title),
            }
        }
        changed
    }

    fn add(&mut self, entries: Vec<Entry>) {
        if self.first_message.is_none() {
            self.first_message = entries
                .iter()
                .find(|entry| entry.role == Role::User)
                .map(|entry| entry.text.clone());
        }
        self.found_links.extend(link_scan::found(&entries, unix_ms_now()));
        folder_repos::remember_edits(&mut self.edits, folder_repos::edited_paths(&entries));
        self.rows.push_all(entries);
        self.save_claims();
        if let Some(first) = self.rows.iter().next() {
            self.view.heights.keep_from(first.seq);
        }
    }

    pub(crate) fn take_panel_facts(&mut self) -> Vec<Timed> {
        std::mem::take(&mut self.panel_facts)
    }

    #[cfg(test)]
    pub(crate) fn push_panel_facts(&mut self, facts: Vec<Timed>) {
        self.panel_facts.extend(facts);
    }

    pub(crate) fn set_working(&self, working: bool) {
        if let Some(feed) = &self.feed {
            feed.set_working(working);
        }
    }

    pub(crate) fn edited_files(&self) -> &[PathBuf] {
        &self.edits
    }

    /// The artifact and GitHub links the agent wrote in the entries read since the last call.
    pub(crate) fn take_links(&mut self) -> Vec<Shared> {
        std::mem::take(&mut self.found_links)
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
    fn before_the_first_message_the_reader_shows_while_the_transcript_is_still_to_come() {
        for link in [Link::Idle, Link::Searching, Link::Found] {
            assert_eq!(shown_before_first_message(true, link), Shown::Reader, "{link:?}");
        }
        assert_eq!(shown_before_first_message(false, Link::Idle), Shown::Terminal(None));
        assert_eq!(
            shown_before_first_message(true, Link::Missing),
            Shown::Terminal(Some(Note::Missing))
        );
        assert_eq!(
            shown_before_first_message(true, Link::Unavailable(Unavailable::NoReader)),
            Shown::Terminal(Some(Note::NoReader))
        );
    }

    #[test]
    fn an_unavailable_reader_always_shows_the_terminal_with_its_reason() {
        for wants in [true, false] {
            assert_eq!(
                shown(wants, Link::Unavailable(Unavailable::NoReader)),
                Shown::Terminal(Some(Note::NoReader))
            );
        }
    }

    #[test]
    fn agents_start_in_the_reader_and_custom_agents_cannot() {
        let claude = Reader::for_agent("claude", Some("id".into()), PathBuf::from("/w"));
        assert_eq!(claude.link(), Link::Idle);
        assert!(claude.wants_reader());
        let codex = Reader::for_agent("codex", None, PathBuf::from("/w"));
        assert_eq!(codex.link(), Link::Idle);
        let custom = Reader::for_agent("aider", None, PathBuf::from("/w"));
        assert_eq!(custom.link(), Link::Unavailable(Unavailable::NoReader));
        assert!(codex.needs_start() && !custom.needs_start());
    }

    #[test]
    fn only_a_codex_reader_names_the_session_from_its_rollout_file() {
        let rollout =
            "/s/2026/10/13/rollout-2026-10-13T19-57-41-01a13669-bce5-7242-a392-fd967b70ea72.jsonl";
        let mut codex = Reader::for_agent("codex", None, PathBuf::from("/w"));
        assert_eq!(codex.codex_session_id(), None);
        codex.file = Some(PathBuf::from(rollout));
        assert_eq!(
            codex.codex_session_id().as_deref(),
            Some("01a13669-bce5-7242-a392-fd967b70ea72")
        );
        let mut claude = Reader::for_agent("claude", None, PathBuf::from("/w"));
        claude.file = Some(PathBuf::from(rollout));
        assert_eq!(claude.codex_session_id(), None);
    }

    #[test]
    fn a_command_opened_in_the_terminal_shows_it_until_the_reader_is_asked_back() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        reader.link = Link::Found;
        assert_eq!(reader.shown(), Shown::Reader);
        reader.open_in_terminal("login");
        assert_eq!(reader.opened(), Some("login"));
        assert_eq!(reader.shown(), Shown::Terminal(Some(Note::ChosenRaw)));
        reader.set_wants_reader(true);
        assert_eq!(reader.opened(), None);
        assert_eq!(reader.shown(), Shown::Reader);
        reader.open_in_terminal("usage");
        assert_eq!(reader.toggle(), Shown::Reader);
        assert_eq!(reader.opened(), None);
    }

    #[test]
    fn typed_steps_go_out_one_gap_apart_in_order() {
        let mut typing = Typing::default();
        typing.queue(
            vec![
                (0.0, Step::Keys(b"2".to_vec())),
                (0.3, Step::Keys(b"1".to_vec())),
                (0.8, Step::Paste("hi".into())),
            ],
            10.0,
        );
        assert_eq!(typing.next_due(), Some(10.0));
        assert_eq!(typing.take_due(9.9), None);
        assert_eq!(typing.take_due(10.0), Some(Step::Keys(b"2".to_vec())));
        assert_eq!(typing.next_due(), Some(10.3));
        assert_eq!(typing.take_due(10.2), None);
        assert_eq!(typing.take_due(10.5), Some(Step::Keys(b"1".to_vec())));
        assert_eq!(typing.take_due(11.0), None);
        assert_eq!(typing.take_due(11.3), Some(Step::Paste("hi".into())));
        assert_eq!((typing.take_due(99.0), typing.next_due()), (None, None));
    }

    #[test]
    fn steps_queued_while_others_wait_follow_them() {
        let mut typing = Typing::default();
        typing.queue(vec![(0.0, Step::Keys(b"a".to_vec()))], 1.0);
        typing.queue(vec![(0.3, Step::Keys(b"b".to_vec()))], 5.0);
        assert_eq!(typing.take_due(1.0), Some(Step::Keys(b"a".to_vec())));
        assert_eq!(typing.take_due(1.2), None);
        assert_eq!(typing.take_due(1.3), Some(Step::Keys(b"b".to_vec())));
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
        let mut custom = Reader::for_agent("aider", None, PathBuf::from("/w"));
        assert_eq!(custom.toggle(), Shown::Terminal(Some(Note::NoReader)));
        assert_eq!(custom.toggle(), Shown::Terminal(Some(Note::NoReader)));
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

    fn user(text: &str) -> Entry {
        Entry { role: Role::User, text: text.into(), tool: None, ts: None, images: Vec::new() }
    }

    #[test]
    fn the_first_user_message_and_the_session_title_are_kept_for_the_tab_label() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        assert_eq!((reader.first_message(), reader.session_title()), (None, None));
        let assistant = Entry { role: Role::Assistant, ..user("Looking.") };
        reader.add(vec![assistant, user("Fix the login redirect"), user("and add a test")]);
        reader.add(vec![user("a later message")]);
        assert_eq!(reader.first_message(), Some("Fix the login redirect"));
        reader.session_title = Some("Login redirect".into());
        assert_eq!(reader.session_title(), Some("Login redirect"));
        assert_eq!(reader.first_message(), Some("Fix the login redirect"));
    }

    #[test]
    fn the_agents_own_links_are_taken_once_and_the_users_never() {
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        let said = |role, text: &str| Entry {
            role,
            text: text.to_owned(),
            tool: None,
            ts: Some(5),
            images: Vec::new(),
        };
        reader.add(vec![
            said(Role::User, "look at https://claude.ai/artifact/mine"),
            said(Role::Assistant, "board: https://claude.ai/artifact/abc"),
        ]);
        let links = reader.take_links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://claude.ai/artifact/abc");
        assert_eq!(reader.take_links().len(), 0, "taken once");
    }

    #[test]
    fn the_files_the_agent_edited_are_kept_oldest_to_newest() {
        use tidewatch_core::transcript::{ToolCall, ToolPhase};
        let mut reader = Reader::for_agent("claude", None, PathBuf::from("/w"));
        let call = |name: &str, file: &str| Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: name.into(),
                summary: file.into(),
                id: None,
                phase: ToolPhase::Call,
                ask: None,
            }),
            ts: None,
            images: Vec::new(),
        };
        reader.add(vec![call("Edit", "/w/a/x.rs"), call("Read", "/w/b/y.rs")]);
        reader.add(vec![call("Write", "/w/b/z.rs"), call("Edit", "/w/a/x.rs")]);
        assert_eq!(reader.edited_files(), [PathBuf::from("/w/b/z.rs"), PathBuf::from("/w/a/x.rs")]);
    }
}
