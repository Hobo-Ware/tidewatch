//! The progress ledger an agent keeps for its task: the model, a parser that never fails and the
//! writer (pure).
//!
//! The format is markdown (`# Goal`, then `##` sections; see `progress_prompt.rs`). Whatever does
//! not fit is kept: an unknown `##` section becomes an [`Extra`], an unknown line stays as text
//! under its section.

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

use crate::civil_date::Date;
use crate::kind::Kind;
use crate::progress_report::{StageKind, StageStatus, WaitingOn};

/// The longest ledger `update_progress` accepts, in bytes.
pub const MAX_LEDGER_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Working,
    NeedsYou,
    Waiting,
    Blocked,
    Done,
}

impl State {
    pub const ALL: [Self; 5] =
        [Self::Working, Self::NeedsYou, Self::Waiting, Self::Blocked, Self::Done];

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_lowercase();
        [
            ("work", Self::Working),
            ("needs", Self::NeedsYou),
            ("in progress", Self::Working),
            ("running", Self::Working),
            ("wait", Self::Waiting),
            ("block", Self::Blocked),
            ("done", Self::Done),
            ("complete", Self::Done),
            ("finish", Self::Done),
        ]
        .into_iter()
        .find_map(|(prefix, state)| text.starts_with(prefix).then_some(state))
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::NeedsYou => "needs_you",
            Self::Waiting => "waiting",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// As written: `2`, `2 Build`, `1 (switch off the old webhook)`.
    pub phase: String,
    pub state: Option<State>,
    /// What `state:` said when no state could be read from it.
    pub unknown_state: Option<String>,
    pub note: String,
    pub kind: Option<Kind>,
    /// Lines of the section that are none of the keys.
    pub other: Vec<String>,
}

impl Status {
    /// The leading number of `phase`, after an optional `phase `.
    #[must_use]
    pub fn phase_number(&self) -> Option<u32> {
        leading_number(strip_word(self.phase.trim(), "phase")).map(|(number, _)| number)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Todo,
    Doing,
    Done,
    Skipped,
    Blocked,
}

impl Mark {
    fn from_char(c: char) -> Self {
        match c {
            '~' => Self::Doing,
            'x' | 'X' => Self::Done,
            '-' => Self::Skipped,
            '!' => Self::Blocked,
            _ => Self::Todo,
        }
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "active",
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
        }
    }

    #[must_use]
    pub fn as_char(self) -> char {
        match self {
            Self::Todo => ' ',
            Self::Doing => '~',
            Self::Done => 'x',
            Self::Skipped => '-',
            Self::Blocked => '!',
        }
    }
}

/// One checklist line: `- [~] text -- why`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub mark: Mark,
    pub text: String,
    pub why: Option<String>,
}

/// One ready answer on a Needs you line: `B*` is the recommended option `B`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub text: String,
    pub recommended: bool,
}

/// One list line of Decisions, Verified, Not verified, Next or Needs you: `- text -- why`, and on a
/// Needs you line `- text -- why -- options: A | B* | C`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    pub why: Option<String>,
    pub options: Vec<Choice>,
}

impl Entry {
    /// A placeholder such as `nothing`, `(none yet)` or `Nothing so far.`
    #[must_use]
    pub fn is_nothing(&self) -> bool {
        let text = self.text.trim_start_matches(['(', ' ']).to_lowercase();
        let word = text.split(|c: char| !c.is_alphabetic()).next().unwrap_or_default();
        matches!(word, "nothing" | "none") && !text.starts_with("none of") || text == "n/a"
    }
}

/// The entries that say something, without the placeholders.
pub fn real(entries: &[Entry]) -> impl Iterator<Item = &Entry> {
    entries.iter().filter(|entry| !entry.is_nothing())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub done: usize,
    /// Everything that is not skipped.
    pub total: usize,
    pub skipped: usize,
}

impl Counts {
    fn of(items: &[Item]) -> Self {
        items.iter().fold(Self::default(), |mut counts, item| {
            match item.mark {
                Mark::Skipped => counts.skipped += 1,
                Mark::Done => {
                    counts.done += 1;
                    counts.total += 1;
                }
                Mark::Todo | Mark::Doing | Mark::Blocked => counts.total += 1,
            }
            counts
        })
    }

