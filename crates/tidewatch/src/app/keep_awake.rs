//! The window's side of keeping awake: the working agents of every repo, every frame, to the host.

use std::time::Duration;

use super::{Tidewatch, WAKE_SLACK_S, wall_ms};
use tidewatch_core::job::JobBoard;
use tidewatch_core::keep_awake::keeps_awake;

use crate::keep_awake_glue::Change;

impl Tidewatch {
    pub(super) fn drive_keep_awake(&mut self, ctx: &egui::Context, now: f64) {
        let working = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                let agent = tab.pane.agent.as_ref()?;
                let job_running = self.job_live.board(tab.id).is_some_and(JobBoard::has_running);
                Some(keeps_awake(agent.run.status(), job_running))
            })
            .filter(|keeps| *keeps)
            .count();
        let (change, wake) = self.keep_awake.tick(working, now);
        if let Some(change) = change {
            if self.status_log {
                let word = match change {
                    Change::Took => "take",
                    Change::Released => "release",
                };
                eprintln!("tidewatch: keep-awake {word} working={working} at {}", wall_ms());
            }
            ctx.request_repaint();
        }
        if let Some(at) = wake {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
    }
}
