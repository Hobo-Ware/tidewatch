//! The window's side of the comment guard: each agent tab's flags read from its state folder (the
//! hook process appends them), the per-tab off switch, and, for the CLIs with no hook of their own,
//! the branch sweep at the end of a turn and the note pasted to the idle agent. The same path pastes
//! the ledger guard's line to such an agent when a turn that used tools ended with no ledger saved.
//!
//! `TIDEWATCH_FAKE_COMMENTS=1` gives every agent tab three stand-in flags and writes nothing;
//! `TIDEWATCH_DEV_COMMENT_POPOVER=1` keeps the chip's list open.

use std::collections::HashMap;
use std::path::Path;

use tidewatch_core::comment_flags::{Origin, Record, TabFiles};
use tidewatch_core::comment_sweep::{self, FileFlags};
use tidewatch_core::guard_comments::{digest, records};
use tidewatch_core::guard_ledger::{Ask, Reminders, ledger_says_working, modified_ms};
use tidewatch_core::guards::{GuardPrefs, Rule};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::sent::{Reason, because};
use tidewatch_core::status::AgentStatus;

use crate::comment_model::{Turn, needs_idle_sweep, paste_note};
use crate::jobs::Jobs;

const LOOK_EVERY_S: f64 = 1.0;

pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) agent_id: &'a str,
    pub(crate) status: &'a AgentStatus,
    pub(crate) dir: &'a Path,
    pub(crate) repo: &'a Path,
    pub(crate) name: &'a str,
    /// When the last turn started, in the egui clock.
    pub(crate) turn_started_at: Option<f64>,
    pub(crate) used_tools: bool,
}

/// A note to paste into a tab's idle agent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Paste {
    pub(crate) tab: u64,
    pub(crate) text: String,
    pub(crate) reason: Reason,
    pub(crate) because: &'static str,
}

struct Swept {
    tab: u64,
    files: Option<Vec<FileFlags>>,
}

struct TabState {
    files: TabFiles,
    records: Vec<Record>,
    flags_len: u64,
    off: bool,
    turn: Turn,
    reminders: Reminders,
    sweeping: bool,
    looked: f64,
}

pub(crate) struct Host {
    tabs: HashMap<u64, TabState>,
    jobs: Jobs<Swept>,
    fake: bool,
    popover: bool,
    wall_ms: fn() -> u64,
}

/// What the strip draws.
pub(crate) struct View<'a> {
    pub(crate) records: &'a [Record],
    pub(crate) off: bool,
}

fn fake_records() -> Vec<Record> {
    let at = |n: u64, file: &str, line: usize, kind: &str, text: &str| Record {
        at_ms: n,
        origin: Origin::Edit,
        file: file.to_owned(),
        line,
        kind: kind.to_owned(),
        text: text.to_owned(),
    };
    vec![
        at(1, "src/retry.rs", 14, "narration", "loop over the attempts"),
        at(2, "src/retry.rs", 31, "dead-code", "let delay = backoff(attempt);"),
        at(3, "web/queue.ts", 8, "alibi", "hack for now, needs a proper fix"),
    ]
}

impl Host {
    pub(crate) fn new() -> Self {
        let fake =
            std::env::var_os("TIDEWATCH_FAKE_COMMENTS").is_some_and(|v| !v.is_empty() && v != "0");
        let popover =
            std::env::var("TIDEWATCH_DEV_COMMENT_POPOVER").is_ok_and(|v| !v.is_empty() && v != "0");
        Self { tabs: HashMap::new(), jobs: Jobs::new(), fake, popover, wall_ms }
    }

    pub(crate) fn open_popover(&self) -> bool {
        self.popover
    }

