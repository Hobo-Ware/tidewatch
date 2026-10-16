//! The Reader's rows: transcript entries mapped to what the view draws. A tool call and its result
//! (separate lines in Claude's and Copilot's files) become one row; the oldest rows are dropped past
//! a cap so a long session keeps the memory bounded. Pure data, no egui.

use std::collections::{HashMap, VecDeque};

use tidewatch_core::note_paste;
use tidewatch_core::transcript::{Entry, Role, ToolPhase};

pub(crate) const MAX_ROWS: usize = 400;
const MCP_PREFIX: &str = "mcp__";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    User,
    Assistant,
    Tool(Tool),
    /// An error or notice the CLI wrote itself.
    Notice,
    /// A review sent to the agent, as the ids of its notes; the row's text is the user's message.
    /// The pasted block itself is not kept.
    Review(Vec<u64>),
    /// Where a workflow stage's agent took over: `--- Stage 2: Review (gemini, Read only) ---`.
    Divider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tool {
    pub(crate) label: String,
    pub(crate) output: String,
    pub(crate) error: bool,
    /// The result has arrived.
    pub(crate) done: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// Counts up from the first row ever pushed; stable while older rows are dropped.
    pub(crate) seq: u64,
    pub(crate) kind: Kind,
    /// Markdown for `User` and `Assistant`, plain text for `Notice`, empty for `Tool`.
    pub(crate) text: String,
}

#[derive(Debug, Default)]
pub(crate) struct Rows {
    list: VecDeque<Row>,
    /// Unix milliseconds of the transcript line each row came from, in step with `list`.
    stamps: VecDeque<Option<i64>>,
    stamp: Option<i64>,
    next_seq: u64,
    calls: HashMap<String, u64>,
    dropped: usize,
}

impl Rows {
    pub(crate) fn push_all(&mut self, entries: Vec<Entry>) {
        for entry in entries {
            self.push(entry);
        }
    }

    pub(crate) fn push(&mut self, entry: Entry) {
        self.stamp = entry.ts;
        match entry.role {
            Role::User => match note_paste::parse(&entry.text) {
                Some(review) => {
                    self.add(Kind::Review(review.ids), review.message);
                }
                None => {
                    self.add(Kind::User, entry.text);
                }
            },
            Role::Assistant => {
                self.add(Kind::Assistant, entry.text);
            }
            Role::System => {
                self.add(Kind::Notice, entry.text);
            }
            Role::Tool => self.tool(entry),
        }
        while self.list.len() > MAX_ROWS {
            self.list.pop_front();
            self.stamps.pop_front();
            self.dropped += 1;
        }
    }

