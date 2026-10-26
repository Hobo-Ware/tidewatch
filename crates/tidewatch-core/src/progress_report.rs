//! The typed progress report: the contract `update_progress` takes, the checks that refuse a report
//! that breaks a rule, the JSON stored for the tab, and the mapping onto and from the [`Ledger`] the
//! screens read (pure).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::civil_date::Date;
use crate::kind::Kind;
use crate::progress::{
    Choice, Entry, Item, Ledger, Mark, Phase, PhaseState, Scheduled, State, When,
};
use crate::progress::{Status, real};
use crate::stages::{Stage, stage_of};

pub const MAX_NOW_CHARS: usize = 120;
pub const MIN_STAGES: usize = 2;
pub const MAX_STAGES: usize = 5;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 4;
const UNNAMED_STAGE: &str = "Steps";
const OPEN_NAMED: usize = 3;
const TEXT_CHARS: usize = 60;

macro_rules! word_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $word:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[must_use]
            pub fn word(self) -> &'static str {
                match self {
                    $(Self::$variant => $word),+
                }
            }

            #[must_use]
            pub fn parse(text: &str) -> Option<Self> {
                let text = text.trim().to_lowercase();
                Self::ALL.iter().copied().find(|value| value.word() == text)
            }
        }
    };
}

word_enum! {
    /// What a stage is for, whatever the agent called it.
    StageKind {
        Understand = "understand",
        Build = "build",
        Ship = "ship",
        Verify = "verify",
        Research = "research",
    }
}

word_enum! {
    StageStatus {
        Todo = "todo",
        Active = "active",
        Done = "done",
        Skipped = "skipped",
    }
}

word_enum! {
    StepStatus {
        Todo = "todo",
        Active = "active",
        Done = "done",
        Skipped = "skipped",
        Blocked = "blocked",
    }
}

impl StepStatus {
    #[must_use]
    pub fn mark(self) -> Mark {
        match self {
            Self::Todo => Mark::Todo,
            Self::Active => Mark::Doing,
            Self::Done => Mark::Done,
            Self::Skipped => Mark::Skipped,
            Self::Blocked => Mark::Blocked,
        }
    }

    #[must_use]
    pub fn of_mark(mark: Mark) -> Self {
        match mark {
            Mark::Todo => Self::Todo,
            Mark::Doing => Self::Active,
            Mark::Done => Self::Done,
            Mark::Skipped => Self::Skipped,
            Mark::Blocked => Self::Blocked,
        }
    }

    fn is_open(self) -> bool {
        matches!(self, Self::Todo | Self::Active | Self::Blocked)
    }
}

/// What the agent waits on: one of a job, a pull request, a date or an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingOn {
    Job(String),
    Pr(u64),
    Date(Date),
    Event(String),
}

impl WaitingOn {
    pub const WORDS: [&'static str; 4] = ["job", "pr", "date", "event"];

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Job(name) | Self::Event(name) => name.clone(),
            Self::Pr(number) => format!("PR #{number}"),
            Self::Date(date) => date.to_string(),
        }
    }
}

/// When later work is due, typed. `you` is only the user's call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportWhen {
    Date(Date),
    About(Date),
    Event(String),
    Parked(String),
    You(bool),
}

