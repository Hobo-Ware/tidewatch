//! Settings: the page, saving profiles and workflows, window preferences and shell events.

use tidewatch_core::house_style;
use tidewatch_core::prefs::WindowPrefs;

use super::{Tidewatch, wall_ms};
use crate::agent_jobs;
use crate::general_view::General;
use crate::login_item;
use crate::macos;
use crate::profiles_view::Probes;
use crate::settings::{Section, Settings};
use crate::settings_view::{self, Files, Host};
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
            Section::Trust | Section::General | Section::Shortcuts | Section::HouseStyle => {}
        }
    }

    fn settings_owns_escape(&self) -> bool {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        !self.trust.dialog_open()
            && !self.leftovers.is_open()
            && !self.flow.modal_open(active_id)
            && self.palette.is_none()
            && self.new_tab.is_none()
            && self.closing.is_none()
    }

    pub(super) fn show_settings_page(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
    ) -> Option<settings_view::Action> {
        let esc_closes = self.settings_owns_escape();
        let settings = self.settings.as_mut()?;
        let files = self
            .paths
            .as_ref()
            .map(|p| Files {
                profiles: p.profiles_file().display().to_string(),
                workflows: p.workflows_file().display().to_string(),
                trust: p.trust_file().display().to_string(),
                general: p.prefs_file().display().to_string(),
                house_style: p.house_style_file().display().to_string(),
            })
            .unwrap_or_default();
        let general = General {
            restore: self.restore.restore_enabled(),
            skipped_by_flag: self.restore.skipped_by_flag(),
            login_offered: crate::macos::LOGIN_ITEM_OFFERED,
            window: window_view::View {
                status: self.shell.hotkey_status(),
                menu_bar_available: self.shell.tray_supported(),
            },
        };
        let probes: Probes = self
            .catalog
            .agent_names()
            .into_iter()
            .map(|(id, _)| {
                let availability = self.catalog.availability(&id);
                (id, availability)
            })
            .collect();
        let trust = self.trust.page();
        let host = Host {
            files: &files,
            trust: &trust,
            general: &general,
            probes: &probes,
            read_login: crate::macos::login_item_status,
            esc_closes,
        };
        settings_view::show(ui, area, settings, &host, &mut self.updates)
    }

    fn apply_login_click(&mut self, click: login_item::Click, ctx: &egui::Context) {
        match click {
            login_item::Click::Set(on) => {
                if let Err(why) = macos::set_login_item(on) {
                    self.show_toast(format!("Open at login failed: {why}"), ctx);
                }
            }
            login_item::Click::Approve => macos::open_login_items_settings(),
        }
        if let Some(settings) = &mut self.settings {
            settings.login.read_again();
        }
    }

    fn open_house_style(&mut self, ctx: &egui::Context) {
        let Some(paths) = &self.paths else { return };
        match house_style::ensure(paths) {
            Ok(file) => open_in_editor(file),
            Err(e) => self.show_toast(format!("Could not create the house style: {e}"), ctx),
        }
    }

    fn reset_house_style(&mut self, ctx: &egui::Context) {
        let Some(paths) = &self.paths else { return };
        let text = match house_style::reset(paths) {
            Ok(_) => "House style reset to the default".to_owned(),
            Err(e) => format!("Could not reset the house style: {e}"),
        };
        self.show_toast(text, ctx);
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
            Some(settings_view::Action::LoginItem(click)) => self.apply_login_click(click, ctx),
            Some(settings_view::Action::OpenHouseStyle) => self.open_house_style(ctx),
            Some(settings_view::Action::ResetHouseStyle) => self.reset_house_style(ctx),
            None => {}
        }
    }
}

fn open_in_editor(file: std::path::PathBuf) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new(opener).arg(&file).status() {
            eprintln!("tidewatch: cannot open {}: {e}", file.display());
        }
    });
}