    fn add(self, other: Self) -> Self {
        Self {
            done: self.done + other.done,
            total: self.total + other.total,
            skipped: self.skipped + other.skipped,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseState {
    Todo,
    Doing,
    Blocked,
    Done,
    /// Every item is skipped.
    Skipped,
}

/// A `### 2. Build` heading and the items under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Phase {
    pub number: Option<u32>,
    pub title: String,
    /// What the agent named the stage's kind, from a typed report.
    pub kind: Option<StageKind>,
    /// What the agent said of the stage itself, from a typed report. It beats what the steps say.
    pub stated: Option<StageStatus>,
    pub items: Vec<Item>,
    /// Lines under the heading that are not checklist items.
    pub text: Vec<String>,
}

impl Phase {
    #[must_use]
    pub fn counts(&self) -> Counts {
        Counts::of(&self.items)
    }

    #[must_use]
    pub fn state(&self) -> PhaseState {
        let counts = self.counts();
        let any = |mark| self.items.iter().any(|item| item.mark == mark);
        match self.stated {
            Some(StageStatus::Done) => return PhaseState::Done,
            Some(StageStatus::Skipped) => return PhaseState::Skipped,
            Some(StageStatus::Todo) => return PhaseState::Todo,
            Some(StageStatus::Active) if any(Mark::Blocked) => return PhaseState::Blocked,
            Some(StageStatus::Active) => return PhaseState::Doing,
            None => {}
        }
        if any(Mark::Blocked) {
            PhaseState::Blocked
        } else if counts.total == 0 && counts.skipped > 0 {
            PhaseState::Skipped
        } else if counts.total > 0 && counts.done == counts.total {
            PhaseState::Done
        } else if any(Mark::Doing) || counts.done > 0 {
            PhaseState::Doing
        } else {
            PhaseState::Todo
        }
    }

    fn is_over(&self) -> bool {
        matches!(self.state(), PhaseState::Done | PhaseState::Skipped)
    }
}

/// When a scheduled item is due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    Date(Date),
    /// `~YYYY-MM-DD`: about then.
    About(Date),
    /// `on: <event>`: when something happens.
    On(String),
    /// `parked: <until what>`.
    Parked(String),
    /// Only the user can decide it.
    You,
    /// Whatever stood in the first column when it is none of the above.
    Unknown(String),
}

impl When {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if text.eq_ignore_ascii_case("you") {
            return Self::You;
        }
        if let Some(date) = text.strip_prefix('~') {
            return Date::parse(date.trim())
                .map_or_else(|| Self::Unknown(text.to_owned()), Self::About);
        }
        if let Some(date) = Date::parse(text) {
            return Self::Date(date);
        }
        for (prefix, build) in [("on:", Self::On as fn(String) -> Self), ("parked:", Self::Parked)]
        {
            if text.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix)) {
                let rest = text[prefix.len()..].trim();
                let dated = Self::parse(rest);
                if prefix == "on:" && dated.date().is_some() {
                    return dated;
                }
                return build(rest.to_owned());
            }
        }
        Self::Unknown(text.to_owned())
    }

    /// The date of a `Date` or an `About`.
    #[must_use]
    pub fn date(&self) -> Option<Date> {
        match self {
            Self::Date(date) | Self::About(date) => Some(*date),
            _ => None,
        }
    }

    /// A date or an about-date that is `today` or earlier.
    #[must_use]
    pub fn due_on(&self, today: Date) -> bool {
        self.days_until(today).is_some_and(|days| days <= 0)
    }

    /// Days from `today` to the date: negative when overdue. `None` without a date.
    #[must_use]
    pub fn days_until(&self, today: Date) -> Option<i64> {
        self.date().map(|date| today.days_until(date))
    }
}

impl fmt::Display for When {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(date) => write!(f, "{date}"),
            Self::About(date) => write!(f, "~{date}"),
            Self::On(event) => write!(f, "on: {event}"),
            Self::Parked(until) => write!(f, "parked: {until}"),
            Self::You => f.write_str("you"),
            Self::Unknown(text) => f.write_str(text),
        }
    }
}

