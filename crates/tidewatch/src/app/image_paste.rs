//! The window's side of a picture on the clipboard: a Cmd+V the key monitor took from egui becomes
//! a job, and what it resolves to goes to the tab that was in front.

use tidewatch_core::image_paste::{Target, ctrl_v};

use super::frame::FrameIntent;
use super::{Tidewatch, wall_ms};
use crate::image_paste::{Delivery, Done};
use crate::pane::Body;
use crate::reader::Shown;
use crate::reader_glue;
use stdusk_core::paste::prepare;
use stdusk_egui::terminal_view::paste_into;

use super::terminal::PASTE_RULES;

const PASTES_DIR: &str = "pastes";

pub(crate) fn appended(reply: &str, text: &str) -> String {
    if reply.is_empty() || reply.ends_with(char::is_whitespace) {
        format!("{reply}{text}")
    } else {
        format!("{reply} {text}")
    }
}

impl Tidewatch {
    pub(super) fn drive_image_paste(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        intent: &FrameIntent,
    ) {
        self.image_paste.tick_dev(now, ctx);
        for done in self.image_paste.drain() {
            self.deliver_picture(done, ctx, intent.active_id);
        }
        if self.image_paste.take_requests() > 0 {
            self.start_picture_paste(ctx, intent);
        }
    }

    fn start_picture_paste(&mut self, ctx: &egui::Context, intent: &FrameIntent) {
        let screen_free = !self.overview && self.settings.is_none() && self.welcome.is_none();
        let typing = intent.keys.typing_in_a_panel;
        if !typing && (intent.keys.modal_open || !screen_free) {
            return;
        }
        let pane = self.tabs.active_pane_mut();
        let reader_shown = matches!(reader_glue::shown(pane), Some(Shown::Reader));
        let agent_id = pane.agent.as_ref().map(|agent| agent.run.agent_id.clone());
        let tab_dir = pane
            .agent
            .as_ref()
            .and_then(|agent| agent.run.notes_file.as_deref())
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let is_shell = matches!(pane.body, Body::Shell(_));
        let target = if typing || reader_shown {
            Target::TextField
        } else if is_shell {
            let wiring = agent_id.and_then(|id| self.catalog.agent(&id)).map(|agent| agent.wiring);
            Target::Terminal(wiring)
        } else {
            return;
        };
        let state = self.paths.as_ref().map(tidewatch_core::paths::TidewatchPaths::state_dir);
        let Some(dir) = tab_dir.or(state).map(|dir| dir.join(PASTES_DIR)) else {
            self.show_toast("No place to save the picture", ctx);
            return;
        };
        self.image_paste.start(ctx, (intent.active_id, target), dir, wall_ms());
    }

    fn deliver_picture(&mut self, done: Done, ctx: &egui::Context, active_id: u64) {
        let Done { tab, target, delivery } = done;
        match delivery {
            Delivery::Nothing => {}
            Delivery::Failed(message) => self.show_toast(message, ctx),
            Delivery::CtrlV => {
                if let Some(Body::Shell(term)) = self.tab_body_mut(tab) {
                    term.send(ctrl_v());
                }
            }
            Delivery::Text(text) => match target {
                Target::Terminal(_) => {
                    if let (Some(Body::Shell(term)), Some(text)) =
                        (self.tab_body_mut(tab), prepare(&text, PASTE_RULES))
                    {
                        paste_into(term, &text);
                    }
                }
                Target::TextField => self.insert_into_field(ctx, tab, active_id, text),
            },
        }
    }

    fn insert_into_field(&mut self, ctx: &egui::Context, tab: u64, active_id: u64, text: String) {
        if tab == active_id && ctx.text_edit_focused() {
            crate::image_paste::inject_paste(ctx, text);
            return;
        }
        let agent = self.tabs.get_mut(tab).and_then(|tab| tab.pane.agent.as_mut());
        if let Some(agent) = agent {
            agent.reader.view.reply = appended(&agent.reader.view.reply, &text);
            agent.reader.view.request_reply_focus();
            ctx.request_repaint();
        }
    }

    fn tab_body_mut(&mut self, tab: u64) -> Option<&mut Body<stdusk_core::terminal::PtyTerm>> {
        self.tabs.get_mut(tab).map(|tab| &mut tab.pane.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_added_after_the_text_already_typed() {
        let cases = [
            ("", "/s/shot-1.png", "/s/shot-1.png"),
            ("look at ", "/s/shot-1.png", "look at /s/shot-1.png"),
            ("look at", "/s/shot-1.png", "look at /s/shot-1.png"),
            ("a\n", "/s/shot-1.png", "a\n/s/shot-1.png"),
        ];
        for (reply, text, want) in cases {
            assert_eq!(appended(reply, text), want, "{reply:?}");
        }
    }
}
