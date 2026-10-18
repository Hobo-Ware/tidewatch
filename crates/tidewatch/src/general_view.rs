//! The General page: the Startup card (restore, open at login), the Window section and the Updates section.

use egui::Ui;
use tidewatch_core::prefs::{GithubPrefs, NewTabPrefs, ResumeAgents};

use crate::github_view;
use crate::login_item;
use crate::macos::LoginItem;
use crate::new_tab_prefs_view::{self, ProfileOption};
use crate::settings_ui::{self, Control, footer_note, page_header, section, switch_row};
use crate::settings_view::Action;
use crate::updates::Updates;
use crate::updates_view;
use crate::window_settings;
use crate::window_view;

pub(crate) struct General {
    pub(crate) restore: bool,
    pub(crate) resume: ResumeAgents,
    pub(crate) skipped_by_flag: bool,
    pub(crate) login_offered: bool,
    pub(crate) window: window_view::View,
    pub(crate) github: GithubPrefs,
    pub(crate) new_tab: NewTabPrefs,
    pub(crate) profiles: Vec<ProfileOption>,
}

pub(crate) fn page(
    ui: &mut Ui,
    general: &General,
    window: &mut window_settings::Editor,
    updates: &mut Updates,
    login: &mut login_item::Row,
    read_login: impl FnOnce() -> LoginItem,
    file: &str,
) -> Option<Action> {
    page_header(ui, "General", "Saved as you change them");
    let mut restore = None;
    let mut resume = None;
    let mut open_at_login = None;
    section(ui, "Startup", |ui| {
        restore = switch_row(
            ui,
            "Restore the last session",
            "Tabs reopen where they were and agents pick up their conversation.",
            general.restore,
        );
        resume = resume_row(ui, general.resume, general.restore);
        if general.login_offered {
            let now = ui.ctx().input(|input| input.time);
            open_at_login = login_item::show(ui, login, now, read_login);
        }
        if general.skipped_by_flag {
            crate::settings_ui::row(
                ui,
                "Skipped on this run",
                "This run was started with --no-restore, so nothing was restored.",
                Control::NONE,
                |_| {},
            );
        }
    });
    let window = window_view::section(ui, window, &general.window).map(Action::Window);
    let new_tab =
        new_tab_prefs_view::card(ui, &general.new_tab, &general.profiles).map(Action::NewTab);
    let github = github_view::card(ui, general.github).map(Action::Github);
    let updates = updates_view::section(ui, updates).map(Action::Updates);
    footer_note(ui, file);
    restore
        .map(Action::Restore)
        .or(resume.map(Action::ResumeAgents))
        .or(open_at_login.map(Action::LoginItem))
        .or(window)
        .or(new_tab)
        .or(github)
        .or(updates)
}

fn resume_row(ui: &mut Ui, current: ResumeAgents, enabled: bool) -> Option<ResumeAgents> {
    let options = [
        (ResumeAgents::Auto, "Resume"),
        (ResumeAgents::Prefill, "Type only"),
        (ResumeAgents::Off, "Off"),
    ];
    let labels = options.map(|(_, label)| label);
    let active = options.iter().position(|(mode, _)| *mode == current);
    settings_ui::row(
        ui,
        "Resume agents",
        "Resume runs the agent's command. Type only leaves Enter to you. After a crash it is always typed.",
        Control::new(settings_ui::inline_width(ui, &labels), 28.0),
        |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                settings_ui::pick(ui, "resume", &labels, active).map(|index| options[index].0)
            })
            .inner
        },
    )
}
