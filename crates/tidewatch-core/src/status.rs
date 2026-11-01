//! Agent status (PLAN.md, D5): one `AgentStatus` per tab, folded from three layers.
//!
//! 1. Hook events the agent CLI posts to the listener ([`parse_hook`] turns a payload into a
//!    [`HookEvent`]).
//! 2. Terminal signals from the pty (OSC 9;4 progress, title change, bell), fed in as plain values.
//! 3. Process facts: alive, exit code, time since the last output.
//!
//! The fold is [`resolve`], a pure function of [`StatusInputs`] and the current time in seconds.
//! [`StatusTracker`] keeps the inputs up to date. Time is always passed in, never read.
//!
//! Rules, highest first:
//! - A tab with no agent is `Shell`; one still preparing its worktree is `SettingUp`.
//! - A dead process wins over everything: exit code 0 is `Done`, anything else `Failed`.
//! - A hook state wins over the terminal and process layers, but it decays: `Working` with no output
//!   for `STATUS_QUIET_SECS` becomes `NeedsYou(Quiet)` (the agent may be waiting on something no hook
//!   reported), and a permission or question prompt flips back to `Working` once the terminal is
//!   printing again (no hook reports the answer).
//! - A permission holds until its own tool call ends; one that names no call waits on those in flight.
//! - A call that starts while the state is `Done` or `Failed` reads `Working`; a late end never does.
//! - An idle notice (`Notification` `idle_prompt`) means "at the prompt", so it reads `Done`; only a
//!   permission prompt, a question or an explicit ask reads `NeedsYou`.
//! - With no hook state, progress, bell, title activity and quiet time decide.
//!
//! Claude's `SessionStart` http hook never fired in `claude` 2.1.296 (interactive or `-p`), so the
//! first Claude status comes from `UserPromptSubmit` or from the lower layers.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Output silence after which an agent is assumed to be waiting for you (PLAN.md, Constants).
pub const STATUS_QUIET_SECS: f64 = 8.0;
/// A hung CLI never clears its progress bar, so the bar alone cannot hold `Working` forever.
pub const BUSY_QUIET_SECS: f64 = 1800.0;
pub const REST_AFTER_SECS: f64 = 600.0;
/// A prompt hook state is trusted for this long before output can show the agent moved on.
pub const HOOK_SETTLE_SECS: f64 = 2.0;
/// Output newer than this counts as "printing right now".
pub const ACTIVE_WINDOW_SECS: f64 = 1.5;
/// A bell counts as unanswered until the terminal prints something at least this long after it.
pub const BELL_SETTLE_SECS: f64 = 1.0;

const MESSAGE_MAX_CHARS: usize = 400;
/// A hook's last message is kept nearly whole: the Done / Needs you / Next block sits at its end.
const TURN_MESSAGE_MAX_CHARS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    /// A tool call waits for approval.
    Permission,
    /// The agent asked a question.
    Question,
    /// The agent is waiting for your next message.
    Input,
    /// The terminal rang the bell.
    Attention,
    /// No hook said anything and the terminal went quiet.
    Quiet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    SettingUp,
    Working,
    NeedsYou(Reason),
    Done,
    Failed(String),
    /// An agent tab whose agent is not running: its resume command waits at a shell prompt.
    Stopped,
    Shell,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookKind {
    SessionStart,
    TurnStarted,
    PermissionNeeded,
    /// A permission prompt for one identified tool call (the key from [`tool_key`]).
    ToolPermission(String),
    /// `id` is the CLI's `tool_use_id` when it sends one; `key` is what the prompt and the end carry.
    ToolStarted {
        key: String,
        id: Option<String>,
    },
    ToolFinished {
        key: String,
        id: Option<String>,
    },
    QuestionAsked,
    IdleWaiting,
    TurnDone,
    TurnFailed(String),
    SessionEnd,
}

/// What one hook payload (or Copilot transcript event) says, in CLI-neutral terms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEvent {
    pub kind: HookKind,
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    /// The agent's last message on `TurnDone`, the prompt text on a permission or question, the error
    /// on `TurnFailed`.
    pub message: Option<String>,
}

impl HookEvent {
    fn bare(kind: HookKind) -> Self {
        Self { kind, session_id: None, transcript_path: None, message: None }
    }
}

/// Turns one hook payload into a [`HookEvent`]. `None` for events that carry no status (tool hooks,
/// `auth_success` notifications) and for payloads that are not objects.
///
/// A Copilot payload without `hook_event_name` is one of its `events.jsonl` lines.
#[must_use]
pub fn parse_hook(agent: &str, payload: &Value) -> Option<HookEvent> {
    let object = payload.as_object()?;
    let mut event = if agent == "copilot" && !object.contains_key("hook_event_name") {
        parse_copilot_event(object)?
    } else {
        parse_named_event(agent, object)?
    };
    event.session_id = event.session_id.or_else(|| text(object.get("session_id")));
    event.transcript_path =
        event.transcript_path.or_else(|| text(object.get("transcript_path")).map(PathBuf::from));
    Some(event)
}

fn parse_named_event(agent: &str, object: &serde_json::Map<String, Value>) -> Option<HookEvent> {
    let name = object.get("hook_event_name")?.as_str()?;
    let message = |key: &str| text(object.get(key)).map(|m| clip_tail(&m, TURN_MESSAGE_MAX_CHARS));
    let (kind, message) = match name {
        "SessionStart" => (HookKind::SessionStart, None),
        "UserPromptSubmit" | "BeforeAgent" => (HookKind::TurnStarted, None),
        "PermissionRequest" => match tool_key(object) {
            Some(key) if matches!(agent, "claude" | "codex") => {
                (HookKind::ToolPermission(key), tool_summary(object))
            }
            _ => (HookKind::PermissionNeeded, tool_summary(object)),
        },
        "PreToolUse" | "BeforeTool" => {
            let started = HookKind::ToolStarted {
                key: tool_key(object)?,
                id: text(object.get("tool_use_id")),
            };
            (started, None)
        }
        "PostToolUse" | "PostToolUseFailure" | "AfterTool" => {
            let finished = HookKind::ToolFinished {
                key: tool_key(object)?,
                id: text(object.get("tool_use_id")),
            };
            (finished, None)
        }
        "Notification" => match object.get("notification_type").and_then(Value::as_str)? {
            "permission_prompt" | "ToolPermission" => {
                (HookKind::PermissionNeeded, message("message"))
            }
            "idle_prompt" => (HookKind::IdleWaiting, message("message")),
            "elicitation_dialog" => (HookKind::QuestionAsked, message("message")),
            _ => return None,
        },
        "Stop" => (HookKind::TurnDone, message("last_assistant_message")),
        "AfterAgent" => (HookKind::TurnDone, message("prompt_response")),
        "StopFailure" => {
            let reason = [object.get("error"), object.get("error_details")]
                .into_iter()
                .filter_map(text)
                .collect::<Vec<_>>()
                .join(": ");
            let reason =
                if reason.is_empty() { "the agent stopped on an error".into() } else { reason };
            (HookKind::TurnFailed(clip(&reason)), None)
        }
        "SessionEnd" => (HookKind::SessionEnd, None),
        _ => return None,
    };
    Some(HookEvent { message, ..HookEvent::bare(kind) })
}

fn parse_copilot_event(object: &serde_json::Map<String, Value>) -> Option<HookEvent> {
    let kind = object.get("type")?.as_str()?;
    let data = object.get("data");
    let field = |key: &str| text(data.and_then(|d| d.get(key)));
    let event = match kind {
        "session.start" => HookEvent::bare(HookKind::SessionStart),
        "user.message" | "assistant.turn_start" | "permission.completed" => {
            HookEvent::bare(HookKind::TurnStarted)
        }
        "assistant.message" => {
            let has_tools = data
                .and_then(|d| d.get("toolRequests"))
                .and_then(Value::as_array)
                .is_some_and(|tools| !tools.is_empty());
            if has_tools {
                HookEvent::bare(HookKind::TurnStarted)
            } else {
                HookEvent {
                    message: field("content").map(|m| clip(&m)),
                    ..HookEvent::bare(HookKind::TurnDone)
                }
            }
        }
        "permission.requested" => {
            let request = data.and_then(|d| d.get("permissionRequest"));
            let summary = text(request.and_then(|r| r.get("intention")))
                .or_else(|| text(request.and_then(|r| r.get("kind"))));
            HookEvent {
                message: summary.map(|m| clip(&m)),
                ..HookEvent::bare(HookKind::PermissionNeeded)
            }
        }
        "session.task_complete" => HookEvent {
            message: field("summary").map(|m| clip(&m)),
            ..HookEvent::bare(HookKind::TurnDone)
        },
        "abort" => HookEvent::bare(HookKind::TurnDone),
        "session.error" => HookEvent::bare(HookKind::TurnFailed(clip(
            &field("message").unwrap_or_else(|| "Copilot reported an error".into()),
        ))),
        "session.shutdown" => HookEvent::bare(HookKind::SessionEnd),
        _ => return None,
    };
    Some(event)
}

/// A shell call is keyed by its command alone: Codex's prompt carries a description its `PostToolUse` lacks.
fn tool_key(object: &serde_json::Map<String, Value>) -> Option<String> {
    let tool = text(object.get("tool_name"))?;
    let input = object.get("tool_input").unwrap_or(&Value::Null);
    let call = input
        .get("command")
        .and_then(Value::as_str)
        .map_or_else(|| input.to_string(), str::to_owned);
    Some(format!("{tool}\n{call}"))
}

