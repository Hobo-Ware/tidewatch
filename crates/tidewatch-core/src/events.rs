//! What the listener tells the app: one [`TabMessage`] per hook payload or MCP tool call.

use crate::question::{AskReply, Question};
use crate::status::HookEvent;
use crate::workflows::Workflow;

/// Asks the UI for a redraw from a background thread. The app passes `ctx.request_repaint`.
pub type Repaint = Box<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabEvent {
    /// A hook payload that carries a status (see `status::parse_hook`).
    Hook(HookEvent),
    /// The agent called `stage_done`.
    StageDone { summary: String },
    /// The agent called `report_finding`. The app turns it into a note in the diff.
    Finding { file: String, line: u32, blocking: bool, text: String },
    /// The agent called `reply_to_note` and the reply is already saved in the tab's `notes.json`.
    NoteReply { note_id: u64, text: String },
    /// The agent called `propose_workflow` with a workflow that passed the editor's rules. The app
    /// opens it as an unsaved draft in Settings, Workflows.
    Proposal { workflow: Workflow },
    /// The agent called `ask_user` and its call is held until `reply` is used.
    Ask { questions: Vec<Question>, reply: AskReply },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabMessage {
    pub tab_id: u64,
    /// The generation the sender registered with (see `TabRegistration::generation`).
    pub generation: u64,
    pub event: TabEvent,
}
