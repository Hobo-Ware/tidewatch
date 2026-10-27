//! The Reader's turns: everything the agent did between two of the user's messages, folded into one
//! message with one row of counted chips instead of one line per tool call. Pure, no egui: the
//! grouping of rows, the chip counts and the words and times the view prints.

use std::collections::HashSet;

use jiff::tz::TimeZone;

use crate::job_model::clock_label;
use crate::reader_rows::{Kind, Row, Rows, Tool, ToolKind};

/// One chip of the summary row: how many calls of a kind, and how many of them failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    pub(crate) kind: ToolKind,
    pub(crate) count: usize,
    pub(crate) failed: usize,
}

impl Chip {
    /// `6 commands`, `1 edit`: the count and its noun, failures left out.
    pub(crate) fn counted(&self) -> String {
        format!("{} {}", self.count, noun(self.kind, self.count))
    }
}

fn noun(kind: ToolKind, count: usize) -> &'static str {
    let one = count == 1;
    match kind {
        ToolKind::Command if one => "command",
        ToolKind::Command => "commands",
        ToolKind::Edit if one => "edit",
        ToolKind::Edit => "edits",
        ToolKind::Read if one => "read",
        ToolKind::Read => "reads",
        ToolKind::Search if one => "search",
        ToolKind::Search => "searches",
        ToolKind::Agent if one => "agent",
        ToolKind::Agent => "agents",
        ToolKind::Other if one => "other tool",
        ToolKind::Other => "other tools",
    }
}

/// Set on the key of a turn to get the key of its folded updates in the set of open lists.
pub(crate) const NOTES_BIT: u64 = 1 << 63;

/// One turn of the agent: the rows it wrote, as the indices into the Reader's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    /// The first of its rows: the one the whole turn is drawn at.
    pub(crate) anchor: usize,
    /// The `seq` of that row; the key of the turn's height and of its open list.
    pub(crate) key: u64,
    pub(crate) texts: Vec<usize>,
    pub(crate) calls: Vec<usize>,
    /// The call the agent is on, or was on last: the newest one without a result, else the newest.
    pub(crate) current: Option<usize>,
    pub(crate) chips: Vec<Chip>,
    pub(crate) started_ms: Option<i64>,
    pub(crate) ended_ms: Option<i64>,
    /// A message of the user came after it, so the turn is over whatever the agent's status says.
    pub(crate) closed: bool,
}

impl Group {
    /// How long the turn took, from its first line to its last.
    pub(crate) fn elapsed_ms(&self) -> Option<i64> {
        Some((self.ended_ms? - self.started_ms?).max(0))
    }

    /// The text the bubble shows: the last one the agent wrote, finished turn or not.
    pub(crate) fn answer(&self) -> Option<usize> {
        self.texts.last().copied()
    }

    /// The texts before the answer, in order: the progress notes the agent wrote between calls.
    pub(crate) fn notes(&self) -> &[usize] {
        self.texts.split_last().map_or(&[], |(_, notes)| notes)
    }

    /// `12 updates`, `1 update`; `None` when the turn has no note to fold.
    pub(crate) fn notes_label(&self) -> Option<String> {
        let count = self.notes().len();
        (count > 0).then(|| format!("{count} {}", if count == 1 { "update" } else { "updates" }))
    }

    /// The key of the folded updates in the set of open lists.
    pub(crate) fn notes_key(&self) -> u64 {
        self.key | NOTES_BIT
    }

    /// `18 steps`, `1 step`: what the toggle chip says.
    pub(crate) fn steps_label(&self) -> String {
        let steps = self.calls.len();
        format!("{steps} {}", if steps == 1 { "step" } else { "steps" })
    }

    /// `5 commands · 2 edits · 3 reads`: the kinds of call in the turn, in the chip order.
    pub(crate) fn summary(&self) -> String {
        let parts: Vec<String> = self.chips.iter().map(Chip::counted).collect();
        parts.join(" \u{b7} ")
    }

