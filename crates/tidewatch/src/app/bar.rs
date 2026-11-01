//! The status bar panel: gathers what the bar shows from the window and draws it below the content.

use egui::{Frame, Panel};

use super::{Tidewatch, slow};
use crate::brand;
use crate::commands::Platform;
use crate::pane::Body;
use crate::reader::Shown;
use crate::status_bar::{self, Bar, Input, Screen, Workspace};
use crate::{agent_frame, reader_glue, reader_view};

impl Tidewatch {
    pub(super) fn show_status_bar(&mut self, ui: &mut egui::Ui, active_id: u64) {
        let bar = self.status_bar(ui.ctx(), active_id);
        let clicks = Panel::bottom("statusbar")
            .exact_size(status_bar::HEIGHT)
            .frame(Frame::new().fill(brand::SIDEBAR))
            .show(ui, |ui| status_bar::show(ui, ui.max_rect(), &bar))
            .inner;
        if clicks.open_review {
            self.open_review();
        }
        if clicks.cancel_switch
            && let Some((tab, _)) = self.switching_line()
        {
            self.cancel_switch(tab);
        }
        if let Some(id) = clicks.cancel_slow {
            slow::cancel(id);
        }
    }

    fn input_in_tab(&self) -> Option<Input> {
        if self.active_is_draft() {
            Some(Input::Form)
        } else if self.awaiting_first_prompt() {
            Some(Input::Prompt)
        } else {
            None
        }
    }

    fn status_bar(&self, ctx: &egui::Context, active_id: u64) -> Bar {
        let counts = status_bar::counts(&agent_frame::tones(&self.tabs));
        let pane = &self.tabs.items()[self.tabs.active()].pane;
        let mut review = None;
        let (screen, place) = if self.welcome.is_some() {
            (Screen::Welcome, None)
        } else if self.settings.is_some() {
            (Screen::Settings, None)
        } else if self.overview {
            (Screen::Overview, None)
        } else {
            let reader = reader_glue::shown(pane).map(|shown| shown == Shown::Reader);
            let replying = reader == Some(true) && reader_view::reply_has_focus(ctx);
            let workspace = Workspace {
                focus: self.focus,
                history: self.side_layout() && self.panes_showing(),
                file_open: self.file_view_showing(active_id),
                reader,
                input: self.input_in_tab().or(replying.then_some(Input::Prompt)),
            };
            review = self.review_chip(active_id);
            let place = match &pane.body {
                Body::Draft(form) => Some(status_bar::draft_place(form.kind())),
                _ => pane.agent.as_ref().and(pane.worktree.as_ref()).map(|worktree| {
                    let base = self.bases.get(self.workspace().dir());
                    status_bar::place(&worktree.branch, base)
                }),
            };
            (Screen::Workspace(workspace), place)
        };
        Bar {
            counts,
            place,
            review,
            note_hint: status_bar::note_hint(screen),
            hints: status_bar::hints(screen, Platform::CURRENT),
            awake: self.keep_awake.tip(),
            fetch_failed: self.fetch_failure().filter(|_| matches!(screen, Screen::Workspace(_))),
            pr: self.pr_bar_item(active_id).filter(|_| matches!(screen, Screen::Workspace(_))),
            switching: self.switching_line().map(|(_, text)| text),
            slow: slow::line(ctx),
            budget: self.budget_mark(active_id),
            meters: self.budget_meters(),
        }
    }
}