impl Serialize for When {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for When {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

/// `- [ ] <when> -- <what> -- <why>` and its indented sub-steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduled {
    pub mark: Mark,
    pub when: When,
    /// The first column as the agent wrote it.
    pub when_text: String,
    pub what: String,
    pub why: Option<String>,
    pub steps: Vec<Item>,
}

impl Scheduled {
    #[must_use]
    pub fn step_counts(&self) -> Counts {
        Counts::of(&self.steps)
    }
}

/// A `##` section the format does not know. An empty name holds the text before the first heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extra {
    pub name: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    pub goal: String,
    /// The short name for the panel's heading, when the agent gave one.
    pub title: String,
    pub status: Status,
    /// What the agent waits on, from a typed report.
    pub waiting_on: Option<WaitingOn>,
    pub checklist: Vec<Phase>,
    pub decisions: Vec<Entry>,
    pub verified: Vec<Entry>,
    pub not_verified: Vec<Entry>,
    pub next: Vec<Entry>,
    pub needs_you: Vec<Entry>,
    pub scheduled: Vec<Scheduled>,
    /// Lines of the Scheduled section that are not items.
    pub scheduled_text: Vec<String>,
    pub extra: Vec<Extra>,
}

impl Ledger {
    /// Reads any text. Never fails: what is not understood is kept.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut ledger = Self::default();
        for (name, lines) in split_sections(&unfence(text)) {
            ledger.take_section(&name, &lines);
        }
        ledger
    }

    fn take_section(&mut self, name: &str, lines: &[&str]) {
        match name.to_lowercase().as_str() {
            "goal" if self.goal.is_empty() => self.goal = one_line(lines),
            "goal" => {}
            "status" => self.status.take(lines),
            "checklist" => self.take_checklist(lines),
            "decisions" => self.decisions.extend(parse_entries(lines)),
            "verified" => self.verified.extend(parse_entries(lines)),
            "not verified" => self.not_verified.extend(parse_entries(lines)),
            "next" | "next steps" => self.next.extend(parse_entries(lines)),
            "needs you" | "needs-you" => self.needs_you.extend(parse_entries(lines)),
            "scheduled" => self.take_scheduled(lines),
            _ => {
                let lines = trim_blank(lines).iter().map(|line| (*line).to_owned()).collect();
                self.extra.push(Extra { name: name.to_owned(), lines });
            }
        }
    }

    /// A second Checklist section carries on under the last phase until it has a heading of its own.
    fn take_checklist(&mut self, lines: &[&str]) {
        let mut phases = parse_checklist(lines).into_iter().peekable();
        let headless = phases.next_if(|phase| phase.number.is_none() && phase.title.is_empty());
        match (headless, self.checklist.last_mut()) {
            (Some(headless), Some(last)) => {
                last.items.extend(headless.items);
                last.text.extend(headless.text);
            }
            (Some(headless), None) => self.checklist.push(headless),
            (None, _) => {}
        }
        self.checklist.extend(phases);
    }

    fn take_scheduled(&mut self, lines: &[&str]) {
        for line in lines.iter().filter(|line| !line.trim().is_empty()) {
            let indented = line.starts_with(char::is_whitespace);
            let bullet = strip_bullet(line);
            if let Some((mark, rest)) = bullet.and_then(strip_box) {
                match self.scheduled.last_mut().filter(|_| indented) {
                    Some(last) => last.steps.push(item(mark, rest)),
                    None => self.scheduled.push(scheduled(mark, rest)),
                }
            } else if let Some(rest) = bullet.filter(|rest| !indented && starts_like_when(rest)) {
                self.scheduled.push(scheduled(Mark::Todo, rest));
            } else {
                self.scheduled_text.push(line.trim_end().to_owned());
            }
        }
    }

    /// The problems that make a ledger useless, as sentences the agent can act on.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        if self.goal.trim().is_empty() {
            vec![
                "The progress report has no goal. Start it with `# Goal` and the goal in one line below."
                    .to_owned(),
            ]
        } else {
            Vec::new()
        }
    }

    /// Done and total items over the whole checklist; skipped items are in neither.
    #[must_use]
    pub fn counts(&self) -> Counts {
        self.checklist.iter().map(Phase::counts).fold(Counts::default(), Counts::add)
    }

    /// The phase the work is in: the one `phase:` names, else the first that is not over, else the
    /// last. An index into `checklist`.
    #[must_use]
    pub fn current_phase_index(&self) -> Option<usize> {
        let phases = &self.checklist;
        let by_number = self.status.phase_number().and_then(|number| {
            phases.iter().position(|phase| phase.number == Some(number)).or_else(|| {
                let unnumbered = phases.iter().all(|phase| phase.number.is_none());
                let index = usize::try_from(number).ok()?.checked_sub(1)?;
                (unnumbered && index < phases.len()).then_some(index)
            })
        });
        let said = self.status.phase.trim().to_lowercase();
        by_number
            .or_else(|| {
                phases.iter().position(|phase| {
                    !phase.title.is_empty() && said.contains(&phase.title.to_lowercase())
                })
            })
            .or_else(|| phases.iter().position(|phase| !phase.is_over()))
            .or_else(|| phases.len().checked_sub(1))
    }

    #[must_use]
    pub fn current_phase(&self) -> Option<&Phase> {
        self.current_phase_index().and_then(|index| self.checklist.get(index))
    }

    /// Needs you holds something other than a placeholder.
    #[must_use]
    pub fn has_question(&self) -> bool {
        real(&self.needs_you).next().is_some()
    }

    /// The work is not done and the ledger holds it back on the owner: a question, a state of
    /// needs you or blocked, or a wait that names nothing but the owner.
    #[must_use]
    pub fn awaits_owner(&self) -> bool {
        match self.status.state {
            Some(State::Done) => false,
            Some(State::NeedsYou | State::Blocked) => true,
            Some(State::Waiting) => self.has_question() || self.waiting_on.is_none(),
            Some(State::Working) | None => self.has_question(),
        }
    }

    /// The Next entries of a ledger that stopped (waiting, blocked or done) with nothing open in
    /// Scheduled: work that was meant to happen later but sits where only this turn belongs.
    #[must_use]
    pub fn stranded_next(&self) -> Vec<&Entry> {
        let stopped = matches!(
            self.status.state,
            Some(State::NeedsYou | State::Waiting | State::Blocked | State::Done)
        );
        let open_scheduled =
            self.scheduled.iter().any(|item| !matches!(item.mark, Mark::Done | Mark::Skipped));
        if !stopped || open_scheduled {
            return Vec::new();
        }
        real(&self.next).collect()
    }

    /// The one state the screens show. It is the state the agent reported, except that a question
    /// or a wait that names only the owner makes it needs you, and a ledger with no state at all
    /// reads from its steps.
    #[must_use]
    pub fn shown_state(&self) -> State {
        match self.status.state {
            Some(State::Done) => State::Done,
            Some(State::Blocked) => State::Blocked,
            _ if self.awaits_owner() => State::NeedsYou,
            Some(state) => state,
            None => {
                let counts = self.counts();
                let blocked = self
                    .checklist
                    .iter()
                    .any(|phase| phase.items.iter().any(|item| item.mark == Mark::Blocked));
                if blocked {
                    State::Blocked
                } else if counts.total > 0 && counts.done == counts.total {
                    State::Done
                } else {
                    State::Working
                }
            }
        }
    }

    /// The agent says it is blocked, or an item is, and it is not done.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        match self.status.state {
            Some(State::Blocked) => true,
            Some(State::Done) => false,
            _ => self
                .checklist
                .iter()
                .any(|phase| phase.items.iter().any(|item| item.mark == Mark::Blocked)),
        }
    }

    /// The agent has not said it is done and something is still open: a step, a question, or a
    /// state of working, waiting or blocked.
    #[must_use]
    pub fn is_open(&self) -> bool {
        if self.status.state == Some(State::Done) {
            return false;
        }
        let counts = self.counts();
        self.has_question() || self.status.state.is_some() || counts.done < counts.total
    }

    fn phase_label(&self) -> Option<String> {
        let label = match self.current_phase() {
            Some(phase) => {
                let number = phase.number.map(|number| number.to_string());
                [number, Some(phase.title.clone())]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            None => strip_word(self.status.phase.trim(), "phase").to_owned(),
        };
        (!label.is_empty()).then_some(label)
    }

    /// One line for the agent's tool reply: `saved: phase 2 Build, 3/9 done, 1 scheduled`.
    #[must_use]
    pub fn summary(&self) -> String {
        let counts = self.counts();
        let parts: Vec<String> = [
            self.phase_label().map(|label| format!("phase {label}")),
            (counts.total > 0).then(|| format!("{}/{} done", counts.done, counts.total)),
            (!self.scheduled.is_empty()).then(|| format!("{} scheduled", self.scheduled.len())),
        ]
        .into_iter()
        .flatten()
        .collect();
        if parts.is_empty() { "saved".to_owned() } else { format!("saved: {}", parts.join(", ")) }
    }

    /// The canonical text: parsing it gives the same ledger back.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for extra in self.extra.iter().filter(|extra| extra.name.is_empty()) {
            push_lines(&mut out, &extra.lines);
        }
        let _ = writeln!(out, "# Goal\n{}\n", self.goal);
        self.write_status(&mut out);
        self.write_checklist(&mut out);
        for (name, entries) in [
            ("Decisions", &self.decisions),
            ("Verified", &self.verified),
            ("Not verified", &self.not_verified),
            ("Next", &self.next),
            ("Needs you", &self.needs_you),
        ] {
            if !entries.is_empty() {
                let _ = writeln!(out, "## {name}");
                for entry in entries {
                    let _ = writeln!(out, "- {}", entry_line(entry));
                }
                out.push('\n');
            }
        }
        self.write_scheduled(&mut out);
        for extra in self.extra.iter().filter(|extra| !extra.name.is_empty()) {
            let _ = writeln!(out, "## {}", extra.name);
            push_lines(&mut out, &extra.lines);
        }
        out.truncate(out.trim_end_matches('\n').len());
        out.push('\n');
        out
    }

    fn write_status(&self, out: &mut String) {
        let status = &self.status;
        let lines: Vec<String> = [
            (!status.phase.is_empty()).then(|| format!("phase: {}", status.phase)),
            status.state.map(|state| format!("state: {}", state.word())),
            status.kind.map(|kind| format!("kind: {}", kind.word())),
            (!status.note.is_empty()).then(|| format!("note: {}", status.note)),
        ]
        .into_iter()
        .flatten()
        .chain(status.other.iter().cloned())
        .collect();
        if !lines.is_empty() {
            out.push_str("## Status\n");
            push_lines(out, &lines);
        }
    }

    fn write_checklist(&self, out: &mut String) {
        if self.checklist.is_empty() {
            return;
        }
        out.push_str("## Checklist\n");
        for phase in &self.checklist {
            if phase.number.is_some() || !phase.title.is_empty() {
                let number = phase.number.map(|n| format!("{n}. ")).unwrap_or_default();
                let _ = writeln!(out, "### {number}{}", phase.title);
            }
            for item in &phase.items {
                let _ = writeln!(out, "{}", item_line(item));
            }
            for line in &phase.text {
                let _ = writeln!(out, "{line}");
            }
        }
        out.push('\n');
    }

    fn write_scheduled(&self, out: &mut String) {
        if self.scheduled.is_empty() && self.scheduled_text.is_empty() {
            return;
        }
        out.push_str("## Scheduled\n");
        for entry in &self.scheduled {
            let mut line = format!("- [{}] {}", entry.mark.as_char(), entry.when);
            for part in
                [Some(entry.what.as_str()).filter(|what| !what.is_empty()), entry.why.as_deref()]
                    .into_iter()
                    .flatten()
            {
                let _ = write!(line, " -- {part}");
            }
            let _ = writeln!(out, "{line}");
            for step in &entry.steps {
                let _ = writeln!(out, "  {}", item_line(step));
            }
        }
        push_lines(out, &self.scheduled_text);
        out.push('\n');
    }
}

