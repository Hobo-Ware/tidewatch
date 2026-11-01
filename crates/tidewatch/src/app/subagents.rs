//! The window's side of the running agents list: each Claude tab's transcript and whether its agent
//! works go to the host every frame, and the host says when it needs the next one.

use std::time::Duration;

use tidewatch_core::clock::now_ms;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::transcript::Source;

use super::Tidewatch;
use crate::subagents_glue::Facts;

impl Tidewatch {
    pub(super) fn drive_subagents(&mut self, ctx: &egui::Context) {
        let facts: Vec<Facts<'_>> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let claude = agent.reader.source() == Some(Source::Claude);
                Some(Facts {
                    id: tab.id,
                    transcript: agent.reader.transcript().filter(|_| claude),
                    working: *agent.run.status() == AgentStatus::Working,
                })
            })
            .collect();
        let now = now_ms();
        if let Some(at) = self.subagent_live.tick(&facts, ctx, now) {
            let wait = u64::try_from((at - now).max(0)).unwrap_or(0);
            ctx.request_repaint_after(Duration::from_millis(wait));
        }
    }
}
