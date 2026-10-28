//! The Recently closed list's data and rules (pure): the records the window last read, which rows
//! they make (closed in the last 14 days, a session id, not open in a tab again), the fold, the
//! rows being reopened and when the file is read again.

use std::collections::HashSet;
use std::path::Path;

use tidewatch_core::closed_tabs::{self, Closed};
use tidewatch_core::elsewhere::age_text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) session_id: String,
    pub(crate) cli_id: String,
    pub(crate) title: String,
    pub(crate) repo: String,
    pub(crate) closed: String,
    pub(crate) busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) rows: Vec<Row>,
    pub(crate) open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardAction {
    Toggle,
    Reopen(String),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Fold {
    #[default]
    Folded,
    Open,
}

#[derive(Default)]
pub(crate) struct State {
    records: Vec<Closed>,
    fold: Fold,
    busy: HashSet<String>,
    stale: bool,
    loading: bool,
    was_shown: bool,
}

impl State {
    pub(crate) fn new(open: bool) -> Self {
        Self { fold: if open { Fold::Open } else { Fold::Folded }, ..Self::default() }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.fold == Fold::Open
    }

    pub(crate) fn toggle(&mut self) {
        self.fold = if self.is_open() { Fold::Folded } else { Fold::Open };
    }

    /// A tab closed since the file was read.
    pub(crate) fn mark_changed(&mut self) {
        self.stale = true;
    }

    /// Whether to read the file now, and if so it counts as started: when the Overview opens and
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

    pub(crate) fn card(
        &self,
        now: u64,
        is_open: &dyn Fn(&str) -> bool,
        place: &dyn Fn(&Path) -> String,
    ) -> Option<Card> {
        let rows: Vec<Row> = closed_tabs::listing(&self.records, now, is_open)
            .into_iter()
            .filter_map(|closed| {
                let id = closed.session_id.as_deref()?;
                Some(Row {
                    session_id: id.to_owned(),
                    cli_id: closed.agent.clone(),
                    title: closed.title.clone(),
                    repo: place(&closed.repo),
                    closed: closed_text(now, closed.at),
                    busy: self.busy.contains(id),
                })
            })
            .collect();
        (!rows.is_empty()).then_some(Card { rows, open: self.is_open() })
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
pub(crate) fn fake_card(open: bool) -> Card {
    let row = |n: &str, title: &str, repo: &str, closed: &str| Row {
        session_id: format!("fake-{n}"),
        cli_id: "claude".to_owned(),
        title: title.to_owned(),
        repo: repo.to_owned(),
        closed: closed.to_owned(),
        busy: false,
    };
    Card {
        rows: vec![
            row("a", "retry-stage-gates", "acme-web", "closed 2h ago"),
            row("b", "fix-login-redirect", "acme-api", "closed 1d ago"),
            row("c", "upgrade-egui-0-36", "acme-app", "closed 5d ago"),
        ],
        open,
    }
}

#[cfg(test)]
mod tests;
