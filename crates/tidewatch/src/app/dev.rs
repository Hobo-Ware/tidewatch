//! The `TIDEWATCH_DEV_*` hooks: start an agent, resume a run, press keys, drive the leftovers dialog and the panes.

use kelp_ui::graph::Selection;

use super::Tidewatch;
use crate::dev_agent::{DevDraft, Mode as DevMode};
use crate::dev_panes;
use crate::dev_session::LeftoverStep;
use crate::new_tab::Kind;
use crate::note_actions;
use crate::pane::{Body, FirstPrompt};
use crate::repo::Workspace;
use crate::restore_glue;
use crate::settings::Section;
use crate::{reader_glue, reader_view};

impl Tidewatch {
    /// Runs the `TIDEWATCH_DEV_AGENT` / `TIDEWATCH_DEV_DIALOG` switch once the profiles are loaded.
    pub(super) fn run_dev_agent(&mut self, ctx: &egui::Context) {
        let Some(dev) = self.dev_agent.take() else { return };
        if dev.mode == DevMode::Settings {
            self.toggle_settings();
            if let Some(settings) = &mut self.settings {
                match dev.profile.as_deref() {
                    Some("workflows") => settings.section = Section::Workflows,
                    Some("trust") => settings.section = Section::Trust,
                    Some("general") => settings.section = Section::General,
                    Some("shortcuts") => settings.section = Section::Shortcuts,
                    _ => {}
                }
            }
            return;
        }
        if dev.mode == DevMode::Palette {
            self.open_palette(dev.profile.as_deref().unwrap_or_default());
            return;
        }
        if dev.mode == DevMode::Resume {
            self.dev_resume(ctx, dev.profile.as_deref().unwrap_or_default());
            return;
        }
        let mut state = self.dialog_state(None);
        match dev.profile.as_deref() {
            None => {}
            Some("agent") => {
                state.choose(Kind::Agent, &self.catalog);
            }
            Some("workflow") => {
                state.choose(Kind::Workflow, &self.catalog);
            }
            Some(preselect) => state.jump(preselect, &self.catalog),
        }
        if let Some(level) = dev.level
            && !state.set_level(level, &self.catalog)
        {
            eprintln!("tidewatch: dev agent not started: {} is not open here", level.label());
            return;
        }
        match dev.mode {
            DevMode::Settings | DevMode::Palette | DevMode::Resume => {}
            DevMode::Dialog => {
                self.bases.refresh(ctx, state.repo());
                self.new_tab = Some(state);
            }
            DevMode::Start => match state.request(&self.catalog) {
                Ok(request) => {
                    self.open_tab(ctx, request);
                    if !dev.task.is_empty() {
                        self.dev_draft = Some(DevDraft { text: dev.task, start: dev.start });
                    }
                }
                Err(reason) => eprintln!("tidewatch: dev agent not started: {reason}"),
            },
        }
    }

    pub(super) fn drive_dev_draft(&mut self, ctx: &egui::Context, now: f64) {
        let Some(draft) = self.dev_draft.take() else { return };
        let waiting = self.tabs.items().iter().rev().find_map(|tab| {
            let agent = tab.pane.agent.as_ref()?;
            match agent.workflow.as_deref() {
                Some(run) => run.awaiting_task().then_some((tab.id, true)),
                None => (agent.first_prompt == FirstPrompt::Awaiting
                    && matches!(tab.pane.body, Body::Shell(_)))
                .then_some((tab.id, false)),
            }
        });
        let Some((id, is_run)) = waiting else {
            self.dev_draft = Some(draft);
            return;
        };
        let Some(agent) = self.tabs.get_mut(id).and_then(|tab| tab.pane.agent.as_mut()) else {
            return;
        };
        if let Some(run) = agent.workflow.as_deref_mut().filter(|_| is_run) {
            draft.text.clone_into(&mut run.ui.task);
            if draft.start {
                let execs = run.start_with(&draft.text);
                self.run_asks.push((id, execs));
            }
        } else {
            draft.text.clone_into(&mut agent.reader.view.reply);
            if draft.start {
                agent.reader.view.reply.clear();
                let asked = vec![reader_view::Action::Reply(draft.text)];
                reader_glue::apply(asked, &mut self.tabs, id, &mut self.flow, ctx, now);
            }
        }
    }

    pub(super) fn drive_dev_leftovers(&mut self, ctx: &egui::Context) {
        let Some(step) = self.dev_session.leftovers(self.leftovers.count() > 0) else { return };
        if !self.leftovers.is_open() {
            self.leftovers.open(ctx);
            self.dev_session.leftovers_opened();
        } else if step == LeftoverStep::Clean
            && self.leftovers.press_clean_up(ctx, self.paths.as_ref())
        {
            self.dev_session.leftovers_done();
        }
    }

    /// `TIDEWATCH_DEV_RESUME`: the restart path, for the one saved run in the named worktree.
    fn dev_resume(&mut self, ctx: &egui::Context, name: &str) {
        let (Workspace::Repo(repo), Some(paths)) = (self.workspace(), &self.paths) else { return };
        let plan = restore_glue::dev_resume_plan(paths, &repo.root, name);
        self.drive_restore(ctx, ctx.input(|i| i.time), Some(plan));
    }

    /// Applies the `TIDEWATCH_DEV_PICK` and `TIDEWATCH_DEV_NOTE` switches once the view can take
    /// them. Returns whether a note was added.
    pub(super) fn apply_dev_panes(&mut self, view_id: u64, now: i64) -> bool {
        let Some(view) = self.views.get_mut(&view_id) else { return false };
        if let Some(row) = self.dev_panes.pick_row
            && view.status_known()
            && let Some(loaded) = &view.loaded
        {
            self.dev_panes.pick_row = None;
            if row < loaded.history.len() {
                view.select(Selection::Commit(row));
            }
        }
        let Some(text) = &self.dev_panes.note else { return false };
        if !view.notes_ready || view.is_loading_source() {
            return false;
        }
        let Some(open) = &view.open else { return false };
        let Some(action) = dev_panes::note_on_first_added_line(&open.diff, text, now) else {
            return false;
        };
        let (path, commit) =
            (open.diff.path.clone(), view.wanted_commit().map(|id| id.to_string()));
        self.dev_panes.note = None;
        note_actions::apply(&mut view.notes, &path, commit.as_deref(), vec![action])
    }

    pub(super) fn send_dev_keys(&mut self, now: f64) {
        let Some(keys) = self.dev_keys.as_ref().filter(|keys| now >= keys.after) else { return };
        let target =
            self.tabs.items_mut().iter_mut().rev().filter(|tab| tab.pane.agent.is_some()).find_map(
                |tab| match &mut tab.pane.body {
                    Body::Shell(term) => Some(term),
                    _ => None,
                },
            );
        if let Some(term) = target {
            term.send(&keys.bytes);
            self.dev_keys = None;
        }
    }
}