impl Status {
    fn take(&mut self, lines: &[&str]) {
        for raw in lines.iter().filter(|line| !line.trim().is_empty()) {
            let line = raw.trim();
            let body = strip_bullet(line).unwrap_or(line);
            let (key, value) = body.split_once(':').unwrap_or((body, ""));
            let value = value.trim().to_owned();
            match key.trim().to_lowercase().as_str() {
                "phase" => self.phase = value,
                "state" => {
                    self.state = State::parse(&value);
                    self.unknown_state =
                        Some(value).filter(|word| self.state.is_none() && !word.is_empty());
                }
                "note" => self.note = value,
                "kind" if Kind::parse(&value).is_some() => self.kind = Kind::parse(&value),
                _ => self.other.push(raw.trim_end().to_owned()),
            }
        }
    }
}

/// The checklist of a template: the phases and items under `###` headings.
#[must_use]
pub fn checklist_phases(text: &str) -> Vec<Phase> {
    parse_checklist(&text.lines().collect::<Vec<_>>())
}

fn push_lines(out: &mut String, lines: &[String]) {
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
}

fn item_line(item: &Item) -> String {
    format!("- [{}] {}", item.mark.as_char(), with_why(&item.text, item.why.as_deref()))
}

fn entry_line(entry: &Entry) -> String {
    let line = with_why(&entry.text, entry.why.as_deref());
    if entry.options.is_empty() {
        return line;
    }
    let options: Vec<String> = entry
        .options
        .iter()
        .map(|choice| format!("{}{}", choice.text, if choice.recommended { "*" } else { "" }))
        .collect();
    format!("{line} -- {OPTIONS_LABEL} {}", options.join(" | "))
}

