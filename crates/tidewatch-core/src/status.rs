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
//! - With no hook state, progress, bell, title activity and quiet time decide.
//!
//! Claude's `SessionStart` http hook never fired in `claude` 2.1.296 (interactive or `-p`), so the
//! first Claude status comes from `UserPromptSubmit` or from the lower layers.

use std::path::PathBuf;

use serde_json::Value;

/// Output silence after which an agent is assumed to be waiting for you (PLAN.md, Constants).
pub const STATUS_QUIET_SECS: f64 = 8.0;
/// A prompt hook state is trusted for this long before output can show the agent moved on.
pub const HOOK_SETTLE_SECS: f64 = 2.0;
/// Output newer than this counts as "printing right now".
pub const ACTIVE_WINDOW_SECS: f64 = 1.5;
/// A bell counts as unanswered until the terminal prints something at least this long after it.
pub const BELL_SETTLE_SECS: f64 = 1.0;

const MESSAGE_MAX_CHARS: usize = 400;
/// A hook's last message is kept nearly whole: the Done / Needs you / Next block sits at its end.
const TURN_MESSAGE_MAX_CHARS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Shell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookKind {
    SessionStart,
    TurnStarted,
    PermissionNeeded,
    QuestionAsked,
    IdleWaiting,
    TurnDone,
    TurnFailed(String),
    SessionEnd,
}

/// What one hook payload (or Copilot transcript event) says, in CLI-neutral terms.
#[derive(Clone, Debug, PartialEq, Eq)]
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
/// Claude, Codex and Gemini name the event in `hook_event_name`. Copilot has no hook wiring in v0;
/// its `events.jsonl` lines (`{"type": "...", "data": {...}}`) go through here instead, which is how
/// the spikes found its status. Checked against claude 2.1.296, gemini 0.46.0 and copilot 1.0.91;
/// Codex is assumed to follow Claude's names (`docs/research/phase-0-spikes.md`).
#[must_use]
pub fn parse_hook(agent: &str, payload: &Value) -> Option<HookEvent> {
    let object = payload.as_object()?;
    let mut event =
        if agent == "copilot" { parse_copilot_event(object)? } else { parse_named_event(object)? };
    event.session_id = event.session_id.or_else(|| text(object.get("session_id")));
    event.transcript_path =
        event.transcript_path.or_else(|| text(object.get("transcript_path")).map(PathBuf::from));
    Some(event)
}

fn parse_named_event(object: &serde_json::Map<String, Value>) -> Option<HookEvent> {
    let name = object.get("hook_event_name")?.as_str()?;
    let message = |key: &str| text(object.get(key)).map(|m| clip_to(&m, TURN_MESSAGE_MAX_CHARS));
    let (kind, message) = match name {
        "SessionStart" => (HookKind::SessionStart, None),
        "UserPromptSubmit" | "BeforeAgent" => (HookKind::TurnStarted, None),
        "PermissionRequest" => (HookKind::PermissionNeeded, tool_summary(object)),
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
    clip_to(message, MESSAGE_MAX_CHARS)
}

fn clip_to(message: &str, max: usize) -> String {
    if message.chars().count() <= max {
        return message.to_owned();
    }
    let mut clipped: String = message.chars().take(max).collect();
    clipped.push_str("...");
    clipped
}

/// OSC 9;4 progress as the status layer needs it. The app maps its terminal's progress onto this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressSignal {
    #[default]
    Off,
    Working,
    Error,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TerminalSignal {
    Progress(ProgressSignal),
    TitleChanged,
    Bell,
}

/// Layer 3: what the operating system says about the agent process.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessFacts {
    pub alive: bool,
    /// `None` while alive, or when a signal killed it.
    pub exit_code: Option<i32>,
    pub started_at: f64,
    pub last_output_at: Option<f64>,
}

impl ProcessFacts {
    #[must_use]
    pub fn started(at: f64) -> Self {
        Self { alive: true, exit_code: None, started_at: at, last_output_at: None }
    }

    fn quiet_for(&self, now: f64) -> f64 {
        now - self.last_output_at.unwrap_or(self.started_at)
    }

    fn printing_since(&self, since: f64, now: f64) -> bool {
        self.last_output_at.is_some_and(|t| t > since && now - t <= ACTIVE_WINDOW_SECS)
    }
}

/// Layer 2: the latest terminal signals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerminalState {
    pub progress: ProgressSignal,
    pub title_changed_at: Option<f64>,
    pub bell_at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HookStatus {
    Working,
    NeedsYou(Reason),
    Done,
    Failed(String),
}

/// Layer 1: the latest status a hook reported, and when it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct HookState {
    status: HookStatus,
    at: f64,
}

