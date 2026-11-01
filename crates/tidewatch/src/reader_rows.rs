//! The Reader's rows: transcript entries mapped to what the view draws. A tool call and its result
//! (separate lines in Claude's and Copilot's files) become one row; the oldest rows are dropped past
//! a cap so a long session keeps the memory bounded. Pure data, no egui.

use std::collections::{HashMap, HashSet, VecDeque};

use tidewatch_core::launch::MCP_SERVER_NAME;
use tidewatch_core::mcp::REPORT_TOOLS;
use tidewatch_core::note_paste;
use tidewatch_core::question::{AskReply, Question};
use tidewatch_core::transcript::{AskPart, Entry, Role, ToolPhase};
use tidewatch_core::transcript_image::Picture;
use tidewatch_core::user_markup;

use tidewatch_core::sent::{Reason, Record};

use crate::reader_sent::Sent;

pub(crate) const MAX_ROWS: usize = 400;
const MCP_PREFIX: &str = "mcp__";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    User,
    /// A user turn Tidewatch sent, matched to its record; `said_for` has the record.
    Tidewatch(Reason),
    Assistant,
    Tool(Tool),
    /// An error or notice the CLI wrote itself.
    Notice,
    /// A slash command the user ran, as `/name args`.
    SlashCommand,
    /// What a local command printed.
    CommandOutput,
    /// Where Claude Code compacted the conversation; the row's text is its label.
    Compacted,
    /// A review sent to the agent, as the ids of its notes; the row's text is the user's message.
    /// The pasted block itself is not kept.
    Review(Vec<u64>),
    /// Questions the agent asked, live until answered.
    Question(QuestionRow),
}

/// How the questions reached the Reader, and so how an answer goes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Claude Code's own question tool: the answer is typed into its menu.
    Claude,
    /// Tidewatch's `ask_user` tool: the answer goes to the held call.
    Tidewatch(AskReply),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    Pending,
    /// The answer was given and the agent has not confirmed it yet.
    Sent(Vec<(String, String)>),
    Answered(Vec<(String, String)>),
    /// The user skipped the questions, or the agent's tool reported a refusal.
    Declined,
    Ended,
    /// The menu left the agent's screen without Tidewatch answering it: it was answered or closed
    /// in the terminal.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QuestionRow {
    pub(crate) questions: Vec<Question>,
    pub(crate) asked: Asked,
    pub(crate) origin: Origin,
}

/// What a tool call does, as far as the turn chips are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ToolKind {
    Command,
    Edit,
    Read,
    Search,
    Agent,
    Other,
}

impl ToolKind {
    pub(crate) const ALL: [Self; 6] =
        [Self::Command, Self::Edit, Self::Read, Self::Search, Self::Agent, Self::Other];

