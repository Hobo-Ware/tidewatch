//! The closed sessions a new tab can resume, as data (pure): the records the window last read, the
//! rows of one repo (closed in the last 14 days, a session id, not open in a tab again), the rows
//! being reopened and when the file is read again.

use std::collections::HashSet;
use std::path::Path;

use tidewatch_core::avatar::Avatar;
use tidewatch_core::closed_tabs::{self, Closed};
use tidewatch_core::elsewhere::age_text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) session_id: String,
    pub(crate) cli_id: String,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) branch: String,
    pub(crate) closed: String,
    pub(crate) busy: bool,
    pub(crate) avatar: Option<Avatar>,
}

#[derive(Default)]
pub(crate) struct State {
    records: Vec<Closed>,
    busy: HashSet<String>,
    stale: bool,
    loading: bool,
    was_shown: bool,
}

impl State {
    /// A tab closed since the file was read.
    pub(crate) fn mark_changed(&mut self) {
        self.stale = true;
    }

    /// Whether to read the file now, and if so it counts as started: when a new-tab form shows and
    /// after a tab closed, one read at a time.
    pub(crate) fn begin_load(&mut self, shown: bool) -> bool {
        let opened = shown && !self.was_shown;
        self.was_shown = shown;
        if self.loading || !shown || !(opened || self.stale) {
            return false;
        }
        self.loading = true;
        self.stale = false;
        true
    }

    pub(crate) fn finish_load(&mut self, records: Vec<Closed>) {
        self.loading = false;
        self.records = records;
    }

    /// Takes the newest record of a session for a reopen and marks it busy. `None` while that
    /// session is already being reopened or has no record.
    pub(crate) fn press(&mut self, session_id: &str, now: u64) -> Option<Closed> {
        if self.busy.contains(session_id) {
            return None;
        }
        let found = closed_tabs::listing(&self.records, now, &|_| false)
            .into_iter()
            .find(|closed| closed.session_id.as_deref() == Some(session_id))
            .cloned()?;
        self.busy.insert(session_id.to_owned());
        Some(found)
    }

    pub(crate) fn finished(&mut self, session_id: &str) {
        self.busy.remove(session_id);
    }

    /// The sessions of `repo` that can come back, newest first.
    pub(crate) fn rows(&self, repo: &Path, now: u64, is_open: &dyn Fn(&str) -> bool) -> Vec<Row> {
        closed_tabs::listing(&self.records, now, is_open)
            .into_iter()
            .filter(|closed| closed.repo == repo)
            .filter_map(|closed| {
                let id = closed.session_id.as_deref()?;
                Some(Row {
                    session_id: id.to_owned(),
                    cli_id: closed.agent.clone(),
                    title: closed.shown().to_owned(),
                    summary: closed.summary.clone().filter(|text| !text.trim().is_empty()),
                    branch: closed.branch.clone(),
                    closed: closed_text(now, closed.at),
                    busy: self.busy.contains(id),
                    avatar: closed.avatar.clone(),
                })
            })
            .collect()
    }
}

pub(crate) fn closed_text(now: u64, at: u64) -> String {
    let elapsed_ms = i64::try_from(now.saturating_sub(at).saturating_mul(1000)).unwrap_or(i64::MAX);
    format!("closed {}", age_text(elapsed_ms))
}

pub(crate) fn fake_requested() -> bool {
    std::env::var_os("TIDEWATCH_FAKE_CLOSED").is_some()
}

/// `TIDEWATCH_FAKE_CLOSED=1`: three stand-in rows for captures; nothing is read or reopened.
pub(crate) fn fake_rows() -> Vec<Row> {
    let row = |n: &str, title: &str, summary: Option<&str>, closed: &str, art: usize| Row {
        session_id: format!("fake-{n}"),
        cli_id: "claude".to_owned(),
        title: title.to_owned(),
        summary: summary.map(str::to_owned),
        branch: format!("agent/{title}"),
        closed: closed.to_owned(),
        busy: false,
        avatar: (art > 0)
            .then(|| crate::avatar_glue::fake_icon_face(art).map(|avatar| Avatar::clone(&avatar)))
            .flatten(),
    };
    vec![
        row(
            "a",
            "retry-stage-gates",
            Some(
                "Retry the stage gates when a runner drops, so one flaky host no longer fails a release",
            ),
            "closed 2h ago",
            1,
        ),
        row(
            "b",
            "fix-login-redirect",
            Some("Send people back to the page they came from after they sign in"),
            "closed 1d ago",
            2,
        ),
        row("c", "upgrade-egui-0-36", None, "closed 5d ago", 0),
    ]
}

#[cfg(test)]
mod tests;
