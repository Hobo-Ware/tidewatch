//! The window's side of Message all agents: open the sheet from the palette, list the repo's agent
//! tabs in it, and send the one message to each ticked tab through the reply box's own path.

use super::Tidewatch;
use crate::agent_avatar::Face;
use crate::broadcast::{self, Candidate, Kind, Sheet};
use crate::broadcast_view::{self, Outcome};
use crate::pane::Pane;
use crate::reader_glue;

const DEV_OPEN_AFTER_S: f64 = 4.0;

fn kind_of(pane: &Pane<stdusk_core::terminal::PtyTerm>) -> Kind {
    match &pane.agent {
        Some(agent) if agent.meeseeks.is_some() => Kind::Meeseeks,
        Some(_) => Kind::Agent,
        None => Kind::Other,
    }
}

impl Tidewatch {
    pub(super) fn open_broadcast(&mut self) {
        self.broadcast = Some(Sheet::open(std::mem::take(&mut self.broadcast_draft)));
    }

    /// Closes the sheet and keeps its draft for the next open. False when it was not open.
    pub(super) fn close_broadcast(&mut self) -> bool {
        let Some(sheet) = self.broadcast.take() else { return false };
        self.broadcast_draft = sheet.draft;
        true
    }

    fn broadcast_candidates(&self) -> Vec<Candidate> {
        self.tabs
            .in_group(self.tabs.active_group())
            .map(|(_, tab)| Candidate {
                id: tab.id,
                name: crate::tab_title::label(tab.pane.agent.as_ref(), &tab.title),
                kind: kind_of(&tab.pane),
                status: tab
                    .pane
                    .agent
                    .as_ref()
                    .map_or(tidewatch_core::status::AgentStatus::Shell, |agent| {
                        agent.run.status().clone()
                    }),
                parked: tab.park.is_some(),
            })
            .collect()
    }

    fn drive_dev_broadcast(&mut self, ctx: &egui::Context) {
        if self.dev_broadcast.is_none() {
            return;
        }
        if ctx.input(|input| input.time) < DEV_OPEN_AFTER_S {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        } else if let Some(draft) = self.dev_broadcast.take() {
            self.broadcast_draft = draft;
            self.open_broadcast();
        }
    }

    pub(super) fn show_broadcast(&mut self, ctx: &egui::Context) {
        self.drive_dev_broadcast(ctx);
        let Some(mut sheet) = self.broadcast.take() else { return };
        let rows = sheet.rows(&self.broadcast_candidates());
        let faces: Vec<Option<Face>> = rows.iter().map(|row| self.tab_face(row.id)).collect();
        match broadcast_view::show(ctx, &mut sheet, &rows, &faces) {
            Some(Outcome::Close) => self.broadcast_draft = sheet.draft,
            Some(Outcome::Send(text)) => {
                let targets = broadcast::ticked(&rows);
                let now = ctx.input(|input| input.time);
                let went = reader_glue::reply_all(
                    &mut self.tabs,
                    &mut self.flow,
                    &targets,
                    &text,
                    (ctx, now),
                );
                self.show_toast(broadcast::sent_toast(went, targets.len()), ctx);
            }
            None => self.broadcast = Some(sheet),
        }
    }
}