    pub(crate) fn failed(&self) -> usize {
        self.chips.iter().map(|chip| chip.failed).sum()
    }

    fn count(&self, kind: ToolKind) -> usize {
        self.chips.iter().find(|chip| chip.kind == kind).map_or(0, |chip| chip.count)
    }

    /// `2 commands so far` while the turn runs; `None` before the first call.
    pub(crate) fn so_far(&self) -> Option<String> {
        let commands = self.count(ToolKind::Command);
        if commands > 0 {
            return Some(format!("{commands} {} so far", noun(ToolKind::Command, commands)));
        }
        let calls = self.calls.len();
        (calls > 0).then(|| format!("{calls} {} so far", if calls == 1 { "tool" } else { "tools" }))
    }
}

/// What the view does with a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Drawn on its own: the user's messages, notices, questions and reviews.
    Plain,
    /// Part of a turn drawn at its anchor.
    Hidden,
    /// The anchor of turn `n`: draws the whole turn.
    Group(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) slots: Vec<Slot>,
    pub(crate) groups: Vec<Group>,
    /// The newest row is the user's message and the agent has not answered yet.
    pub(crate) awaiting: bool,
}

/// Groups `rows` into turns. A turn is every agent row between two user messages; a notice or a
/// question inside it stays a row of its own.
pub(crate) fn layout(rows: &Rows) -> Layout {
    let mut out = Layout { slots: Vec::with_capacity(rows.len()), ..Layout::default() };
    let mut open: Option<usize> = None;
    let mut last_boundary = false;
    for (index, (row, at)) in rows.stamped().enumerate() {
        match &row.kind {
            Kind::User | Kind::Tidewatch(_) | Kind::Review(_) => {
                if let Some(group) = open.take() {
                    out.groups[group].closed = true;
                }
                out.slots.push(Slot::Plain);
                last_boundary = true;
            }
            Kind::Assistant | Kind::Tool(_) => {
                last_boundary = false;
                let at_group = *open.get_or_insert_with(|| {
                    out.groups.push(Group {
                        anchor: index,
                        key: row.seq,
                        texts: Vec::new(),
                        calls: Vec::new(),
                        current: None,
                        chips: Vec::new(),
                        started_ms: at,
                        ended_ms: at,
                        closed: false,
                    });
                    out.groups.len() - 1
                });
                let group = &mut out.groups[at_group];
                out.slots.push(if group.anchor == index {
                    Slot::Group(at_group)
                } else {
                    Slot::Hidden
                });
                group.ended_ms = at.or(group.ended_ms);
                match &row.kind {
                    Kind::Tool(tool) => add_call(group, index, tool),
                    _ => group.texts.push(index),
                }
            }
            Kind::Notice
            | Kind::SlashCommand
            | Kind::CommandOutput
            | Kind::Compacted
            | Kind::Question(_) => {
                last_boundary = false;
                out.slots.push(Slot::Plain);
            }
        }
    }
    out.awaiting = last_boundary;
    let list: Vec<&Row> = rows.iter().collect();
    settle(&mut out.groups, &list);
    out
}

fn add_call(group: &mut Group, index: usize, tool: &Tool) {
    group.calls.push(index);
    match group.chips.iter_mut().find(|chip| chip.kind == tool.kind) {
        Some(chip) => {
            chip.count += 1;
            chip.failed += usize::from(tool.error);
        }
        None => {
            group.chips.push(Chip { kind: tool.kind, count: 1, failed: usize::from(tool.error) });
        }
    }
}

fn settle(groups: &mut [Group], list: &[&Row]) {
    let unfinished = |index: &&usize| matches!(&list[**index].kind, Kind::Tool(tool) if !tool.done);
    for group in groups {
        group.current = group.calls.iter().rev().find(unfinished).or(group.calls.last()).copied();
        group.chips.sort_by_key(|chip| ToolKind::ALL.iter().position(|kind| *kind == chip.kind));
    }
}