fn tool_summary(object: &serde_json::Map<String, Value>) -> Option<String> {
    let tool = text(object.get("tool_name"))?;
    let detail = object
        .get("tool_input")
        .and_then(|input| input.get("command").or_else(|| input.get("file_path")))
        .and_then(|v| text(Some(v)));
    Some(clip(&match detail {
        Some(detail) => format!("{tool}: {detail}"),
        None => tool,
    }))
}

fn text(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn clip(message: &str) -> String {
    crate::text::clip(message, MESSAGE_MAX_CHARS)
}

fn clip_tail(message: &str, max: usize) -> String {
    let count = message.chars().count();
    if count <= max {
        return message.to_owned();
    }
    let tail: String = message.chars().skip(count - max).collect();
    format!("...{tail}")
}

/// OSC 9;4 progress as the status layer needs it. The app maps its terminal's progress onto this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressSignal {
    #[default]
    Off,
    Working,
    Error,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TerminalSignal {
    Progress(ProgressSignal),
    TitleChanged,
    Bell,
}

/// Layer 3: what the operating system says about the agent process.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessFacts {
    pub alive: bool,
    /// `None` while alive, or when a signal killed it.
    pub exit_code: Option<i32>,
    pub started_at: f64,
    pub last_output_at: Option<f64>,
    pub last_word_at: Option<f64>,
    pub hooked: bool,
}

impl ProcessFacts {
    #[must_use]
    pub fn started(at: f64) -> Self {
        Self {
            alive: true,
            exit_code: None,
            started_at: at,
            last_output_at: None,
            last_word_at: None,
            hooked: false,
        }
    }

    fn quiet_for(&self, now: f64) -> f64 {
        now - self.last_output_at.unwrap_or(self.started_at)
    }

    fn last_heard_at(&self) -> f64 {
        let output = self.last_output_at.filter(|_| !self.hooked);
        [output, self.last_word_at].into_iter().flatten().fold(self.started_at, f64::max)
    }

    pub(crate) fn silent_for(&self, now: f64) -> f64 {
        now - self.last_heard_at()
    }

    fn printing_since(&self, since: f64, now: f64) -> bool {
        self.last_output_at.is_some_and(|t| t > since && now - t <= ACTIVE_WINDOW_SECS)
    }
}

/// Layer 2: the latest terminal signals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TerminalState {
    pub progress: ProgressSignal,
    pub title_changed_at: Option<f64>,
    pub bell_at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum HookStatus {
    Working,
    NeedsYou(Reason),
    Done,
    Failed(String),
}

/// Layer 1: the latest status a hook reported, and when it arrived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookState {
    status: HookStatus,
    at: f64,
    pending: Vec<String>,
}

impl HookState {
    /// The state an event sets, if it sets one. `SessionStart` and `SessionEnd` carry none: the lower
    /// layers (and the process exit) say more.
    #[must_use]
    pub fn from_event(event: &HookEvent, now: f64) -> Option<Self> {
        let status = match &event.kind {
            HookKind::TurnStarted => HookStatus::Working,
            HookKind::PermissionNeeded | HookKind::ToolPermission(_) => {
                HookStatus::NeedsYou(Reason::Permission)
            }
            HookKind::QuestionAsked => HookStatus::NeedsYou(Reason::Question),
            HookKind::TurnDone | HookKind::IdleWaiting => HookStatus::Done,
            HookKind::TurnFailed(reason) => HookStatus::Failed(reason.clone()),
            HookKind::SessionStart
            | HookKind::SessionEnd
            | HookKind::ToolStarted { .. }
            | HookKind::ToolFinished { .. } => return None,
        };
        let pending = match &event.kind {
            HookKind::ToolPermission(key) => vec![key.clone()],
            _ => Vec::new(),
        };
        Some(Self { status, at: now, pending })
    }

    fn holds_permission(&self) -> bool {
        self.status == HookStatus::NeedsYou(Reason::Permission) && !self.pending.is_empty()
    }

    /// An idle notice (a minute after a turn, or after one the agent never reported finishing, such
    /// as an interrupted one) says the agent is at its prompt. It only ever settles a turn that is
    /// still `Working`: a finished, failed or prompting state is the better report. A held permission
    /// is the exception: an agent at its prompt is not waiting on a tool call.
    fn after_idle(self, previous: Option<&Self>) -> Self {
        match previous {
            Some(previous)
                if previous.status != HookStatus::Working && !previous.holds_permission() =>
            {
                previous.clone()
            }
            _ => self,
        }
    }

