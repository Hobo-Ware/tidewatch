//! The git and disk work behind the History and Diff panes, run on `Jobs` threads. Each function
//! starts one job and returns at once; its `Done` arrives later through `Jobs::drain`. Results
//! carry the ticket of the request that made them, so the view can drop an answer that a newer
//! request has overtaken.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};

use kelp_core::commit::FileChange;
use kelp_core::diff::FileDiff;
use kelp_core::ops::Op;
use kelp_core::status::WorkingStatus;
use kelp_core::watch::{Change, Watcher};
use tidewatch_core::history::{self, Loaded, Scope, Source};
use tidewatch_core::notes::Notes;

use crate::agent_events;
use crate::agent_run::Finding;
use crate::checkout::{self, Outcome};
use crate::jobs::Jobs;

pub(crate) enum Done {
    History { tab: u64, ticket: u64, scope: Scope, result: Result<Box<Loaded>, String> },
    Status { tab: u64, ticket: u64, result: Result<WorkingStatus, String> },
    Files { tab: u64, ticket: u64, source: Source, result: Result<Box<Files>, String> },
    Diff { tab: u64, ticket: u64, result: Result<Box<FileDiff>, String> },
    Notes { tab: u64, notes: Box<Notes> },
    Watching { tab: u64, result: Result<Watcher, String> },
    Checkout { tab: u64, outcome: Outcome },
}

impl Done {
    pub(crate) fn tab(&self) -> u64 {
        match self {
            Self::History { tab, .. }
            | Self::Status { tab, .. }
            | Self::Files { tab, .. }
            | Self::Diff { tab, .. }
            | Self::Notes { tab, .. }
            | Self::Watching { tab, .. }
            | Self::Checkout { tab, .. } => *tab,
        }
    }
}

/// The files a source touched and the diff of the one to show first.
pub(crate) struct Files {
    pub(crate) list: Vec<FileChange>,
    pub(crate) shown: Option<(String, Result<FileDiff, String>)>,
}

pub(crate) fn load_history(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    ticket: u64,
    dir: PathBuf,
    scope: Scope,
) {
    jobs.spawn(ctx, move || Done::History {
        tab,
        ticket,
        scope,
        result: history::load(&dir, scope).map(Box::new).map_err(|e| format!("{e:#}")),
    });
}

pub(crate) fn load_status(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    ticket: u64,
    dir: PathBuf,
) {
    jobs.spawn(ctx, move || Done::Status {
        tab,
        ticket,
        result: history::working_status(&dir).map_err(|e| format!("{e:#}")),
    });
}

/// Lists the files of `source` and loads the diff of `prefer` when it is among them, else of the
/// first file.
pub(crate) fn load_files(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    ticket: u64,
    dir: PathBuf,
    source: Source,
    prefer: Option<String>,
) {
    jobs.spawn(ctx, move || Done::Files {
        tab,
        ticket,
        source,
        result: files(&dir, source, prefer.as_deref()).map(Box::new).map_err(|e| format!("{e:#}")),
    });
}

fn files(dir: &Path, source: Source, prefer: Option<&str>) -> anyhow::Result<Files> {
    let list = history::changed_files(dir, source)?;
    let shown = pick_file(&list, prefer).map(|path| {
        let diff = history::file_diff(dir, source, path).map_err(|e| format!("{e:#}"));
        (path.to_owned(), diff)
    });
    Ok(Files { list, shown })
}

pub(crate) fn pick_file<'a>(list: &'a [FileChange], prefer: Option<&str>) -> Option<&'a str> {
    prefer
        .and_then(|wanted| list.iter().find(|f| f.path == wanted))
        .or_else(|| list.first())
        .map(|f| f.path.as_str())
}

pub(crate) fn load_diff(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    ticket: u64,
    dir: PathBuf,
    source: Source,
    path: String,
) {
    jobs.spawn(ctx, move || Done::Diff {
        tab,
        ticket,
        result: history::file_diff(&dir, source, &path).map(Box::new).map_err(|e| format!("{e:#}")),
    });
}

/// Jobs that read a tab's notes file, change it and write it back take this turn, so two of them (a
/// finding, a send, a PR pull) never overwrite each other.
pub(crate) static NOTES_TURN: Mutex<()> = Mutex::new(());

/// Adds an agent's finding to the tab's notes file and answers with the notes as saved. Findings
/// take turns, so two arriving together never overwrite each other. The file is read from `dir`
/// (the tab's folder) only to anchor the note to the line's text.
pub(crate) fn add_finding(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    file: PathBuf,
    dir: PathBuf,
    author: String,
    finding: Finding,
    now: i64,
) {
    jobs.spawn(ctx, move || {
        let _turn = NOTES_TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notes = Notes::load(&file);
        let text = agent_events::stays_inside(&finding.file)
            .then(|| std::fs::read_to_string(dir.join(&finding.file)).ok())
            .flatten();
        agent_events::add_finding(&mut notes, &finding, &author, text.as_deref(), now);
        if let Err(e) = notes.save(&file) {
            eprintln!("tidewatch: could not save the finding to {}: {e}", file.display());
        }
        Done::Notes { tab, notes: Box::new(notes) }
    });
}

pub(crate) fn load_notes(jobs: &Jobs<Done>, ctx: &egui::Context, tab: u64, file: PathBuf) {
    jobs.spawn(ctx, move || Done::Notes { tab, notes: Box::new(Notes::load(&file)) });
}

/// Starts the file watcher off the UI thread (a recursive watch can take a while on a big tree).
/// Settled changes go to `changes`, and each one wakes the UI.
pub(crate) fn watch(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    dir: PathBuf,
    changes: Sender<Change>,
) {
    let wake = ctx.clone();
    jobs.spawn(ctx, move || Done::Watching {
        tab,
        result: history::watch(&dir, move |change| {
            let _ = changes.send(change);
            wake.request_repaint();
        })
        .map_err(|e| format!("{e:#}")),
    });
}

pub(crate) fn run_checkout(jobs: &Jobs<Done>, ctx: &egui::Context, tab: u64, dir: PathBuf, op: Op) {
    jobs.spawn(ctx, move || Done::Checkout { tab, outcome: checkout::run(&dir, &op) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::commit::ChangeKind;

    fn change(path: &str) -> FileChange {
        FileChange { path: path.into(), kind: ChangeKind::Modified }
    }

    #[test]
    fn the_preferred_file_wins_when_it_is_still_listed() {
        let list = [change("a.rs"), change("b.rs")];
        assert_eq!(pick_file(&list, Some("b.rs")), Some("b.rs"));
    }

    #[test]
    fn a_missing_or_unset_preference_falls_back_to_the_first_file() {
        let list = [change("a.rs"), change("b.rs")];
        assert_eq!(pick_file(&list, Some("gone.rs")), Some("a.rs"));
        assert_eq!(pick_file(&list, None), Some("a.rs"));
    }

    #[test]
    fn no_files_means_nothing_to_show() {
        assert_eq!(pick_file(&[], Some("a.rs")), None);
    }
}