fn with_why(text: &str, why: Option<&str>) -> String {
    why.map_or_else(|| text.to_owned(), |why| format!("{text} -- {why}"))
}

/// Keeps what is inside the first fence that opens before any heading and closes at a fence line
/// of its own, which is how agents quote the ledger; the words around the fence go.
fn unfence(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().collect();
    let open = lines.iter().position(|line| line.trim_start().starts_with("```"));
    let close = lines.iter().rposition(|line| line.trim() == "```");
    match (open, close) {
        (Some(open), Some(close))
            if open < close && lines[..open].iter().all(|line| heading(line).is_none()) =>
        {
            lines[open + 1..close].join("\n")
        }
        _ => text.to_owned(),
    }
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let rest = line[hashes..].strip_prefix(' ')?;
    (1..=2).contains(&hashes).then(|| rest.trim().trim_end_matches(':').trim())
}

fn split_sections(text: &str) -> Vec<(String, Vec<&str>)> {
    let mut sections = vec![(String::new(), Vec::new())];
    for line in text.lines() {
        match heading(line) {
            Some(name) => sections.push((name.to_owned(), Vec::new())),
            None => {
                if let Some((_, lines)) = sections.last_mut() {
                    lines.push(line);
                }
            }
        }
    }
    sections.retain(|(name, lines)| !name.is_empty() || !trim_blank(lines).is_empty());
    sections
}