    pub(crate) fn divider(&mut self, text: String) {
        self.stamp = None;
        self.add(Kind::Divider, text);
        while self.list.len() > MAX_ROWS {
            self.list.pop_front();
            self.stamps.pop_front();
            self.dropped += 1;
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Row> {
        self.list.iter()
    }

    /// Each row with the time its transcript line was written, when it said.
    pub(crate) fn stamped(&self) -> impl Iterator<Item = (&Row, Option<i64>)> {
        self.list.iter().zip(self.stamps.iter().copied())
    }

    pub(crate) fn newest_seq(&self) -> Option<u64> {
        self.list.back().map(|row| row.seq)
    }

    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    /// How many older rows were dropped to stay under the cap.
    pub(crate) fn dropped(&self) -> usize {
        self.dropped
    }

    fn add(&mut self, kind: Kind, text: String) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.list.push_back(Row { seq, kind, text });
        self.stamps.push_back(self.stamp);
        seq
    }

    fn tool(&mut self, entry: Entry) {
        let Some(call) = entry.tool else {
            self.add(Kind::Notice, entry.text);
            return;
        };
        let label = tool_label(&call.name, &call.summary);
        match call.phase {
            ToolPhase::Call => {
                let tool = Tool { label, output: String::new(), error: false, done: false };
                let seq = self.add(Kind::Tool(tool), String::new());
                if let Some(id) = call.id.filter(|id| !id.is_empty()) {
                    self.calls.insert(id, seq);
                }
            }
            ToolPhase::Result { error } => {
                let waiting = call
                    .id
                    .as_ref()
                    .and_then(|id| self.calls.remove(id))
                    .and_then(|seq| self.row_mut(seq))
                    .and_then(|row| match &mut row.kind {
                        Kind::Tool(tool) => Some(tool),
                        _ => None,
                    });
                if let Some(tool) = waiting {
                    tool.output = entry.text;
                    tool.error = error;
                    tool.done = true;
                } else {
                    let tool = Tool { label, output: entry.text, error, done: true };
                    self.add(Kind::Tool(tool), String::new());
                }
            }
        }
    }

    fn row_mut(&mut self, seq: u64) -> Option<&mut Row> {
        let first = self.list.front()?.seq;
        let index = usize::try_from(seq.checked_sub(first)?).ok()?;
        self.list.get_mut(index)
    }
}

/// The one line a collapsed tool call shows: the tool and its telling argument.
pub(crate) fn tool_label(name: &str, summary: &str) -> String {
    let name = friendly_name(name);
    if summary.is_empty() { name } else { format!("{name}  {summary}") }
}

/// An MCP tool is named `mcp__<server>__<tool>`; show `<server>:<tool>`.
fn friendly_name(name: &str) -> String {
    match name.strip_prefix(MCP_PREFIX) {
        Some(rest) => rest.replacen("__", ":", 1),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::transcript::ToolCall;

    fn text(role: Role, body: &str) -> Entry {
        Entry { role, text: body.into(), tool: None, ts: None }
    }

    fn call(id: &str, name: &str, summary: &str) -> Entry {
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
            ts: None,
        }
    }

    fn result(id: &str, name: &str, summary: &str, output: &str, error: bool) -> Entry {
        Entry {
            role: Role::Tool,
            text: output.into(),
            tool: Some(ToolCall {
                name: name.into(),
                summary: summary.into(),
                id: Some(id.into()),
                phase: ToolPhase::Result { error },
                ask: None,
            }),
            ts: None,
        }
    }

    fn tool_of(rows: &Rows, index: usize) -> Tool {
        match &rows.iter().nth(index).unwrap().kind {
            Kind::Tool(tool) => tool.clone(),
            other => panic!("not a tool row: {other:?}"),
        }
    }

    #[test]
    fn turns_map_to_rows_in_order() {
        let mut rows = Rows::default();
        rows.push_all(vec![
            text(Role::User, "fix it"),
            text(Role::Assistant, "# Done"),
            text(Role::System, "API error"),
        ]);
        let kinds: Vec<_> = rows.iter().map(|row| row.kind.clone()).collect();
        assert_eq!(kinds, [Kind::User, Kind::Assistant, Kind::Notice]);
        assert_eq!(rows.iter().nth(1).unwrap().text, "# Done");
    }

    #[test]
    fn a_pasted_review_becomes_one_review_row_without_its_raw_text() {
        use tidewatch_core::notes::Notes;
        let mut notes = Notes::default();
        let anchor = kelp_core::review::Anchor {
            side: kelp_core::review::Side::New,
            line_hint: 3,
            text: "x".into(),
            before: vec![],
            after: vec![],
        };
        let comment =
            kelp_core::review::Comment { author: "You".into(), time: 1, body: "rename".into() };
        let a = notes.add("a.rs", None, anchor.clone(), comment.clone());
        let b = notes.add("b.rs", None, anchor, comment);
        let threads: Vec<_> = notes.threads.iter().collect();
        let pasted = note_paste::format_review(&threads, "Start with b");
        let mut rows = Rows::default();
        rows.push_all(vec![
            text(Role::User, "fix it"),
            text(Role::User, &pasted),
            text(Role::Assistant, "on it"),
        ]);
        let kinds: Vec<_> = rows.iter().map(|row| row.kind.clone()).collect();
        assert_eq!(kinds, [Kind::User, Kind::Review(vec![a, b]), Kind::Assistant]);
        let review = rows.iter().nth(1).unwrap();
        assert_eq!(review.text, "Start with b");
        assert!(rows.iter().all(|row| !row.text.contains("Tidewatch review notes")));
    }

    #[test]
    fn a_result_fills_the_row_of_its_call() {
        let mut rows = Rows::default();
        rows.push(call("t1", "Bash", "ls"));
        assert!(!tool_of(&rows, 0).done);
        rows.push(text(Role::Assistant, "looking"));
        rows.push(result("t1", "Bash", "ls", "a.txt", false));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            tool_of(&rows, 0),
            Tool { label: "Bash  ls".into(), output: "a.txt".into(), error: false, done: true }
        );
    }

