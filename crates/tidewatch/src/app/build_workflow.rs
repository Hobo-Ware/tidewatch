//! Build workflow from this session in the app: pasting the prompt into the active agent tab,
//! waiting for the answer (the `propose_workflow` call, or a `toml` block in the Reader as the
//! fallback) and opening a valid proposal as an unsaved draft in Settings, Workflows.

use std::time::Duration;

use tidewatch_core::propose;
use tidewatch_core::workflows::Workflow;

use super::Tidewatch;
use crate::build_workflow::{self, Asked, Step, availability};
use crate::note_send::ENTER_DELAY_SECS;
use crate::pane::{Body, FirstPrompt};
use crate::settings::Section;

const WAKE_SLACK_S: f64 = 0.05;

impl Tidewatch {
    fn asking(&self, tab: u64) -> bool {
        self.build_asks.iter().any(|ask| ask.tab == tab)
    }

    /// Why the command cannot run on the active tab, `None` when it can.
    pub(super) fn build_workflow_blocked(&self) -> Option<&'static str> {
        let tab = self.tabs.items().get(self.tabs.active())?;
        availability(&tab.pane, self.asking(tab.id)).err().map(build_workflow::Blocked::reason)
    }

    pub(super) fn run_build_workflow(&mut self, ctx: &egui::Context) {
        if let Some(reason) = self.build_workflow_blocked() {
            self.show_toast(format!("Cannot build a workflow: {reason}"), ctx);
            return;
        }
        let now = ctx.input(|i| i.time);
        let active = self.tabs.active();
        let Some(tab) = self.tabs.items_mut().get_mut(active) else { return };
        let (id, title) = (tab.id, tab.title.clone());
        let (Body::Shell(term), Some(agent)) = (&mut tab.pane.body, tab.pane.agent.as_mut()) else {
            return;
        };
        let prompt = propose::prompt();
        term.paste(&prompt);
        agent.reader.tidewatch_said(&prompt, "Workflow");
        if agent.first_prompt == FirstPrompt::Awaiting {
            agent.first_prompt = FirstPrompt::Sent;
        }
        self.build_asks.push(Asked::new(id, agent.reader.rows.newest_seq(), now));
        self.flow.queue_enter(id, now);
        ctx.request_repaint_after(Duration::from_secs_f64(ENTER_DELAY_SECS + WAKE_SLACK_S));
        self.show_toast(format!("Asked {title} for a workflow"), ctx);
    }

    /// Opens `workflow`, proposed from the tab titled `from`, as a new draft that nothing has
    /// written yet.
    pub(super) fn open_proposal(&mut self, from: &str, workflow: &Workflow, ctx: &egui::Context) {
        if self.settings.is_none() {
            self.toggle_settings();
        }
        let Some(settings) = &mut self.settings else {
            self.show_toast("Tidewatch is still loading its settings", ctx);
            return;
        };
        self.overview = false;
        self.palette = None;
        settings.section = Section::Workflows;
        let name = settings.workflows.propose(workflow, from);
        self.show_toast(
            format!("\"{name}\" is a draft in Settings, Workflows. Save keeps it, Revert drops it"),
            ctx,
        );
    }

    /// The agent called `propose_workflow` from `tab`.
    pub(super) fn proposal_arrived(&mut self, tab: u64, workflow: &Workflow, ctx: &egui::Context) {
        self.build_asks.retain(|ask| ask.tab != tab);
        let from = self.tab_title(tab);
        self.open_proposal(&from, workflow, ctx);
    }

    fn tab_title(&self, tab: u64) -> String {
        self.tabs.items().iter().find(|t| t.id == tab).map_or_else(String::new, |t| t.title.clone())
    }

    /// Per frame: follow every request that is out until its agent answers, stops or runs out of
    /// time. A `toml` block in the Reader opens like a tool call would.
    pub(super) fn drive_build_workflow(&mut self, ctx: &egui::Context, now: f64) {
        for mut ask in std::mem::take(&mut self.build_asks) {
            let Some(tab) = self.tabs.items().iter().find(|tab| tab.id == ask.tab) else {
                continue;
            };
            let Some(agent) = &tab.pane.agent else { continue };
            let (title, agent_id) = (tab.title.clone(), agent.run.agent_id.clone());
            let mut step = ask.step(agent.run.status(), now);
            let mut reply = None;
            if step == Step::Scan {
                reply = build_workflow::toml_after(agent.reader.rows.iter(), ask.mark());
                if reply.is_none() {
                    step = ask.scanned_empty();
                }
            }
            if let Some(text) = reply {
                match build_workflow::workflow_from_toml(&text, &agent_id) {
                    Ok(workflow) => self.open_proposal(&title, &workflow, ctx),
                    Err(problem) => self.show_toast(
                        format!("{title} sent a workflow Tidewatch could not use: {problem}"),
                        ctx,
                    ),
                }
                continue;
            }
            match step {
                Step::Dropped(dropped) => self.show_toast(dropped.text(&title), ctx),
                Step::Wait(at) => {
                    if let Some(at) = at {
                        let wait = (at - now).max(0.0) + WAKE_SLACK_S;
                        ctx.request_repaint_after(Duration::from_secs_f64(wait));
                    }
                    self.build_asks.push(ask);
                }
                Step::Scan => self.build_asks.push(ask),
            }
        }
    }
}