    /// The kind of the tool the CLI calls `name`: Claude's `Bash`, Codex's `exec`, Gemini's
    /// `run_shell_command` and so on. A name no list knows, an MCP tool among them, is `Other`.
    pub(crate) fn of(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "bash" | "shell" | "exec" | "exec_command" | "run_shell_command" | "local_shell"
            | "shell_command" | "run_command" => Self::Command,
            "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" | "write_file"
            | "replace" | "str_replace" | "str_replace_editor" | "create" | "patch" => Self::Edit,
            "read" | "read_file" | "read_many_files" | "notebookread" | "view" => Self::Read,
            "grep"
            | "glob"
            | "ls"
            | "list_directory"
            | "search_file_content"
            | "websearch"
            | "webfetch"
            | "web_search"
            | "web_fetch"
            | "google_web_search"
            | "find" => Self::Search,
            "task" | "agent" | "subagent" => Self::Agent,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tool {
    pub(crate) kind: ToolKind,
    pub(crate) label: String,
    pub(crate) output: String,
    pub(crate) error: bool,
    /// The result has arrived.
    pub(crate) done: bool,
}

impl Tool {
    /// A call to one of Tidewatch's own report tools, however the CLI spells the server and tool
    /// together: `tidewatch:update_progress`, `mcp_tidewatch_update_progress`.
    pub(crate) fn reports(&self) -> bool {
        let name = self.label.split_once("  ").map_or(self.label.as_str(), |(name, _)| name);
        let name = name.to_ascii_lowercase();
        name.contains(MCP_SERVER_NAME) && REPORT_TOOLS.iter().any(|tool| name.ends_with(tool))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// Counts up from the first row ever pushed; stable while older rows are dropped.
    pub(crate) seq: u64,
    pub(crate) kind: Kind,
    /// Markdown for `User` and `Assistant`, plain text for `Notice`, empty for `Tool`.
    pub(crate) text: String,
    /// The pictures the user pasted into a `User` row, in order.
    pub(crate) pictures: Vec<Picture>,
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
    revision: u64,
    sent: Sent,
    auto_paused: bool,
    auto_picked: HashSet<u64>,
}

fn is_shell_command(row: &Row) -> bool {
    row.kind == Kind::SlashCommand && row.text.starts_with(user_markup::BASH_PREFIX)
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
            Role::User => {
                if let Some(review) = note_paste::parse(&entry.text) {
                    self.add(Kind::Review(review.ids), review.message);
                } else {
                    let seq = self.add(Kind::User, entry.text);
                    if !entry.images.is_empty()
                        && let Some(row) = self.row_mut(seq)
                    {
                        row.pictures = entry.images;
                    }
                    self.settle_sent();
                }
            }
            Role::Assistant => {
                self.add(Kind::Assistant, entry.text);
            }
            Role::System => {
                self.add(Kind::Notice, entry.text);
            }
            Role::Command => {
                let shell = entry.text.starts_with(user_markup::BASH_PREFIX);
                self.add(Kind::SlashCommand, entry.text);
                if shell {
                    self.settle_sent();
                }
            }
            Role::CommandOutput => {
                self.add(Kind::CommandOutput, entry.text);
            }
            Role::Compacted => {
                self.add(Kind::Compacted, entry.text);
            }
            Role::Tool => self.tool(entry),
        }
        while self.list.len() > MAX_ROWS {
            self.list.pop_front();
            self.stamps.pop_front();
            self.dropped += 1;
            self.revision += 1;
        }
    }

    pub(crate) fn tidewatch_said(&mut self, records: Vec<Record>) {
        for record in records {
            self.sent.said(record);
        }
        self.settle_sent();
        self.revision += 1;
    }

    pub(crate) fn sent_records(&self) -> &[Record] {
        self.sent.records()
    }

    pub(crate) fn set_auto_paused(&mut self, paused: bool) {
        self.auto_paused = paused;
    }

    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(crate) fn take_new_claims(&mut self) -> Vec<(u64, u64)> {
        self.sent.take_fresh()
    }

    pub(crate) fn timed(&self) -> Vec<&Record> {
        self.sent.timed().collect()
    }

    /// The user sent `text` while the agent was busy; it shows as queued until the transcript has it.
    pub(crate) fn queue_user(&mut self, text: &str, at_ms: i64) {
        self.sent.queue(text, at_ms);
        self.settle_sent();
        self.revision += 1;
    }

    /// Tidewatch answered the question in row `seq` because the user stayed away.
    pub(crate) fn mark_auto_picked(&mut self, seq: u64) {
        self.auto_picked.insert(seq);
        self.revision += 1;
    }

    pub(crate) fn auto_picked(&self, seq: u64) -> bool {
        self.auto_picked.contains(&seq)
    }

    pub(crate) fn said_for(&self, seq: u64) -> Option<&Record> {
        self.sent.said_for(seq)
    }

    pub(crate) fn told_ids(&self) -> Vec<u64> {
        self.sent.ids().collect()
    }

    pub(crate) fn told_auto_fix(&self, text: &str) -> bool {
        self.sent.told_auto_fix(text)
    }

    pub(crate) fn queued(&self) -> impl Iterator<Item = &crate::reader_sent::Queued> {
        self.sent.waiting()
    }

    pub(crate) fn queued_len(&self) -> usize {
        self.sent.waiting_len()
    }

    /// Stops showing queued message `id`: the owner sent it again or dismissed it.
    pub(crate) fn forget_queued(&mut self, id: u64) {
        if self.sent.forget_queued(id) {
            self.revision += 1;
        }
    }

    fn settle_sent(&mut self) {
        if self.sent.is_empty() {
            return;
        }
        let lines: Vec<_> = self
            .list
            .iter()
            .zip(self.stamps.iter())
            .filter(|(row, _)| {
                matches!(row.kind, Kind::User | Kind::Tidewatch(_)) || is_shell_command(row)
            })
            .map(|(row, at)| (row.seq, row.text.as_str(), *at))
            .collect();
        self.sent.settle(&lines);
        let mut changed = false;
        for row in &mut self.list {
            if !matches!(row.kind, Kind::User | Kind::Tidewatch(_)) {
                continue;
            }
            let kind =
                self.sent.said_for(row.seq).map_or(Kind::User, |said| Kind::Tidewatch(said.reason));
            if row.kind != kind {
                row.kind = kind;
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    /// Questions from an `ask_user` call: a row that waits for the user.
    pub(crate) fn push_ask(&mut self, questions: Vec<Question>, reply: AskReply) {
        let row =
            QuestionRow { questions, asked: Asked::Pending, origin: Origin::Tidewatch(reply) };
        self.add(Kind::Question(row), String::new());
        self.trim();
    }

    #[cfg(test)]
    pub(crate) fn push_question_row(&mut self, row: QuestionRow) -> u64 {
        self.add(Kind::Question(row), String::new())
    }

    /// Every question still waiting for an answer that can be given from the Reader, oldest first.
    /// A Claude menu counts only while it is the newest row: anything after it means the menu is
    /// gone.
    pub(crate) fn pending_questions(&self) -> Vec<&Row> {
        let newest = self.newest_seq();
        self.list
            .iter()
            .filter(|row| match &row.kind {
                Kind::Question(q) => {
                    q.asked == Asked::Pending
                        && (matches!(q.origin, Origin::Tidewatch(_)) || Some(row.seq) == newest)
                }
                _ => false,
            })
            .collect()
    }

    pub(crate) fn close_ended_asks(&mut self) -> bool {
        let mut closed = false;
        for row in &mut self.list {
            if let Kind::Question(q) = &mut row.kind
                && q.asked == Asked::Pending
                && matches!(&q.origin, Origin::Tidewatch(held) if held.has_ended())
            {
                q.asked = Asked::Ended;
                closed = true;
            }
        }
        if closed {
            self.revision += 1;
        }
        closed
    }

    /// The oldest of them.
    pub(crate) fn pending_question(&self) -> Option<&Row> {
        self.pending_questions().into_iter().next()
    }

    pub(crate) fn question_of(&self, seq: u64) -> Option<&QuestionRow> {
        self.list.iter().find_map(|row| match &row.kind {
            Kind::Question(question) if row.seq == seq => Some(question),
            _ => None,
        })
    }

    /// Records what happened to the question in row `seq`; false when the row is gone.
    pub(crate) fn set_asked(&mut self, seq: u64, asked: Asked) -> bool {
        match self.row_mut(seq).map(|row| &mut row.kind) {
            Some(Kind::Question(q)) => {
                q.asked = asked;
                self.revision += 1;
                true
            }
            _ => false,
        }
    }

    /// The tool call waiting for the user's permission: the newest row, when it is a tool with no
    /// result yet.
    pub(crate) fn waiting_tool(&self) -> Option<&Tool> {
        match self.list.back().map(|row| &row.kind) {
            Some(Kind::Tool(tool)) if !tool.done => Some(tool),
            _ => None,
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

    /// Changes whenever a row is added, dropped or filled in, so a view can keep what it derived
    /// from the rows until then.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// How many older rows were dropped to stay under the cap.
    pub(crate) fn dropped(&self) -> usize {
        self.dropped
    }

    fn trim(&mut self) {
        while self.list.len() > MAX_ROWS {
            self.list.pop_front();
            self.dropped += 1;
            self.revision += 1;
        }
    }

    fn add(&mut self, kind: Kind, text: String) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.revision += 1;
        self.list.push_back(Row { seq, kind, text, pictures: Vec::new() });
        self.stamps.push_back(self.stamp);
        seq
    }

    fn tool(&mut self, entry: Entry) {
        let Some(call) = entry.tool else {
            self.add(Kind::Notice, entry.text);
            return;
        };
        if let Some(part) = call.ask.clone() {
            self.ask(call.id.as_ref(), part, entry.text);
            return;
        }
        let label = tool_label(&call.name, &call.summary);
        match call.phase {
            ToolPhase::Call => {
                let tool = Tool {
                    kind: ToolKind::of(&call.name),
                    label,
                    output: String::new(),
                    error: false,
                    done: false,
                };
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
                    self.revision += 1;
                } else {
                    let tool = Tool {
                        kind: ToolKind::of(&call.name),
                        label,
                        output: entry.text,
                        error,
                        done: true,
                    };
                    self.add(Kind::Tool(tool), String::new());
                }
            }
        }
    }

    /// A question tool's call becomes a row that waits; its result settles that row.
    fn ask(&mut self, id: Option<&String>, part: AskPart, text: String) {
        let id = id.cloned().filter(|id| !id.is_empty());
        match part {
            AskPart::Asked(questions) => {
                let row = QuestionRow { questions, asked: Asked::Pending, origin: Origin::Claude };
                let seq = self.add(Kind::Question(row), String::new());
                if let Some(id) = id {
                    self.calls.insert(id, seq);
                }
            }
            AskPart::Answered(pairs) => {
                let pairs = if pairs.is_empty() { vec![(String::new(), text)] } else { pairs };
                let seq = id.and_then(|id| self.calls.remove(&id));
                self.settle_ask(seq, Asked::Answered(pairs));
            }
            AskPart::Declined => {
                let seq = id.and_then(|id| self.calls.remove(&id));
                self.settle_ask(seq, Asked::Declined);
            }
        }
    }

    fn settle_ask(&mut self, seq: Option<u64>, asked: Asked) {
        if seq.is_some_and(|seq| self.set_asked(seq, asked.clone())) {
            return;
        }
        let row = QuestionRow { questions: Vec::new(), asked, origin: Origin::Claude };
        self.add(Kind::Question(row), String::new());
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
        Entry { role, text: body.into(), tool: None, ts: None, images: Vec::new() }
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
            images: Vec::new(),
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
            images: Vec::new(),
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
            text(Role::Command, "/rate-limit-options"),
            text(Role::CommandOutput, "continues at 3pm"),
        ]);
        let kinds: Vec<_> = rows.iter().map(|row| row.kind.clone()).collect();
        assert_eq!(
            kinds,
            [Kind::User, Kind::Assistant, Kind::Notice, Kind::SlashCommand, Kind::CommandOutput]
        );
        assert_eq!(rows.iter().nth(1).unwrap().text, "# Done");
    }

    #[test]
    fn a_queued_shell_command_leaves_the_queue_when_its_bash_input_line_arrives() {
        let mut rows = Rows::default();
        rows.queue_user("! pscale branch create db media", 1_000);
        rows.queue_user("! pscale shell db", 2_000);
        assert_eq!(rows.queued_len(), 2);
        rows.push_all(vec![
            text(Role::Command, "! pscale branch create db  media"),
            text(Role::CommandOutput, "created"),
        ]);
        let waiting: Vec<&str> = rows.queued().map(|queued| queued.text.as_str()).collect();
        assert_eq!(waiting, ["! pscale shell db"]);
        let kinds: Vec<_> = rows.iter().map(|row| row.kind.clone()).collect();
        assert_eq!(kinds, [Kind::SlashCommand, Kind::CommandOutput]);
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
            span: 0,
            block: vec![],
        };
        let comment =
            kelp_core::review::Comment { author: "You".into(), time: 1, body: "rename".into() };
        let a = notes.add("a.rs", None, anchor.clone(), comment.clone());
        let b = notes.add("b.rs", None, anchor, comment);
        let threads: Vec<_> = notes.threads.iter().collect();
        let pasted = note_paste::format_review(
            &threads,
            "Start with b",
            tidewatch_core::workflow_profile::Workflow::default(),
        );
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
        assert!(rows.iter().all(|row| !row.text.contains("Review from you")));
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
            Tool {
                kind: ToolKind::Command,
                label: "Bash  ls".into(),
                output: "a.txt".into(),
                error: false,
                done: true
            }
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
    fn a_report_call_is_one_of_tidewatchs_own_report_tools_in_any_cli_spelling() {
        let tool = |label: &str| Tool {
            kind: ToolKind::Other,
            label: label.to_owned(),
            output: String::new(),
            error: false,
            done: true,
        };
        let cases = [
            ("tidewatch:update_progress  saved", true),
            ("tidewatch:update_progress", true),
            ("tidewatch:get_progress", true),
            ("tidewatch:get_workflow", true),
            ("tidewatch:report_job", true),
            ("tidewatch:set_avatar", true),
            ("tidewatch:report_finding", true),
            ("tidewatch:reply_to_note", true),
            ("mcp_tidewatch_update_progress", true),
            ("tidewatch-update_progress", true),
            ("tidewatch:ask_user", false),
            ("tidewatch:list_notes", false),
            ("tidewatch:meeseeks_done", false),
            ("other:update_progress", false),
            ("Bash  tidewatch update_progress", false),
            ("Bash", false),
        ];
        for (label, reports) in cases {
            assert_eq!(tool(label).reports(), reports, "{label}");
        }
    }

    #[test]
    fn an_mcp_tool_shows_server_and_tool() {
        assert_eq!(
            tool_label("mcp__tidewatch__report_finding", "all green"),
            "tidewatch:report_finding  all green"
        );
    }

    fn ask_entry(id: &str, phase: ToolPhase, part: AskPart) -> Entry {
        Entry {
            role: Role::Tool,
            text: "raw result".into(),
            tool: Some(ToolCall {
                name: "AskUserQuestion".into(),
                summary: String::new(),
                id: Some(id.into()),
                phase,
                ask: Some(part),
            }),
            ts: None,
            images: Vec::new(),
        }
    }

    fn one_question() -> Vec<Question> {
        use tidewatch_core::question::{QuestionOption, Select};
        vec![Question {
            text: "Where?".into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: vec![QuestionOption::new("A")],
        }]
    }

    fn asked_of(rows: &Rows, index: usize) -> Asked {
        match &rows.iter().nth(index).unwrap().kind {
            Kind::Question(q) => q.asked.clone(),
            other => panic!("not a question row: {other:?}"),
        }
    }

    #[test]
    fn a_claude_question_waits_as_a_row_until_its_result_settles_it() {
        let mut rows = Rows::default();
        rows.push(ask_entry("q1", ToolPhase::Call, AskPart::Asked(one_question())));
        assert_eq!(asked_of(&rows, 0), Asked::Pending);
        assert_eq!(rows.pending_question().map(|row| row.seq), Some(0));
        let pairs = vec![("Where?".to_owned(), "A".to_owned())];
        rows.push(ask_entry(
            "q1",
            ToolPhase::Result { error: false },
            AskPart::Answered(pairs.clone()),
        ));
        assert_eq!(rows.len(), 1);
        assert_eq!(asked_of(&rows, 0), Asked::Answered(pairs));
        assert!(rows.pending_question().is_none());
    }

    #[test]
    fn a_refused_claude_question_is_declined_and_a_result_without_its_call_still_shows() {
        let mut rows = Rows::default();
        rows.push(ask_entry("q1", ToolPhase::Call, AskPart::Asked(one_question())));
        rows.push(ask_entry("q1", ToolPhase::Result { error: true }, AskPart::Declined));
        assert_eq!(asked_of(&rows, 0), Asked::Declined);
        rows.push(ask_entry("gone", ToolPhase::Result { error: false }, AskPart::Answered(vec![])));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            asked_of(&rows, 1),
            Asked::Answered(vec![(String::new(), "raw result".to_owned())])
        );
    }

    #[test]
    fn a_claude_menu_counts_as_open_only_while_it_is_the_newest_row() {
        let mut rows = Rows::default();
        rows.push(ask_entry("q1", ToolPhase::Call, AskPart::Asked(one_question())));
        assert!(rows.pending_question().is_some());
        rows.push(text(Role::Assistant, "carrying on"));
        assert!(rows.pending_question().is_none());
    }

    #[test]
    fn an_ask_user_question_stays_open_whatever_follows_and_the_oldest_comes_first() {
        let (first, _keep_one) = std::sync::mpsc::channel();
        let (second, _keep_two) = std::sync::mpsc::channel();
        let mut rows = Rows::default();
        rows.push_ask(one_question(), AskReply::new(first));
        rows.push_ask(one_question(), AskReply::new(second));
        rows.push(text(Role::Assistant, "waiting"));
        assert_eq!(rows.pending_question().map(|row| row.seq), Some(0));
        assert!(rows.set_asked(0, Asked::Declined));
        assert_eq!(rows.pending_question().map(|row| row.seq), Some(1));
        assert!(!rows.set_asked(99, Asked::Declined));
    }

    #[test]
    fn a_question_whose_call_ended_closes_and_the_others_stay_open() {
        let (ended, _keep_ended) = std::sync::mpsc::channel();
        let (held, _keep_held) = std::sync::mpsc::channel();
        let (ended, held) = (AskReply::new(ended), AskReply::new(held));
        let mut rows = Rows::default();
        rows.push_ask(one_question(), ended.clone());
        rows.push_ask(one_question(), held);
        assert!(!rows.close_ended_asks(), "nothing ended yet");
        ended.end_mark().mark();
        assert!(rows.close_ended_asks());
        assert_eq!(asked_of(&rows, 0), Asked::Ended);
        assert_eq!(rows.pending_question().map(|row| row.seq), Some(1));
        assert!(!rows.close_ended_asks(), "closing is done once");
    }

    #[test]
    fn the_tool_waiting_for_permission_is_the_newest_row_without_a_result() {
        let mut rows = Rows::default();
        assert!(rows.waiting_tool().is_none());
        rows.push(call("t1", "Bash", "ls"));
        assert_eq!(rows.waiting_tool().map(|tool| tool.label.as_str()), Some("Bash  ls"));
        rows.push(result("t1", "Bash", "ls", "a", false));
        assert!(rows.waiting_tool().is_none());
    }

    #[test]
    fn a_row_remembers_when_its_line_was_written() {
        let mut rows = Rows::default();
        let entry = |role, ts| Entry { role, text: "x".into(), tool: None, ts, images: Vec::new() };
        rows.push(entry(Role::User, Some(1_000)));
        rows.push(entry(Role::Assistant, None));
        let stamps: Vec<_> = rows.stamped().map(|(_, at)| at).collect();
        assert_eq!(stamps, [Some(1_000), None]);
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
                images: Vec::new(),
            });
        }
        let (row, at) = rows.stamped().next().unwrap();
        assert_eq!((row.text.as_str(), at), ("3", Some(3)));
        assert_eq!(rows.stamped().count(), MAX_ROWS);
    }
}