    #[test]
    fn an_error_result_marks_the_tool_row() {
        let mut rows = Rows::default();
        rows.push(call("t1", "Bash", "false"));
        rows.push(result("t1", "Bash", "false", "exit 1", true));
        assert!(tool_of(&rows, 0).error);
    }

    #[test]
    fn a_result_without_its_call_gets_a_row_of_its_own() {
        let mut rows = Rows::default();
        rows.push(result("gone", "Read", "a.rs", "fn main() {}", false));
        assert_eq!(rows.len(), 1);
        assert!(tool_of(&rows, 0).done);
    }

    #[test]
    fn rows_past_the_cap_are_dropped_oldest_first_and_counted() {
        let mut rows = Rows::default();
        for n in 0..MAX_ROWS + 25 {
            rows.push(text(Role::Assistant, &n.to_string()));
        }
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows.dropped(), 25);
        assert_eq!(rows.iter().next().unwrap().text, "25");
        assert_eq!(rows.iter().next().unwrap().seq, 25);
    }

    #[test]
    fn a_result_for_a_dropped_call_does_not_touch_another_row() {
        let mut rows = Rows::default();
        rows.push(call("old", "Bash", "ls"));
        for n in 0..MAX_ROWS {
            rows.push(text(Role::Assistant, &n.to_string()));
        }
        rows.push(result("old", "Bash", "ls", "late", false));
        let outputs = rows
            .iter()
            .filter_map(|row| match &row.kind {
                Kind::Tool(tool) => Some(tool.output.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(outputs, ["late"]);
    }

    #[test]
    fn the_tool_line_is_name_then_summary() {
        assert_eq!(tool_label("Bash", "git status"), "Bash  git status");
        assert_eq!(tool_label("Read", ""), "Read");
    }

    #[test]
    fn an_mcp_tool_shows_server_and_tool() {
        assert_eq!(
            tool_label("mcp__tidewatch__stage_done", "all green"),
            "tidewatch:stage_done  all green"
        );
    }

    #[test]
    fn a_divider_is_a_row_of_its_own_between_two_stages() {
        let mut rows = Rows::default();
        rows.push(text(Role::Assistant, "built it"));
        rows.divider("--- Stage 2: Review (gemini, Read only) ---".into());
        rows.push(text(Role::Assistant, "found a bug"));
        let kinds: Vec<_> = rows.iter().map(|row| row.kind.clone()).collect();
        assert_eq!(kinds, [Kind::Assistant, Kind::Divider, Kind::Assistant]);
        assert_eq!(rows.iter().nth(1).unwrap().text, "--- Stage 2: Review (gemini, Read only) ---");
    }

    #[test]
    fn a_row_remembers_when_its_line_was_written_and_a_divider_does_not() {
        let mut rows = Rows::default();
        let entry = |role, ts| Entry { role, text: "x".into(), tool: None, ts };
        rows.push(entry(Role::User, Some(1_000)));
        rows.push(entry(Role::Assistant, None));
        rows.divider("--- Stage 2 ---".into());
        let stamps: Vec<_> = rows.stamped().map(|(_, at)| at).collect();
        assert_eq!(stamps, [Some(1_000), None, None]);
    }

    #[test]
    fn the_times_are_dropped_with_their_rows() {
        let mut rows = Rows::default();
        for n in 0..MAX_ROWS + 3 {
            rows.push(Entry {
                role: Role::Assistant,
                text: n.to_string(),
                tool: None,
                ts: Some(n as i64),
            });
        }
        let (row, at) = rows.stamped().next().unwrap();
        assert_eq!((row.text.as_str(), at), ("3", Some(3)));
        assert_eq!(rows.stamped().count(), MAX_ROWS);
    }

    #[test]
    fn dividers_count_toward_the_cap_like_any_row() {
        let mut rows = Rows::default();
        for n in 0..=MAX_ROWS {
            rows.divider(format!("--- {n} ---"));
        }
        assert_eq!((rows.len(), rows.dropped()), (MAX_ROWS, 1));
    }
}
