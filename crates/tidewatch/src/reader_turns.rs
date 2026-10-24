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
    /// `6 commands`, `1 edit`, `2 other tools`, and `6 commands · 1 failed` when a call failed.
    pub(crate) fn label(&self) -> String {
        let noun = noun(self.kind, self.count);
        if self.failed == 0 {
            format!("{} {noun}", self.count)
        } else {
            format!("{} {noun} \u{b7} {} failed", self.count, self.failed)
        }
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
    /// Drawn on its own: the user's messages, notices, questions, reviews and dividers.
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

/// Groups `rows` into turns. A turn is every agent row between two user messages (or a stage
/// divider); a notice or a question inside it stays a row of its own.
pub(crate) fn layout(rows: &Rows) -> Layout {
    let mut out = Layout { slots: Vec::with_capacity(rows.len()), ..Layout::default() };
    let mut open: Option<usize> = None;
    let mut last_boundary = false;
    for (index, (row, at)) in rows.stamped().enumerate() {
        match &row.kind {
            Kind::User | Kind::Review(_) | Kind::Divider => {
                if let Some(group) = open.take() {
                    out.groups[group].closed = true;
                }
                out.slots.push(Slot::Plain);
                last_boundary = !matches!(row.kind, Kind::Divider);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::transcript::{Entry, Role, ToolCall, ToolPhase};

    fn text(role: Role, body: &str, ts: i64) -> Entry {
        Entry { role, text: body.into(), tool: None, ts: Some(ts) }
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
        assert_eq!(group.chips[0].label(), "2 commands \u{b7} 1 failed");
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
    fn a_stage_divider_starts_a_new_turn() {
        let mut rows = rows_of(vec![text(Role::Assistant, "built", 1)]);
        rows.divider("--- Stage 2 ---".into());
        rows.push(text(Role::Assistant, "reviewed", 2));
        let layout = layout(&rows);
        assert_eq!(layout.groups.len(), 2);
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
        let entry = |role, body: &str| Entry { role, text: body.into(), tool: None, ts: None };
        let layout = layout(&rows_of(vec![entry(Role::User, "a"), entry(Role::Assistant, "b")]));
        assert_eq!(layout.groups[0].elapsed_ms(), None);
        assert_eq!(span_label(None, None, &TimeZone::UTC), None);
    }

    #[test]
    fn the_chip_words_follow_the_count() {
        let chip = |kind, count, failed| Chip { kind, count, failed }.label();
        let cases = [
            (chip(ToolKind::Command, 6, 0), "6 commands"),
            (chip(ToolKind::Command, 1, 0), "1 command"),
            (chip(ToolKind::Edit, 2, 0), "2 edits"),
            (chip(ToolKind::Read, 3, 0), "3 reads"),
            (chip(ToolKind::Search, 2, 0), "2 searches"),
            (chip(ToolKind::Search, 1, 0), "1 search"),
            (chip(ToolKind::Agent, 1, 0), "1 agent"),
            (chip(ToolKind::Other, 2, 0), "2 other tools"),
            (chip(ToolKind::Edit, 3, 2), "3 edits \u{b7} 2 failed"),
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
            ("mcp__tidewatch__stage_done", ToolKind::Other),
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

    #[test]
    fn the_open_list_flips_per_turn() {
        let mut open = HashSet::new();
        toggle(&mut open, 3);
        assert!(open.contains(&3));
        toggle(&mut open, 5);
        toggle(&mut open, 3);
        assert_eq!(open.into_iter().collect::<Vec<_>>(), [5]);
    }
}
