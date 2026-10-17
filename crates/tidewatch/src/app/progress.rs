//! The app's side of the progress ledgers: the agent tabs' facts go to the host each frame, and a
//! ledger an agent sends is stored and logged.

use tidewatch_core::progress::Ledger;

use super::settings::open_in_editor;
use super::{Tidewatch, wall_ms};
use crate::coming_up::{Row, coming_up};
use crate::overview::Overview;
use crate::overview_checklist::Source;
use crate::pane::Body;
use crate::progress_glue::today;
use crate::progress_model::look;
use crate::progress_state::Facts;
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