    /// A permission notice that follows the prompt it announces adds nothing, and must not drop the
    /// tool call the prompt named.
    fn after_notice(self, previous: Option<&Self>) -> Self {
        match previous {
            Some(previous) if previous.holds_permission() => previous.clone(),
            _ => self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifecycle {
    /// A plain terminal: no agent, no status.
    Shell,
    /// The worktree or its setup command is still running.
    SettingUp,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusInputs {
    pub lifecycle: Lifecycle,
    pub hook: Option<HookState>,
    pub terminal: TerminalState,
    pub process: ProcessFacts,
}

/// The three layers folded into one status at time `now` (seconds on any monotonic clock the caller
/// uses for every input).
#[must_use]
pub fn resolve(inputs: &StatusInputs, now: f64) -> AgentStatus {
    match inputs.lifecycle {
        Lifecycle::Shell => return AgentStatus::Shell,
        Lifecycle::SettingUp => return AgentStatus::SettingUp,
        Lifecycle::Agent => {}
    }
    if !inputs.process.alive {
        return exited(inputs);
    }
    if let Some(hook) = &inputs.hook {
        return from_hook(hook, inputs, now);
    }
    from_signals(inputs, now)
}

fn exited(inputs: &StatusInputs) -> AgentStatus {
    if let Some(HookState { status: HookStatus::Failed(reason), .. }) = &inputs.hook {
        return AgentStatus::Failed(reason.clone());
    }
    match inputs.process.exit_code {
        Some(0) => AgentStatus::Done,
        Some(code) => AgentStatus::Failed(format!("exited with code {code}")),
        None => AgentStatus::Failed("the agent process ended".into()),
    }
}

fn from_hook(hook: &HookState, inputs: &StatusInputs, now: f64) -> AgentStatus {
    let process = &inputs.process;
    match &hook.status {
        HookStatus::Failed(reason) => AgentStatus::Failed(reason.clone()),
        HookStatus::Done => AgentStatus::Done,
        HookStatus::Working => {
            let busy = inputs.terminal.progress == ProgressSignal::Working;
            let silent_for = now - process.last_output_at.unwrap_or(hook.at).max(hook.at);
            let hung = process.silent_for(now) >= BUSY_QUIET_SECS;
            if (!busy && silent_for >= STATUS_QUIET_SECS) || hung {
                AgentStatus::NeedsYou(Reason::Quiet)
            } else {
                AgentStatus::Working
            }
        }
        HookStatus::NeedsYou(Reason::Permission) if hook.holds_permission() => {
            AgentStatus::NeedsYou(Reason::Permission)
        }
        HookStatus::NeedsYou(reason @ (Reason::Permission | Reason::Question)) => {
            let moved_on =
                now - hook.at >= HOOK_SETTLE_SECS && process.printing_since(hook.at, now);
            if moved_on { AgentStatus::Working } else { AgentStatus::NeedsYou(*reason) }
        }
        HookStatus::NeedsYou(reason) => AgentStatus::NeedsYou(*reason),
    }
}

fn from_signals(inputs: &StatusInputs, now: f64) -> AgentStatus {
    let (terminal, process) = (&inputs.terminal, &inputs.process);
    match terminal.progress {
        ProgressSignal::Working if process.silent_for(now) >= BUSY_QUIET_SECS => {
            return AgentStatus::NeedsYou(Reason::Quiet);
        }
        ProgressSignal::Working => return AgentStatus::Working,
        ProgressSignal::Error => return AgentStatus::Failed("the agent reported an error".into()),
        ProgressSignal::Paused => return AgentStatus::NeedsYou(Reason::Input),
        ProgressSignal::Off => {}
    }
    let unanswered_bell = terminal
        .bell_at
        .is_some_and(|bell| process.last_output_at.is_none_or(|t| t - bell <= BELL_SETTLE_SECS));
    if unanswered_bell {
        return AgentStatus::NeedsYou(Reason::Attention);
    }
    if terminal.title_changed_at.is_some_and(|t| now - t <= ACTIVE_WINDOW_SECS) {
        return AgentStatus::Working;
    }
    if process.quiet_for(now) >= STATUS_QUIET_SECS {
        AgentStatus::NeedsYou(Reason::Quiet)
    } else {
        AgentStatus::Working
    }
}

#[must_use]
pub fn at_rest(status: &AgentStatus, silent_for: f64) -> AgentStatus {
    match status {
        AgentStatus::Working | AgentStatus::NeedsYou(Reason::Quiet)
            if silent_for >= REST_AFTER_SECS =>
        {
            AgentStatus::Done
        }
        other => other.clone(),
    }
}

/// The earliest instant after `now` at which [`resolve`] can answer differently with no new input,
/// or `None` when the answer only changes on input (or never). The app sleeps until then, so a
/// quiet agent costs no frames and the decay rules still fire on time.
#[must_use]
pub fn next_change(inputs: &StatusInputs, now: f64) -> Option<f64> {
    if inputs.lifecycle != Lifecycle::Agent || !inputs.process.alive {
        return None;
    }
    let (process, terminal) = (&inputs.process, &inputs.terminal);
    let marks: Vec<f64> = match &inputs.hook {
        Some(HookState { status: HookStatus::Working, at, .. }) => {
            if terminal.progress == ProgressSignal::Working {
                vec![busy_cap(process)]
            } else {
                vec![process.last_output_at.unwrap_or(*at).max(*at) + STATUS_QUIET_SECS]
            }
        }
        Some(hook) if hook.holds_permission() => Vec::new(),
        Some(HookState {
            status: HookStatus::NeedsYou(Reason::Permission | Reason::Question),
            at,
            ..
        }) => {
            let mut marks = vec![at + HOOK_SETTLE_SECS];
            marks.extend(process.last_output_at.map(|t| t + ACTIVE_WINDOW_SECS));
            marks
        }
        Some(_) => Vec::new(),
        None if terminal.progress == ProgressSignal::Working => vec![busy_cap(process)],
        None if terminal.progress != ProgressSignal::Off => Vec::new(),
        None => {
            let mut marks =
                vec![process.last_output_at.unwrap_or(process.started_at) + STATUS_QUIET_SECS];
            marks.extend(terminal.title_changed_at.map(|t| t + ACTIVE_WINDOW_SECS));
            marks
        }
    };
    marks.into_iter().filter(|mark| *mark > now).min_by(f64::total_cmp)
}

fn busy_cap(process: &ProcessFacts) -> f64 {
    process.last_heard_at() + BUSY_QUIET_SECS
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OpenCall {
    call: String,
    key: String,
}

/// Keeps one tab's [`StatusInputs`] current. Every method takes the time it happened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusTracker {
    inputs: StatusInputs,
    open_tools: Vec<OpenCall>,
}

impl StatusTracker {
    /// A tab that runs no agent.
    #[must_use]
    pub fn shell() -> Self {
        Self::with(Lifecycle::Shell, 0.0)
    }

    /// A tab whose worktree or setup command is still running.
    #[must_use]
    pub fn setting_up(now: f64) -> Self {
        Self::with(Lifecycle::SettingUp, now)
    }

    /// A tab whose agent process starts now.
    #[must_use]
    pub fn agent(now: f64) -> Self {
        Self::with(Lifecycle::Agent, now)
    }

    fn with(lifecycle: Lifecycle, now: f64) -> Self {
        Self {
            inputs: StatusInputs {
                lifecycle,
                hook: None,
                terminal: TerminalState::default(),
                process: ProcessFacts::started(now),
            },
            open_tools: Vec::new(),
        }
    }

    /// Setup finished: the agent process starts now.
    pub fn agent_started(&mut self, now: f64) {
        self.inputs.lifecycle = Lifecycle::Agent;
        let hooked = self.inputs.process.hooked;
        self.inputs.process = ProcessFacts { hooked, ..ProcessFacts::started(now) };
        self.inputs.hook = None;
        self.inputs.terminal = TerminalState::default();
        self.open_tools.clear();
    }

    /// The agent process was resumed from a saved session: it waits at its prompt, which reads as
    /// `Done` until its next turn starts. No hook speaks for a resumed session before that, and the
    /// quiet decay would call it `NeedsYou` for good.
    pub fn agent_resumed(&mut self, now: f64) {
        self.agent_started(now);
        self.inputs.hook =
            Some(HookState { status: HookStatus::Done, at: now, pending: Vec::new() });
    }

    /// Whether the hook state says a turn is on: working, or waiting on a permission or a question.
    #[must_use]
    pub fn turn_open(&self) -> bool {
        matches!(
            self.inputs.hook.as_ref().map(|hook| &hook.status),
            Some(HookStatus::Working | HookStatus::NeedsYou(_))
        )
    }

    /// Hooks stopped being heard (a restart, a listener gap): what they last said no longer holds,
    /// so the terminal and process layers decide until a hook speaks again.
    pub fn hooks_lost(&mut self) {
        self.inputs.hook = None;
        self.open_tools.clear();
    }

    pub fn on_hook(&mut self, event: &HookEvent, now: f64) {
        self.inputs.process.last_word_at = Some(now);
        match &event.kind {
            HookKind::ToolStarted { key, id } => return self.tool_started(key, id.as_deref(), now),
            HookKind::ToolFinished { key, id } => {
                return self.tool_finished(key, id.as_deref(), now);
            }
            HookKind::TurnStarted => self.open_tools.clear(),
            _ => {}
        }
        let Some(mut state) = HookState::from_event(event, now) else { return };
        let previous = self.inputs.hook.as_ref();
        self.inputs.hook = Some(match event.kind {
            HookKind::IdleWaiting => state.after_idle(previous),
            HookKind::PermissionNeeded => {
                state.pending = self.open_tools.iter().map(|open| open.key.clone()).collect();
                state.after_notice(previous)
            }
            _ => state,
        });
    }

    fn resume_after_finished_turn(&mut self, now: f64) {
        if let Some(hook) = &self.inputs.hook
            && matches!(hook.status, HookStatus::Done | HookStatus::Failed(_))
        {
            self.inputs.hook =
                Some(HookState { status: HookStatus::Working, at: now, pending: Vec::new() });
        }
    }

    fn tool_started(&mut self, key: &str, id: Option<&str>, now: f64) {
        self.open_tools.push(OpenCall { call: id.unwrap_or(key).to_owned(), key: key.to_owned() });
        self.resume_after_finished_turn(now);
        let moved_on = self.inputs.hook.as_ref().is_some_and(|hook| {
            hook.holds_permission()
                && !hook.pending.iter().any(|pending| pending == key)
                && now - hook.at >= HOOK_SETTLE_SECS
        });
        if moved_on {
            self.settle_permission(now);
        }
    }

    fn tool_finished(&mut self, key: &str, id: Option<&str>, now: f64) {
        let call = id.unwrap_or(key);
        let began = self.open_tools.iter().position(|open| open.call == call);
        if let Some(at) = began {
            self.open_tools.remove(at);
        }
        if id.is_some() && began.is_none() {
            self.resume_after_finished_turn(now);
        }
        let Some(hook) = self.inputs.hook.as_mut().filter(|hook| hook.holds_permission()) else {
            return;
        };
        if let Some(at) = hook.pending.iter().position(|call| call == key) {
            hook.pending.remove(at);
            if hook.pending.is_empty() {
                self.settle_permission(now);
            }
        }
    }

    /// The user answered the pending permission prompt (no hook reports a denial).
    pub fn on_permission_answered(&mut self, now: f64) {
        if self.inputs.hook.as_ref().is_some_and(HookState::holds_permission) {
            self.settle_permission(now);
        }
    }

    fn settle_permission(&mut self, now: f64) {
        if let Some(hook) = &self.inputs.hook {
            self.open_tools.retain(|open| !hook.pending.contains(&open.key));
        }
        self.inputs.hook =
            Some(HookState { status: HookStatus::Working, at: now, pending: Vec::new() });
    }

    pub fn on_terminal(&mut self, signal: TerminalSignal, now: f64) {
        let terminal = &mut self.inputs.terminal;
        match signal {
            TerminalSignal::Progress(progress) => terminal.progress = progress,
            TerminalSignal::TitleChanged => terminal.title_changed_at = Some(now),
            TerminalSignal::Bell => terminal.bell_at = Some(now),
        }
    }

    pub fn set_hooked(&mut self, hooked: bool) {
        self.inputs.process.hooked = hooked;
    }

    pub fn on_transcript(&mut self, now: f64) {
        self.inputs.process.last_word_at = Some(now);
    }

    pub fn on_output(&mut self, now: f64) {
        self.inputs.process.last_output_at = Some(now);
    }

    pub fn on_exit(&mut self, code: Option<i32>) {
        self.inputs.process.alive = false;
        self.inputs.process.exit_code = code;
    }

    #[must_use]
    pub fn status(&self, now: f64) -> AgentStatus {
        resolve(&self.inputs, now)
    }

    /// See [`next_change`].
    #[must_use]
    pub fn next_change_at(&self, now: f64) -> Option<f64> {
        next_change(&self.inputs, now)
    }

    #[must_use]
    pub fn inputs(&self) -> &StatusInputs {
        &self.inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(agent: &str, payload: &Value) -> Option<(HookKind, Option<String>)> {
        parse_hook(agent, payload).map(|e| (e.kind, e.message))
    }

    #[test]
    fn real_claude_payloads_map_to_hook_kinds() {
        let sid = "df94b279-06fb-4828-bb81-3be72cef88b1";
        let cases = [
            (
                json!({"session_id": sid, "prompt_id": "p", "permission_mode": "default",
                    "hook_event_name": "UserPromptSubmit", "prompt": "Run the bash command touch zz.txt"}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"session_id": sid, "permission_mode": "default", "hook_event_name": "PermissionRequest",
                    "tool_name": "Bash", "tool_input": {"command": "touch zz.txt", "description": "Create empty file"},
                    "permission_suggestions": [{"type": "addDirectories"}]}),
                Some((
                    HookKind::ToolPermission("Bash\ntouch zz.txt".into()),
                    Some("Bash: touch zz.txt".into()),
                )),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Notification",
                    "message": "Claude needs your permission", "notification_type": "permission_prompt"}),
                Some((HookKind::PermissionNeeded, Some("Claude needs your permission".into()))),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Notification",
                    "message": "Claude is waiting for your input", "notification_type": "idle_prompt"}),
                Some((HookKind::IdleWaiting, Some("Claude is waiting for your input".into()))),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Notification",
                    "message": "Pick one", "notification_type": "elicitation_dialog"}),
                Some((HookKind::QuestionAsked, Some("Pick one".into()))),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Notification",
                    "message": "Signed in", "notification_type": "auth_success"}),
                None,
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Stop", "stop_hook_active": false,
                    "last_assistant_message": "**The folder has 2 files:**", "background_tasks": []}),
                Some((HookKind::TurnDone, Some("**The folder has 2 files:**".into()))),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "StopFailure", "error": "rate_limit",
                    "error_details": "429 Too Many Requests"}),
                Some((HookKind::TurnFailed("rate_limit: 429 Too Many Requests".into()), None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "StopFailure"}),
                Some((HookKind::TurnFailed("the agent stopped on an error".into()), None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "SessionStart", "source": "startup"}),
                Some((HookKind::SessionStart, None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "SessionEnd", "reason": "exit"}),
                Some((HookKind::SessionEnd, None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "PreToolUse", "tool_name": "Bash"}),
                Some((HookKind::ToolStarted { key: "Bash\nnull".into(), id: None }, None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "PreToolUse", "tool_name": "Bash",
                    "tool_use_id": "toolu_9"}),
                Some((
                    HookKind::ToolStarted { key: "Bash\nnull".into(), id: Some("toolu_9".into()) },
                    None,
                )),
            ),
        ];
        for (payload, want) in cases {
            assert_eq!(parse("claude", &payload), want, "payload {payload}");
        }
    }

    #[test]
    fn real_gemini_payloads_map_to_hook_kinds() {
        let sid = "96c6af47-e341-4e59-afe8-dcfe284d6db5";
        let cases = [
            (
                json!({"session_id": sid, "hook_event_name": "SessionStart", "timestamp": "t", "source": "startup"}),
                Some((HookKind::SessionStart, None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "BeforeAgent", "timestamp": "t", "prompt": "Run touch zz.txt"}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "Notification", "timestamp": "t",
                    "notification_type": "ToolPermission", "message": "Tool Confirm Shell Command requires execution",
                    "details": {"type": "exec", "title": "Confirm Shell Command", "command": "touch zz.txt"}}),
                Some((
                    HookKind::PermissionNeeded,
                    Some("Tool Confirm Shell Command requires execution".into()),
                )),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "AfterAgent", "timestamp": "t",
                    "prompt": "reply with the word hi", "prompt_response": "hi ", "stop_hook_active": false}),
                Some((HookKind::TurnDone, Some("hi".into()))),
            ),
            (
                json!({"session_id": sid, "hook_event_name": "SessionEnd", "timestamp": "t", "reason": "exit"}),
                Some((HookKind::SessionEnd, None)),
            ),
        ];
        for (payload, want) in cases {
            assert_eq!(parse("gemini", &payload), want, "payload {payload}");
        }
    }

    #[test]
    fn copilot_transcript_events_map_to_hook_kinds() {
        let cases = [
            (json!({"type": "session.start", "data": {}}), Some((HookKind::SessionStart, None))),
            (
                json!({"type": "user.message", "data": {"content": "hi"}}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"type": "assistant.turn_start", "data": {"turnId": "0"}}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"type": "assistant.message", "data": {"content": "", "toolRequests": [{"name": "bash"}]}}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"type": "assistant.message", "data": {"content": "All done.", "toolRequests": []}}),
                Some((HookKind::TurnDone, Some("All done.".into()))),
            ),
            (json!({"type": "assistant.turn_end", "data": {"turnId": "0"}}), None),
            (
                json!({"type": "permission.requested", "data": {"requestId": "r",
                    "permissionRequest": {"kind": "shell", "intention": "Copy prompt to clipboard"}}}),
                Some((HookKind::PermissionNeeded, Some("Copy prompt to clipboard".into()))),
            ),
            (
                json!({"type": "permission.requested", "data": {"permissionRequest": {"kind": "write"}}}),
                Some((HookKind::PermissionNeeded, Some("write".into()))),
            ),
            (
                json!({"type": "permission.completed", "data": {"requestId": "r"}}),
                Some((HookKind::TurnStarted, None)),
            ),
            (
                json!({"type": "session.task_complete", "data": {"summary": "Installed 14 skills", "success": true}}),
                Some((HookKind::TurnDone, Some("Installed 14 skills".into()))),
            ),
            (
                json!({"type": "abort", "data": {"reason": "user_abort"}}),
                Some((HookKind::TurnDone, None)),
            ),
            (
                json!({"type": "session.error", "data": {"message": "Not authenticated", "errorType": "x"}}),
                Some((HookKind::TurnFailed("Not authenticated".into()), None)),
            ),
            (
                json!({"type": "session.shutdown", "data": {"shutdownType": "routine"}}),
                Some((HookKind::SessionEnd, None)),
            ),
            (json!({"type": "hook.start", "data": {}}), None),
        ];
        for (payload, want) in cases {
            assert_eq!(parse("copilot", &payload), want, "payload {payload}");
        }
    }

    const GEMINI_HOOKS: &str = include_str!("../tests/fixtures/gemini_hooks.jsonl");
    const CODEX_HOOKS: &str = include_str!("../tests/fixtures/codex_hooks.jsonl");
    const COPILOT_HOOKS: &str = include_str!("../tests/fixtures/copilot_hooks.jsonl");

    fn copilot_turn() -> Vec<Value> {
        COPILOT_HOOKS.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    #[test]
    fn real_copilot_hook_payloads_map_to_hook_kinds() {
        let want = [
            Some((HookKind::SessionStart, None)),
            Some((HookKind::TurnStarted, None)),
            Some((
                HookKind::ToolStarted { key: "Bash\ntouch zz-probe.txt".into(), id: None },
                None,
            )),
            None,
            Some((HookKind::PermissionNeeded, Some("Run command: touch zz-probe.txt".into()))),
            Some((
                HookKind::ToolFinished { key: "Bash\ntouch zz-probe.txt".into(), id: None },
                None,
            )),
            Some((HookKind::TurnDone, None)),
            Some((HookKind::SessionEnd, None)),
        ];
        let turn = copilot_turn();
        assert_eq!(turn.len(), want.len());
        for (payload, want) in turn.iter().zip(want) {
            assert_eq!(parse("copilot", payload), want, "payload {payload}");
        }
    }

    #[test]
    fn a_copilot_turn_that_stops_reads_done_instead_of_waiting_for_you() {
        let mut tracker = StatusTracker::agent(0.0);
        let mut at = 1.0;
        for payload in copilot_turn() {
            if let Some(event) = parse_hook("copilot", &payload) {
                tracker.on_hook(&event, at);
            }
            at += 1.0;
        }
        assert_eq!(tracker.status(at + 30.0), AgentStatus::Done);
    }

    #[test]
    fn real_gemini_hook_payloads_map_to_hook_kinds() {
        let call = "run_shell_command\ntouch zz-probe.txt";
        let want = [
            Some((HookKind::SessionStart, None)),
            Some((HookKind::TurnStarted, None)),
            Some((HookKind::ToolStarted { key: call.into(), id: None }, None)),
            Some((
                HookKind::PermissionNeeded,
                Some("Tool Confirm Shell Command requires execution".into()),
            )),
            Some((HookKind::ToolFinished { key: call.into(), id: None }, None)),
            Some((HookKind::TurnDone, Some("All done.".into()))),
        ];
        let turn = payloads(GEMINI_HOOKS);
        assert_eq!(turn.len(), want.len());
        for (payload, want) in turn.iter().zip(want) {
            assert_eq!(parse("gemini", payload), want, "payload {payload}");
        }
    }

    #[test]
    fn real_codex_hook_payloads_map_to_hook_kinds() {
        let call = "Bash\ntouch zz-probe.txt";
        let want = [
            Some((HookKind::SessionStart, None)),
            Some((HookKind::TurnStarted, None)),
            Some((HookKind::ToolStarted { key: call.into(), id: Some("call_1".into()) }, None)),
            Some((HookKind::ToolPermission(call.into()), Some("Bash: touch zz-probe.txt".into()))),
            Some((HookKind::ToolFinished { key: call.into(), id: Some("call_1".into()) }, None)),
            Some((HookKind::TurnDone, Some("All done.".into()))),
        ];
        let turn = payloads(CODEX_HOOKS);
        assert_eq!(turn.len(), want.len());
        for (payload, want) in turn.iter().zip(want) {
            assert_eq!(parse("codex", payload), want, "payload {payload}");
        }
    }

    fn payloads(fixture: &str) -> Vec<Value> {
        fixture.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    #[test]
    fn a_permission_holds_until_its_call_ends_for_codex_gemini_and_copilot() {
        let walks = [("codex", CODEX_HOOKS), ("gemini", GEMINI_HOOKS), ("copilot", COPILOT_HOOKS)];
        for (agent, fixture) in walks {
            let events: Vec<HookEvent> =
                payloads(fixture).iter().filter_map(|p| parse_hook(agent, p)).collect();
            let at = |is: fn(&HookKind) -> bool| events.iter().position(|e| is(&e.kind)).unwrap();
            let asked =
                at(|k| matches!(k, HookKind::PermissionNeeded | HookKind::ToolPermission(_)));
            let ended = at(|k| matches!(k, HookKind::ToolFinished { .. }));
            let stopped = at(|k| matches!(k, HookKind::TurnDone));
            let mut tracker = StatusTracker::agent(0.0);
            for (second, event) in events[..=asked].iter().enumerate() {
                tracker.on_hook(event, 1.0 + second as f64);
            }
            for second in 10..70 {
                tracker.on_output(f64::from(second));
            }
            let needs = AgentStatus::NeedsYou(Reason::Permission);
            assert_eq!(tracker.status(70.5), needs, "{agent}: a spinner behind the prompt");
            assert_eq!(tracker.next_change_at(70.5), None, "{agent}");
            for event in &events[asked + 1..ended] {
                tracker.on_hook(event, 71.0);
            }
            tracker.on_hook(&events[ended], 72.0);
            assert_eq!(tracker.status(72.1), AgentStatus::Working, "{agent}: its call ended");
            tracker.on_hook(&events[stopped], 73.0);
            assert_eq!(tracker.status(73.1), AgentStatus::Done, "{agent}");
        }
    }

    #[test]
    fn a_prompt_for_a_call_that_began_before_it_waits_on_every_call_in_flight() {
        let started =
            |key: &str| HookEvent::bare(HookKind::ToolStarted { key: key.into(), id: None });
        let ended =
            |key: &str| HookEvent::bare(HookKind::ToolFinished { key: key.into(), id: None });
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 1.0);
        tracker.on_hook(&started("read\na.rs"), 2.0);
        tracker.on_hook(&started("write\nb.rs"), 2.0);
        tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 2.1);
        tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 2.2);
        tracker.on_output(10.0);
        let needs = AgentStatus::NeedsYou(Reason::Permission);
        assert_eq!(tracker.status(10.1), needs);
        tracker.on_hook(&ended("read\na.rs"), 11.0);
        tracker.on_output(11.0);
        assert_eq!(tracker.status(11.1), needs, "one of two calls is still open");
        tracker.on_hook(&ended("write\nb.rs"), 12.0);
        assert_eq!(tracker.status(12.1), AgentStatus::Working);
    }

    #[test]
    fn a_new_call_two_seconds_after_the_prompt_means_it_was_answered() {
        let started =
            |key: &str| HookEvent::bare(HookKind::ToolStarted { key: key.into(), id: None });
        let prompted = || {
            let mut tracker = StatusTracker::agent(0.0);
            tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 1.0);
            tracker.on_hook(&started("shell\nrm x"), 2.0);
            tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 2.1);
            tracker
        };
        let needs = AgentStatus::NeedsYou(Reason::Permission);
        let mut racing = prompted();
        racing.on_hook(&started("shell\nls"), 2.3);
        racing.on_output(5.0);
        assert_eq!(racing.status(5.1), needs, "a call of the same batch, begun right after");
        let mut denied = prompted();
        denied.on_hook(&started("shell\nls"), 9.0);
        denied.on_output(9.0);
        assert_eq!(denied.status(9.1), AgentStatus::Working, "the agent went on after a denial");
        denied.on_hook(
            &HookEvent::bare(HookKind::ToolFinished { key: "shell\nrm x".into(), id: None }),
            9.5,
        );
        assert_eq!(denied.status(9.6), AgentStatus::Working);
    }

    #[test]
    fn codex_and_unknown_agents_read_claude_style_names() {
        let stop = json!({"hook_event_name": "Stop", "last_assistant_message": "done"});
        for agent in ["codex", "aider", ""] {
            assert_eq!(
                parse(agent, &stop),
                Some((HookKind::TurnDone, Some("done".into()))),
                "{agent}"
            );
        }
    }

    #[test]
    fn session_id_and_transcript_path_come_along() {
        let payload = json!({"hook_event_name": "Stop", "session_id": "abc", "transcript_path": "/t/x.jsonl"});
        let event = parse_hook("claude", &payload).unwrap();
        assert_eq!(event.session_id.as_deref(), Some("abc"));
        assert_eq!(event.transcript_path, Some(PathBuf::from("/t/x.jsonl")));
    }

    #[test]
    fn garbage_payloads_are_ignored() {
        let cases = [
            json!(null),
            json!(7),
            json!("Stop"),
            json!([1, 2]),
            json!({}),
            json!({"hook_event_name": 3}),
            json!({"hook_event_name": "Notification"}),
            json!({"hook_event_name": "Notification", "notification_type": 4}),
            json!({"type": 5}),
        ];
        for payload in &cases {
            for agent in ["claude", "gemini", "copilot", "codex"] {
                assert_eq!(parse_hook(agent, payload), None, "{agent} {payload}");
            }
        }
    }

    #[test]
    fn a_turns_last_message_is_kept_whole_up_to_a_large_cap() {
        let answer = format!("{}\n\nDone\n- a\n\nNeeds you\n- b\n\nNext\n- c", "x".repeat(2000));
        let event = parse_hook(
            "claude",
            &json!({"hook_event_name": "Stop", "last_assistant_message": answer}),
        )
        .unwrap();
        assert_eq!(event.message.as_deref(), Some(answer.as_str()));
    }

    #[test]
    fn long_messages_are_clipped() {
        let long = format!("{}\n\nDone\n- shipped", "x".repeat(TURN_MESSAGE_MAX_CHARS * 2));
        let event = parse_hook(
            "claude",
            &json!({"hook_event_name": "Stop", "last_assistant_message": long}),
        )
        .unwrap();
        let message = event.message.unwrap();
        assert_eq!(message.chars().count(), TURN_MESSAGE_MAX_CHARS + 3);
        assert!(message.starts_with("..."));
        assert!(message.ends_with("\n\nDone\n- shipped"));
    }

    fn agent_inputs() -> StatusInputs {
        StatusInputs {
            lifecycle: Lifecycle::Agent,
            hook: None,
            terminal: TerminalState::default(),
            process: ProcessFacts::started(0.0),
        }
    }

    fn hook(kind: HookKind, at: f64) -> Option<HookState> {
        HookState::from_event(&HookEvent::bare(kind), at)
    }

    struct Case {
        name: &'static str,
        inputs: StatusInputs,
        now: f64,
        want: AgentStatus,
    }

    fn with(edit: impl FnOnce(&mut StatusInputs)) -> StatusInputs {
        let mut inputs = agent_inputs();
        edit(&mut inputs);
        inputs
    }

    #[test]
    fn the_three_layers_fold_into_one_status() {
        use AgentStatus::{Done, Failed, NeedsYou, SettingUp, Shell, Working};
        let printing = |t: f64| Some(t);
        let cases = vec![
            Case {
                name: "a shell tab",
                inputs: with(|i| i.lifecycle = Lifecycle::Shell),
                now: 100.0,
                want: Shell,
            },
            Case {
                name: "setup running",
                inputs: with(|i| i.lifecycle = Lifecycle::SettingUp),
                now: 1.0,
                want: SettingUp,
            },
            Case {
                name: "fresh agent with no signal is still starting up",
                inputs: agent_inputs(),
                now: 3.0,
                want: Working,
            },
            Case {
                name: "fresh agent silent past the quiet limit is waiting for you",
                inputs: agent_inputs(),
                now: 9.0,
                want: NeedsYou(Reason::Quiet),
            },
            Case {
                name: "output keeps the quiet clock at zero",
                inputs: with(|i| i.process.last_output_at = printing(20.0)),
                now: 25.0,
                want: Working,
            },
            Case {
                name: "progress active means working even when silent",
                inputs: with(|i| i.terminal.progress = ProgressSignal::Working),
                now: 60.0,
                want: Working,
            },
            Case {
                name: "progress error",
                inputs: with(|i| i.terminal.progress = ProgressSignal::Error),
                now: 1.0,
                want: Failed("the agent reported an error".into()),
            },
            Case {
                name: "progress paused",
                inputs: with(|i| i.terminal.progress = ProgressSignal::Paused),
                now: 1.0,
                want: NeedsYou(Reason::Input),
            },
            Case {
                name: "unanswered bell",
                inputs: with(|i| {
                    i.terminal.bell_at = Some(5.0);
                    i.process.last_output_at = printing(5.0);
                }),
                now: 6.0,
                want: NeedsYou(Reason::Attention),
            },
            Case {
                name: "output after the bell clears it",
                inputs: with(|i| {
                    i.terminal.bell_at = Some(5.0);
                    i.process.last_output_at = printing(7.0);
                }),
                now: 7.5,
                want: Working,
            },
            Case {
                name: "title change in the last second means working",
                inputs: with(|i| {
                    i.terminal.title_changed_at = Some(49.5);
                    i.process.last_output_at = printing(40.0);
                }),
                now: 50.0,
                want: Working,
            },
            Case {
                name: "hook working",
                inputs: with(|i| i.hook = hook(HookKind::TurnStarted, 1.0)),
                now: 3.0,
                want: Working,
            },
            Case {
                name: "hook working then silence decays to needs you",
                inputs: with(|i| {
                    i.hook = hook(HookKind::TurnStarted, 1.0);
                    i.process.last_output_at = printing(2.0);
                }),
                now: 11.0,
                want: NeedsYou(Reason::Quiet),
            },
            Case {
                name: "hook working with progress active never decays",
                inputs: with(|i| {
                    i.hook = hook(HookKind::TurnStarted, 1.0);
                    i.terminal.progress = ProgressSignal::Working;
                }),
                now: 100.0,
                want: Working,
            },
            Case {
                name: "hook permission",
                inputs: with(|i| i.hook = hook(HookKind::PermissionNeeded, 10.0)),
                now: 10.2,
                want: NeedsYou(Reason::Permission),
            },
            Case {
                name: "hook permission stays while the screen is silent",
                inputs: with(|i| {
                    i.hook = hook(HookKind::PermissionNeeded, 10.0);
                    i.process.last_output_at = printing(9.9);
                }),
                now: 60.0,
                want: NeedsYou(Reason::Permission),
            },
            Case {
                name: "output right after the prompt is the prompt drawing, not an answer",
                inputs: with(|i| {
                    i.hook = hook(HookKind::PermissionNeeded, 10.0);
                    i.process.last_output_at = printing(10.5);
                }),
                now: 11.0,
                want: NeedsYou(Reason::Permission),
            },
            Case {
                name: "permission answered: the terminal prints again after the settle time",
                inputs: with(|i| {
                    i.hook = hook(HookKind::PermissionNeeded, 10.0);
                    i.process.last_output_at = printing(12.4);
                }),
                now: 12.5,
                want: Working,
            },
            Case {
                name: "question answered the same way",
                inputs: with(|i| {
                    i.hook = hook(HookKind::QuestionAsked, 10.0);
                    i.process.last_output_at = printing(13.0);
                }),
                now: 13.1,
                want: Working,
            },
            Case {
                name: "idle prompt reads as done and typing does not undo it",
                inputs: with(|i| {
                    i.hook = hook(HookKind::IdleWaiting, 10.0);
                    i.process.last_output_at = printing(30.0);
                }),
                now: 30.1,
                want: Done,
            },
            Case {
                name: "hook done beats terminal progress",
                inputs: with(|i| {
                    i.hook = hook(HookKind::TurnDone, 10.0);
                    i.terminal.progress = ProgressSignal::Working;
                    i.process.last_output_at = printing(10.0);
                }),
                now: 10.5,
                want: Done,
            },
            Case {
                name: "hook done stays done when quiet",
                inputs: with(|i| i.hook = hook(HookKind::TurnDone, 10.0)),
                now: 500.0,
                want: Done,
            },
            Case {
                name: "hook failure",
                inputs: with(|i| i.hook = hook(HookKind::TurnFailed("rate_limit".into()), 10.0)),
                now: 11.0,
                want: Failed("rate_limit".into()),
            },
            Case {
                name: "session start alone says nothing, the lower layers decide",
                inputs: with(|i| i.hook = hook(HookKind::SessionStart, 1.0)),
                now: 3.0,
                want: Working,
            },
            Case {
                name: "exit 0 is done even while a hook says working",
                inputs: with(|i| {
                    i.hook = hook(HookKind::TurnStarted, 1.0);
                    i.process.alive = false;
                    i.process.exit_code = Some(0);
                }),
                now: 2.0,
                want: Done,
            },
            Case {
                name: "nonzero exit fails with the code",
                inputs: with(|i| {
                    i.process.alive = false;
                    i.process.exit_code = Some(2);
                }),
                now: 2.0,
                want: Failed("exited with code 2".into()),
            },
            Case {
                name: "killed by a signal fails",
                inputs: with(|i| i.process.alive = false),
                now: 2.0,
                want: Failed("the agent process ended".into()),
            },
            Case {
                name: "the hook's own failure text survives a clean exit",
                inputs: with(|i| {
                    i.hook = hook(HookKind::TurnFailed("billing_error".into()), 1.0);
                    i.process.alive = false;
                    i.process.exit_code = Some(0);
                }),
                now: 2.0,
                want: Failed("billing_error".into()),
            },
            Case {
                name: "a dead shell stays a shell",
                inputs: with(|i| {
                    i.lifecycle = Lifecycle::Shell;
                    i.process.alive = false;
                    i.process.exit_code = Some(1);
                }),
                now: 2.0,
                want: Shell,
            },
        ];
        for case in cases {
            assert_eq!(resolve(&case.inputs, case.now), case.want, "{}", case.name);
        }
    }

    #[test]
    fn tracker_walks_a_claude_turn_with_a_permission_prompt() {
        let event = |kind| HookEvent::bare(kind);
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_output(0.5);
        assert_eq!(tracker.status(1.0), AgentStatus::Working);
        tracker.on_hook(&event(HookKind::TurnStarted), 2.0);
        assert_eq!(tracker.status(3.0), AgentStatus::Working);
        tracker.on_hook(&event(HookKind::PermissionNeeded), 4.0);
        tracker.on_hook(&event(HookKind::PermissionNeeded), 4.1);
        assert_eq!(tracker.status(4.3), AgentStatus::NeedsYou(Reason::Permission));
        assert_eq!(tracker.status(30.0), AgentStatus::NeedsYou(Reason::Permission));
        tracker.on_output(31.0);
        assert_eq!(tracker.status(31.2), AgentStatus::Working);
        tracker.on_hook(&event(HookKind::TurnDone), 35.0);
        tracker.on_hook(&event(HookKind::IdleWaiting), 95.0);
        assert_eq!(tracker.status(96.0), AgentStatus::Done);
        tracker.on_hook(&event(HookKind::TurnStarted), 100.0);
        assert_eq!(tracker.status(101.0), AgentStatus::Working);
        tracker.on_exit(Some(0));
        assert_eq!(tracker.status(102.0), AgentStatus::Done);
    }

    fn tool_started(id: &str) -> HookEvent {
        claude_hook(&json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_use_id": id, "tool_input": {"command": "ls"}}))
    }

    fn tool_finished(id: &str) -> HookEvent {
        claude_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_use_id": id, "tool_input": {"command": "ls"}, "tool_response": {}}))
    }

    #[test]
    fn a_tool_that_began_after_stop_reopens_the_turn_and_one_that_began_before_never_does() {
        let turn_started = || HookEvent::bare(HookKind::TurnStarted);
        let stop = || HookEvent::bare(HookKind::TurnDone);
        let failed = || HookEvent::bare(HookKind::TurnFailed("rate_limit".into()));
        let cases = [
            (
                "a Stop guard sends the agent back: the next tool starts",
                vec![
                    turn_started(),
                    tool_started("a"),
                    tool_finished("a"),
                    stop(),
                    tool_started("b"),
                ],
                AgentStatus::Working,
            ),
            (
                "the same, seen only by its end (a tab with no start hook)",
                vec![turn_started(), tool_finished("a"), stop(), tool_finished("b")],
                AgentStatus::Working,
            ),
            (
                "after a failed turn",
                vec![turn_started(), failed(), tool_started("b")],
                AgentStatus::Working,
            ),
            (
                "the last tool's end arrives after the Stop",
                vec![turn_started(), tool_started("a"), stop(), tool_finished("a")],
                AgentStatus::Done,
            ),
            (
                "two parallel tools, both ends arrive after the Stop",
                vec![
                    turn_started(),
                    tool_started("a"),
                    tool_started("b"),
                    stop(),
                    tool_finished("b"),
                    tool_finished("a"),
                ],
                AgentStatus::Done,
            ),
            (
                "an end with no id says nothing about where it began",
                vec![
                    turn_started(),
                    stop(),
                    HookEvent::bare(HookKind::ToolFinished { key: "Bash\n{}".into(), id: None }),
                ],
                AgentStatus::Done,
            ),
            (
                "a continuation that stops again",
                vec![turn_started(), stop(), tool_started("b"), tool_finished("b"), stop()],
                AgentStatus::Done,
            ),
        ];
        for (name, events, want) in cases {
            let mut tracker = StatusTracker::agent(0.0);
            for (at, event) in events.iter().enumerate() {
                tracker.on_hook(event, 1.0 + at as f64);
            }
            tracker.on_output(10.0);
            assert_eq!(tracker.status(10.1), want, "{name}");
        }
    }

    fn named(all: &[Value], names: &[&str]) -> Vec<Value> {
        names
            .iter()
            .map(|name| {
                all.iter()
                    .find(|p| p["hook_event_name"] == *name)
                    .unwrap_or_else(|| panic!("{name}"))
                    .clone()
            })
            .collect()
    }

    #[test]
    fn on_every_cli_a_late_tool_end_never_reopens_a_finished_turn_but_a_new_call_does() {
        let claude = vec![
            json!({"hook_event_name": "UserPromptSubmit", "session_id": "s"}),
            json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_use_id": "toolu_1",
                "tool_input": {"command": "ls"}}),
            json!({"hook_event_name": "Stop", "session_id": "s"}),
            json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "toolu_1",
                "tool_input": {"command": "ls"}}),
        ];
        let begins = ["UserPromptSubmit", "PreToolUse", "Stop", "PostToolUse"];
        let gemini = ["BeforeAgent", "BeforeTool", "AfterAgent", "AfterTool"];
        let rows = [
            ("claude", claude, begins),
            ("codex", payloads(CODEX_HOOKS), begins),
            ("gemini", payloads(GEMINI_HOOKS), gemini),
            ("copilot", payloads(COPILOT_HOOKS), begins),
        ];
        for (agent, all, [begin, start, stop, end]) in rows {
            let events: Vec<HookEvent> = named(&all, &[begin, start, stop, end])
                .iter()
                .map(|p| parse_hook(agent, p).unwrap_or_else(|| panic!("{agent} {p}")))
                .collect();
            let mut tracker = StatusTracker::agent(0.0);
            for (at, event) in events.iter().enumerate() {
                tracker.on_hook(event, 1.0 + at as f64);
            }
            tracker.on_output(10.0);
            assert_eq!(tracker.status(10.1), AgentStatus::Done, "{agent}: the late end");
            tracker.on_hook(&events[1], 11.0);
            assert_eq!(tracker.status(11.1), AgentStatus::Working, "{agent}: a new call");
        }
    }

    fn claude_hook(payload: &Value) -> HookEvent {
        parse_hook("claude", payload).unwrap()
    }

    fn permission_for_bash() -> Vec<HookEvent> {
        let command = "git status --short --branch; ls -la ~/.claude/ledgers/";
        vec![
            claude_hook(&json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash",
                "tool_input": {"command": command, "description": "Check state"}})),
            claude_hook(
                &json!({"hook_event_name": "Notification", "notification_type": "permission_prompt",
                "message": "Claude needs your permission to use Bash"}),
            ),
        ]
    }

    fn bash_finished() -> HookEvent {
        claude_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_use_id": "toolu_1", "tool_response": {"stdout": ""},
            "tool_input": {"command": "git status --short --branch; ls -la ~/.claude/ledgers/",
                "description": "Check state"}}))
    }

    fn tidewatch_call_finished(tool: &str) -> HookEvent {
        claude_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": tool,
            "tool_input": {"note": "x"}, "tool_response": {"ok": true}}))
    }

    fn tracker_at_a_bash_permission() -> StatusTracker {
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 1.0);
        for event in permission_for_bash() {
            tracker.on_hook(&event, 5.0);
        }
        tracker
    }

    #[test]
    fn a_permission_holds_while_parallel_tool_calls_finish_and_the_terminal_prints() {
        let mut tracker = tracker_at_a_bash_permission();
        let needs = AgentStatus::NeedsYou(Reason::Permission);
        assert_eq!(tracker.status(5.5), needs);
        for (tool, at) in [("mcp__tidewatch__progress", 8.0), ("mcp__tidewatch__note", 9.0)] {
            tracker.on_output(at - 0.1);
            tracker.on_hook(&tidewatch_call_finished(tool), at);
            tracker.on_output(at);
            assert_eq!(tracker.status(at + 0.5), needs, "{tool}");
        }
        for second in 10..120 {
            tracker.on_output(f64::from(second));
        }
        assert_eq!(tracker.status(120.5), needs, "a spinner behind the prompt is not an answer");
        assert_eq!(tracker.next_change_at(120.5), None);
        tracker.on_hook(&bash_finished(), 121.0);
        assert_eq!(tracker.status(121.1), AgentStatus::Working);
    }

    #[test]
    fn a_held_permission_ends_when_it_is_answered_or_the_turn_moves_on() {
        use AgentStatus::{Done, NeedsYou, Working};
        let other_bash =
            claude_hook(&json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_input": {"command": "ls"}}));
        let cases: Vec<(&str, Vec<HookEvent>, bool, AgentStatus)> = vec![
            ("another Bash call ending", vec![other_bash], false, NeedsYou(Reason::Permission)),
            (
                "its own call failing",
                vec![claude_hook(&json!({"hook_event_name": "PostToolUseFailure",
                "tool_name": "Bash", "tool_input": {"command": "git status --short --branch; ls -la ~/.claude/ledgers/",
                "description": "Check state"}}))],
                false,
                Working,
            ),
            ("the Reader answering", vec![], true, Working),
            ("the turn stopping", vec![HookEvent::bare(HookKind::TurnDone)], false, Done),
            ("the next prompt", vec![HookEvent::bare(HookKind::TurnStarted)], false, Working),
            ("an idle notice", vec![HookEvent::bare(HookKind::IdleWaiting)], false, Done),
        ];
        for (name, events, answered, want) in cases {
            let mut tracker = tracker_at_a_bash_permission();
            for event in &events {
                tracker.on_hook(event, 20.0);
            }
            if answered {
                tracker.on_permission_answered(20.0);
            }
            tracker.on_output(20.1);
            assert_eq!(tracker.status(20.2), want, "{name}");
        }
    }

    #[test]
    fn a_permission_without_a_named_tool_call_still_decays_when_the_terminal_prints() {
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 4.0);
        tracker.on_hook(&bash_finished(), 5.0);
        assert_eq!(tracker.status(5.0), AgentStatus::NeedsYou(Reason::Permission));
        tracker.on_output(30.0);
        assert_eq!(tracker.status(30.1), AgentStatus::Working);
    }

    #[test]
    fn an_idle_notice_reads_as_done_unless_something_asks_for_you() {
        use AgentStatus::{Done, Failed, NeedsYou, Working};
        type Sequence = (&'static str, Vec<(HookKind, f64)>, f64, AgentStatus);
        let sequences: Vec<Sequence> = vec![
            (
                "after a finished turn",
                vec![
                    (HookKind::TurnStarted, 1.0),
                    (HookKind::TurnDone, 5.0),
                    (HookKind::IdleWaiting, 65.0),
                ],
                500.0,
                Done,
            ),
            ("no turn reported at all", vec![(HookKind::IdleWaiting, 65.0)], 3600.0, Done),
            (
                "a turn whose stop was never reported, such as an interrupt",
                vec![(HookKind::TurnStarted, 1.0), (HookKind::IdleWaiting, 65.0)],
                3600.0,
                Done,
            ),
            (
                "a permission prompt still open",
                vec![
                    (HookKind::TurnStarted, 1.0),
                    (HookKind::PermissionNeeded, 2.0),
                    (HookKind::IdleWaiting, 65.0),
                ],
                66.0,
                NeedsYou(Reason::Permission),
            ),
            (
                "a question still open",
                vec![
                    (HookKind::TurnStarted, 1.0),
                    (HookKind::QuestionAsked, 2.0),
                    (HookKind::IdleWaiting, 65.0),
                ],
                66.0,
                NeedsYou(Reason::Question),
            ),
            (
                "a failed turn",
                vec![
                    (HookKind::TurnFailed("rate_limit".into()), 5.0),
                    (HookKind::IdleWaiting, 65.0),
                ],
                66.0,
                Failed("rate_limit".into()),
            ),
            (
                "the next turn after the notice",
                vec![
                    (HookKind::TurnDone, 5.0),
                    (HookKind::IdleWaiting, 65.0),
                    (HookKind::TurnStarted, 70.0),
                ],
                71.0,
                Working,
            ),
        ];
        for (name, events, now, want) in sequences {
            let mut tracker = StatusTracker::agent(0.0);
            for (kind, at) in &events {
                tracker.on_hook(&HookEvent::bare(kind.clone()), *at);
            }
            assert_eq!(tracker.status(now), want, "{name}");
        }
    }

    #[test]
    fn a_resumed_agent_waits_as_done_until_its_next_turn() {
        let mut tracker = StatusTracker::setting_up(0.0);
        tracker.agent_resumed(2.0);
        tracker.on_output(3.0);
        assert_eq!(tracker.status(4.0), AgentStatus::Done);
        assert_eq!(tracker.status(3600.0), AgentStatus::Done);
        assert_eq!(tracker.next_change_at(4.0), None);
        tracker.on_hook(&HookEvent::bare(HookKind::IdleWaiting), 62.0);
        assert_eq!(tracker.status(63.0), AgentStatus::Done);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 70.0);
        assert_eq!(tracker.status(71.0), AgentStatus::Working);
        tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 72.0);
        assert_eq!(tracker.status(73.0), AgentStatus::NeedsYou(Reason::Permission));
    }

    #[test]
    fn tracker_uses_terminal_signals_when_no_hook_ever_arrives() {
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 1.0);
        assert_eq!(tracker.status(30.0), AgentStatus::Working);
        tracker.on_terminal(TerminalSignal::Progress(ProgressSignal::Off), 31.0);
        tracker.on_output(31.0);
        assert_eq!(tracker.status(35.0), AgentStatus::Working);
        assert_eq!(tracker.status(40.0), AgentStatus::NeedsYou(Reason::Quiet));
        tracker.on_terminal(TerminalSignal::Bell, 41.0);
        tracker.on_output(41.0);
        assert_eq!(tracker.status(41.5), AgentStatus::NeedsYou(Reason::Attention));
        tracker.on_terminal(TerminalSignal::TitleChanged, 50.0);
        tracker.on_output(50.0);
        assert_eq!(tracker.status(50.5), AgentStatus::Working);
    }

    #[test]
    fn tracker_moves_from_setting_up_to_agent_and_forgets_the_old_run() {
        let mut tracker = StatusTracker::setting_up(0.0);
        assert_eq!(tracker.status(5.0), AgentStatus::SettingUp);
        tracker.agent_started(6.0);
        assert_eq!(tracker.status(7.0), AgentStatus::Working);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnDone), 8.0);
        assert_eq!(tracker.status(9.0), AgentStatus::Done);
        tracker.agent_started(10.0);
        assert_eq!(tracker.status(11.0), AgentStatus::Working);
        assert_eq!(StatusTracker::shell().status(1.0), AgentStatus::Shell);
    }

    fn quiet_until_next_change(tracker: &StatusTracker, now: f64) -> Option<f64> {
        let next = tracker.next_change_at(now);
        let end = next.unwrap_or(now + 120.0);
        let first = tracker.status(now);
        let mut t = now;
        while t < end - 0.01 {
            assert_eq!(tracker.status(t), first, "status moved at {t} before the deadline {end}");
            t += 0.05;
        }
        next
    }

    #[test]
    fn next_change_names_each_decay_and_nothing_else() {
        let event = |kind| HookEvent::bare(kind);

        let mut working = StatusTracker::agent(0.0);
        working.on_hook(&event(HookKind::TurnStarted), 2.0);
        working.on_output(3.0);
        assert_eq!(quiet_until_next_change(&working, 4.0), Some(3.0 + STATUS_QUIET_SECS));
        assert_eq!(working.status(3.0 + STATUS_QUIET_SECS), AgentStatus::NeedsYou(Reason::Quiet));

        let mut busy = StatusTracker::agent(0.0);
        busy.on_hook(&event(HookKind::TurnStarted), 2.0);
        busy.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 2.0);
        assert_eq!(quiet_until_next_change(&busy, 4.0), Some(2.0 + BUSY_QUIET_SECS));

        let mut prompt = StatusTracker::agent(0.0);
        prompt.on_hook(&event(HookKind::PermissionNeeded), 10.0);
        assert_eq!(quiet_until_next_change(&prompt, 10.5), Some(10.0 + HOOK_SETTLE_SECS));
        assert_eq!(quiet_until_next_change(&prompt, 13.0), None);
        prompt.on_output(14.0);
        assert_eq!(quiet_until_next_change(&prompt, 14.1), Some(14.0 + ACTIVE_WINDOW_SECS));

        let mut done = StatusTracker::agent(0.0);
        done.on_hook(&event(HookKind::TurnDone), 5.0);
        assert_eq!(quiet_until_next_change(&done, 6.0), None);

        let mut fresh = StatusTracker::agent(0.0);
        assert_eq!(quiet_until_next_change(&fresh, 1.0), Some(STATUS_QUIET_SECS));
        fresh.on_terminal(TerminalSignal::TitleChanged, 2.0);
        assert_eq!(quiet_until_next_change(&fresh, 2.1), Some(2.0 + ACTIVE_WINDOW_SECS));

        let mut progress = StatusTracker::agent(0.0);
        progress.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 1.0);
        assert_eq!(quiet_until_next_change(&progress, 2.0), Some(BUSY_QUIET_SECS));
    }

    #[test]
    fn a_busy_progress_bar_holds_working_only_until_the_quiet_cap() {
        let event = |kind| HookEvent::bare(kind);

        let mut hooked = StatusTracker::agent(0.0);
        hooked.on_hook(&event(HookKind::TurnStarted), 2.0);
        hooked.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 2.0);
        hooked.on_output(10.0);
        let cap = 10.0 + BUSY_QUIET_SECS;
        assert_eq!(hooked.status(cap - 1.0), AgentStatus::Working);
        assert_eq!(quiet_until_next_change(&hooked, 20.0), Some(cap));
        assert_eq!(hooked.status(cap), AgentStatus::NeedsYou(Reason::Quiet));
        hooked.on_output(cap + 1.0);
        assert_eq!(hooked.status(cap + 2.0), AgentStatus::Working);

        let mut bare = StatusTracker::agent(0.0);
        bare.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 1.0);
        assert_eq!(bare.next_change_at(2.0), Some(BUSY_QUIET_SECS));
        assert_eq!(bare.status(BUSY_QUIET_SECS - 1.0), AgentStatus::Working);
        assert_eq!(bare.status(BUSY_QUIET_SECS), AgentStatus::NeedsYou(Reason::Quiet));
    }

    #[test]
    fn a_hook_counts_as_hearing_from_the_agent_for_the_quiet_cap() {
        let mut busy = StatusTracker::agent(0.0);
        busy.on_hook(&HookEvent::bare(HookKind::TurnStarted), 2.0);
        busy.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 2.0);
        let started = HookKind::ToolStarted { key: "Bash\nsleep".into(), id: None };
        busy.on_hook(&HookEvent::bare(started), 1000.0);
        assert_eq!(busy.next_change_at(1001.0), Some(1000.0 + BUSY_QUIET_SECS));
        assert_eq!(busy.status(BUSY_QUIET_SECS + 1.0), AgentStatus::Working);
        assert_eq!(busy.status(1000.0 + BUSY_QUIET_SECS), AgentStatus::NeedsYou(Reason::Quiet));
    }

    fn spinning(hooked: bool) -> StatusTracker {
        let mut tracker = StatusTracker::agent(0.0);
        tracker.set_hooked(hooked);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 2.0);
        tracker.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 2.0);
        let mut at = 3.0;
        while at < BUSY_QUIET_SECS + 60.0 {
            tracker.on_output(at);
            at += 1.0;
        }
        tracker
    }

    #[test]
    fn a_spinner_on_a_hooked_cli_still_reaches_the_quiet_cap() {
        let tracker = spinning(true);
        let cap = 2.0 + BUSY_QUIET_SECS;
        assert_eq!(tracker.status(cap - 1.0), AgentStatus::Working);
        assert_eq!(tracker.status(cap), AgentStatus::NeedsYou(Reason::Quiet));
        assert_eq!(tracker.next_change_at(10.0), Some(cap));
    }

    #[test]
    fn a_transcript_line_keeps_a_hooked_cli_past_the_quiet_cap() {
        let mut tracker = spinning(true);
        tracker.on_transcript(1500.0);
        assert_eq!(tracker.status(2.0 + BUSY_QUIET_SECS), AgentStatus::Working);
        assert_eq!(tracker.status(1500.0 + BUSY_QUIET_SECS), AgentStatus::NeedsYou(Reason::Quiet));
    }

    #[test]
    fn terminal_output_keeps_a_cli_with_no_hooks_working() {
        let tracker = spinning(false);
        assert_eq!(tracker.status(BUSY_QUIET_SECS + 2.0), AgentStatus::Working);
    }

    #[test]
    fn a_silent_working_or_quiet_agent_counts_as_at_rest_after_ten_minutes() {
        use AgentStatus::{Done, Failed, NeedsYou, Stopped, Working};
        let cases = [
            ("working, just heard", Working, 5.0, Working),
            ("working, nine minutes", Working, 540.0, Working),
            ("working, ten minutes", Working, REST_AFTER_SECS, Done),
            ("quiet, ten minutes", NeedsYou(Reason::Quiet), REST_AFTER_SECS, Done),
            ("quiet, a minute", NeedsYou(Reason::Quiet), 60.0, NeedsYou(Reason::Quiet)),
            (
                "permission prompt, an hour",
                NeedsYou(Reason::Permission),
                3600.0,
                NeedsYou(Reason::Permission),
            ),
            ("question, an hour", NeedsYou(Reason::Question), 3600.0, NeedsYou(Reason::Question)),
            ("failed, an hour", Failed("x".into()), 3600.0, Failed("x".into())),
            ("stopped, an hour", Stopped, 3600.0, Stopped),
        ];
        for (name, status, silent, want) in cases {
            assert_eq!(at_rest(&status, silent), want, "{name}");
        }
    }

    #[test]
    fn next_change_is_none_for_shells_setup_and_dead_processes() {
        assert_eq!(StatusTracker::shell().next_change_at(1.0), None);
        assert_eq!(StatusTracker::setting_up(0.0).next_change_at(1.0), None);
        let mut exited = StatusTracker::agent(0.0);
        exited.on_exit(Some(0));
        assert_eq!(exited.next_change_at(1.0), None);
    }

    #[test]
    fn a_turn_is_open_from_its_start_until_a_hook_ends_it() {
        let hook = |kind| HookEvent::bare(kind);
        let mut tracker = StatusTracker::agent(0.0);
        assert!(!tracker.turn_open());
        tracker.on_hook(&hook(HookKind::TurnStarted), 1.0);
        assert!(tracker.turn_open());
        tracker.on_hook(&hook(HookKind::PermissionNeeded), 2.0);
        assert!(tracker.turn_open(), "a prompt is inside the turn");
        tracker.on_hook(&hook(HookKind::TurnDone), 3.0);
        assert!(!tracker.turn_open());
        tracker.on_hook(&hook(HookKind::ToolStarted { key: "k".into(), id: None }), 4.0);
        assert!(tracker.turn_open(), "a call after the stop reopens the turn");
        tracker.on_hook(&hook(HookKind::TurnFailed("x".into())), 5.0);
        assert!(!tracker.turn_open());
    }

    #[test]
    fn lost_hooks_leave_the_terminal_and_the_process_to_decide() {
        let mut tracker = StatusTracker::agent(0.0);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 1.0);
        tracker.on_terminal(TerminalSignal::Progress(ProgressSignal::Working), 1.0);
        tracker.on_output(1.0);
        tracker.hooks_lost();
        assert!(!tracker.turn_open());
        assert_eq!(tracker.status(2.0), AgentStatus::Working, "the terminal still says busy");
        tracker.on_terminal(TerminalSignal::Progress(ProgressSignal::Off), 3.0);
        assert_eq!(tracker.status(3.5), AgentStatus::Working, "printing a moment ago");
        assert_eq!(tracker.status(20.0), AgentStatus::NeedsYou(Reason::Quiet));
    }

    #[test]
    fn a_status_tracker_survives_a_json_round_trip() {
        let mut tracker = StatusTracker::agent(0.5);
        tracker.set_hooked(true);
        tracker.on_hook(&HookEvent::bare(HookKind::TurnStarted), 1.25);
        tracker.on_hook(
            &HookEvent::bare(HookKind::ToolStarted {
                key: "Bash\nls".into(),
                id: Some("c1".into()),
            }),
            1.5,
        );
        tracker.on_hook(&HookEvent::bare(HookKind::PermissionNeeded), 1.75);
        tracker.on_output(2.0);
        let text = serde_json::to_string(&tracker).unwrap();
        assert_eq!(serde_json::from_str::<StatusTracker>(&text).unwrap(), tracker);
    }
}
