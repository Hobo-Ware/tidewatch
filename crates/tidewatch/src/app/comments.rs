//! The app's side of the comment guard: each agent tab's facts to the host, the notes it pastes to an
//! idle agent (the comment guard's, and the ledger guard's for a CLI with no stop hook), the strip
//! under the status line and its off switch.

use std::time::Duration;

use egui::{Rect, Ui};

use super::Tidewatch;
use crate::comment_glue::Facts;
use crate::comment_strip;
use crate::outbox::Send;
use crate::pane::Body;
use crate::{flow_glue, layout};

const WORKING_LOOK_S: f64 = 2.0;

impl Tidewatch {
    pub(super) fn drive_comments(&mut self, ctx: &egui::Context, now: f64) {
        let Some(paths) = &self.paths else { return };
        let mut working = false;
        let facts: Vec<Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let worktree = tab.pane.worktree.as_ref()?;
                if !matches!(tab.pane.body, Body::Shell(_)) {
                    return None;
                }
                working |=
                    matches!(agent.run.status(), tidewatch_core::status::AgentStatus::Working);
                Some(Facts {
                    id: tab.id,
                    agent_id: &agent.run.agent_id,
                    status: agent.run.status(),
                    dir: &worktree.path,
                    repo: &worktree.repo,
                    name: worktree.name(),
                    turn_started_at: agent.run.turn_started_at(),
                    used_tools: agent.run.used_tools_this_turn(),
                })
            })
            .collect();
        let pastes = self.comments.tick(&facts, paths, &self.guards, ctx, now);
        for paste in pastes {
            let send = Send::auto(&paste.text, paste.reason, paste.because);
            flow_glue::send_or_queue(&mut self.tabs, &mut self.flow, paste.tab, send, ctx, now);
        }
        if working {
            ctx.request_repaint_after(Duration::from_secs_f64(WORKING_LOOK_S));
        }
    }

    pub(super) fn show_comment_strip(&mut self, ui: &mut Ui, terminal: &mut Rect, active_id: u64) {
        let Some(view) = self.comments.view(active_id) else { return };
        let strip = comment_strip::Strip {
            records: view.records,
            roots: view.roots,
            off: view.off,
            dev_open_popover: self.comments.open_popover(),
        };
        let band = layout::band_above(terminal, comment_strip::HEIGHT);
        if let Some(comment_strip::Action::SetOff(off)) = comment_strip::show(ui, band, &strip) {
            self.comments.set_off(active_id, off);
        }
    }
}