fn trim_blank<'a, 'b>(lines: &'b [&'a str]) -> &'b [&'a str] {
    let start = lines.iter().position(|line| !line.trim().is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|line| !line.trim().is_empty()).map_or(start, |i| i + 1);
    &lines[start..end]
}

fn one_line(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `- x`, `* x`, `+ x`, `1. x` or `1) x`: what follows the marker.
fn strip_bullet(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let mut chars = line.chars();
    let first = chars.next()?;
    let rest = if matches!(first, '-' | '*' | '+') {
        line[1..].strip_prefix(' ')
    } else if first.is_ascii_digit() {
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        line[digits..].strip_prefix(['.', ')']).and_then(|rest| rest.strip_prefix(' '))
    } else {
        None
    };
    rest.map(str::trim_start)
}

/// `[x] text`: the mark and the text.
fn strip_box(text: &str) -> Option<(Mark, &str)> {
    let rest = text.strip_prefix('[')?;
    let mut chars = rest.chars();
    let mark = Mark::from_char(chars.next()?);
    let rest = chars.as_str().strip_prefix(']')?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| (mark, rest.trim()))
}

/// The parts of a line cut at ` -- ` (or an em or en dash with spaces) outside backticks.
fn split_parts(text: &str, limit: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_code = false;
    for (at, c) in text.char_indices() {
        if c == '`' {
            in_code = !in_code;
        }
        if in_code || at < start || parts.len() + 1 >= limit {
            continue;
        }
        let rest = &text[at..];
        let separator = [" -- ", " \u{2014} ", " \u{2013} "]
            .into_iter()
            .find(|separator| rest.starts_with(separator));
        if let Some(separator) = separator {
            parts.push(text[start..at].trim().to_owned());
            start = at + separator.len();
        }
    }
    parts.push(text[start..].trim().to_owned());
    parts
}

fn split_why(text: &str) -> (String, Option<String>) {
    let mut parts = split_parts(text, 2).into_iter();
    let first = parts.next().unwrap_or_default();
    (first, parts.next().filter(|why| !why.is_empty()))
}

const OPTIONS_LABEL: &str = "options:";

