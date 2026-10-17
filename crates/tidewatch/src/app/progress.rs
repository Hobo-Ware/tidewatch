//! The app's side of the progress ledgers: the agent tabs' facts go to the host each frame, and a
//! ledger an agent sends is stored and logged.

use tidewatch_core::progress::Ledger;

use std::time::Duration;

use super::WAKE_SLACK_S;
use super::settings::open_in_editor;
use super::{Tidewatch, wall_ms};
use crate::coming_up::{Row, coming_up};
use crate::due::{Due, Key, ask_blocked, due_items, secs_to_midnight, tell_all};
use crate::note_send::ENTER_DELAY_SECS;
use crate::notify;
use crate::overview::Overview;
use crate::overview_checklist::Source;
use crate::pane::Body;
use crate::progress_glue::today;
use crate::progress_model::look;
use crate::progress_state::Facts;
use crate::{due_view, flow_glue};
use tidewatch_core::progress::Mark;

impl Tidewatch {
    pub(super) fn drive_progress(&mut self, ctx: &egui::Context) {
        let facts: Vec<Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter(|tab| !matches!(tab.pane.body, Body::Draft(_)))
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                Some(Facts {
                    id: tab.id,
                    notes_file: agent.run.notes_file.as_deref()?,
                    worktree: tab
                        .pane
                        .worktree
                        .as_ref()
                        .map(|worktree| (worktree.repo.as_path(), worktree.name())),
                    profile_id: agent
                        .launched
                        .as_ref()
                        .map(|launched| launched.profile_id.as_str()),
                    session_id: agent
                        .launched
                        .as_ref()
                        .and_then(|launched| launched.session_id.as_deref()),
                })
            })
            .collect();
        self.progress.tick(&facts, ctx);
        let open: std::collections::HashSet<u64> = facts.iter().map(|facts| facts.id).collect();
        self.progress_ui.forget_except(|id| open.contains(&id));
    }

    /// The marks of the steps of a scheduled item, when its tab is open and its ledger lists them.
    pub(super) fn steps_of(&self, row: &Row) -> Vec<Mark> {
        self.open_tab_of(&row.repo, &row.tab)
            .and_then(|id| self.progress.ledger(id))
            .and_then(|known| known.ledger.scheduled.iter().find(|item| item.what == row.what))
            .map_or_else(Vec::new, |item| item.steps.iter().map(|step| step.mark).collect())
    }

    /// The items due in tab `id` that the Reader offers: open, dated today or before, not snoozed.
    pub(super) fn due_of(&self, id: u64) -> Vec<Due> {
        let tab = self.tabs.items().iter().find(|tab| tab.id == id);
        let (Some(worktree), Some(known)) =
            (tab.and_then(|tab| tab.pane.worktree.as_ref()), self.progress.ledger(id))
        else {
            return Vec::new();
        };
        let place = (worktree.repo.as_path(), worktree.name());
        due_items(today(), &known.ledger, place, &self.snoozes)
    }

    pub(super) fn apply_due_clicks(
        &mut self,
        tab: u64,
        clicks: Vec<due_view::Action>,
        ctx: &egui::Context,
        now: f64,
    ) {
        for click in clicks {
            match click {
                due_view::Action::Ask(text) => self.ask_to_start(tab, &text, ctx, now),
                due_view::Action::Snooze(what) => {
                    let key = self.due_key(tab, &what);
                    if let Some(key) = key {
                        self.snoozes.snooze(key, today());
                        self.show_toast("Snoozed until tomorrow", ctx);
                    }
                }
            }
        }
    }

    fn due_key(&self, tab: u64, what: &str) -> Option<Key> {
        let worktree = self.tabs.items().iter().find(|t| t.id == tab)?.pane.worktree.as_ref()?;
        Some(Key::new(&worktree.repo, worktree.name(), what))
    }

    /// Pastes the message and presses Enter, only while the agent is idle.
    fn ask_to_start(&mut self, tab: u64, text: &str, ctx: &egui::Context, now: f64) {
        let status = self
            .tabs
            .items()
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pane.agent.as_ref())
            .map(|agent| agent.run.status().clone());
        if let Some(reason) = ask_blocked(status.as_ref()) {
            self.show_toast(reason, ctx);
            return;
        }
        flow_glue::paste(&mut self.tabs, tab, text);
        self.flow.queue_enter(tab, now);
        ctx.request_repaint_after(Duration::from_secs_f64(ENTER_DELAY_SECS + WAKE_SLACK_S));
    }

    /// Tells the desktop about each item that turned due, once a day, while the window is away.
    pub(super) fn drive_due(&mut self, ctx: &egui::Context) {
        if self.progress.index().tabs.is_empty() || self.screenshot.is_some() {
            return;
        }
        let today = today();
        let rows = coming_up(today, self.progress.index());
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
        let away = notify::window_is_away(focused, minimized);
        let notifier = &mut self.notifier;
        let sent = tell_all(today, &rows, &self.snoozes, &mut self.told, |note| {
            notifier.offer_due(note, away)
        });
        if sent > 0 && self.status_log {
            eprintln!("tidewatch: due told {sent} at {}", wall_ms());
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        ctx.request_repaint_after(Duration::from_secs(secs_to_midnight(secs) + 1));
    }

    pub(super) fn add_looks(&self, overview: &mut Overview) {
        let places = overview.places.iter_mut().flat_map(|place| place.rows.iter_mut());
        for row in overview.needs.iter_mut().chain(places) {
            row.look = self.progress.ledger(row.tab_id).map(|known| look(&known.ledger));
        }
    }

    pub(super) fn drive_checklist(&mut self, ctx: &egui::Context, now: f64) {
        let shown = self.overview && self.settings.is_none() && self.welcome.is_none();
        let repo = self.workspace().dir().to_path_buf();
        self.checklist.tick(self.paths.as_ref(), &repo, shown, now, ctx);
    }

    pub(super) fn edit_checklist(&mut self, ctx: &egui::Context) {
        let repo = self.workspace().dir().to_path_buf();
        let (Some(paths), Some(list)) = (&self.paths, self.checklist.of(&repo)) else { return };
        let file = match list.source {
            Source::Repo => Ok(list.file.clone()),
            Source::Default | Source::Yours => tidewatch_core::checklist::ensure(paths),
        };
        match file {
            Ok(file) => open_in_editor(file),
            Err(e) => self.show_toast(format!("Could not create the checklist: {e}"), ctx),
        }
    }

    /// The Answer button: the Reader in front, its reply box asking for the keyboard.
    pub(super) fn open_reply_box(&mut self) {
        if let Some(agent) = self.tabs.active_pane_mut().agent.as_mut() {
            agent.reader.set_wants_reader(true);
            agent.reader.view.request_reply_focus();
        }
        self.with_active_view(crate::tab_view::TabView::back_to_terminal);
    }

    pub(super) fn progress_arrived(&mut self, tab: u64, ledger: Ledger, ctx: &egui::Context) {
        self.progress.arrived(tab, ledger, wall_ms() as i64);
        if self.status_log
            && let Some(known) = self.progress.ledger(tab)
        {
            let ledger = &known.ledger;
            let counts = ledger.counts();
            eprintln!(
                "tidewatch: progress tab={tab} state={} phase={:?} done={}/{} scheduled={} coming_up={} at {}",
                ledger.status.state.map_or("none", |state| state.word()),
                ledger.current_phase().map(|phase| phase.title.as_str()).unwrap_or_default(),
                counts.done,
                counts.total,
                ledger.scheduled.len(),
                coming_up(today(), self.progress.index()).len(),
                wall_ms()
            );
        }
        ctx.request_repaint();
    }
}
