//! What the running agents list says as plain data (pure, table-tested): the rows of the agents a
//! Claude session started, their header line and which of them fit.

use tidewatch_core::subagents::{Agent, Phase, shown};
use tidewatch_core::text::plural;
use tidewatch_core::token_usage::compact;

pub(crate) const MAX_ROWS: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) what: String,
    pub(crate) tag: Option<String>,
    pub(crate) right: String,
    pub(crate) running: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct List {
    pub(crate) rows: Vec<Row>,
    pub(crate) running: usize,
    pub(crate) finished: usize,
}

/// How much of a list is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) rows: usize,
    pub(crate) more: usize,
    pub(crate) can_fold_back: bool,
}

/// `4 agents running`, none for no agent running: the line of a tab's tooltip and an Overview card.
pub(crate) fn running_line(running: usize) -> Option<String> {
    (running > 0).then(|| format!("{} running", plural(running, "agent", "agents")))
}

/// `42s`, `4m 13s`, `1h 02m`.
pub(crate) fn clock(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn tag(agent: &Agent) -> Option<String> {
    let parts: Vec<&str> =
        [agent.model.as_deref(), agent.effort.as_deref()].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

fn row(agent: &Agent, now_ms: i64) -> Row {
    let time = clock(agent.elapsed_ms(now_ms));
    let (what, right) = match agent.phase {
        Phase::Running => {
            let right = if agent.tokens == 0 {
                time
            } else {
                format!("{time} \u{b7} {}", compact(agent.tokens))
            };
            (agent.now.clone(), right)
        }
        Phase::Finished(_) => {
            let what = if agent.description.is_empty() { &agent.now } else { &agent.description };
            (what.clone(), format!("done {time}"))
        }
    };
    Row { id: agent.id.clone(), what, tag: tag(agent), right, running: agent.is_running() }
}

/// The list at `now_ms`: running agents and those finished within the last minute. None when
/// there is nothing to show.
pub(crate) fn list(agents: &[Agent], now_ms: i64) -> Option<List> {
    let visible = shown(agents, now_ms);
    let running = visible.iter().filter(|agent| agent.is_running()).count();
    (!visible.is_empty()).then(|| List {
        rows: visible.iter().map(|agent| row(agent, now_ms)).collect(),
        running,
        finished: visible.len() - running,
    })
}

impl List {
    pub(crate) fn head(&self) -> String {
        if let Some(line) = running_line(self.running) {
            line
        } else {
            format!("{} finished", plural(self.finished, "agent", "agents"))
        }
    }

    /// What stands before the Hide or Show word at the right of the header.
    pub(crate) fn finished_note(&self) -> Option<String> {
        (self.running > 0 && self.finished > 0).then(|| format!("{} finished", self.finished))
    }

    /// How many rows are drawn when `fit` rows have room: `MAX_ROWS` until the rest is opened, and
    /// never more than fit.
    pub(crate) fn plan(&self, opened: bool, fit: usize) -> Plan {
        let total = self.rows.len();
        let limit = if opened { total } else { MAX_ROWS };
        let rows = limit.min(total).min(fit.max(1));
        Plan { rows, more: total - rows, can_fold_back: opened && total > MAX_ROWS }
    }
}

#[cfg(test)]
mod tests;
