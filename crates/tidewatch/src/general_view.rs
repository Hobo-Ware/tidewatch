//! The General page: the Startup card (restore, open at login), the Window, Terminal and Updates
//! sections.

use egui::Ui;
use tidewatch_core::guards::GuardPrefs;
use tidewatch_core::prefs::{
    DoneCheckPrefs, GithubPrefs, KeepAwakePrefs, NewTabPrefs, ParkAfter, ResumeAgents, TabsPrefs,
};

use crate::font_view;
use crate::github_view;
use crate::guards_view;
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
    pub(crate) tabs: TabsPrefs,
    pub(crate) done_check: DoneCheckPrefs,
    pub(crate) keep_awake: KeepAwakePrefs,
    pub(crate) guards: GuardPrefs,
    /// The repo on show, when it is one: a guard rule can be set for it alone.
    pub(crate) guards_repo: Option<guards_view::Repo>,
    pub(crate) profiles: Vec<ProfileOption>,
    /// The active repo's remote default branch, for the Start from label.
    pub(crate) default_branch: Option<String>,
    pub(crate) terminal: font_view::View,
}

pub(crate) fn page(
    ui: &mut Ui,
    general: &General,
    window: &mut window_settings::Editor,
    font: &mut font_view::Picker,
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
    let terminal = font_view::section(ui, font, &general.terminal).map(Action::TerminalFont);
    let new_tab = new_tab_prefs_view::card(
        ui,
        &general.new_tab,
        &general.profiles,
        general.default_branch.as_deref(),
    )
    .map(Action::NewTab);
    let tabs = tabs_card(ui, general.tabs).map(Action::Tabs);
    let done_check = done_check_card(ui, general.done_check.enabled).map(Action::DoneCheck);
    let keep_awake = keep_awake_card(ui, general.keep_awake.enabled).map(Action::KeepAwake);
    let github = github_view::card(ui, general.github).map(Action::Github);
    let guards =
        guards_view::card(ui, &general.guards, general.guards_repo.as_ref()).map(Action::Guards);
    let updates = updates_view::section(ui, updates).map(Action::Updates);
    footer_note(ui, file);
    restore
        .map(Action::Restore)
        .or(resume.map(Action::ResumeAgents))
        .or(open_at_login.map(Action::LoginItem))
        .or(window)
        .or(terminal)
        .or(new_tab)
        .or(tabs)
        .or(done_check)
        .or(keep_awake)
        .or(github)
        .or(guards)
        .or(updates)
}

fn tabs_card(ui: &mut Ui, current: TabsPrefs) -> Option<TabsPrefs> {
    let labels = ParkAfter::ALL.map(ParkAfter::label);
    let active = ParkAfter::ALL.iter().position(|after| *after == current.park_after);
    let mut picked = None;
    section(ui, "Tabs", |ui| {
        picked = settings_ui::row(
            ui,
            "Park quiet tabs after",
            "A tab nobody touched for this long leaves the strip and waits under Parked. It keeps running.",
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "park-after", &labels, active),
        );
    });
    picked.map(|index| TabsPrefs { park_after: ParkAfter::ALL[index] })
}

pub(crate) fn done_check_card(ui: &mut Ui, on: bool) -> Option<bool> {
    let mut changed = None;
    section(ui, "Done check", |ui| {
        changed = switch_row(
            ui,
            "Check the work when an agent says it is done",
            "Looks at whether the work is committed, pushed and its pull request is green, and shows Done, Blocked or Not verified on the tab. It never changes anything.",
            on,
        );
    });
    changed
}

pub(crate) fn keep_awake_card(ui: &mut Ui, on: bool) -> Option<bool> {
    let mut changed = None;
    section(ui, "Keep awake", |ui| {
        changed = switch_row(
            ui,
            "Keep the computer awake while agents work",
            "Stops it from going to sleep on its own while any agent is working, so a long turn is not frozen. The screen can still turn off and lock. It lets go two minutes after the last agent stops or waits for you.",
            on,
        );
    });
    changed
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