fn parse_choices(list: &str) -> Vec<Choice> {
    list.split('|')
        .filter_map(|part| {
            let part = part.trim();
            let text = part.trim_end_matches('*').trim_end();
            let recommended = text.len() < part.len();
            (!text.is_empty()).then(|| Choice { text: text.to_owned(), recommended })
        })
        .collect()
}

/// The text, the why and the answer choices of an entry line. The choices are a last part that
/// starts with `options:`.
fn split_entry(text: &str) -> (String, Option<String>, Vec<Choice>) {
    let mut parts = split_parts(text, usize::MAX);
    let tail = parts.len() > 1
        && parts.last().is_some_and(|last| {
            last.get(..OPTIONS_LABEL.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(OPTIONS_LABEL))
        });
    if !tail {
        let (text, why) = split_why(text);
        return (text, why, Vec::new());
    }
    let options = parts.pop().map(|last| parse_choices(&last[OPTIONS_LABEL.len()..]));
    let mut rest = parts.into_iter();
    let first = rest.next().unwrap_or_default();
    let why = rest.collect::<Vec<_>>().join(" -- ");
    (first, Some(why).filter(|why| !why.is_empty()), options.unwrap_or_default())
}

fn item(mark: Mark, text: &str) -> Item {
    let (text, why) = split_why(text);
    Item { mark, text, why }
}

fn starts_like_when(text: &str) -> bool {
    let first = split_parts(text, 2).into_iter().next().unwrap_or_default();
    !matches!(When::parse(&first), When::Unknown(_))
}

fn scheduled(mark: Mark, text: &str) -> Scheduled {
    let mut parts = split_parts(text, 3).into_iter();
    let when_text = parts.next().unwrap_or_default();
    let when = When::parse(&when_text);
    let what = parts.next().unwrap_or_default();
    let why = parts.next().filter(|why| !why.is_empty());
    Scheduled { mark, when, when_text, what, why, steps: Vec::new() }
}

fn parse_entries(lines: &[&str]) -> Vec<Entry> {
    let mut raw: Vec<String> = Vec::new();
    for line in lines.iter().filter(|line| !line.trim().is_empty()) {
        let continuation = line.starts_with(char::is_whitespace) && strip_bullet(line).is_none();
        match raw.last_mut().filter(|_| continuation) {
            Some(last) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            None => raw.push(strip_bullet(line).unwrap_or_else(|| line.trim()).to_owned()),
        }
    }
    raw.iter()
        .map(|text| {
            let (text, why, options) = split_entry(text);
            Entry { text, why, options }
        })
        .collect()
}

fn parse_checklist(lines: &[&str]) -> Vec<Phase> {
    let mut phases: Vec<Phase> = Vec::new();
    for line in lines.iter().filter(|line| !line.trim().is_empty()) {
        let trimmed = line.trim();
        let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
        if hashes >= 3
            && let Some(title) = trimmed[hashes..].strip_prefix(' ')
        {
            phases.push(phase_heading(title.trim()));
            continue;
        }
        if phases.is_empty() {
            phases.push(Phase::default());
        }
        if let Some(phase) = phases.last_mut() {
            match strip_bullet(trimmed).and_then(strip_box) {
                Some((mark, rest)) => phase.items.push(item(mark, rest)),
                None => phase.text.push(line.trim_end().to_owned()),
            }
        }
    }
    phases
}

/// `2. Build`, `2) Build`, `Phase 2: Build`, `2 Build` or `Build`.
fn phase_heading(title: &str) -> Phase {
    let numbered = strip_word(title, "phase");
    let (title, number) = match leading_number(numbered) {
        Some((number, rest)) => (rest.trim_start_matches(['.', ')', ':', '-', ' ']), Some(number)),
        None => (title, None),
    };
    Phase { number, title: title.trim().to_owned(), ..Phase::default() }
}

fn strip_word<'a>(text: &'a str, word: &str) -> &'a str {
    match text.get(..word.len()) {
        Some(head) if head.eq_ignore_ascii_case(word) => text[word.len()..].trim_start(),
        _ => text,
    }
}

fn leading_number(text: &str) -> Option<(u32, &str)> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let number = text[..digits].parse().ok()?;
    Some((number, &text[digits..]))
}

#[cfg(test)]
mod tests;
