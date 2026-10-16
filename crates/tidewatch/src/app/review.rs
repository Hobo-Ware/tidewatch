//! The window's side of the review: what the Reader gets to draw of it, and carrying out what the
//! review card, the chips and the sent review's card ask for.

use tidewatch_core::notes::Notes;
use tidewatch_core::status::AgentStatus;

use super::Tidewatch;
use crate::flow_glue::unix_now;
use crate::flow_jobs::Which;
use crate::note_send::review_gate;
use crate::review::{self, Draft, ReviewFacts, SendState};
use crate::review_view::ReviewAction;

/// The owned half of [`ReviewFacts`]: what a frame works out about the active agent tab before it
/// borrows the tab and its notes for drawing.
pub(super) struct Parts {
    draft: Draft,
    send: SendState,
    agent_id: String,
    profile: String,
    working: bool,
}

impl Parts {
    pub(super) fn facts<'a>(&'a self, notes: &'a Notes, keys_free: bool) -> ReviewFacts<'a> {
        ReviewFacts {
            draft: &self.draft,
            send: self.send,
            notes,
            agent_id: &self.agent_id,
            profile: &self.profile,
            working: self.working,
            keys_free,
        }
    }
}

impl Tidewatch {
    /// The review of the active tab, when it is an agent tab whose notes are loaded.
    pub(super) fn review_parts(&self, active_id: u64) -> Option<Parts> {
        let agent = self.tabs.items()[self.tabs.active()].pane.agent.as_ref()?;
        self.flow.tab(active_id)?;
        let view = self.views.get(&active_id).filter(|view| view.notes_ready)?;
        let me = view.loaded.as_ref().map_or("", |loaded| loaded.author.as_str());
        let status = agent.run.status();
        Some(Parts {
            draft: Draft::of(&view.notes, &agent.run.agent_id, me),
            send: SendState::of(review_gate(Some(status)), self.flow.review_held(active_id)),
            agent_id: agent.run.agent_id.clone(),
            profile: agent.profile_name.clone(),
            working: *status == AgentStatus::Working,
        })
    }

    /// `Review: 3 notes` for the status bar, while the active tab has a draft review.
    pub(super) fn review_chip(&self, active_id: u64) -> Option<String> {
        let count = self.review_parts(active_id)?.draft.count();
        (count > 0).then(|| review::chip_text(count))
    }

    /// Brings the review into view: leaves the file view and shows the agent tab's Reader.
    pub(super) fn open_review(&mut self) {
        self.with_active_view(crate::tab_view::TabView::back_to_terminal);
        if let Some(agent) = self.tabs.active_pane_mut().agent.as_mut() {
            agent.reader.set_wants_reader(true);
        }
    }

    pub(super) fn apply_review(
        &mut self,
        actions: Vec<ReviewAction>,
        active_id: u64,
        ctx: &egui::Context,
    ) {
        let side = self.side_layout();
        for action in actions {
            match action {
                ReviewAction::Send(message) => {
                    let toast = self.flow.send(active_id, Which::Review(message), ctx, unix_now());
                    if let Some(text) = toast {
                        self.show_toast(text, ctx);
                    }
                }
                ReviewAction::CancelHeld => self.flow.release_review(active_id),
                ReviewAction::Discard(ids) => {
                    self.edit_notes(active_id, |notes| {
                        for id in &ids {
                            notes.delete(*id);
                        }
                    });
                }
                ReviewAction::ResolveAnswered(ids) => {
                    self.edit_notes(active_id, |notes| {
                        for id in &ids {
                            notes.set_resolved(*id, true);
                        }
                    });
                }
                ReviewAction::OpenFile(path) => {
                    self.with_active_view(|view| {
                        if side { view.open_file(&path) } else { view.select_file(&path) }
                    });
                }
            }
        }
        ctx.request_repaint();
    }

    fn edit_notes(&mut self, tab: u64, change: impl FnOnce(&mut Notes)) {
        if let Some(view) = self.views.get_mut(&tab) {
            change(&mut view.notes);
        }
        self.save_notes(tab);
    }
}
