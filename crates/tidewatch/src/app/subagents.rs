//! The window's side of the running agents list: each Claude tab's transcript and whether its agent
//! works go to the host every frame, and the host says when it needs the next one.

use std::time::Duration;

use tidewatch_core::clock::now_ms;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::subagents::{dir_of, shown};
use tidewatch_core::transcript::Source;

use super::Tidewatch;
use super::frame::FrameIntent;
use crate::agent_avatar::Face;
use crate::reader::Shown;
use crate::reader_glue;
use crate::subagent_chat::{Open, Source as ChatSource};
use crate::subagent_chat_view::{self, Out};
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
        self.drive_dev_subagent_open(ctx);
    }

    fn reader_in_front(&mut self) -> bool {
        reader_glue::shown(self.tabs.active_pane_mut()) == Some(Shown::Reader)
    }

    fn drive_dev_subagent_open(&mut self, ctx: &egui::Context) {
        let Some(index) = self.subagent_chat.dev_open() else { return };
        let tab = self.tabs.items()[self.tabs.active()].id;
        let listed = shown(self.subagent_live.agents(tab), now_ms());
        let Some(id) = listed.get(index).map(|agent| agent.id.clone()) else { return };
        if self.reader_in_front() {
            self.subagent_chat.done_dev_open();
            self.open_subagent_chat(tab, &id, ctx);
        }
    }

    pub(super) fn open_subagent_chat(&mut self, tab: u64, id: &str, ctx: &egui::Context) {
        let Some(agent) = self.subagent_live.agents(tab).iter().find(|agent| agent.id == id) else {
            return;
        };
        let agent = agent.clone();
        let source = if self.subagent_live.is_fake() {
            ChatSource::Fake { finished: !agent.is_running() }
        } else {
            let transcript = self
                .tabs
                .items()
                .iter()
                .find(|item| item.id == tab)
                .and_then(|item| item.pane.agent.as_ref())
                .and_then(|agent| agent.reader.transcript());
            let Some(transcript) = transcript else { return };
            ChatSource::File(dir_of(transcript).join(format!("{id}.jsonl")))
        };
        self.subagent_chat.open(Open::new(tab, agent, source, ctx));
        ctx.request_repaint();
    }

    /// Draws the open sub-agent chat of the front tab in place of its Reader, and says whether it
    /// did.
    pub(super) fn show_subagent_chat(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        active_id: u64,
        frame: &FrameIntent,
    ) -> bool {
        let Some(id) = self.subagent_chat.id_over(active_id).map(str::to_owned) else {
            return false;
        };
        if !self.reader_in_front() {
            return false;
        }
        let Some(tab) = self.tabs.active_pane_mut().agent.as_ref() else { return false };
        let (name, agent_id) = (tab.profile_name.clone(), tab.run.agent_id.clone());
        let face = Face::of(&name, &agent_id, self.avatar_live.get(active_id).cloned());
        let found =
            self.subagent_live.agents(active_id).iter().find(|agent| agent.id == id).cloned();
        let now = now_ms();
        let Some(chat) = self.subagent_chat.chat_mut(active_id) else { return false };
        chat.drain();
        chat.refresh(found.as_ref(), now);
        let keys_free = !frame.keys.modal_open && !frame.keys.typing_in_a_panel;
        let facts =
            subagent_chat_view::Facts { chat, parent: (&name, &face), now_ms: now, keys_free };
        if subagent_chat_view::show(ui, area, facts) == Some(Out::Back) {
            self.subagent_chat.close();
            ui.ctx().request_repaint();
        }
        true
    }
}
