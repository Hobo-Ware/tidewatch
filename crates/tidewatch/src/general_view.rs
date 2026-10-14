//! The General page: the Startup card (restore, open at login), the Window section and the Updates section.

use egui::Ui;

use crate::login_item;
use crate::macos::LoginItem;
use crate::settings_ui::{Control, footer_note, page_header, section, switch_row};
use crate::settings_view::Action;
use crate::updates::Updates;
use crate::updates_view;
use crate::window_settings;
use crate::window_view;

pub(crate) struct General {
    pub(crate) restore: bool,
    pub(crate) skipped_by_flag: bool,
    pub(crate) login_offered: bool,
    pub(crate) window: window_view::View,
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
    let mut open_at_login = None;
    section(ui, "Startup", |ui| {
        restore = switch_row(
            ui,
            "Restore the last session",
            "Tabs reopen where they were and agents pick up their conversation.",
            general.restore,
        );
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
    let updates = updates_view::section(ui, updates).map(Action::Updates);
    footer_note(ui, file);
    restore.map(Action::Restore).or(open_at_login.map(Action::LoginItem)).or(window).or(updates)
}
