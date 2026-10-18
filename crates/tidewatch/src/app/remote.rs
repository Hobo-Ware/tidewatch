//! What the Remote pill asks for: copying the link or the name, and turning remote control off for
//! the next start.

use super::Tidewatch;
use crate::remote_pill::Action;

impl Tidewatch {
    pub(super) fn apply_remote_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::CopyLink(link) => {
                ctx.copy_text(link);
                self.show_toast("Link copied", ctx);
            }
            Action::CopyName(name) => {
                ctx.copy_text(name);
                self.show_toast("Name copied", ctx);
            }
            Action::TurnOff => {
                let launched = self.tabs.active_pane_mut().agent.as_mut();
                if let Some(launched) = launched.and_then(|agent| agent.launched.as_mut()) {
                    launched.remote = false;
                }
            }
        }
    }
}
