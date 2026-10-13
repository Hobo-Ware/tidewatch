//! Settings: the page, saving profiles and workflows, window preferences and shell events.

use tidewatch_core::prefs::WindowPrefs;

use super::{Tidewatch, wall_ms};
use crate::agent_jobs;
use crate::settings::{Section, Settings};
use crate::settings_view::{self, Files, General};
use crate::shell::{self, Event};
use crate::updates;
use crate::window_view;

impl Tidewatch {
    pub(super) fn toggle_settings(&mut self) {
        if self.settings.take().is_some() {
            return;
        }
        if let (Some(profiles), Some(workflows)) =
            (self.catalog.profiles(), self.catalog.workflows())
        {
            self.settings = Some(Settings::new(
                profiles,
                workflows,
                self.catalog.agent_names(),
                self.shell.prefs(),
            ));
        }
    }

    /// What another launch, the hotkey or the menu-bar item asked of the app itself.
    pub(super) fn shell_event(&mut self, event: &Event, ctx: &egui::Context) {
        if self.status_log {
            eprintln!("tidewatch: shell {} at {}", shell::describe(event), wall_ms());
        }
        match event {
            Event::Overview => {
                self.overview = true;
                self.settings = None;
            }
            Event::Settings if self.settings.is_none() => self.toggle_settings(),
            Event::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Event::Opened(path) => self.open_workspace(path.clone(), ctx),
            Event::Settings | Event::Toggled => {}
        }
    }

    fn set_window_prefs(&mut self, prefs: WindowPrefs, ctx: &egui::Context) {
        if let Some(paths) = &self.paths {
            shell::save(paths.clone(), prefs.clone());
        }
        self.shell.apply_prefs(ctx, prefs);
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
        let (Some(settings), Some(paths)) = (&mut self.settings, &self.paths) else { return };
        match settings.section {
            Section::Profiles => match settings.profiles.build() {
                Ok(profiles) => {
                    settings.profiles.saving = true;
                    agent_jobs::save(&self.agent_jobs, ctx, paths.clone(), profiles);
                }
                Err(message) => settings.profiles.error = Some(message),
            },
            Section::Workflows => match settings.workflows.build() {
                Ok(workflows) => {
                    settings.workflows.saving = true;
                    agent_jobs::save_workflows(&self.agent_jobs, ctx, paths.clone(), workflows);
                }
                Err(message) => settings.workflows.error = Some(message),
            },
            Section::Trust | Section::General | Section::Shortcuts => {}
        }
    }

    pub(super) fn show_settings_page(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
    ) -> Option<settings_view::Action> {
        let settings = self.settings.as_mut()?;
        let files = self.paths.as_ref().map_or_else(
            || Files {
                profiles: String::new(),
                workflows: String::new(),
                trust: String::new(),
                general: String::new(),
            },
            |p| Files {
                profiles: p.profiles_file().display().to_string(),
                workflows: p.workflows_file().display().to_string(),
                trust: p.trust_file().display().to_string(),
                general: p.prefs_file().display().to_string(),
            },
        );
        let general = General {
            restore: self.restore.restore_enabled(),
            skipped_by_flag: self.restore.skipped_by_flag(),
            window: window_view::View {
                status: self.shell.hotkey_status(),
                menu_bar_available: self.shell.tray_supported(),
            },
        };
        settings_view::show(
            ui,
            area,
            settings,
            &files,
            &self.trust.page(),
            &general,
            &mut self.updates,
        )
    }

    pub(super) fn apply_settings_action(
        &mut self,
        action: Option<settings_view::Action>,
        ctx: &egui::Context,
    ) {
        match action {
            Some(settings_view::Action::Close) => self.settings = None,
            Some(settings_view::Action::Save) => self.save_settings(ctx),
            Some(settings_view::Action::Trust(event)) => self.trust.send(event, ctx),
            Some(settings_view::Action::Restore(on)) => {
                if let Some(paths) = &self.paths {
                    self.restore.set_restore_enabled(paths, on);
                }
            }
            Some(settings_view::Action::Window(prefs)) => self.set_window_prefs(prefs, ctx),
            Some(settings_view::Action::Updates(prefs)) => {
                if let Some(paths) = &self.paths {
                    updates::save(paths.clone(), prefs);
                }
            }
            None => {}
        }
    }
}
