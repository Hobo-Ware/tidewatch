//! Settings: the page, saving profiles and workflows, window preferences and shell events.

use tidewatch_core::house_style;
use tidewatch_core::prefs::{
    DoneCheckPrefs, KeepAwakePrefs, NewTabPrefs, Prefs, TabsPrefs, WindowPrefs,
};
use tidewatch_core::setup_recipe::{Recipe, Store};
use tidewatch_core::ui_zoom::{self, Direction};

use super::{Tidewatch, wall_ms};
use crate::agent_jobs;
use crate::font_view;
use crate::general_view::General;
use crate::guards_view;
use crate::login_item;
use crate::macos;
use crate::new_tab_prefs_view::ProfileOption;
use crate::profiles_view::Probes;
use crate::repo::Workspace;
use crate::settings::{Section, Settings};
use crate::settings_view::{self, Files, Host};
use crate::setup_editor::{Editor, RepoEntry};
use crate::shell::{self, Event};
use crate::sync_glue::wall_secs;
use crate::sync_view;
use crate::updates;
use crate::window_settings;
use crate::window_view;

impl Tidewatch {
    pub(super) fn toggle_settings(&mut self) {
        if self.settings.take().is_some() {
            return;
        }
        if let (Some(profiles), Some(workflows)) =
            (self.catalog.profiles(), self.catalog.workflows())
        {
            let mut settings =
                Settings::new(profiles, workflows, self.catalog.agent_names(), self.shell.prefs());
            let repos = self
                .workspaces
                .iter()
                .filter_map(|workspace| match workspace {
                    Workspace::Repo(repo) => {
                        Some(RepoEntry { name: repo.label.clone(), dir: repo.root.clone() })
                    }
                    Workspace::Other { .. } => None,
                })
                .collect();
            settings.setup = Editor::new(repos, self.paths.clone(), self.workspace().dir());
            settings.sync = sync_view::Editor::new(self.settings_sync.repo());
            self.settings = Some(settings);
        }
    }

    pub(super) fn open_updates_settings(&mut self) {
        if self.settings.is_none() {
            self.toggle_settings();
        }
        if let Some(settings) = &mut self.settings {
            settings.section = Section::General;
        }
    }

    pub(super) fn toggle_shortcuts_settings(&mut self) {
        if self.settings.as_ref().is_some_and(|settings| settings.section == Section::Shortcuts) {
            self.settings = None;
            return;
        }
        if self.settings.is_none() {
            self.toggle_settings();
        }
        if let Some(settings) = &mut self.settings {
            settings.section = Section::Shortcuts;
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
            Event::AdoptStdusk(socket) => self.adopt_from_stdusk(socket.clone(), ctx),
            Event::Settings | Event::Toggled => {}
        }
    }

    pub(super) fn step_zoom(&mut self, direction: Direction, ctx: &egui::Context) {
        let zoom = ui_zoom::step(self.shell.prefs().ui_zoom, direction);
        let prefs = WindowPrefs { ui_zoom: zoom, ..self.shell.prefs().clone() };
        if let Some(settings) = &mut self.settings {
            settings.window.set_ui_zoom(zoom);
        }
        self.set_window_prefs(prefs, ctx);
        self.show_toast(format!("Interface size {}", ui_zoom::label(zoom)), ctx);
    }

    fn set_window_prefs(&mut self, prefs: WindowPrefs, ctx: &egui::Context) {
        ctx.set_zoom_factor(ui_zoom::render_factor(prefs.ui_zoom));
        if let Some(paths) = &self.paths {
            shell::save(paths.clone(), prefs.clone());
        }
        self.shell.apply_prefs(ctx, prefs);
    }

    pub(super) fn drive_settings_sync(&mut self, ctx: &egui::Context, now: f64) {
        if self.settings_sync.drive(ctx, now) {
            self.reload_synced_settings(ctx);
        }
    }

    fn reload_synced_settings(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone() else { return };
        self.apply_pulled_prefs(Prefs::load(&paths), ctx);
        agent_jobs::reload(&self.agent_jobs, ctx, paths, self.catalog.path.clone());
    }

