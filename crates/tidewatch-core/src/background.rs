//! Background Bash calls of Claude Code (hook shape checked on 2.1.310): an idle agent cannot call `report_job`, so the script writes the feed itself.

use crate::text::squash;
use serde_json::Value;

pub const QUIET_MS: i64 = 90_000;
pub const PROGRESS_EXAMPLE: &str = "printf '{\"name\":\"import\",\"done\":%s,\"total\":%s,\"unit\":\"rows\",\"phase\":\"load\"}\\n' \"$n\" \"$total\" >> \"$TIDEWATCH_PROGRESS\"";

/// What the prompt of a background task's notice starts with.
pub const NOTICE_OPENING: &str = "<task-notification>";

const MAX_LAUNCHES: usize = 16;
const COMMAND_CHARS: usize = 60;
const ENDED: [&str; 3] = ["completed", "failed", "killed"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    Started { tool_use_id: String, command: String },
    Ended { tool_use_id: String },
}

#[must_use]
pub fn from_hook(agent: &str, payload: &Value) -> Option<Signal> {
    if agent != "claude" {
        return None;
    }
    let object = payload.as_object()?;
    match object.get("hook_event_name")?.as_str()? {
        "PreToolUse" => {
            let input = object.get("tool_input")?;
            if object.get("tool_name")?.as_str()? != "Bash"
                || input.get("run_in_background")?.as_bool() != Some(true)
            {
                return None;
            }
            Some(Signal::Started {
                tool_use_id: object.get("tool_use_id")?.as_str()?.to_owned(),
                command: short_command(input.get("command")?.as_str()?),
            })
        }
        "UserPromptSubmit" => {
            let notice = object.get("prompt")?.as_str()?.trim_start();
            if !notice.starts_with(NOTICE_OPENING) {
                return None;
            }
            let tool_use_id = tag(notice, "tool-use-id")?.to_owned();
            ENDED.contains(&tag(notice, "status")?).then_some(Signal::Ended { tool_use_id })
        }
        _ => None,
    }
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(format!("<{name}>").as_str())?;
    Some(rest.split_once(format!("</{name}>").as_str())?.0.trim())
}

fn short_command(command: &str) -> String {
    let line = squash(command);
    if line.chars().count() <= COMMAND_CHARS {
        return line;
    }
    let cut: String = line.chars().take(COMMAND_CHARS).collect();
    format!("{}...", cut.trim_end())
}

#[must_use]
pub fn nudge_text(command: &str) -> String {
    format!(
        "Your background command `{command}` has reported no progress to Tidewatch. Make the job itself append JSON lines to `$TIDEWATCH_PROGRESS`, for example `{PROGRESS_EXAMPLE}`, then tail the file and confirm the first line landed."
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub tool_use_id: String,
    pub command: String,
    pub at_ms: i64,
    nudged: bool,
}

/// One tab's running background launches, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    launches: Vec<Launch>,
}

impl Watch {
    pub fn on_signal(&mut self, signal: Signal, now_ms: i64) {
        match signal {
            Signal::Started { tool_use_id, command } => {
                if self.launches.iter().any(|launch| launch.tool_use_id == tool_use_id) {
                    return;
                }
                if self.launches.len() == MAX_LAUNCHES {
                    self.launches.remove(0);
                }
                self.launches.push(Launch { tool_use_id, command, at_ms: now_ms, nudged: false });
            }
            Signal::Ended { tool_use_id } => {
                self.launches.retain(|launch| launch.tool_use_id != tool_use_id);
            }
        }
    }

    fn quiet(launch: &Launch, last_report_ms: Option<i64>, now_ms: i64) -> bool {
        now_ms - launch.at_ms >= QUIET_MS && last_report_ms.is_none_or(|at| at < launch.at_ms)
    }

    /// Launches that ran at least `QUIET_MS` with no report since they began.
    #[must_use]
    pub fn silent(&self, last_report_ms: Option<i64>, now_ms: i64) -> Vec<&Launch> {
        self.launches.iter().filter(|launch| Self::quiet(launch, last_report_ms, now_ms)).collect()
    }

    /// The oldest silent launch that was not nudged yet, while the agent is idle.
    #[must_use]
    pub fn due(&self, last_report_ms: Option<i64>, idle: bool, now_ms: i64) -> Option<&Launch> {
        if !idle {
            return None;
        }
        self.launches
            .iter()
            .find(|launch| !launch.nudged && Self::quiet(launch, last_report_ms, now_ms))
    }

    pub fn nudged(&mut self, tool_use_id: &str) {
        for launch in &mut self.launches {
            if launch.tool_use_id == tool_use_id {
                launch.nudged = true;
            }
        }
    }

    /// When the next launch turns quiet, or a minute on while one already is.
    #[must_use]
    pub fn next_change_ms(&self, last_report_ms: Option<i64>, now_ms: i64) -> Option<i64> {
        self.launches
            .iter()
            .filter(|launch| last_report_ms.is_none_or(|at| at < launch.at_ms))
            .map(|launch| match launch.at_ms + QUIET_MS {
                quiet if quiet > now_ms => quiet,
                _ => now_ms + 60_000,
            })
            .min()
    }
}

#[cfg(test)]
mod tests;