/// Flips turn `key` in the set of turns whose list is open.
pub(crate) fn toggle(open: &mut HashSet<u64>, key: u64) {
    if !open.remove(&key) {
        open.insert(key);
    }
}

/// `34s`, `1m 12s`, `2m`, `1h 5m`.
pub(crate) fn duration_label(ms: i64) -> String {
    let secs = (ms.max(0) + 500) / 1000;
    let (hours, mins, secs) = (secs / 3600, secs % 3600 / 60, secs % 60);
    match (hours, mins, secs) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// `16:02` for one minute, `16:02 to 16:03` for a turn that spans two.
pub(crate) fn span_label(start: Option<i64>, end: Option<i64>, zone: &TimeZone) -> Option<String> {
    let from = clock_label(start?, zone)?;
    match end.and_then(|end| clock_label(end, zone)) {
        Some(to) if to != from => Some(format!("{from} to {to}")),
        _ => Some(from),
    }
}

/// What the permission card says a call asks for, in plain words.
pub(crate) fn permission_words(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command => "Run a command",
        ToolKind::Edit => "Change a file",
        ToolKind::Read => "Read a file",
        ToolKind::Search => "Search",
        ToolKind::Agent => "Start a helper agent",
        ToolKind::Other => "Use a tool",
    }
}

/// What the live pill says before the command: `Running`, `Reading`...
pub(crate) fn live_verb(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command | ToolKind::Agent | ToolKind::Other => "Running",
        ToolKind::Edit => "Editing",
        ToolKind::Read => "Reading",
        ToolKind::Search => "Searching",
    }
}

/// One line of the open list: `$ git status` for a command, `edit  src/a.rs` for the rest.
pub(crate) fn list_line(tool: &Tool, name: &str, argument: &str) -> String {
    let verb = match tool.kind {
        ToolKind::Command => return format!("$ {argument}"),
        ToolKind::Edit => "edit",
        ToolKind::Read => "read",
        ToolKind::Search => "search",
        ToolKind::Agent => "agent",
        ToolKind::Other => name,
    };
    if argument.is_empty() { verb.to_owned() } else { format!("{verb}  {argument}") }
}

/// One call of a turn as the open list reads it: its row, the tool, and the label split into the
/// tool's name and its argument (paths already shortened to the worktree).
pub(crate) struct Call<'a> {
    pub(crate) tool: &'a Tool,
    pub(crate) name: &'a str,
    pub(crate) argument: String,
}

