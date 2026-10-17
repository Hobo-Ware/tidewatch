//! The app's side of the progress ledgers: the agent tabs' facts go to the host each frame, and a
//! ledger an agent sends is stored and logged.

use tidewatch_core::progress::Ledger;

use super::{Tidewatch, wall_ms};
use crate::coming_up::coming_up;
use crate::pane::Body;
use crate::progress_glue::today;
use crate::progress_state::Facts;

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