impl ReportWhen {
    pub const WORDS: [&'static str; 5] = ["date", "about", "event", "parked", "you"];

    #[must_use]
    pub fn to_when(&self) -> When {
        match self {
            Self::Date(date) => When::Date(*date),
            Self::About(date) => When::About(*date),
            Self::Event(event) => When::On(event.clone()),
            Self::Parked(until) => When::Parked(until.clone()),
            Self::You(_) => When::You,
        }
    }

    #[must_use]
    pub fn of_when(when: &When) -> Self {
        match when {
            When::Date(date) => Self::Date(*date),
            When::About(date) => Self::About(*date),
            When::On(event) | When::Unknown(event) => Self::Event(event.clone()),
            When::Parked(until) => Self::Parked(until.clone()),
            When::You => Self::You(true),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportStage {
    pub name: String,
    pub kind: StageKind,
    pub status: StageStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportStep {
    pub stage: String,
    pub text: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ReportStep {
    fn to_item(&self) -> Item {
        Item {
            mark: self.status.mark(),
            text: self.text.clone(),
            why: self.evidence.clone().or_else(|| self.reason.clone()),
        }
    }

    fn of_item(stage: &str, item: &Item) -> Self {
        let status = StepStatus::of_mark(item.mark);
        let (evidence, reason) = match status {
            StepStatus::Done => (item.why.clone(), None),
            _ => (None, item.why.clone()),
        };
        Self { stage: stage.to_owned(), text: item.text.clone(), status, evidence, reason }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAsk {
    pub question: String,
    pub options: Vec<String>,
    pub recommended: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportScheduled {
    pub when: ReportWhen,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// One line of verified, not verified or decisions: the thing, and the evidence, blocker or reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportNote {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl ReportNote {
    fn of_entries(entries: &[Entry]) -> Vec<Self> {
        real(entries)
            .map(|entry| Self { text: entry.text.clone(), why: entry.why.clone() })
            .collect()
    }

    fn to_entries(notes: &[Self]) -> Vec<Entry> {
        notes
            .iter()
            .map(|note| Entry {
                text: note.text.clone(),
                why: note.why.clone(),
                options: Vec::new(),
            })
            .collect()
    }
}

/// What an agent reports about its task, as `update_progress` takes it and Tidewatch stores it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressReport {
    pub goal: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub state: State,
    pub now: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    pub stages: Vec<ReportStage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ReportStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you: Option<ReportAsk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<WaitingOn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scheduled: Vec<ReportScheduled>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verified: Vec<ReportNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_verified: Vec<ReportNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<ReportNote>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn shortened(text: &str) -> String {
    match text.char_indices().nth(TEXT_CHARS) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

fn or_list(words: impl IntoIterator<Item = &'static str>) -> String {
    let words: Vec<&str> = words.into_iter().collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

impl ProgressReport {
    /// The ledger the screens read, built from the typed facts.
    #[must_use]
    pub fn to_ledger(&self) -> Ledger {
        let checklist: Vec<Phase> = self
            .stages
            .iter()
            .enumerate()
            .map(|(index, stage)| Phase {
                number: u32::try_from(index + 1).ok(),
                title: stage.name.clone(),
                kind: Some(stage.kind),
                stated: Some(stage.status),
                items: self
                    .steps
                    .iter()
                    .filter(|step| same_name(&step.stage, &stage.name))
                    .map(ReportStep::to_item)
                    .collect(),
                text: Vec::new(),
            })
            .collect();
        let phase = self
            .stages
            .iter()
            .position(|stage| stage.status == StageStatus::Active)
            .map(|index| format!("{} {}", index + 1, self.stages[index].name))
            .unwrap_or_default();
        Ledger {
            goal: self.goal.clone(),
            title: self.title.clone(),
            status: Status {
                phase,
                state: Some(self.state),
                unknown_state: None,
                note: self.now.clone(),
                kind: self.kind,
                other: Vec::new(),
            },
            waiting_on: self.waiting_on.clone(),
            checklist,
            decisions: ReportNote::to_entries(&self.decisions),
            verified: ReportNote::to_entries(&self.verified),
            not_verified: ReportNote::to_entries(&self.not_verified),
            needs_you: self.needs_you.iter().map(ReportAsk::to_entry).collect(),
            scheduled: self
                .scheduled
                .iter()
                .map(|item| {
                    let when = item.when.to_when();
                    Scheduled {
                        mark: Mark::Todo,
                        when_text: when.to_string(),
                        when,
                        what: item.text.clone(),
                        why: item.why.clone(),
                        steps: Vec::new(),
                    }
                })
                .collect(),
            ..Ledger::default()
        }
    }

    /// The report an old markdown ledger stands for: Understand, Build, PR and Done become understand, build, ship and verify. Not checked.
    #[must_use]
    pub fn from_ledger(ledger: &Ledger) -> Self {
        let current = ledger.current_phase_index();
        let mut names: Vec<String> = Vec::new();
        let mut stages = Vec::new();
        let mut steps = Vec::new();
        for (index, phase) in ledger.checklist.iter().enumerate() {
            let name = unique_name(&names, &phase.title);
            let mut status = stage_status(phase.state());
            if status == StageStatus::Todo
                && Some(index) == current
                && ledger.status.state != Some(State::Done)
            {
                status = StageStatus::Active;
            }
            stages.push(ReportStage {
                name: name.clone(),
                kind: phase.kind.unwrap_or_else(|| kind_of_title(&phase.title)),
                status,
            });
            steps.extend(phase.items.iter().map(|item| ReportStep::of_item(&name, item)));
            names.push(name);
        }
        let state = ledger.status.state.unwrap_or(State::Working);
        Self {
            goal: ledger.goal.clone(),
            title: ledger.title.clone(),
            state,
            now: now_of(ledger),
            kind: ledger.status.kind,
            stages,
            steps,
            needs_you: real(&ledger.needs_you).next().map(ReportAsk::of_entry),
            waiting_on: ledger.waiting_on.clone(),
            scheduled: ledger
                .scheduled
                .iter()
                .filter(|item| !matches!(item.mark, Mark::Done | Mark::Skipped))
                .map(|item| ReportScheduled {
                    when: ReportWhen::of_when(&item.when),
                    text: item.what.clone(),
                    why: item.why.clone(),
                })
                .collect(),
            verified: ReportNote::of_entries(&ledger.verified),
            not_verified: ReportNote::of_entries(&ledger.not_verified),
            decisions: ReportNote::of_entries(&ledger.decisions),
        }
    }

    /// The report as pretty JSON, which is what `get_progress` returns.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The text of a ledger step that is still open, for the reply of a refused `done`.
    fn open_steps(&self) -> Vec<&ReportStep> {
        self.steps.iter().filter(|step| step.status.is_open()).collect()
    }

    /// Soft problems of a report that was saved: lines that read after the saved line.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let unfinished = (self.state == State::Done)
            .then(|| {
                self.stages
                    .iter()
                    .find(|stage| matches!(stage.status, StageStatus::Todo | StageStatus::Active))
            })
            .flatten()
            .map(|stage| {
                format!(
                    "Warning: state is done but the stage `{}` is not done or skipped. Done means the whole task is finished: mark every stage done or skipped.",
                    shortened(&stage.name)
                )
            });
        let no_steps = self.steps.is_empty().then(|| {
            "Warning: no steps. List the concrete steps of this task under your stages.".to_owned()
        });
        [unfinished, no_steps].into_iter().flatten().collect()
    }
}

impl ReportAsk {
    fn to_entry(&self) -> Entry {
        Entry {
            text: self.question.clone(),
            why: self.why.clone(),
            options: self
                .options
                .iter()
                .enumerate()
                .map(|(index, text)| Choice {
                    text: text.clone(),
                    recommended: index == self.recommended,
                })
                .collect(),
        }
    }

    fn of_entry(entry: &Entry) -> Self {
        Self {
            question: entry.text.clone(),
            options: entry.options.iter().map(|choice| choice.text.clone()).collect(),
            recommended: entry.options.iter().position(|choice| choice.recommended).unwrap_or(0),
            why: entry.why.clone(),
        }
    }
}

fn unique_name(taken: &[String], title: &str) -> String {
    let base = if title.trim().is_empty() { UNNAMED_STAGE } else { title.trim() };
    let mut name = base.to_owned();
    let mut copy = 2;
    while taken.iter().any(|other| same_name(other, &name)) {
        name = format!("{base} ({copy})");
        copy += 1;
    }
    name
}

fn stage_status(state: PhaseState) -> StageStatus {
    match state {
        PhaseState::Todo => StageStatus::Todo,
        PhaseState::Doing | PhaseState::Blocked => StageStatus::Active,
        PhaseState::Done => StageStatus::Done,
        PhaseState::Skipped => StageStatus::Skipped,
    }
}

/// The four fixed stages of the old format map one to one: Understand, Build, PR, Done.
fn kind_of_title(title: &str) -> StageKind {
    match stage_of(title) {
        Some(Stage::Understand) => StageKind::Understand,
        Some(Stage::Answer) => StageKind::Research,
        Some(Stage::Pr) => StageKind::Ship,
        Some(Stage::Done) => StageKind::Verify,
        Some(Stage::Build) | None => StageKind::Build,
    }
}

/// The note, else the step in progress, else the first step to do, else the goal, cut to `now`'s limit.
fn now_of(ledger: &Ledger) -> String {
    let items = || ledger.checklist.iter().flat_map(|phase| &phase.items);
    let said = ledger.status.note.trim();
    let text = if said.is_empty() {
        items()
            .find(|item| item.mark == Mark::Doing)
            .or_else(|| items().find(|item| item.mark == Mark::Todo))
            .map_or_else(|| ledger.goal.trim().to_owned(), |item| item.text.trim().to_owned())
    } else {
        said.to_owned()
    };
    cut(&text, MAX_NOW_CHARS)
}

fn cut(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let head: String = text.chars().take(chars - 3).collect();
    format!("{}...", head.trim_end())
}

#[derive(Deserialize, Default)]
struct RawReport {
    goal: Option<String>,
    title: Option<String>,
    state: Option<String>,
    now: Option<String>,
    kind: Option<String>,
    stages: Option<Vec<RawStage>>,
    steps: Option<Vec<RawStep>>,
    needs_you: Option<RawAsk>,
    waiting_on: Option<Map<String, Value>>,
    scheduled: Option<Vec<RawScheduled>>,
    verified: Option<Vec<RawNote>>,
    not_verified: Option<Vec<RawNote>>,
    decisions: Option<Vec<RawNote>>,
    #[serde(flatten)]
    unknown: Map<String, Value>,
}

#[derive(Deserialize)]
struct RawStage {
    name: Option<String>,
    kind: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
struct RawStep {
    stage: Option<String>,
    text: Option<String>,
    status: Option<String>,
    evidence: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct RawAsk {
    question: Option<String>,
    options: Option<Vec<String>>,
    recommended: Option<i64>,
    why: Option<String>,
}

#[derive(Deserialize)]
struct RawScheduled {
    when: Option<Map<String, Value>>,
    text: Option<String>,
    why: Option<String>,
}

#[derive(Deserialize)]
struct RawNote {
    text: Option<String>,
    why: Option<String>,
}

fn filled(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned()).filter(|text| !text.is_empty())
}

fn word_of<T: Copy>(
    what: &str,
    given: Option<String>,
    words: impl IntoIterator<Item = &'static str> + Clone,
    parse: impl Fn(&str) -> Option<T>,
    problems: &mut Vec<String>,
) -> Option<T> {
    let Some(text) = filled(given) else {
        problems.push(format!("{what} is required: one of {}", or_list(words)));
        return None;
    };
    let parsed = parse(&text);
    if parsed.is_none() {
        problems.push(format!("{what} \"{}\" is not one of {}", shortened(&text), or_list(words)));
    }
    parsed
}

fn single_key<'a>(
    what: &str,
    map: &'a Map<String, Value>,
    words: &[&'static str],
    problems: &mut Vec<String>,
) -> Option<(&'a str, &'a Value)> {
    let mut entries = map.iter();
    let found = entries.next().filter(|(key, _)| words.contains(&key.as_str()));
    if let (Some((key, value)), None) = (found, entries.next()) {
        return Some((key.as_str(), value));
    }
    problems.push(format!(
        "{what} must hold exactly one of {}, such as {{\"{}\": ...}}",
        or_list(words.iter().copied()),
        words[0]
    ));
    None
}

fn text_value(what: &str, value: &Value, problems: &mut Vec<String>) -> Option<String> {
    let text = value.as_str().map(str::trim).filter(|text| !text.is_empty());
    if text.is_none() {
        problems.push(format!("{what} must be a non-empty string"));
    }
    text.map(str::to_owned)
}

fn date_value(what: &str, value: &Value, problems: &mut Vec<String>) -> Option<Date> {
    let date = value.as_str().and_then(|text| Date::parse(text.trim()));
    if date.is_none() {
        problems.push(format!("{what} must be a date written YYYY-MM-DD"));
    }
    date
}

fn check_waiting(map: &Map<String, Value>, problems: &mut Vec<String>) -> Option<WaitingOn> {
    let what = "waiting_on";
    let (key, value) = single_key(what, map, &WaitingOn::WORDS, problems)?;
    match key {
        "job" => text_value("waiting_on.job", value, problems).map(WaitingOn::Job),
        "event" => text_value("waiting_on.event", value, problems).map(WaitingOn::Event),
        "date" => date_value("waiting_on.date", value, problems).map(WaitingOn::Date),
        _ => {
            let number = value.as_u64().filter(|number| *number > 0);
            if number.is_none() {
                problems.push("waiting_on.pr must be a pull request number".to_owned());
            }
            number.map(WaitingOn::Pr)
        }
    }
}

fn check_when(
    at: usize,
    map: &Map<String, Value>,
    problems: &mut Vec<String>,
) -> Option<ReportWhen> {
    let what = format!("scheduled[{at}].when");
    let (key, value) = single_key(&what, map, &ReportWhen::WORDS, problems)?;
    let field = format!("{what}.{key}");
    match key {
        "date" => date_value(&field, value, problems).map(ReportWhen::Date),
        "about" => date_value(&field, value, problems).map(ReportWhen::About),
        "event" => text_value(&field, value, problems).map(ReportWhen::Event),
        "parked" => text_value(&field, value, problems).map(ReportWhen::Parked),
        _ => Some(ReportWhen::You(true)),
    }
}

fn check_notes(
    what: &str,
    raw: Option<Vec<RawNote>>,
    problems: &mut Vec<String>,
) -> Vec<ReportNote> {
    raw.into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(at, note)| {
            let text = filled(note.text);
            if text.is_none() {
                problems.push(format!("{what}[{at}].text is required"));
            }
            Some(ReportNote { text: text?, why: filled(note.why) })
        })
        .collect()
}

fn check_stages(raw: Option<Vec<RawStage>>, problems: &mut Vec<String>) -> Vec<ReportStage> {
    let Some(raw) = raw else {
        problems.push(format!(
            "stages is required: {MIN_STAGES} to {MAX_STAGES} stages, named to fit this task"
        ));
        return Vec::new();
    };
    if !(MIN_STAGES..=MAX_STAGES).contains(&raw.len()) {
        problems.push(format!(
            "stages has {}: send {MIN_STAGES} to {MAX_STAGES}, named to fit this task. Work after a merge, such as a deploy or a backfill, is its own stage",
            raw.len()
        ));
    }
    let mut stages: Vec<ReportStage> = Vec::new();
    for (at, stage) in raw.into_iter().enumerate() {
        let name = filled(stage.name);
        if name.is_none() {
            problems.push(format!("stages[{at}].name is required"));
        }
        let kind = word_of(
            &format!("stages[{at}].kind"),
            stage.kind,
            StageKind::ALL.iter().map(|kind| kind.word()),
            StageKind::parse,
            problems,
        );
        let status = word_of(
            &format!("stages[{at}].status"),
            stage.status,
            StageStatus::ALL.iter().map(|status| status.word()),
            StageStatus::parse,
            problems,
        );
        if let Some(name) = &name
            && stages.iter().any(|other| same_name(&other.name, name))
        {
            problems.push(format!("stages[{at}].name \"{}\" is used twice", shortened(name)));
        }
        if let (Some(name), Some(kind), Some(status)) = (name, kind, status) {
            stages.push(ReportStage { name, kind, status });
        }
    }
    stages
}

fn check_steps(
    raw: Option<Vec<RawStep>>,
    names: &[String],
    problems: &mut Vec<String>,
) -> Vec<ReportStep> {
    let mut steps = Vec::new();
    for (at, step) in raw.into_iter().flatten().enumerate() {
        let text = filled(step.text);
        if text.is_none() {
            problems.push(format!("steps[{at}].text is required"));
        }
        let status = word_of(
            &format!("steps[{at}].status"),
            step.status,
            StepStatus::ALL.iter().map(|status| status.word()),
            StepStatus::parse,
            problems,
        );
        let wanted = filled(step.stage);
        let stage = wanted
            .as_deref()
            .and_then(|wanted| names.iter().find(|name| same_name(name, wanted)).cloned());
        match (&wanted, &stage) {
            (None, _) => problems
                .push(format!("steps[{at}].stage is required: the name of one of your stages")),
            (Some(wanted), None) => problems.push(format!(
                "steps[{at}].stage \"{}\" is not one of your stages ({})",
                shortened(wanted),
                names.join(", ")
            )),
            (Some(_), Some(_)) => {}
        }
        if let (Some(text), Some(status), Some(stage)) = (text, status, stage) {
            steps.push(ReportStep {
                stage,
                text,
                status,
                evidence: filled(step.evidence),
                reason: filled(step.reason),
            });
        }
    }
    steps
}

fn check_ask(raw: RawAsk, problems: &mut Vec<String>) -> Option<ReportAsk> {
    let question = filled(raw.question);
    if question.is_none() {
        problems.push("needs_you.question is required".to_owned());
    }
    let options: Vec<String> = raw
        .options
        .unwrap_or_default()
        .into_iter()
        .map(|option| option.trim().to_owned())
        .collect();
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len()) {
        problems.push(format!(
            "needs_you.options has {}: send {MIN_OPTIONS} to {MAX_OPTIONS} short answers the user can click",
            options.len()
        ));
    }
    if options.iter().any(String::is_empty) {
        problems.push("needs_you.options must not hold an empty answer".to_owned());
    }
    let recommended = raw.recommended.and_then(|at| usize::try_from(at).ok());
    if recommended.is_none_or(|at| at >= options.len()) {
        problems.push(format!(
            "needs_you.recommended must be the index of the answer you recommend, 0 to {}",
            options.len().saturating_sub(1)
        ));
    }
    Some(ReportAsk {
        question: question?,
        options,
        recommended: recommended?,
        why: filled(raw.why),
    })
}

fn check_scheduled(
    raw: Option<Vec<RawScheduled>>,
    problems: &mut Vec<String>,
) -> Vec<ReportScheduled> {
    raw.into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(at, item)| {
            let text = filled(item.text);
            if text.is_none() {
                problems.push(format!("scheduled[{at}].text is required"));
            }
            let when = item.when.as_ref().and_then(|map| check_when(at, map, problems));
            if item.when.is_none() {
                problems.push(format!(
                    "scheduled[{at}].when is required: one of {}",
                    or_list(ReportWhen::WORDS)
                ));
            }
            Some(ReportScheduled { when: when?, text: text?, why: filled(item.why) })
        })
        .collect()
}

impl ProgressReport {
    /// Every rule an `update_progress` call breaks, as one fix per sentence; `saved` lends the goal and title a report leaves out.
    pub fn check(args: &Map<String, Value>, saved: Option<&Self>) -> Result<Self, Vec<String>> {
        let raw: RawReport = match serde_json::from_value(Value::Object(args.clone())) {
            Ok(raw) => raw,
            Err(e) => return Err(vec![format!("the arguments do not match the schema: {e}")]),
        };
        let mut problems: Vec<String> =
            raw.unknown.keys().map(|key| format!("unknown argument \"{key}\"")).collect();
        let goal = filled(raw.goal)
            .or_else(|| saved.map(|saved| saved.goal.clone()).filter(|goal| !goal.is_empty()));
        let title = filled(raw.title).or_else(|| saved.map(|saved| saved.title.clone()));
        if goal.is_none() && title.is_none() {
            problems.push("goal is required on the first report: the task in one line".to_owned());
        }
        let state = word_of(
            "state",
            raw.state,
            State::ALL.iter().map(|state| state.word()),
            parse_state,
            &mut problems,
        );
        let now = filled(raw.now);
        match &now {
            None => problems.push("now is required: one plain sentence on what is happening right now".to_owned()),
            Some(now) if now.chars().count() > MAX_NOW_CHARS => problems.push(format!(
                "now is {} characters, over the {MAX_NOW_CHARS} limit: say it in one short sentence",
                now.chars().count()
            )),
            Some(_) => {}
        }
        let kind = raw.kind.as_deref().and_then(|word| {
            let kind = Kind::parse(word);
            if kind.is_none() {
                problems.push(format!("kind \"{}\" is not read-only or edit", shortened(word)));
            }
            kind
        });
        let names: Vec<String> =
            raw.stages.iter().flatten().filter_map(|stage| filled(stage.name.clone())).collect();
        let stages = check_stages(raw.stages, &mut problems);
        let steps = check_steps(raw.steps, &names, &mut problems);
        let needs_you = raw.needs_you.and_then(|ask| check_ask(ask, &mut problems));
        let asked =
            needs_you.is_some() || problems.iter().any(|problem| problem.starts_with("needs_you"));
        let waiting_on = raw.waiting_on.and_then(|map| check_waiting(&map, &mut problems));
        let waits = waiting_on.is_some()
            || problems.iter().any(|problem| problem.starts_with("waiting_on"));
        let scheduled = check_scheduled(raw.scheduled, &mut problems);
        let verified = check_notes("verified", raw.verified, &mut problems);
        let not_verified = check_notes("not_verified", raw.not_verified, &mut problems);
        let decisions = check_notes("decisions", raw.decisions, &mut problems);
        match (state, asked) {
            (Some(State::NeedsYou), false) => problems.push("state is needs_you, so needs_you is required: a question with 2 to 4 options and the index of the one you recommend".to_owned()),
            (Some(state), true) if state != State::NeedsYou => problems.push(format!(
                "needs_you is set but state is {}: set state to needs_you, or drop needs_you",
                state.word()
            )),
            _ => {}
        }
        match (state, waits) {
            (Some(State::Waiting), false) => problems.push("state is waiting, so waiting_on is required: one of job, pr, date or event, such as {\"job\": \"backfill\"}".to_owned()),
            (Some(State::Done), true) => problems.push("waiting_on is set but state is done: a task that waits is not done. Set state to waiting, or drop waiting_on".to_owned()),
            _ => {}
        }
        let mut report = Self {
            goal: goal.unwrap_or_default(),
            title: title.unwrap_or_default(),
            state: state.unwrap_or(State::Working),
            now: now.unwrap_or_default(),
            kind,
            stages,
            steps,
            needs_you,
            waiting_on,
            scheduled,
            verified,
            not_verified,
            decisions,
        };
        if state == Some(State::Done) {
            let open = report.open_steps();
            if !open.is_empty() {
                let named: Vec<String> = open
                    .iter()
                    .take(OPEN_NAMED)
                    .map(|step| format!("\"{}\" ({})", shortened(&step.text), step.status.word()))
                    .collect();
                let more = open.len().saturating_sub(OPEN_NAMED);
                let more = if more > 0 { format!(" and {more} more") } else { String::new() };
                problems.push(format!(
                    "state is done but {} step{} still open: {}{more}. Finish them, mark them skipped with a reason, or do not report done",
                    open.len(),
                    if open.len() == 1 { " is" } else { "s are" },
                    named.join(", ")
                ));
            }
        }
        if problems.is_empty() {
            report.steps.shrink_to_fit();
            Ok(report)
        } else {
            Err(problems)
        }
    }
}

fn parse_state(text: &str) -> Option<State> {
    let text = text.trim().to_lowercase().replace(['-', ' '], "_");
    State::ALL.iter().copied().find(|state| state.word() == text)
}

#[cfg(test)]
mod tests;