/// What a section of the open list shows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// Each file once, in the order first met.
    Files(Vec<String>),
    /// The calls, one line each, as indices into the turn's calls.
    Lines(Vec<usize>),
    /// Other tools, one entry per server: its tools, each once with a count when repeated.
    Servers(Vec<(String, Vec<String>)>),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) kind: ToolKind,
    pub(crate) head: String,
    pub(crate) body: Body,
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The open list of a turn: a section per kind of call, in the order Read, Edit, Command, Search,
/// Agent, Other, each with a head such as `Read 3 files`.
pub(crate) fn sections(calls: &[Call<'_>]) -> Vec<Section> {
    let order = [
        ToolKind::Read,
        ToolKind::Edit,
        ToolKind::Command,
        ToolKind::Search,
        ToolKind::Agent,
        ToolKind::Other,
    ];
    let mut out = Vec::new();
    for kind in order {
        let of_kind: Vec<usize> =
            (0..calls.len()).filter(|index| calls[*index].tool.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        let (head, body) = match kind {
            ToolKind::Read | ToolKind::Edit => {
                let mut files: Vec<String> = Vec::new();
                for index in &of_kind {
                    let path = &calls[*index].argument;
                    if !path.is_empty() && !files.contains(path) {
                        files.push(path.clone());
                    }
                }
                let verb = if kind == ToolKind::Read { "Read" } else { "Edited" };
                (
                    format!("{verb} {}", plural(files.len().max(1), "file", "files")),
                    Body::Files(files),
                )
            }
            ToolKind::Command => (
                format!("Ran {}", plural(of_kind.len(), "command", "commands")),
                Body::Lines(of_kind),
            ),
            ToolKind::Search => (
                format!("Ran {}", plural(of_kind.len(), "search", "searches")),
                Body::Lines(of_kind),
            ),
            ToolKind::Agent => (
                format!("Started {}", plural(of_kind.len(), "agent", "agents")),
                Body::Lines(of_kind),
            ),
            ToolKind::Other => (
                format!("Used {}", plural(of_kind.len(), "other tool", "other tools")),
                servers(calls, &of_kind),
            ),
        };
        out.push(Section { kind, head, body });
    }
    out
}

fn servers(calls: &[Call<'_>], of_kind: &[usize]) -> Body {
    let mut list: Vec<(String, Vec<(String, usize)>)> = Vec::new();
    for index in of_kind {
        let name = calls[*index].name;
        let (server, tool) = name.split_once(':').unwrap_or((name, ""));
        let at = if let Some(at) = list.iter().position(|(known, _)| known == server) {
            at
        } else {
            list.push((server.to_owned(), Vec::new()));
            list.len() - 1
        };
        if tool.is_empty() {
            continue;
        }
        let tool = tool.replace('_', " ");
        match list[at].1.iter_mut().find(|(known, _)| *known == tool) {
            Some((_, count)) => *count += 1,
            None => list[at].1.push((tool, 1)),
        }
    }
    Body::Servers(
        list.into_iter()
            .map(|(server, tools)| {
                let tools = tools
                    .into_iter()
                    .map(|(tool, count)| if count > 1 { format!("{tool} x{count}") } else { tool })
                    .collect();
                (server, tools)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::transcript::{Entry, Role, ToolCall, ToolPhase};

    fn text(role: Role, body: &str, ts: i64) -> Entry {
        Entry { role, text: body.into(), tool: None, ts: Some(ts), images: Vec::new() }
    }

    fn call(id: &str, name: &str, summary: &str, ts: i64) -> Entry {
        Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: name.into(),
                summary: summary.into(),
                id: Some(id.into()),
                phase: ToolPhase::Call,
                ask: None,
            }),
            ts: Some(ts),
            images: Vec::new(),
        }
    }

    fn done(id: &str, name: &str, error: bool, ts: i64) -> Entry {
        Entry {
            role: Role::Tool,
            text: "out".into(),
            tool: Some(ToolCall {
                name: name.into(),
                summary: String::new(),
                id: Some(id.into()),
                phase: ToolPhase::Result { error },
                ask: None,
            }),
            ts: Some(ts),
            images: Vec::new(),
        }
    }

    fn rows_of(entries: Vec<Entry>) -> Rows {
        let mut rows = Rows::default();
        rows.push_all(entries);
        rows
    }

    fn finished_call(entries: &mut Vec<Entry>, id: &str, name: &str, error: bool, ts: i64) {
        entries.push(call(id, name, "x", ts));
        entries.push(done(id, name, error, ts));
    }

    fn chips(group: &Group) -> Vec<(ToolKind, usize, usize)> {
        group.chips.iter().map(|chip| (chip.kind, chip.count, chip.failed)).collect()
    }

    #[test]
    fn one_turn_is_one_group_with_a_chip_per_kind_in_a_fixed_order() {
        let mut entries =
            vec![text(Role::User, "go", 1_000), text(Role::Assistant, "on it", 2_000)];
        for n in 0..6 {
            finished_call(&mut entries, &format!("b{n}"), "Bash", false, 3_000);
        }
        for n in 0..3 {
            finished_call(&mut entries, &format!("r{n}"), "Read", false, 4_000);
        }
        for n in 0..2 {
            finished_call(&mut entries, &format!("e{n}"), "Edit", false, 5_000);
        }
        entries.push(text(Role::Assistant, "done", 6_000));
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups.len(), 1);
        let group = &layout.groups[0];
        assert_eq!(
            chips(group),
            [(ToolKind::Command, 6, 0), (ToolKind::Edit, 2, 0), (ToolKind::Read, 3, 0)]
        );
        assert_eq!((group.texts.len(), group.calls.len()), (2, 11));
        assert_eq!(
            (group.started_ms, group.ended_ms, group.elapsed_ms()),
            (Some(2_000), Some(6_000), Some(4_000))
        );
        assert!(!group.closed);
        assert_eq!(layout.slots[0], Slot::Plain);
        assert_eq!(layout.slots[1], Slot::Group(0));
        assert!(layout.slots[2..].iter().all(|slot| *slot == Slot::Hidden));
    }

    #[test]
    fn a_failed_call_marks_its_chip_and_only_its_chip() {
        let mut entries = vec![text(Role::User, "go", 1)];
        finished_call(&mut entries, "a", "Bash", false, 2);
        finished_call(&mut entries, "b", "Bash", true, 3);
        finished_call(&mut entries, "c", "Read", false, 4);
        let layout = layout(&rows_of(entries));
        let group = &layout.groups[0];
        assert_eq!(chips(group), [(ToolKind::Command, 2, 1), (ToolKind::Read, 1, 0)]);
        assert_eq!(group.failed(), 1);
        assert_eq!(group.summary(), "2 commands \u{b7} 1 read");
        assert_eq!(group.steps_label(), "3 steps");
    }

    #[test]
    fn a_turn_in_progress_stays_open_and_names_the_call_it_is_on() {
        let mut entries = vec![text(Role::User, "go", 1)];
        finished_call(&mut entries, "a", "Bash", false, 2);
        entries.push(call("b", "Bash", "bun test", 3));
        let rows = rows_of(entries);
        let layout = layout(&rows);
        let group = &layout.groups[0];
        assert!(!group.closed);
        let current = rows.iter().nth(group.current.unwrap()).unwrap();
        assert!(
            matches!(&current.kind, Kind::Tool(tool) if tool.label == "Bash  bun test" && !tool.done)
        );
        assert_eq!(group.so_far().as_deref(), Some("2 commands so far"));
    }

    #[test]
    fn the_current_call_is_the_newest_one_when_every_call_has_its_result() {
        let mut entries = vec![text(Role::User, "go", 1)];
        finished_call(&mut entries, "a", "Bash", false, 2);
        finished_call(&mut entries, "b", "Read", false, 3);
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups[0].current, Some(layout.groups[0].calls[1]));
    }

    #[test]
    fn the_unfinished_call_wins_over_a_newer_finished_one() {
        let entries = vec![
            text(Role::User, "go", 1),
            call("a", "Bash", "sleep 9", 2),
            call("b", "Read", "a.rs", 3),
            done("b", "Read", false, 3),
        ];
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups[0].current, Some(layout.groups[0].calls[0]));
    }

    #[test]
    fn a_turn_with_no_calls_has_no_chips() {
        let layout =
            layout(&rows_of(vec![text(Role::User, "hi", 1), text(Role::Assistant, "hello", 2)]));
        let group = &layout.groups[0];
        assert_eq!((group.chips.len(), group.calls.len(), group.so_far()), (0, 0, None));
    }

    #[test]
    fn a_user_message_ends_a_turn_and_the_next_agent_row_starts_another() {
        let entries = vec![
            text(Role::User, "one", 1),
            text(Role::Assistant, "a", 2),
            call("a", "Bash", "ls", 3),
            text(Role::User, "two", 4),
            call("b", "Edit", "x", 5),
            text(Role::Assistant, "b", 6),
        ];
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups.len(), 2);
        assert!(layout.groups[0].closed);
        assert!(!layout.groups[1].closed);
        assert_eq!(layout.groups[1].anchor, 4);
        assert_eq!(
            layout.slots,
            [Slot::Plain, Slot::Group(0), Slot::Hidden, Slot::Plain, Slot::Group(1), Slot::Hidden]
        );
    }

    #[test]
    fn a_notice_or_question_inside_a_turn_stays_a_row_and_does_not_split_it() {
        let entries = vec![
            text(Role::User, "go", 1),
            call("a", "Bash", "ls", 2),
            text(Role::System, "API error", 3),
            call("b", "Bash", "ls", 4),
        ];
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].calls.len(), 2);
        assert_eq!(layout.slots, [Slot::Plain, Slot::Group(0), Slot::Plain, Slot::Hidden]);
    }

    #[test]
    fn a_message_sent_mid_turn_ends_that_turn_and_starts_another() {
        let entries = vec![
            text(Role::User, "go", 1),
            text(Role::Assistant, "working", 2),
            call("a", "Bash", "make", 3),
            text(Role::User, "also keep the old API", 4),
            done("a", "Bash", false, 5),
            text(Role::Assistant, "kept it", 6),
        ];
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups.len(), 2);
        assert!(layout.groups[0].closed);
        assert_eq!(layout.groups[0].calls.len(), 1);
        assert_eq!(layout.groups[0].ended_ms, Some(3));
        assert_eq!(layout.groups[1].texts.len(), 1);
        assert_eq!(layout.groups[1].started_ms, Some(6));
        assert!(!layout.awaiting);
    }

    #[test]
    fn a_compaction_marker_neither_splits_a_turn_nor_opens_a_user_turn() {
        let entries = vec![
            text(Role::User, "go", 1),
            text(Role::Assistant, "working", 2),
            text(Role::Compacted, "Conversation compacted", 3),
            text(Role::Assistant, "carrying on", 4),
        ];
        let layout = layout(&rows_of(entries));
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].texts.len(), 2);
        assert_eq!(layout.slots, [Slot::Plain, Slot::Group(0), Slot::Plain, Slot::Hidden]);
        assert!(!layout.awaiting);
    }

    #[test]
    fn the_agent_has_not_answered_while_the_newest_row_is_the_users() {
        assert!(layout(&rows_of(vec![text(Role::User, "go", 1)])).awaiting);
        assert!(
            !layout(&rows_of(vec![text(Role::User, "go", 1), text(Role::Assistant, "ok", 2)]))
                .awaiting
        );
        assert!(!layout(&Rows::default()).awaiting);
    }

    #[test]
    fn lines_without_a_time_leave_the_turn_without_one() {
        let entry = |role, body: &str| Entry {
            role,
            text: body.into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        };
        let layout = layout(&rows_of(vec![entry(Role::User, "a"), entry(Role::Assistant, "b")]));
        assert_eq!(layout.groups[0].elapsed_ms(), None);
        assert_eq!(span_label(None, None, &TimeZone::UTC), None);
    }

    #[test]
    fn the_chip_words_follow_the_count() {
        let chip = |kind, count, failed| Chip { kind, count, failed }.counted();
        let cases = [
            (chip(ToolKind::Command, 6, 0), "6 commands"),
            (chip(ToolKind::Command, 1, 0), "1 command"),
            (chip(ToolKind::Edit, 2, 0), "2 edits"),
            (chip(ToolKind::Read, 3, 0), "3 reads"),
            (chip(ToolKind::Search, 2, 0), "2 searches"),
            (chip(ToolKind::Search, 1, 0), "1 search"),
            (chip(ToolKind::Agent, 1, 0), "1 agent"),
            (chip(ToolKind::Other, 2, 0), "2 other tools"),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn each_cli_s_tool_names_fall_into_a_kind() {
        let cases = [
            ("Bash", ToolKind::Command),
            ("bash", ToolKind::Command),
            ("exec", ToolKind::Command),
            ("shell", ToolKind::Command),
            ("run_shell_command", ToolKind::Command),
            ("Edit", ToolKind::Edit),
            ("MultiEdit", ToolKind::Edit),
            ("Write", ToolKind::Edit),
            ("apply_patch", ToolKind::Edit),
            ("Read", ToolKind::Read),
            ("read_file", ToolKind::Read),
            ("Grep", ToolKind::Search),
            ("Glob", ToolKind::Search),
            ("WebSearch", ToolKind::Search),
            ("Task", ToolKind::Agent),
            ("TodoWrite", ToolKind::Other),
            ("mcp__tidewatch__report_finding", ToolKind::Other),
        ];
        for (name, want) in cases {
            assert_eq!(ToolKind::of(name), want, "{name}");
        }
    }

    #[test]
    fn a_duration_reads_in_the_two_largest_units() {
        let cases = [
            (0, "0s"),
            (34_000, "34s"),
            (59_600, "1m"),
            (72_000, "1m 12s"),
            (120_000, "2m"),
            (3_900_000, "1h 5m"),
            (3_600_000, "1h"),
            (-5, "0s"),
        ];
        for (ms, want) in cases {
            assert_eq!(duration_label(ms), want, "{ms}");
        }
    }

    #[test]
    fn the_span_names_one_minute_or_two() {
        let zone = TimeZone::UTC;
        let at = |h: i64, m: i64, s: i64| Some(((h * 60 + m) * 60 + s) * 1000);
        assert_eq!(
            span_label(at(16, 2, 5), at(16, 3, 1), &zone).as_deref(),
            Some("16:02 to 16:03")
        );
        assert_eq!(span_label(at(16, 2, 5), at(16, 2, 50), &zone).as_deref(), Some("16:02"));
        assert_eq!(span_label(at(16, 2, 5), None, &zone).as_deref(), Some("16:02"));
    }

    #[test]
    fn a_list_line_is_the_command_or_the_verb_and_its_target() {
        let tool = |kind| Tool {
            kind,
            label: String::new(),
            output: String::new(),
            error: false,
            done: true,
        };
        assert_eq!(list_line(&tool(ToolKind::Command), "Bash", "git status"), "$ git status");
        assert_eq!(list_line(&tool(ToolKind::Edit), "Edit", "src/a.rs"), "edit  src/a.rs");
        assert_eq!(list_line(&tool(ToolKind::Other), "TodoWrite", ""), "TodoWrite");
    }

    fn texts_turn(notes: usize, running: bool) -> (Rows, Group) {
        let mut entries = vec![text(Role::User, "go", 1)];
        for n in 0..notes {
            entries.push(text(Role::Assistant, &format!("note {n}"), 2 + n as i64));
            finished_call(&mut entries, &format!("c{n}"), "Bash", false, 2 + n as i64);
        }
        entries.push(text(Role::Assistant, "# Answer", 90));
        if !running {
            entries.push(text(Role::User, "thanks", 99));
        }
        let rows = rows_of(entries);
        let group = layout(&rows).groups[0].clone();
        (rows, group)
    }

    #[test]
    fn a_turn_shows_its_last_text_and_folds_the_rest_whether_it_is_finished_or_running() {
        let cases = [
            (0, false, None, None),
            (0, true, None, None),
            (1, false, Some("1 update"), Some(1)),
            (1, true, Some("1 update"), Some(1)),
            (12, false, Some("12 updates"), Some(12)),
            (12, true, Some("12 updates"), Some(12)),
        ];
        for (notes, running, label, folded) in cases {
            let (rows, group) = texts_turn(notes, running);
            assert_eq!(group.closed, !running, "{notes} notes");
            assert_eq!(group.texts.len(), notes + 1, "{notes} notes");
            let list: Vec<&Row> = rows.iter().collect();
            assert_eq!(list[group.answer().unwrap()].text, "# Answer", "{notes} notes");
            assert_eq!(group.notes_label().as_deref(), label, "{notes} notes");
            assert_eq!(Some(group.notes().len()).filter(|n| *n > 0), folded, "{notes} notes");
            let order: Vec<&str> = group.notes().iter().map(|i| list[*i].text.as_str()).collect();
            let want: Vec<String> = (0..notes).map(|n| format!("note {n}")).collect();
            assert_eq!(order, want);
        }
    }

    #[test]
    fn a_turn_with_one_text_has_nothing_to_fold_and_one_with_none_has_no_answer() {
        let (_, group) = texts_turn(0, false);
        assert_eq!((group.answer(), group.notes().len(), group.notes_label()), (Some(1), 0, None));
        let mut entries = vec![text(Role::User, "go", 1)];
        finished_call(&mut entries, "a", "Bash", false, 2);
        let layout = layout(&rows_of(entries));
        let group = &layout.groups[0];
        assert_eq!((group.answer(), group.notes().len(), group.notes_label()), (None, 0, None));
    }

    #[test]
    fn the_folded_updates_open_on_their_own_key_apart_from_the_calls() {
        let (_, group) = texts_turn(3, false);
        assert_ne!(group.notes_key(), group.key);
        assert_eq!(group.notes_key() & !NOTES_BIT, group.key);
        let mut open = HashSet::new();
        toggle(&mut open, group.notes_key());
        assert!(open.contains(&group.notes_key()) && !open.contains(&group.key));
        toggle(&mut open, group.notes_key());
        assert_eq!(open.len(), 0);
    }

    #[test]
    fn the_open_list_flips_per_turn() {
        let mut open = HashSet::new();
        toggle(&mut open, 3);
        assert!(open.contains(&3));
        toggle(&mut open, 5);
        toggle(&mut open, 3);
        assert_eq!(open.into_iter().collect::<Vec<_>>(), [5]);
    }

    fn tool(kind: ToolKind, label: &str, error: bool) -> Tool {
        Tool { kind, label: label.to_owned(), output: String::new(), error, done: true }
    }

    #[test]
    fn the_open_list_groups_calls_by_kind_files_once_and_other_tools_by_server() {
        let tools = [
            (tool(ToolKind::Command, "Bash  cargo fmt", false), "Bash", "cargo fmt"),
            (tool(ToolKind::Read, "Read  src/a.rs", false), "Read", "src/a.rs"),
            (tool(ToolKind::Read, "Read  src/a.rs", false), "Read", "src/a.rs"),
            (tool(ToolKind::Read, "Read  src/b.rs", false), "Read", "src/b.rs"),
            (tool(ToolKind::Edit, "Edit  src/a.rs", false), "Edit", "src/a.rs"),
            (tool(ToolKind::Other, "x", false), "github:get_issue", ""),
            (tool(ToolKind::Other, "x", false), "github:list_comments", ""),
            (tool(ToolKind::Other, "x", false), "github:get_issue", ""),
            (tool(ToolKind::Other, "x", false), "docs:search", ""),
            (tool(ToolKind::Other, "x", false), "TodoWrite", ""),
        ];
        let calls: Vec<Call<'_>> = tools
            .iter()
            .map(|(tool, name, argument)| Call { tool, name, argument: (*argument).to_owned() })
            .collect();
        let got: Vec<(String, Body)> =
            sections(&calls).into_iter().map(|section| (section.head, section.body)).collect();
        let want = vec![
            (
                "Read 2 files".to_owned(),
                Body::Files(vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()]),
            ),
            ("Edited 1 file".to_owned(), Body::Files(vec!["src/a.rs".to_owned()])),
            ("Ran 1 command".to_owned(), Body::Lines(vec![0])),
            (
                "Used 5 other tools".to_owned(),
                Body::Servers(vec![
                    (
                        "github".to_owned(),
                        vec!["get issue x2".to_owned(), "list comments".to_owned()],
                    ),
                    ("docs".to_owned(), vec!["search".to_owned()]),
                    ("TodoWrite".to_owned(), vec![]),
                ]),
            ),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn the_permission_words_name_the_kind_of_call() {
        assert_eq!(permission_words(ToolKind::Command), "Run a command");
        assert_eq!(permission_words(ToolKind::of("mcp:tool")), "Use a tool");
    }
}