impl HookState {
    /// The state an event sets, if it sets one. `SessionStart` and `SessionEnd` carry none: the lower
    /// layers (and the process exit) say more.
    #[must_use]
    pub fn from_event(event: &HookEvent, now: f64) -> Option<Self> {
        let status = match &event.kind {
            HookKind::TurnStarted => HookStatus::Working,
            HookKind::PermissionNeeded => HookStatus::NeedsYou(Reason::Permission),
            HookKind::QuestionAsked => HookStatus::NeedsYou(Reason::Question),
            HookKind::IdleWaiting => HookStatus::NeedsYou(Reason::Input),
            HookKind::TurnDone => HookStatus::Done,
            HookKind::TurnFailed(reason) => HookStatus::Failed(reason.clone()),
            HookKind::SessionStart | HookKind::SessionEnd => return None,
        };
        Some(Self { status, at: now })
    }

    /// An idle notice arrives a minute after a finished turn and must not turn "done" back amber.
    fn after(self, previous: Option<&Self>) -> Self {
        match (&self.status, previous.map(|p| &p.status)) {
            (HookStatus::NeedsYou(Reason::Input), Some(HookStatus::Done)) => {
                previous.cloned().unwrap_or(self)
            }
            _ => self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// A plain terminal: no agent, no status.
    Shell,
    /// The worktree or its setup command is still running.
    SettingUp,
    Agent,
}

#[derive(Clone, Debug, PartialEq)]
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
            if !busy && silent_for >= STATUS_QUIET_SECS {
                AgentStatus::NeedsYou(Reason::Quiet)
            } else {
                AgentStatus::Working
            }
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
        Some(HookState { status: HookStatus::Working, at }) => {
            if terminal.progress == ProgressSignal::Working {
                Vec::new()
            } else {
                vec![process.last_output_at.unwrap_or(*at).max(*at) + STATUS_QUIET_SECS]
            }
        }
        Some(HookState {
            status: HookStatus::NeedsYou(Reason::Permission | Reason::Question),
            at,
        }) => {
            let mut marks = vec![at + HOOK_SETTLE_SECS];
            marks.extend(process.last_output_at.map(|t| t + ACTIVE_WINDOW_SECS));
            marks
        }
        Some(_) => Vec::new(),
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

/// Keeps one tab's [`StatusInputs`] current. Every method takes the time it happened.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusTracker {
    inputs: StatusInputs,
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
        }
    }

    /// Setup finished: the agent process starts now.
    pub fn agent_started(&mut self, now: f64) {
        self.inputs.lifecycle = Lifecycle::Agent;
        self.inputs.process = ProcessFacts::started(now);
        self.inputs.hook = None;
        self.inputs.terminal = TerminalState::default();
    }

    pub fn on_hook(&mut self, event: &HookEvent, now: f64) {
        if let Some(state) = HookState::from_event(event, now) {
            let next = state.after(self.inputs.hook.as_ref());
            self.inputs.hook = Some(next);
        }
    }

    pub fn on_terminal(&mut self, signal: TerminalSignal, now: f64) {
        let terminal = &mut self.inputs.terminal;
        match signal {
            TerminalSignal::Progress(progress) => terminal.progress = progress,
            TerminalSignal::TitleChanged => terminal.title_changed_at = Some(now),
            TerminalSignal::Bell => terminal.bell_at = Some(now),
        }
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
                Some((HookKind::PermissionNeeded, Some("Bash: touch zz.txt".into()))),
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
                None,
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
        let long = "x".repeat(TURN_MESSAGE_MAX_CHARS * 2);
        let event = parse_hook(
            "claude",
            &json!({"hook_event_name": "Stop", "last_assistant_message": long}),
        )
        .unwrap();
        let message = event.message.unwrap();
        assert_eq!(message.chars().count(), TURN_MESSAGE_MAX_CHARS + 3);
        assert!(message.ends_with("..."));
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
                name: "idle prompt is not undone by typing",
                inputs: with(|i| {
                    i.hook = hook(HookKind::IdleWaiting, 10.0);
                    i.process.last_output_at = printing(30.0);
                }),
                now: 30.1,
                want: NeedsYou(Reason::Input),
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
        assert_eq!(quiet_until_next_change(&busy, 4.0), None);

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
        assert_eq!(quiet_until_next_change(&progress, 2.0), None);
    }

    #[test]
    fn next_change_is_none_for_shells_setup_and_dead_processes() {
        assert_eq!(StatusTracker::shell().next_change_at(1.0), None);
        assert_eq!(StatusTracker::setting_up(0.0).next_change_at(1.0), None);
        let mut exited = StatusTracker::agent(0.0);
        exited.on_exit(Some(0));
        assert_eq!(exited.next_change_at(1.0), None);
    }
}