    fn apply_pulled_prefs(&mut self, prefs: Prefs, ctx: &egui::Context) {
        ctx.set_zoom_factor(prefs.window.ui_zoom);
        if let Some(settings) = &mut self.settings {
            settings.window = window_settings::Editor::new(&prefs.window);
        }
        self.shell.apply_prefs(ctx, prefs.window);
        self.restore.reload(prefs.restore_session, prefs.resume_agents);
        self.updates.reload(prefs.updates);
        self.new_tab_prefs = prefs.new_tab;
        self.parking.set_after(prefs.tabs.park_after);
        self.done_check.set_enabled(prefs.done_check.enabled);
        self.keep_awake.set_enabled(prefs.keep_awake.enabled);
        self.guards = prefs.guards;
        self.pr.set_default(prefs.github);
        self.terminal_prefs = prefs.terminal.clone();
        self.terminal_font.choose(ctx, None, prefs.terminal.font);
        ctx.request_repaint();
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
            Section::Trust
            | Section::General
            | Section::Shortcuts
            | Section::HouseStyle
            | Section::WorktreeSetup => {}
        }
    }

    fn settings_owns_escape(&self) -> bool {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        !self.trust.dialog_open()
            && !self.leftovers.is_open()
            && !self.updates.dialog_open()
            && !self.flow.modal_open(active_id)
            && self.palette.is_none()
            && self.closing.is_none()
    }

    fn set_tabs_prefs(&mut self, prefs: TabsPrefs) {
        self.parking.set_after(prefs.park_after);
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.tabs = prefs) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    fn set_done_check(&mut self, enabled: bool) {
        self.done_check.set_enabled(enabled);
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.done_check.enabled = enabled) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    fn set_copy_on_select(&mut self, enabled: bool) {
        self.terminal_prefs.copy_on_select = enabled;
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.terminal.copy_on_select = enabled) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    fn set_keep_awake(&mut self, enabled: bool) {
        self.keep_awake.set_enabled(enabled);
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.keep_awake.enabled = enabled) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    fn set_guards(&mut self, change: &guards_view::Change) {
        change.apply(&mut self.guards);
        if let Some(paths) = self.paths.clone() {
            let change = change.clone();
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| change.apply(&mut p.guards)) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    fn set_new_tab_prefs(&mut self, prefs: NewTabPrefs) {
        self.new_tab_prefs = prefs.clone();
        if let Some(paths) = self.paths.clone() {
            std::thread::spawn(move || {
                if let Err(e) = Prefs::update(&paths, |p| p.new_tab = prefs) {
                    eprintln!("tidewatch: cannot save settings.toml: {e}");
                }
            });
        }
    }

    pub(super) fn show_settings_page(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
    ) -> Option<settings_view::Action> {
        let esc_closes = self.settings_owns_escape();
        let default_branch =
            self.bases.facts(self.workspace().dir()).and_then(|facts| facts.remote_default.clone());
        let guards_repo = match self.workspace() {
            Workspace::Repo(repo) => {
                Some(guards_view::Repo { path: repo.root.clone(), label: repo.label.clone() })
            }
            Workspace::Other { .. } => None,
        };
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
                worktree_setup: p.worktree_setup_file().display().to_string(),
            })
            .unwrap_or_default();
        let general = General {
            restore: self.restore.restore_enabled(),
            resume: self.restore.resume_agents(),
            skipped_by_flag: self.restore.skipped_by_flag(),
            login_offered: crate::macos::LOGIN_ITEM_OFFERED,
            window: window_view::View {
                status: self.shell.hotkey_status(),
                menu_bar_available: self.shell.tray_supported(),
            },
            github: self.pr.default_prefs(),
            new_tab: self.new_tab_prefs.clone(),
            tabs: TabsPrefs { park_after: self.parking.after() },
            git: self.autofetch.prefs(),
            guards: self.guards.clone(),
            guards_repo,
            done_check: DoneCheckPrefs { enabled: self.done_check.enabled() },
            keep_awake: KeepAwakePrefs { enabled: self.keep_awake.enabled() },
            touch_id: self.amphetamine.touch_id(),
            default_branch,
            profiles: self
                .catalog
                .entries()
                .into_iter()
                .map(|(id, profile, agent)| ProfileOption {
                    id: id.to_owned(),
                    name: profile.name.clone(),
                    agent_id: agent.id.clone(),
                })
                .collect(),
            terminal: font_view::View {
                choice: self.terminal_font.choice().clone(),
                status: self.terminal_font.status(),
                families: crate::terminal_font::families(),
                copy_on_select: self.terminal_prefs.copy_on_select,
            },
            sync: self.settings_sync.view(wall_secs()),
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

    fn save_worktree_setup(&mut self, repo: std::path::PathBuf, recipe: Recipe) {
        let Some(paths) = self.paths.clone() else { return };
        std::thread::spawn(move || {
            if let Err(e) = Store::update(&paths, |store| store.set(&repo, &recipe)) {
                eprintln!("tidewatch: cannot save worktree-setup.toml: {e}");
            }
        });
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
            Some(settings_view::Action::ResumeAgents(mode)) => {
                if let Some(paths) = &self.paths {
                    self.restore.set_resume_agents(paths, mode);
                }
            }
            Some(settings_view::Action::Window(prefs)) => self.set_window_prefs(prefs, ctx),
            Some(settings_view::Action::Updates(prefs)) => {
                if let Some(paths) = &self.paths {
                    updates::save(paths.clone(), prefs);
                }
            }
            Some(settings_view::Action::NewTab(prefs)) => self.set_new_tab_prefs(prefs),
            Some(settings_view::Action::Tabs(prefs)) => self.set_tabs_prefs(prefs),
            Some(settings_view::Action::Git(prefs)) => self.set_git_prefs(prefs),
            Some(settings_view::Action::DoneCheck(on)) => self.set_done_check(on),
            Some(settings_view::Action::KeepAwake(on)) => self.set_keep_awake(on),
            Some(settings_view::Action::TouchId(click)) => {
                self.amphetamine.change_touch_id(click, ctx);
            }
            Some(settings_view::Action::Github(prefs)) => self.set_github_default(prefs),
            Some(settings_view::Action::Guards(change)) => self.set_guards(&change),
            Some(settings_view::Action::TerminalFont(choice)) => {
                self.terminal_font.choose(ctx, self.paths.as_ref(), choice);
            }
            Some(settings_view::Action::TerminalCopyOnSelect(on)) => self.set_copy_on_select(on),
            Some(settings_view::Action::LoginItem(click)) => self.apply_login_click(click, ctx),
            Some(settings_view::Action::Sync(change)) => self.settings_sync.apply(change, ctx),
            Some(settings_view::Action::OpenHouseStyle) => self.open_house_style(ctx),
            Some(settings_view::Action::ResetHouseStyle) => self.reset_house_style(ctx),
            Some(settings_view::Action::WorktreeSetup { repo, recipe }) => {
                self.save_worktree_setup(repo, recipe);
            }
            None => {}
        }
    }
}

pub(super) fn open_in_editor(file: std::path::PathBuf) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new(opener).arg(&file).status() {
            eprintln!("tidewatch: cannot open {}: {e}", file.display());
        }
    });
}