    pub(crate) fn view(&self, tab: u64) -> Option<View<'_>> {
        let state = self.tabs.get(&tab)?;
        (!state.records.is_empty() || state.off)
            .then_some(View { records: &state.records, off: state.off })
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.tabs.remove(&tab);
    }

    pub(crate) fn set_off(&mut self, tab: u64, off: bool) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        if self.fake || state.files.set_off(off).is_ok() {
            state.off = off;
        }
    }

    /// One look at every agent tab: new flags read, a finished turn swept (only a CLI with no hook of
    /// its own), a finished sweep turned into a note for an idle agent. Finished sweeps are taken
    /// before a new one starts.
    pub(crate) fn tick(
        &mut self,
        facts: &[Facts<'_>],
        paths: &TidewatchPaths,
        guards: &GuardPrefs,
        ctx: &egui::Context,
        now: f64,
    ) -> Vec<Paste> {
        let mut pastes = Vec::new();
        for swept in self.jobs.drain() {
            let idle = facts
                .iter()
                .find(|f| f.id == swept.tab)
                .is_some_and(|f| matches!(f.status, AgentStatus::Done));
            pastes.extend(self.finish_sweep(swept, idle));
        }
        self.tabs.retain(|id, _| facts.iter().any(|fact| fact.id == *id));
        for fact in facts {
            let fresh = !self.tabs.contains_key(&fact.id);
            let state = self.tabs.entry(fact.id).or_insert_with(|| TabState {
                files: TabFiles::new(&paths.tab_state_dir(fact.repo, fact.name)),
                records: if self.fake { fake_records() } else { Vec::new() },
                flags_len: 0,
                off: false,
                turn: Turn::default(),
                reminders: Reminders::default(),
                sweeping: false,
                looked: f64::NEG_INFINITY,
            });
            if !self.fake && (fresh || now - state.looked >= LOOK_EVERY_S) {
                state.look(now);
            }
            let turn_ended = state.turn.ended(fact.status);
            if turn_ended && needs_idle_sweep(fact.agent_id) && !self.fake {
                pastes.extend(state.ledger_reminder(fact, paths, now, (self.wall_ms)()));
            }
            let rule_on = guards.rules_for(fact.repo).is_on(Rule::Comments);
            if turn_ended
                && rule_on
                && !state.off
                && !state.sweeping
                && needs_idle_sweep(fact.agent_id)
                && !self.fake
            {
                state.sweeping = true;
                let (tab, dir) = (fact.id, fact.dir.to_path_buf());
                self.jobs.spawn(ctx, move || Swept { tab, files: comment_sweep::sweep(&dir) });
            }
        }
        pastes
    }

    fn finish_sweep(&mut self, swept: Swept, idle: bool) -> Option<Paste> {
        let state = self.tabs.get_mut(&swept.tab)?;
        state.sweeping = false;
        let files = swept.files.filter(|files| !files.is_empty());
        let Some(files) = files else {
            let _ = state.files.set_swept(None);
            return None;
        };
        let note = paste_note(&files);
        let seen = digest(&note);
        if !idle || state.off || state.files.swept().as_deref() == Some(seen.as_str()) {
            return None;
        }
        let _ = state.files.set_swept(Some(&seen));
        let at = wall_ms();
        let all: Vec<Record> = files
            .iter()
            .flat_map(|file| records(Origin::Sweep, &file.path, &file.flags, at))
            .collect();
        let _ = state.files.append(&all);
        Some(Paste {
            tab: swept.tab,
            text: note,
            reason: Reason::CommentGuard,
            because: because::COMMENTS,
        })
    }
}

fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl TabState {
    fn ledger_reminder(
        &mut self,
        fact: &Facts<'_>,
        paths: &TidewatchPaths,
        now: f64,
        at: u64,
    ) -> Option<Paste> {
        let started = fact
            .turn_started_at
            .map(|began| at.saturating_sub(((now - began).max(0.0) * 1000.0) as u64));
        let ledger = paths.progress_file(fact.repo, fact.name);
        let ask = self.reminders.at_turn_end(
            fact.used_tools,
            modified_ms(&ledger),
            ledger_says_working(&ledger),
            started,
            at,
        )?;
        let because = match ask {
            Ask::Ledger => because::LEDGER_STOP,
            Ask::FinalState => because::LEDGER_FINAL,
        };
        Some(Paste {
            tab: fact.id,
            text: ask.message().to_owned(),
            reason: Reason::LedgerGuard,
            because,
        })
    }

    fn look(&mut self, now: f64) {
        self.looked = now;
        self.off = self.files.is_off();
        let len = std::fs::metadata(self.files.flags_file()).map_or(0, |meta| meta.len());
        if len != self.flags_len {
            self.flags_len = len;
            self.records = self.files.load();
        }
    }
}

#[cfg(test)]
mod tests;
